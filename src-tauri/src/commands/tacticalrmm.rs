//! Tauri commands for the Tactical RMM plugin integration (see
//! `plugin::tacticalrmm` and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers
//! following exactly the same pattern as `commands::plugins` (NinjaOne) -- a
//! Tactical RMM "connection" is a user-created record (base URL + API key)
//! for exactly one Tactical RMM instance, NOT for exactly one local
//! customer. A single instance can manage agents for multiple clients (e.g.
//! because the user setting up the connection is themselves an MSP who runs
//! several of their own customers as separate Tactical RMM "Clients").
//! Which client corresponds to which local customer (if any) is a separate,
//! granular mapping (`TacticalRmmClientMapping`/
//! `Config::tacticalrmm_client_mappings`), maintained by this module via
//! `map_tacticalrmm_client`/`unmap_tacticalrmm_client`. The fully qualified
//! identifier `"tacticalrmm:<connection_id>"` serves both as the keyring
//! account (`plugin::secrets`) and as `external_refs.plugin_id`, so the
//! existing one-row-per-(system_id,plugin_id) upsert semantics keep working
//! unchanged.
//!
//! IMPORTANT, verified difference from `commands::plugins`'s
//! `group_devices_by_organization`: Tactical RMM's agent list carries no
//! numeric client ID at all (see `plugin::tacticalrmm` module docs) -- only
//! a flat `client_name` string. `group_agents_by_client` below therefore
//! joins agents to clients by NAME, not by ID, even though
//! `TacticalRmmClientMapping.client_id` (and the client list itself) do
//! carry a real numeric ID.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::tacticalrmm::{
    test_credentials, TacticalRmmAgent, TacticalRmmClient, TacticalRmmClientMapping,
    TacticalRmmConnectionMeta, TacticalRmmPlugin,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TacticalRmmConnectionDto {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    /// `"online"`/`"offline"`/`"overdue"`, passed through verbatim from
    /// Tactical RMM -- see `plugin::tacticalrmm` module docs.
    pub status: Option<String>,
    /// `"windows"`/`"linux"`/`"darwin"`, passed through verbatim.
    pub platform: Option<String>,
    /// Informational only -- NOT part of the client-mapping join (that uses
    /// `client_name`, see module docs); just the agent's own site name for
    /// display, since mapping granularity deliberately stays at the Client
    /// level (see `plugin::tacticalrmm` module docs).
    pub site_name: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// agents of an unmapped client -- without a `customer_id` there's no
    /// meaningful way to cross-reference against `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TacticalRmmClientDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this client hasn't been mapped to a local customer
    /// yet (`Config::tacticalrmm_client_mappings` has no matching row for
    /// this connection+client).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TacticalRmmClientDeviceGroupDto {
    pub client_id: String,
    pub client_name: String,
    /// `None` if this client isn't mapped to a local customer (yet) -- in
    /// this case the frontend shows "not mapped" and disables linking this
    /// group's agents.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_tacticalrmm_connection` run, cached under
/// `data_dir/plugin-cache/tacticalrmm-<connection_id>.json` (see
/// `write_tacticalrmm_cache`/`read_tacticalrmm_cache`), so
/// `get_cached_tacticalrmm_sync` works without network access -- exactly the
/// same pattern as `commands::plugins::CachedNinjaSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedTacticalRmmSyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<TacticalRmmClientDeviceGroupDto>,
}

fn to_dto(meta: &TacticalRmmConnectionMeta) -> TacticalRmmConnectionDto {
    TacticalRmmConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// The fully qualified `plugin_id` value for a Tactical RMM connection --
/// both the keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("tacticalrmm:{connection_id}")
}

/// Generates a stable, low-collision connection ID from a user label,
/// following exactly the pattern of
/// `commands::plugins::generate_connection_id`.
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
        slug.push_str("tacticalrmm");
    }
    slug
}

fn find_connection(
    config: &Config,
    connection_id: &str,
) -> Result<TacticalRmmConnectionMeta, AppError> {
    config
        .tacticalrmm_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Tactical-RMM-Verbindung {connection_id} nicht gefunden"
            ))
        })
}

/// Builds the runnable plugin object from a connection metadata row, plus
/// the associated credentials (the API key) from the keyring. Like
/// Level.io/Snipe-IT, the Tactical RMM secret is already the complete
/// secret string -- no JSON encoding needed (unlike Ninja, which needs two
/// values).
fn build_plugin(
    meta: &TacticalRmmConnectionMeta,
) -> Result<(TacticalRmmPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Tactical-RMM-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        TacticalRmmPlugin::new(plugin_id, meta.base_url.clone()),
        PluginCredentials { secret },
    ))
}

/// Best-effort, analogous to
/// `commands::plugins::delete_keyring_secret_best_effort`.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The same `data_dir/plugin-cache/` directory as `commands::plugins`/
/// `commands::level`/`commands::snipeit` -- one shared folder for all
/// plugin cache files, just with a different filename prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn tacticalrmm_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("tacticalrmm-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::plugins::write_ninja_cache`.
fn write_tacticalrmm_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[TacticalRmmClientDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedTacticalRmmSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache).map_err(|e| {
        AppError::Plugin(format!(
            "Tactical-RMM-Cache konnte nicht kodiert werden: {e}"
        ))
    })?;
    std::fs::write(tacticalrmm_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_tacticalrmm_cache`.
/// `Ok(None)` if this connection has never been synced -- not an error case,
/// analogous to `commands::plugins::read_ninja_cache`.
fn read_tacticalrmm_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedTacticalRmmSyncDto>, AppError> {
    let path = tacticalrmm_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedTacticalRmmSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Tactical-RMM-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(
    agent: TacticalRmmAgent,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: agent.external_id,
        name: agent.name,
        hostname: agent.hostname,
        ip_address: agent.ip_address,
        status: agent.status,
        platform: agent.platform,
        site_name: agent.site_name,
        linked_system_id,
    }
}

/// Intermediate result of the pure grouping logic: a client along with its
/// agents (still as `TacticalRmmAgent`, not as a DTO) and -- if mapped --
/// the local `customer_id`. Analogous to `commands::plugins::OrgGroup`/
/// `commands::snipeit::CompanyGroup`.
struct ClientGroup {
    client_id: String,
    client_name: String,
    customer_id: Option<i64>,
    devices: Vec<TacticalRmmAgent>,
}

/// Groups agents by client and enriches each group with the configured
/// `customer_id` mapping (if any). A pure function -- no network, no
/// database access -- so it's testable with hardcoded
/// `TacticalRmmClient`/`TacticalRmmAgent`/`TacticalRmmClientMapping` values,
/// structurally analogous to `commands::plugins::group_devices_by_organization`
/// -- with ONE deliberate, verified difference: agents are joined to their
/// client by NAME (`agent.client_name == client.name`), not by ID, because
/// Tactical RMM's agent list API carries no numeric client ID at all (see
/// `plugin::tacticalrmm` module docs). `TacticalRmmClientMapping` lookups
/// (`mapped_customer_id`) still use the real `client_id`, since that IS
/// available on the client list itself. A client with no agents at all still
/// shows up as a group (empty `devices` list) so a future UI can display it
/// for mapping. Agents whose `client_name` doesn't match any known client's
/// `name` (shouldn't normally happen, but not impossible -- e.g. a client
/// renamed between fetching clients and agents in the same sync run) are not
/// silently dropped, but appended as their own leftover group, keyed by the
/// raw name (used as both the synthetic ID and the display name, exactly
/// like `plugin::ninja`'s leftover-organization handling).
fn group_agents_by_client(
    clients: &[TacticalRmmClient],
    agents: &[TacticalRmmAgent],
    mappings: &[TacticalRmmClientMapping],
    connection_id: &str,
) -> Vec<ClientGroup> {
    let mapped_customer_id = |client_id: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.client_id == client_id)
            .map(|m| m.customer_id)
    };

    let mut agents_by_client_name: HashMap<String, Vec<TacticalRmmAgent>> = HashMap::new();
    for agent in agents {
        agents_by_client_name
            .entry(agent.client_name.clone())
            .or_default()
            .push(agent.clone());
    }

    let mut groups = Vec::with_capacity(clients.len());
    for client in clients {
        let client_devices = agents_by_client_name
            .remove(&client.name)
            .unwrap_or_default();
        groups.push(ClientGroup {
            client_id: client.id.clone(),
            client_name: client.name.clone(),
            customer_id: mapped_customer_id(&client.id),
            devices: client_devices,
        });
    }

    let mut leftover_names: Vec<String> = agents_by_client_name.keys().cloned().collect();
    leftover_names.sort();
    for name in leftover_names {
        if let Some(devices) = agents_by_client_name.remove(&name) {
            groups.push(ClientGroup {
                customer_id: mapped_customer_id(&name),
                client_name: name.clone(),
                client_id: name,
                devices,
            });
        }
    }

    groups
}

#[tauri::command]
pub fn test_tacticalrmm_connection(base_url: String, api_key: String) -> Result<(), AppError> {
    test_credentials(&base_url, &api_key)?;
    Ok(())
}

#[tauri::command]
pub fn list_tacticalrmm_connections(
    state: State<AppState>,
) -> Result<Vec<TacticalRmmConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.tacticalrmm_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_tacticalrmm_connection(
    state: State<AppState>,
    label: String,
    base_url: String,
    api_key: String,
) -> Result<TacticalRmmConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    plugin::secrets::store_secret(&plugin_id, &api_key)?;

    let meta = TacticalRmmConnectionMeta {
        id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.tacticalrmm_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_tacticalrmm_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.tacticalrmm_connections.len();
    config.tacticalrmm_connections.retain(|c| c.id != id);
    if config.tacticalrmm_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Tactical-RMM-Verbindung {id} nicht gefunden"
        )));
    }
    // Cleanup: client mappings for this connection are meaningless without
    // the connection and would otherwise be left behind as orphaned data.
    config
        .tacticalrmm_client_mappings
        .retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = tacticalrmm_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Tactical-RMM-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

/// Live fetch of a connection's client list. Analogous to
/// `commands::plugins::list_ninja_organizations`/
/// `commands::snipeit::list_snipeit_companies`, kept for symmetry and a
/// possible initial-setup use case -- normal frontend operation doesn't need
/// this command (cache-first, see
/// `get_cached_tacticalrmm_sync`/`sync_tacticalrmm_connection`).
#[tauri::command]
pub fn list_tacticalrmm_clients(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<TacticalRmmClientDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<TacticalRmmClientMapping> = config
            .tacticalrmm_client_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings)
    };

    let clients = plugin.list_clients(&credentials)?;
    Ok(clients
        .into_iter()
        .map(|client| {
            let mapped_customer_id = mappings
                .iter()
                .find(|m| m.client_id == client.id)
                .map(|m| m.customer_id);
            TacticalRmmClientDto {
                id: client.id,
                name: client.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_tacticalrmm_client(
    state: State<AppState>,
    connection_id: String,
    client_id: String,
    client_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .tacticalrmm_client_mappings
        .retain(|m| !(m.connection_id == connection_id && m.client_id == client_id));
    config
        .tacticalrmm_client_mappings
        .push(TacticalRmmClientMapping {
            connection_id,
            client_id,
            client_name,
            customer_id,
        });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_tacticalrmm_client(
    state: State<AppState>,
    connection_id: String,
    client_id: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Not an error if no matching mapping exists -- the result (no mapping
    // left) is the same, analogous to `db::external_refs::delete`. Existing
    // `external_refs` links are left untouched: unmapping a client is
    // deliberately not an automatic unlinking of its already-linked agents.
    config
        .tacticalrmm_client_mappings
        .retain(|m| !(m.connection_id == connection_id && m.client_id == client_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_tacticalrmm_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<TacticalRmmClientDeviceGroupDto>, AppError> {
    let (plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<TacticalRmmClientMapping> = config
            .tacticalrmm_client_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings, config.data_dir.clone())
    };

    let clients = plugin.list_clients(&credentials)?;
    let agents = plugin.list_agents(&credentials)?;
    let groups = group_agents_by_client(&clients, &agents, &mappings, &connection_id);

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, built ONCE from ALL
    // external_refs of this plugin -- exactly the pattern
    // `commands::plugins::sync_ninja_connection` uses (and the bug fix
    // documented there): a device stays "linked" in the UI even if its
    // client mapping was corrected to a different customer AFTER the link
    // was made, because `db::external_refs::list_for_plugin` searches
    // independent of the currently-mapped customer.
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

        result.push(TacticalRmmClientDeviceGroupDto {
            client_id: group.client_id,
            client_name: group.client_name,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_tacticalrmm_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_tacticalrmm_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedTacticalRmmSyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<TacticalRmmClientMapping> = config
            .tacticalrmm_client_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_tacticalrmm_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-resolved here against the CURRENT client
    // mappings instead of using the value frozen into the cache file during
    // the last `sync_tacticalrmm_connection` run -- otherwise mapping or
    // unmapping a client would only become visible after the next live
    // sync, even though this exact command is meant to show the frontend the
    // current mapping state without network access (analogous to
    // `commands::plugins::get_cached_ninja_sync`).
    if let Some(cache) = cached.as_mut() {
        for group in &mut cache.groups {
            group.customer_id = mappings
                .iter()
                .find(|m| m.client_id == group.client_id)
                .map(|m| m.customer_id);
        }
    }
    Ok(cached)
}

#[tauri::command]
pub fn link_system_to_tacticalrmm(
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
pub fn unlink_system_from_tacticalrmm(
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
pub fn get_tacticalrmm_system_details(
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
        assert_eq!(slugify("***"), "tacticalrmm");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_tacticalrmm_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "tacticalrmm:acme-123");
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
        config
            .tacticalrmm_connections
            .push(TacticalRmmConnectionMeta {
                id: "acme-1".to_string(),
                label: "ACME".to_string(),
                base_url: "https://api.rmm.example.com".to_string(),
            });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    fn sample_client(id: &str, name: &str) -> TacticalRmmClient {
        TacticalRmmClient {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn sample_agent(id: &str, client_name: &str) -> TacticalRmmAgent {
        TacticalRmmAgent {
            external_id: id.to_string(),
            name: format!("Agent {id}"),
            hostname: None,
            ip_address: None,
            status: None,
            platform: None,
            operating_system: None,
            client_name: client_name.to_string(),
            site_name: None,
        }
    }

    #[test]
    fn groups_agents_under_their_client_by_name() {
        let clients = vec![sample_client("1", "ACME"), sample_client("2", "Contoso")];
        let agents = vec![
            sample_agent("a1", "ACME"),
            sample_agent("a2", "ACME"),
            sample_agent("a3", "Contoso"),
        ];

        let groups = group_agents_by_client(&clients, &agents, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].client_id, "1");
        assert_eq!(groups[0].devices.len(), 2);
        assert_eq!(groups[1].client_id, "2");
        assert_eq!(groups[1].devices.len(), 1);
    }

    #[test]
    fn client_without_agents_still_appears_as_empty_group() {
        let clients = vec![sample_client("1", "ACME")];
        let groups = group_agents_by_client(&clients, &[], &[], "conn-1");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].devices.is_empty());
    }

    #[test]
    fn mapped_client_carries_its_customer_id() {
        let clients = vec![sample_client("1", "ACME")];
        let mappings = vec![TacticalRmmClientMapping {
            connection_id: "conn-1".to_string(),
            client_id: "1".to_string(),
            client_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_agents_by_client(&clients, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, Some(42));
    }

    #[test]
    fn unmapped_client_has_no_customer_id() {
        let clients = vec![sample_client("1", "ACME")];
        let groups = group_agents_by_client(&clients, &[], &[], "conn-1");
        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn mapping_for_a_different_connection_is_ignored() {
        let clients = vec![sample_client("1", "ACME")];
        let mappings = vec![TacticalRmmClientMapping {
            connection_id: "other-connection".to_string(),
            client_id: "1".to_string(),
            client_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_agents_by_client(&clients, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn agents_for_an_unlisted_client_name_form_their_own_leftover_group() {
        let clients = vec![sample_client("1", "ACME")];
        let agents = vec![sample_agent("a9", "Orphan Client")];

        let groups = group_agents_by_client(&clients, &agents, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups
            .iter()
            .find(|g| g.client_id == "Orphan Client")
            .unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.client_name, "Orphan Client");
    }

    fn sample_group() -> TacticalRmmClientDeviceGroupDto {
        TacticalRmmClientDeviceGroupDto {
            client_id: "1".to_string(),
            client_name: "ACME".to_string(),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "abc123".to_string(),
                name: "SRV-01".to_string(),
                hostname: Some("SRV-01".to_string()),
                ip_address: Some("10.0.0.5".to_string()),
                status: Some("online".to_string()),
                platform: Some("windows".to_string()),
                site_name: Some("Hauptsitz".to_string()),
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn tacticalrmm_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_tacticalrmm_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_tacticalrmm_cache(dir.path(), "conn-1")
            .unwrap()
            .unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].client_id, "1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "abc123");
        assert_eq!(
            loaded.groups[0].devices[0].status.as_deref(),
            Some("online")
        );
    }

    #[test]
    fn tacticalrmm_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_tacticalrmm_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn tacticalrmm_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_tacticalrmm_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_tacticalrmm_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_tacticalrmm_cache(dir.path(), "conn-1")
            .unwrap()
            .unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
