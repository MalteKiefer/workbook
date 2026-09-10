//! Tauri commands for the Atera plugin integration (see `plugin::atera` and
//! `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers following exactly the same
//! pattern as `commands::tacticalrmm` -- an Atera "connection" is a
//! user-created record (just an API key, no base URL -- Atera is a fixed
//! SaaS host, see `plugin::atera` module docs) for exactly one Atera
//! account, NOT for exactly one local customer. A single Atera account can
//! host multiple Atera "Customers" (e.g. because the user setting up the
//! connection is themselves an MSP running several of their own customers
//! as separate Atera Customers). Which Atera Customer corresponds to which
//! local customer (if any) is a separate, granular mapping
//! (`AteraCustomerMapping`/`Config::atera_customer_mappings`), maintained by
//! this module via `map_atera_customer`/`unmap_atera_customer`. The fully
//! qualified identifier `"atera:<connection_id>"` serves both as the
//! keyring account (`plugin::secrets`) and as `external_refs.plugin_id`, so
//! the existing one-row-per-(system_id,plugin_id) upsert semantics keep
//! working unchanged.
//!
//! IMPORTANT, verified difference from `commands::tacticalrmm`'s
//! `group_agents_by_client`: Atera's agent list carries a REAL numeric
//! `CustomerID` foreign key (see `plugin::atera` module docs) -- unlike
//! Tactical RMM's name-only join. `group_agents_by_customer` below
//! therefore joins agents to customers by ID, exactly like
//! `commands::plugins::group_devices_by_organization` (NinjaOne), NOT by
//! name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::atera::{
    test_credentials, AteraAgent, AteraCustomer, AteraCustomerMapping, AteraConnectionMeta,
    AteraPlugin,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AteraConnectionDto {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    /// `"online"`/`"offline"`, passed through verbatim from Atera -- see
    /// `plugin::atera` module docs.
    pub status: Option<String>,
    /// Atera's free-form `OS` display string, passed through verbatim.
    pub platform: Option<String>,
    /// Direct link to this agent in Atera's web UI, if Atera's own API
    /// provided one -- `None` if absent, never hand-constructed (see
    /// `plugin::atera` module docs).
    pub view_url: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// agents of an unmapped Atera Customer -- without a local
    /// `customer_id` there's no meaningful way to cross-reference against
    /// `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AteraCustomerDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this Atera Customer hasn't been mapped to a local
    /// customer yet (`Config::atera_customer_mappings` has no matching row
    /// for this connection+customer).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AteraCustomerDeviceGroupDto {
    /// The Atera Customer's own id/name -- prefixed `atera_` throughout this
    /// module to keep it visually distinct from `customer_id` below, which
    /// (as everywhere else in this codebase) always means the LOCAL
    /// `Customer` entity.
    pub atera_customer_id: String,
    pub atera_customer_name: String,
    /// `None` if this Atera Customer isn't mapped to a local customer (yet)
    /// -- in this case the frontend shows "not mapped" and disables linking
    /// this group's agents.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_atera_connection` run, cached under
/// `data_dir/plugin-cache/atera-<connection_id>.json` (see
/// `write_atera_cache`/`read_atera_cache`), so `get_cached_atera_sync` works
/// without network access -- exactly the same pattern as
/// `commands::tacticalrmm::CachedTacticalRmmSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedAteraSyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<AteraCustomerDeviceGroupDto>,
}

fn to_dto(meta: &AteraConnectionMeta) -> AteraConnectionDto {
    AteraConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
    }
}

/// The fully qualified `plugin_id` value for an Atera connection -- both the
/// keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("atera:{connection_id}")
}

/// Generates a stable, low-collision connection ID from a user label,
/// following exactly the pattern of
/// `commands::tacticalrmm::generate_connection_id`.
fn generate_connection_id(label: &str) -> String {
    let slug = slugify(label);
    let now_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{slug}-{now_millis}")
}

fn slugify(label: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("atera");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<AteraConnectionMeta, AppError> {
    config
        .atera_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("Atera-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Builds the runnable plugin object from a connection metadata row, plus
/// the associated credentials (the API key) from the keyring. Like Tactical
/// RMM/Level.io/Snipe-IT, the Atera secret is already the complete secret
/// string -- no JSON encoding needed (unlike Ninja, which needs two
/// values).
fn build_plugin(meta: &AteraConnectionMeta) -> Result<(AteraPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Atera-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((AteraPlugin::new(plugin_id), PluginCredentials { secret }))
}

/// Best-effort, analogous to
/// `commands::tacticalrmm::delete_keyring_secret_best_effort`.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The same `data_dir/plugin-cache/` directory as `commands::plugins`/
/// `commands::level`/`commands::tacticalrmm` -- one shared folder for all
/// plugin cache files, just with a different filename prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn atera_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("atera-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::tacticalrmm::write_tacticalrmm_cache`.
fn write_atera_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[AteraCustomerDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedAteraSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Atera-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(atera_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_atera_cache`.
/// `Ok(None)` if this connection has never been synced -- not an error
/// case, analogous to `commands::tacticalrmm::read_tacticalrmm_cache`.
fn read_atera_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedAteraSyncDto>, AppError> {
    let path = atera_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedAteraSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Atera-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(agent: AteraAgent, linked_system_id: Option<i64>) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: agent.external_id,
        name: agent.name,
        hostname: agent.hostname,
        ip_address: agent.ip_address,
        status: agent.status,
        platform: agent.platform,
        view_url: agent.view_url,
        linked_system_id,
    }
}

/// Intermediate result of the pure grouping logic: an Atera Customer along
/// with its agents (still as `AteraAgent`, not as a DTO) and -- if mapped
/// -- the local `customer_id`. Analogous to
/// `commands::plugins::OrgGroup`/`commands::tacticalrmm::ClientGroup`.
struct CustomerGroup {
    atera_customer_id: String,
    atera_customer_name: String,
    customer_id: Option<i64>,
    devices: Vec<AteraAgent>,
}

/// Groups agents by Atera Customer and enriches each group with the
/// configured local `customer_id` mapping (if any). A pure function -- no
/// network, no database access -- so it's testable with hardcoded
/// `AteraCustomer`/`AteraAgent`/`AteraCustomerMapping` values. UNLIKE
/// `commands::tacticalrmm::group_agents_by_client`, this joins agents to
/// their Atera Customer by a real numeric ID
/// (`agent.customer_id == customer.id`), not by name -- Atera's agent list
/// API carries a genuine `CustomerID` foreign key (see `plugin::atera`
/// module docs), so this function is structurally identical to
/// `commands::plugins::group_devices_by_organization` (NinjaOne), just
/// renamed for Atera's vocabulary. A Customer with no agents at all still
/// shows up as a group (empty `devices` list) so the UI can display it for
/// mapping. Agents whose `customer_id` doesn't match any customer reported
/// by `customers` (shouldn't happen per Atera's data model, but not
/// impossible -- e.g. a customer deleted between fetching customers and
/// agents in the same sync run) are not silently dropped, but appended as
/// their own leftover group, keyed by the raw ID (used as both the
/// synthetic display name and the id, since no name is known for it),
/// exactly like `plugin::ninja`'s leftover-organization handling.
fn group_agents_by_customer(
    customers: &[AteraCustomer],
    agents: &[AteraAgent],
    mappings: &[AteraCustomerMapping],
    connection_id: &str,
) -> Vec<CustomerGroup> {
    let mapped_local_customer_id = |atera_customer_id: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.customer_id == atera_customer_id)
            .map(|m| m.local_customer_id)
    };

    let mut agents_by_customer_id: HashMap<String, Vec<AteraAgent>> = HashMap::new();
    for agent in agents {
        agents_by_customer_id
            .entry(agent.customer_id.clone())
            .or_default()
            .push(agent.clone());
    }

    let mut groups = Vec::with_capacity(customers.len());
    for customer in customers {
        let devices = agents_by_customer_id
            .remove(&customer.id)
            .unwrap_or_default();
        groups.push(CustomerGroup {
            atera_customer_id: customer.id.clone(),
            atera_customer_name: customer.name.clone(),
            customer_id: mapped_local_customer_id(&customer.id),
            devices,
        });
    }

    let mut leftover_ids: Vec<String> = agents_by_customer_id.keys().cloned().collect();
    leftover_ids.sort();
    for id in leftover_ids {
        if let Some(devices) = agents_by_customer_id.remove(&id) {
            groups.push(CustomerGroup {
                customer_id: mapped_local_customer_id(&id),
                atera_customer_name: id.clone(),
                atera_customer_id: id,
                devices,
            });
        }
    }

    groups
}

#[tauri::command]
pub fn test_atera_connection(api_key: String) -> Result<(), AppError> {
    test_credentials(&api_key)?;
    Ok(())
}

#[tauri::command]
pub fn list_atera_connections(state: State<AppState>) -> Result<Vec<AteraConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.atera_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_atera_connection(
    state: State<AppState>,
    label: String,
    api_key: String,
) -> Result<AteraConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    plugin::secrets::store_secret(&plugin_id, &api_key)?;

    let meta = AteraConnectionMeta { id, label };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.atera_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_atera_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.atera_connections.len();
    config.atera_connections.retain(|c| c.id != id);
    if config.atera_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Atera-Verbindung {id} nicht gefunden"
        )));
    }
    // Cleanup: customer mappings for this connection are meaningless
    // without the connection and would otherwise be left behind as orphaned
    // data.
    config
        .atera_customer_mappings
        .retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = atera_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Atera-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

/// Live fetch of a connection's Atera Customer list. Analogous to
/// `commands::tacticalrmm::list_tacticalrmm_clients`, kept for symmetry and
/// a possible initial-setup use case -- normal frontend operation doesn't
/// need this command (cache-first, see
/// `get_cached_atera_sync`/`sync_atera_connection`).
#[tauri::command]
pub fn list_atera_customers(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<AteraCustomerDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<AteraCustomerMapping> = config
            .atera_customer_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings)
    };

    let customers = plugin.list_customers(&credentials)?;
    Ok(customers
        .into_iter()
        .map(|customer| {
            let mapped_customer_id = mappings
                .iter()
                .find(|m| m.customer_id == customer.id)
                .map(|m| m.local_customer_id);
            AteraCustomerDto {
                id: customer.id,
                name: customer.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_atera_customer(
    state: State<AppState>,
    connection_id: String,
    customer_id: String,
    customer_name: String,
    local_customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .atera_customer_mappings
        .retain(|m| !(m.connection_id == connection_id && m.customer_id == customer_id));
    config.atera_customer_mappings.push(AteraCustomerMapping {
        connection_id,
        customer_id,
        customer_name,
        local_customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_atera_customer(
    state: State<AppState>,
    connection_id: String,
    customer_id: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Not an error if no matching mapping exists -- the result (no mapping
    // left) is the same, analogous to `db::external_refs::delete`. Existing
    // `external_refs` links are left untouched: unmapping a customer is
    // deliberately not an automatic unlinking of its already-linked agents.
    config
        .atera_customer_mappings
        .retain(|m| !(m.connection_id == connection_id && m.customer_id == customer_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_atera_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<AteraCustomerDeviceGroupDto>, AppError> {
    let (plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<AteraCustomerMapping> = config
            .atera_customer_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings, config.data_dir.clone())
    };

    let customers = plugin.list_customers(&credentials)?;
    let agents = plugin.list_agents(&credentials)?;
    let groups = group_agents_by_customer(&customers, &agents, &mappings, &connection_id);

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, built ONCE from ALL
    // external_refs of this plugin -- exactly the pattern
    // `commands::tacticalrmm::sync_tacticalrmm_connection` uses: a device
    // stays "linked" in the UI even if its customer mapping was corrected
    // to a different local customer AFTER the link was made, because
    // `db::external_refs::list_for_plugin` searches independent of the
    // currently-mapped customer.
    let linked_by_external_id: HashMap<String, i64> =
        db::external_refs::list_for_plugin(&conn, &plugin_id)?
            .into_iter()
            .map(|reference| (reference.external_id, reference.system_id))
            .collect();

    let mut result = Vec::with_capacity(groups.len());
    for group in groups {
        let mut device_dtos = Vec::with_capacity(group.devices.len());
        for device in group.devices {
            let linked_system_id = linked_by_external_id.get(&device.external_id).copied();
            if let Some(system_id) = linked_system_id {
                let payload = plugin.get_system_details(&credentials, &device.external_id)?;
                db::external_refs::upsert(
                    &conn,
                    system_id,
                    &plugin_id,
                    &device.external_id,
                    &payload.to_string(),
                    &tz,
                )?;
            }
            device_dtos.push(to_external_system_dto(device, linked_system_id));
        }

        result.push(AteraCustomerDeviceGroupDto {
            atera_customer_id: group.atera_customer_id,
            atera_customer_name: group.atera_customer_name,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_atera_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_atera_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedAteraSyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<AteraCustomerMapping> = config
            .atera_customer_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_atera_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-resolved here against the CURRENT
    // customer mappings instead of using the value frozen into the cache
    // file during the last `sync_atera_connection` run -- otherwise mapping
    // or unmapping an Atera Customer would only become visible after the
    // next live sync, even though this exact command is meant to show the
    // frontend the current mapping state without network access (analogous
    // to `commands::tacticalrmm::get_cached_tacticalrmm_sync`).
    if let Some(cache) = cached.as_mut() {
        for group in &mut cache.groups {
            group.customer_id = mappings
                .iter()
                .find(|m| m.customer_id == group.atera_customer_id)
                .map(|m| m.local_customer_id);
        }
    }
    Ok(cached)
}

#[tauri::command]
pub fn link_system_to_atera(
    state: State<AppState>,
    system_id: i64,
    connection_id: String,
    external_id: String,
) -> Result<(), AppError> {
    let (plugin, credentials) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        build_plugin(&meta)?
    };

    let payload = plugin.get_system_details(&credentials, &external_id)?;
    plugin.link_system(system_id, &external_id)?;

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    db::external_refs::upsert(
        &conn,
        system_id,
        plugin.id(),
        &external_id,
        &payload.to_string(),
        &tz,
    )?;
    Ok(())
}

#[tauri::command]
pub fn unlink_system_from_atera(
    state: State<AppState>,
    system_id: i64,
    connection_id: String,
) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    db::external_refs::delete(&conn, system_id, &plugin_id_for(&connection_id))
}

#[tauri::command]
pub fn get_atera_system_details(
    state: State<AppState>,
    connection_id: String,
    external_id: String,
) -> Result<serde_json::Value, AppError> {
    let (plugin, credentials) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        build_plugin(&meta)?
    };
    Ok(plugin.get_system_details(&credentials, &external_id)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("ACME  Kunde GmbH!!"), "acme-kunde-gmbh");
    }

    #[test]
    fn slugify_falls_back_when_label_has_no_alphanumerics() {
        assert_eq!(slugify("***"), "atera");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_atera_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "atera:acme-123");
    }

    #[test]
    fn find_connection_returns_not_found_for_unknown_id() {
        let config = Config::default();
        let result = find_connection(&config, "does-not-exist");
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[test]
    fn find_connection_returns_matching_metadata() {
        let mut config = Config::default();
        config.atera_connections.push(AteraConnectionMeta {
            id: "acme-1".to_string(),
            label: "ACME".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    fn sample_customer(id: &str, name: &str) -> AteraCustomer {
        AteraCustomer {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn sample_agent(id: &str, customer_id: &str) -> AteraAgent {
        AteraAgent {
            external_id: id.to_string(),
            name: format!("Agent {id}"),
            hostname: None,
            ip_address: None,
            status: None,
            platform: None,
            customer_id: customer_id.to_string(),
            customer_name: format!("Kunde {customer_id}"),
            view_url: None,
        }
    }

    #[test]
    fn groups_agents_under_their_atera_customer_by_id() {
        let customers = vec![sample_customer("1", "ACME"), sample_customer("2", "Contoso")];
        let agents = vec![
            sample_agent("a1", "1"),
            sample_agent("a2", "1"),
            sample_agent("a3", "2"),
        ];

        let groups = group_agents_by_customer(&customers, &agents, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].atera_customer_id, "1");
        assert_eq!(groups[0].devices.len(), 2);
        assert_eq!(groups[1].atera_customer_id, "2");
        assert_eq!(groups[1].devices.len(), 1);
    }

    #[test]
    fn customer_without_agents_still_appears_as_empty_group() {
        let customers = vec![sample_customer("1", "ACME")];
        let groups = group_agents_by_customer(&customers, &[], &[], "conn-1");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].devices.is_empty());
    }

    #[test]
    fn mapped_customer_carries_its_local_customer_id() {
        let customers = vec![sample_customer("1", "ACME")];
        let mappings = vec![AteraCustomerMapping {
            connection_id: "conn-1".to_string(),
            customer_id: "1".to_string(),
            customer_name: "ACME".to_string(),
            local_customer_id: 42,
        }];

        let groups = group_agents_by_customer(&customers, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, Some(42));
    }

    #[test]
    fn unmapped_customer_has_no_local_customer_id() {
        let customers = vec![sample_customer("1", "ACME")];
        let groups = group_agents_by_customer(&customers, &[], &[], "conn-1");
        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn mapping_for_a_different_connection_is_ignored() {
        let customers = vec![sample_customer("1", "ACME")];
        let mappings = vec![AteraCustomerMapping {
            connection_id: "other-connection".to_string(),
            customer_id: "1".to_string(),
            customer_name: "ACME".to_string(),
            local_customer_id: 42,
        }];

        let groups = group_agents_by_customer(&customers, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn agents_for_an_unlisted_atera_customer_id_form_their_own_leftover_group() {
        let customers = vec![sample_customer("1", "ACME")];
        let agents = vec![sample_agent("a9", "99")];

        let groups = group_agents_by_customer(&customers, &agents, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups
            .iter()
            .find(|g| g.atera_customer_id == "99")
            .unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.atera_customer_name, "99");
    }

    fn sample_group() -> AteraCustomerDeviceGroupDto {
        AteraCustomerDeviceGroupDto {
            atera_customer_id: "1".to_string(),
            atera_customer_name: "ACME".to_string(),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "501".to_string(),
                name: "SRV-01".to_string(),
                hostname: Some("SRV-01".to_string()),
                ip_address: Some("10.0.0.5".to_string()),
                status: Some("online".to_string()),
                platform: Some("Windows Server 2022".to_string()),
                view_url: Some("https://app.atera.com/agent/501".to_string()),
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn atera_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_atera_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_atera_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].atera_customer_id, "1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "501");
        assert_eq!(
            loaded.groups[0].devices[0].status.as_deref(),
            Some("online")
        );
    }

    #[test]
    fn atera_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_atera_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn atera_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_atera_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_atera_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_atera_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
