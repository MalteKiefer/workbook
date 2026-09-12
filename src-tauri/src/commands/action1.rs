//! Tauri commands for the Action1 plugin integration (see `plugin::action1`
//! and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers following exactly the
//! same pattern as `commands::tacticalrmm` -- an Action1 "connection" is a
//! user-created record (region base URL + client ID + client secret) for
//! exactly one Action1 account, NOT for exactly one local customer. A single
//! account can manage multiple "Organizations" (e.g. because the user
//! setting up the connection is themselves an MSP running several of their
//! own customers as separate Action1 organizations). Which organization
//! corresponds to which local customer (if any) is a separate, granular
//! mapping (`Action1OrgMapping`/`Config::action1_org_mappings`), maintained
//! by this module via `map_action1_organization`/`unmap_action1_organization`.
//! The fully qualified identifier `"action1:<connection_id>"` serves both as
//! the keyring account (`plugin::secrets`) and as `external_refs.plugin_id`,
//! the same one-row-per-(system_id,plugin_id) upsert semantics as every
//! other plugin here.
//!
//! IMPORTANT, verified difference from `commands::tacticalrmm`'s
//! `group_agents_by_client`: Action1's endpoint list carries a genuine,
//! verified `organization_id` FOREIGN KEY (a UUID string) directly on each
//! endpoint object -- unlike Tactical RMM's flat `client_name` string, which
//! forces a name-based join (see `plugin::tacticalrmm` module docs).
//! `group_endpoints_by_organization` below therefore joins endpoints to
//! organizations by ID, structurally the same shape as
//! `commands::plugins::group_devices_by_organization` (NinjaOne, also a real
//! ID-based join), NOT copied from Tactical RMM's name-join workaround.
//!
//! Also IMPORTANT, unlike every other plugin's sync command here: Action1
//! has no "list every device of the account in one call" endpoint --
//! `GET /endpoints/managed/{orgId}` is scoped to exactly one organization
//! (see `plugin::action1` module docs), so fetching a connection's full
//! device list genuinely requires one endpoint-list call per organization.
//! Combined with Action1's comparatively tight rate limit (30 requests/
//! minute, see `plugin::action1` module docs), `sync_action1_connection`
//! below deliberately uses `Action1Plugin::list_organizations_with_endpoints`
//! (a single OAuth2 token fetch, reused across the organization list AND
//! every organization's endpoint list) rather than calling
//! `list_organizations` followed by N separate `list_endpoints` calls, which
//! would re-authenticate N extra times for no reason.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::action1::{
    test_credentials, Action1ConnectionMeta, Action1Endpoint, Action1OrgMapping,
    Action1Organization, Action1Plugin,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Action1ConnectionDto {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub ip_address: Option<String>,
    /// `"Connected"`/`"Disconnected"`/`"Pending Uninstall"`, passed through
    /// verbatim from Action1 -- see `plugin::action1` module docs. This is
    /// connectivity, deliberately NOT Action1's separate `online_status`
    /// health flag (never surfaced anywhere in this app, see module docs).
    pub status: Option<String>,
    pub platform: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// endpoints of an unmapped organization -- without a `customer_id`
    /// there's no meaningful way to cross-reference against `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Action1OrganizationDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this organization hasn't been mapped to a local
    /// customer yet (`Config::action1_org_mappings` has no matching row for
    /// this connection+organization).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Action1OrgDeviceGroupDto {
    pub organization_id: String,
    pub organization_name: String,
    /// `None` if this organization isn't mapped to a local customer (yet) --
    /// in this case the frontend shows "not mapped" and disables linking
    /// this group's endpoints.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_action1_connection` run, cached under
/// `data_dir/plugin-cache/action1-<connection_id>.json` (see
/// `write_action1_cache`/`read_action1_cache`), so `get_cached_action1_sync`
/// works without network access -- exactly the same pattern as
/// `commands::tacticalrmm::CachedTacticalRmmSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedAction1SyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<Action1OrgDeviceGroupDto>,
}

fn to_dto(meta: &Action1ConnectionMeta) -> Action1ConnectionDto {
    Action1ConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// The fully qualified `plugin_id` value for an Action1 connection -- both
/// the keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("action1:{connection_id}")
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
        slug.push_str("action1");
    }
    slug
}

fn find_connection(
    config: &Config,
    connection_id: &str,
) -> Result<Action1ConnectionMeta, AppError> {
    config
        .action1_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("Action1-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Builds the runnable plugin object from a connection metadata row, plus
/// the associated credentials (client ID + client secret, JSON-encoded) from
/// the keyring -- the same two-value pattern as `plugin::ninja::
/// NinjaCredentials` (see `add_action1_connection` below for the encoding
/// side).
fn build_plugin(
    meta: &Action1ConnectionMeta,
) -> Result<(Action1Plugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Action1-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        Action1Plugin::new(plugin_id, meta.base_url.clone()),
        PluginCredentials { secret },
    ))
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

/// The same `data_dir/plugin-cache/` directory as every other plugin's
/// command module -- one shared folder, just a different filename prefix.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn action1_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("action1-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::tacticalrmm::write_tacticalrmm_cache`.
fn write_action1_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[Action1OrgDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedAction1SyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Action1-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(action1_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_action1_cache`.
/// `Ok(None)` if this connection has never been synced -- not an error case,
/// analogous to `commands::tacticalrmm::read_tacticalrmm_cache`.
fn read_action1_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedAction1SyncDto>, AppError> {
    let path = action1_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedAction1SyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Action1-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(
    endpoint: Action1Endpoint,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: endpoint.external_id,
        name: endpoint.name,
        ip_address: endpoint.ip_address,
        status: endpoint.status,
        platform: endpoint.platform,
        linked_system_id,
    }
}

/// Intermediate result of the pure grouping logic: an organization along
/// with its endpoints (still as `Action1Endpoint`, not as a DTO) and -- if
/// mapped -- the local `customer_id`. Analogous to
/// `commands::plugins::OrgGroup`/`commands::tacticalrmm::ClientGroup`.
struct OrgGroup {
    organization_id: String,
    organization_name: String,
    customer_id: Option<i64>,
    devices: Vec<Action1Endpoint>,
}

/// Groups endpoints by organization and enriches each group with the
/// configured `customer_id` mapping (if any). A pure function -- no network,
/// no database access -- so it's testable with hardcoded
/// `Action1Organization`/`Action1Endpoint`/`Action1OrgMapping` values.
/// Structurally analogous to `commands::plugins::group_devices_by_organization`
/// (NinjaOne) -- both join by a genuine ID (`endpoint.organization_id`),
/// UNLIKE `commands::tacticalrmm::group_agents_by_client`'s name-based join
/// (Tactical RMM's agent list has no numeric client ID at all, see
/// `plugin::tacticalrmm` module docs -- Action1's endpoint list does carry a
/// real `organization_id`, see `plugin::action1` module docs). An
/// organization with no endpoints at all still shows up as a group (empty
/// `devices` list) so the UI can display it for mapping. Endpoints whose
/// `organization_id` doesn't match any known organization's `id` (shouldn't
/// normally happen, but not impossible -- e.g. an organization deleted
/// between fetching organizations and its own endpoints in the same sync
/// run) are not silently dropped, but appended as their own leftover group,
/// keyed by the raw ID (used as both the synthetic ID and, absent a better
/// name, the display name too), exactly like `plugin::ninja`'s leftover-
/// organization handling.
fn group_endpoints_by_organization(
    organizations: &[Action1Organization],
    endpoints: &[Action1Endpoint],
    mappings: &[Action1OrgMapping],
    connection_id: &str,
) -> Vec<OrgGroup> {
    let mapped_customer_id = |organization_id: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.organization_id == organization_id)
            .map(|m| m.customer_id)
    };

    let mut endpoints_by_org: HashMap<String, Vec<Action1Endpoint>> = HashMap::new();
    for endpoint in endpoints {
        endpoints_by_org
            .entry(endpoint.organization_id.clone())
            .or_default()
            .push(endpoint.clone());
    }

    let mut groups = Vec::with_capacity(organizations.len());
    for org in organizations {
        let devices = endpoints_by_org.remove(&org.id).unwrap_or_default();
        groups.push(OrgGroup {
            organization_id: org.id.clone(),
            organization_name: org.name.clone(),
            customer_id: mapped_customer_id(&org.id),
            devices,
        });
    }

    let mut leftover_ids: Vec<String> = endpoints_by_org.keys().cloned().collect();
    leftover_ids.sort();
    for organization_id in leftover_ids {
        if let Some(devices) = endpoints_by_org.remove(&organization_id) {
            groups.push(OrgGroup {
                customer_id: mapped_customer_id(&organization_id),
                organization_name: organization_id.clone(),
                organization_id,
                devices,
            });
        }
    }

    groups
}

#[tauri::command]
pub fn test_action1_connection(
    base_url: String,
    client_id: String,
    client_secret: String,
) -> Result<(), AppError> {
    test_credentials(&base_url, &client_id, &client_secret)?;
    Ok(())
}

#[tauri::command]
pub fn list_action1_connections(
    state: State<AppState>,
) -> Result<Vec<Action1ConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.action1_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_action1_connection(
    state: State<AppState>,
    label: String,
    base_url: String,
    client_id: String,
    client_secret: String,
) -> Result<Action1ConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    // Two-value JSON encoding, mirroring `commands::ninja` -- the credential
    // principle keeps this out of `config.toml` entirely, only in the OS
    // keyring (see `plugin::secrets`).
    let secret = serde_json::to_string(&serde_json::json!({
        "client_id": client_id,
        "client_secret": client_secret,
    }))
    .map_err(|e| {
        AppError::Plugin(format!(
            "Action1-Zugangsdaten konnten nicht kodiert werden: {e}"
        ))
    })?;
    plugin::secrets::store_secret(&plugin_id, &secret)?;

    let meta = Action1ConnectionMeta {
        id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.action1_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_action1_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.action1_connections.len();
    config.action1_connections.retain(|c| c.id != id);
    if config.action1_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Action1-Verbindung {id} nicht gefunden"
        )));
    }
    // Cleanup: organization mappings for this connection are meaningless
    // without the connection and would otherwise be left behind as orphaned
    // data.
    config
        .action1_org_mappings
        .retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = action1_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Action1-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

/// Live fetch of a connection's organization list. Analogous to
/// `commands::plugins::list_ninja_organizations`/
/// `commands::tacticalrmm::list_tacticalrmm_clients`, kept for symmetry and
/// a possible initial-setup use case -- normal frontend operation doesn't
/// need this command (cache-first, see
/// `get_cached_action1_sync`/`sync_action1_connection`).
#[tauri::command]
pub fn list_action1_organizations(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<Action1OrganizationDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<Action1OrgMapping> = config
            .action1_org_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings)
    };

    let organizations = plugin.list_organizations(&credentials)?;
    Ok(organizations
        .into_iter()
        .map(|org| {
            let mapped_customer_id = mappings
                .iter()
                .find(|m| m.organization_id == org.id)
                .map(|m| m.customer_id);
            Action1OrganizationDto {
                id: org.id,
                name: org.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_action1_organization(
    state: State<AppState>,
    connection_id: String,
    organization_id: String,
    organization_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .action1_org_mappings
        .retain(|m| !(m.connection_id == connection_id && m.organization_id == organization_id));
    config.action1_org_mappings.push(Action1OrgMapping {
        connection_id,
        organization_id,
        organization_name,
        customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_action1_organization(
    state: State<AppState>,
    connection_id: String,
    organization_id: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Not an error if no matching mapping exists -- the result (no mapping
    // left) is the same, analogous to `db::external_refs::delete`. Existing
    // `external_refs` links are left untouched: unmapping an organization is
    // deliberately not an automatic unlinking of its already-linked
    // endpoints.
    config
        .action1_org_mappings
        .retain(|m| !(m.connection_id == connection_id && m.organization_id == organization_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_action1_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<Action1OrgDeviceGroupDto>, AppError> {
    let (plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<Action1OrgMapping> = config
            .action1_org_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings, config.data_dir.clone())
    };

    // A single OAuth2 token fetch, reused across the organization list AND
    // every organization's endpoint list -- see module docs on why this
    // matters for Action1's tighter rate limit specifically.
    let (organizations, endpoints) = plugin.list_organizations_with_endpoints(&credentials)?;
    let groups =
        group_endpoints_by_organization(&organizations, &endpoints, &mappings, &connection_id);

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, built ONCE from ALL
    // external_refs of this plugin -- exactly the pattern
    // `commands::tacticalrmm::sync_tacticalrmm_connection` uses: a device
    // stays "linked" in the UI even if its organization mapping was
    // corrected to a different customer AFTER the link was made, because
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
                // Only already-linked devices get a live detail refresh here
                // (bounded by how many the user has actually linked, not the
                // whole fleet) -- the same cost-limiting principle
                // `commands::tacticalrmm::sync_tacticalrmm_connection`
                // already applies, worth calling out again here given
                // Action1's tighter rate limit (see `plugin::action1` module
                // docs): each of these is its own OAuth2 token fetch plus
                // detail call.
                let payload = plugin.get_endpoint_details(
                    &credentials,
                    &device.organization_id,
                    &device.external_id,
                )?;
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

        result.push(Action1OrgDeviceGroupDto {
            organization_id: group.organization_id,
            organization_name: group.organization_name,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_action1_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_action1_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedAction1SyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<Action1OrgMapping> = config
            .action1_org_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_action1_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-resolved here against the CURRENT
    // organization mappings instead of using the value frozen into the cache
    // file during the last `sync_action1_connection` run -- otherwise
    // mapping or unmapping an organization would only become visible after
    // the next live sync, even though this exact command is meant to show
    // the frontend the current mapping state without network access
    // (analogous to `commands::tacticalrmm::get_cached_tacticalrmm_sync`).
    if let Some(cache) = cached.as_mut() {
        for group in &mut cache.groups {
            group.customer_id = mappings
                .iter()
                .find(|m| m.organization_id == group.organization_id)
                .map(|m| m.customer_id);
        }
    }
    Ok(cached)
}

#[tauri::command]
pub fn link_system_to_action1(
    state: State<AppState>,
    system_id: i64,
    connection_id: String,
    organization_id: String,
    external_id: String,
) -> Result<(), AppError> {
    let (plugin, credentials) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        build_plugin(&meta)?
    };

    // Action1's single-device detail call genuinely needs both IDs in its
    // URL path (see `plugin::action1` module docs) -- unlike Tactical RMM's
    // `link_system_to_tacticalrmm`, this command therefore takes
    // `organization_id` explicitly rather than relying on the `Plugin`
    // trait's single-`external_id` `get_system_details` method.
    let payload = plugin.get_endpoint_details(&credentials, &organization_id, &external_id)?;
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
pub fn unlink_system_from_action1(
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
pub fn get_action1_system_details(
    state: State<AppState>,
    connection_id: String,
    organization_id: String,
    external_id: String,
) -> Result<serde_json::Value, AppError> {
    let (plugin, credentials) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        build_plugin(&meta)?
    };
    Ok(plugin.get_endpoint_details(&credentials, &organization_id, &external_id)?)
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
        assert_eq!(slugify("***"), "action1");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_action1_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "action1:acme-123");
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
        config.action1_connections.push(Action1ConnectionMeta {
            id: "acme-1".to_string(),
            label: "ACME".to_string(),
            base_url: "https://app.action1.com/api/3.0".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    fn sample_org(id: &str, name: &str) -> Action1Organization {
        Action1Organization {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn sample_endpoint(id: &str, organization_id: &str) -> Action1Endpoint {
        Action1Endpoint {
            external_id: id.to_string(),
            name: format!("Endpoint {id}"),
            ip_address: None,
            status: None,
            platform: None,
            operating_system: None,
            organization_id: organization_id.to_string(),
            organization_name: None,
        }
    }

    #[test]
    fn groups_endpoints_under_their_organization_by_id() {
        let organizations = vec![sample_org("org-1", "ACME"), sample_org("org-2", "Contoso")];
        let endpoints = vec![
            sample_endpoint("ep-1", "org-1"),
            sample_endpoint("ep-2", "org-1"),
            sample_endpoint("ep-3", "org-2"),
        ];

        let groups = group_endpoints_by_organization(&organizations, &endpoints, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].organization_id, "org-1");
        assert_eq!(groups[0].devices.len(), 2);
        assert_eq!(groups[1].organization_id, "org-2");
        assert_eq!(groups[1].devices.len(), 1);
    }

    #[test]
    fn organization_without_endpoints_still_appears_as_empty_group() {
        let organizations = vec![sample_org("org-1", "ACME")];
        let groups = group_endpoints_by_organization(&organizations, &[], &[], "conn-1");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].devices.is_empty());
    }

    #[test]
    fn mapped_organization_carries_its_customer_id() {
        let organizations = vec![sample_org("org-1", "ACME")];
        let mappings = vec![Action1OrgMapping {
            connection_id: "conn-1".to_string(),
            organization_id: "org-1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_endpoints_by_organization(&organizations, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, Some(42));
    }

    #[test]
    fn unmapped_organization_has_no_customer_id() {
        let organizations = vec![sample_org("org-1", "ACME")];
        let groups = group_endpoints_by_organization(&organizations, &[], &[], "conn-1");
        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn mapping_for_a_different_connection_is_ignored() {
        let organizations = vec![sample_org("org-1", "ACME")];
        let mappings = vec![Action1OrgMapping {
            connection_id: "other-connection".to_string(),
            organization_id: "org-1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_endpoints_by_organization(&organizations, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn endpoints_for_an_unlisted_organization_id_form_their_own_leftover_group() {
        let organizations = vec![sample_org("org-1", "ACME")];
        let endpoints = vec![sample_endpoint("ep-9", "org-orphan")];

        let groups = group_endpoints_by_organization(&organizations, &endpoints, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups
            .iter()
            .find(|g| g.organization_id == "org-orphan")
            .unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.organization_name, "org-orphan");
    }

    fn sample_group() -> Action1OrgDeviceGroupDto {
        Action1OrgDeviceGroupDto {
            organization_id: "org-1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "ep-1".to_string(),
                name: "SRV-01".to_string(),
                ip_address: Some("10.0.0.5".to_string()),
                status: Some("Connected".to_string()),
                platform: Some("Windows".to_string()),
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn action1_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_action1_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_action1_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].organization_id, "org-1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "ep-1");
        assert_eq!(
            loaded.groups[0].devices[0].status.as_deref(),
            Some("Connected")
        );
    }

    #[test]
    fn action1_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_action1_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn action1_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_action1_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_action1_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_action1_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
