//! Tauri commands for the netcup plugin integration (see `plugin::netcup`
//! and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers following exactly the
//! same pattern as `commands::level`: netcup has no organization/tenant
//! concept (see the `plugin::netcup` module documentation), so a netcup
//! "connection" here corresponds directly to exactly one local customer
//! (`NetcupConnectionMeta.customer_id`). No granular organization mapping
//! layer like `NinjaOrgMapping`/`map_ninja_organization` is needed: every
//! synced server automatically belongs to the connection's customer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::netcup::{test_credentials, NetcupConnectionMeta, NetcupPlugin, NetcupServer};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct NetcupConnectionDto {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    /// ALWAYS `None`: netcup's list endpoint (`GET /servers`) has no
    /// status field at all (verified live, see `plugin::netcup` module
    /// docs, "Servers list"). Real server state (`serverLiveInfo.state`,
    /// e.g. `"RUNNING"`/`"SHUTOFF"`) is only available via
    /// `get_netcup_system_details` (the per-server detail call), fetched on
    /// demand by the frontend: `sync_netcup_connection` never calls it
    /// once per server (that would be an N+1 pattern against an
    /// undocumented rate limit, see module docs). This field still exists
    /// on the DTO so the frontend's shape stays structurally uniform with
    /// other plugins' device lists, never because the list call itself
    /// ever actually populates it (see `plugin::netcup::NetcupServer`'s own
    /// doc comment on why THAT type has no such field at all).
    pub status: Option<String>,
    /// ALWAYS `None`, for exactly the same reason as `status` above:
    /// netcup's list endpoint has no IP address field either. Real IPs
    /// (`ipv4Addresses[].ip`) are only available via
    /// `get_netcup_system_details`.
    pub ip_address: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`.
    pub linked_system_id: Option<i64>,
}

/// Snapshot of the last `sync_netcup_connection` run, cached under
/// `data_dir/plugin-cache/netcup-<connection_id>.json` (see
/// `write_netcup_cache`/`read_netcup_cache`), so `get_cached_netcup_sync`
/// works without network access. Simpler than `CachedNinjaSyncDto`: no
/// organization grouping, since netcup has no concept of organizations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedNetcupSyncDto {
    pub synced_at_utc: String,
    pub devices: Vec<ExternalSystemDto>,
}

fn to_dto(meta: &NetcupConnectionMeta) -> NetcupConnectionDto {
    NetcupConnectionDto {
        id: meta.id.clone(),
        customer_id: meta.customer_id,
        label: meta.label.clone(),
    }
}

/// The fully-qualified `plugin_id` value for a netcup connection: both the
/// key store account and `external_refs.plugin_id`, analogous to
/// `commands::level::plugin_id_for`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("netcup:{connection_id}")
}

/// Generates a stable, low-collision connection ID from a user label,
/// exactly following the pattern of
/// `commands::level::generate_connection_id`.
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
        slug.push_str("netcup");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<NetcupConnectionMeta, AppError> {
    config
        .netcup_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("netcup-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Builds the runnable plugin object plus its associated credentials (the
/// API token) from the key store, based on a connection metadata row.
/// Unlike Ninja, the netcup API token is already the full secret string:
/// no JSON encoding needed, since netcup only needs a single secret value.
fn build_plugin(
    meta: &NetcupConnectionMeta,
) -> Result<(NetcupPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für netcup-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((NetcupPlugin::new(plugin_id), PluginCredentials { secret }))
}

/// Best-effort, analogous to
/// `commands::level::delete_keyring_secret_best_effort`.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The same `data_dir/plugin-cache/` directory as `commands::level` (and
/// every other plugin module): a shared folder for all plugin cache
/// files, just with a different file name prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn netcup_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("netcup-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::level::write_level_cache`.
fn write_netcup_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    devices: &[ExternalSystemDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedNetcupSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        devices: devices.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("netcup-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(netcup_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_netcup_cache`.
/// `Ok(None)` if this connection was never synced: not an error case,
/// analogous to `commands::level::read_level_cache`.
fn read_netcup_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedNetcupSyncDto>, AppError> {
    let path = netcup_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedNetcupSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("netcup-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

/// Builds the DTO for one server. `status`/`ip_address` are always `None`
/// here (see `ExternalSystemDto`'s own doc comment on why), exactly what
/// `NetcupServer` itself provides: the list call never carries either
/// field.
fn to_external_system_dto(
    server: NetcupServer,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: server.external_id,
        name: server.name,
        status: None,
        ip_address: None,
        linked_system_id,
    }
}

#[tauri::command]
pub fn test_netcup_connection(api_token: String) -> Result<(), AppError> {
    test_credentials(&api_token)?;
    Ok(())
}

#[tauri::command]
pub fn list_netcup_connections(
    state: State<AppState>,
) -> Result<Vec<NetcupConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.netcup_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_netcup_connection(
    state: State<AppState>,
    customer_id: i64,
    label: String,
    api_token: String,
) -> Result<NetcupConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    plugin::secrets::store_secret(&plugin_id, &api_token)?;

    let meta = NetcupConnectionMeta {
        id,
        customer_id,
        label,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.netcup_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_netcup_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.netcup_connections.len();
    config.netcup_connections.retain(|c| c.id != id);
    if config.netcup_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "netcup-Verbindung {id} nicht gefunden"
        )));
    }
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = netcup_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "netcup-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn sync_netcup_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<ExternalSystemDto>, AppError> {
    let (customer_id, plugin, credentials, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        (
            meta.customer_id,
            plugin,
            credentials,
            config.data_dir.clone(),
        )
    };

    // Deliberately the minimal `list_servers` call only, NO per-server
    // detail call here (would be N+1 against an undocumented rate limit,
    // see `plugin::netcup` module docs). `status`/`ip_address` stay `None`
    // in every resulting DTO; a per-server detail fetch only ever happens
    // on demand, via `get_netcup_system_details`.
    let servers = plugin.list_servers(&credentials)?;

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, across ALL systems of
    // this connection's customer. Unlike Ninja, no "mapped/unmapped" case
    // distinction is needed, every netcup connection always has exactly one
    // `customer_id`.
    let mut linked_by_external_id: HashMap<String, i64> = HashMap::new();
    for system in db::systems::list_by_customer(&conn, customer_id, true)? {
        for reference in db::external_refs::list_for_system(&conn, system.id)? {
            if reference.plugin_id == plugin_id {
                linked_by_external_id.insert(reference.external_id, system.id);
            }
        }
    }

    let mut result = Vec::with_capacity(servers.len());
    for server in servers {
        let linked_system_id = linked_by_external_id.get(&server.external_id).copied();
        if let Some(system_id) = linked_system_id {
            let payload = plugin.get_system_details(&credentials, &server.external_id)?;
            db::external_refs::upsert(
                &conn,
                system_id,
                &plugin_id,
                &server.external_id,
                &payload.to_string(),
                &tz,
            )?;
        }
        result.push(to_external_system_dto(server, linked_system_id));
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_netcup_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_netcup_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedNetcupSyncDto>, AppError> {
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    read_netcup_cache(&data_dir, &connection_id)
}

#[tauri::command]
pub fn link_system_to_netcup(
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
pub fn unlink_system_from_netcup(
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
pub fn get_netcup_system_details(
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
    fn to_external_system_dto_carries_linked_system_id_through_and_leaves_status_ip_none() {
        let server = NetcupServer {
            external_id: "111".to_string(),
            name: "Server 01".to_string(),
            operating_system: None,
        };
        let dto = to_external_system_dto(server, Some(3));
        assert_eq!(dto.external_id, "111");
        assert_eq!(dto.name, "Server 01");
        assert_eq!(dto.status, None);
        assert_eq!(dto.ip_address, None);
        assert_eq!(dto.linked_system_id, Some(3));
    }

    #[test]
    fn to_external_system_dto_leaves_linked_system_id_none_when_unlinked() {
        let server = NetcupServer {
            external_id: "222".to_string(),
            name: "Server 02".to_string(),
            operating_system: None,
        };
        let dto = to_external_system_dto(server, None);
        assert_eq!(dto.linked_system_id, None);
    }

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("ACME  Kunde GmbH!!"), "acme-kunde-gmbh");
    }

    #[test]
    fn slugify_falls_back_when_label_has_no_alphanumerics() {
        assert_eq!(slugify("***"), "netcup");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_netcup_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "netcup:acme-123");
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
        config.netcup_connections.push(NetcupConnectionMeta {
            id: "acme-1".to_string(),
            customer_id: 7,
            label: "ACME".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
        assert_eq!(found.customer_id, 7);
    }

    fn sample_device(id: &str) -> ExternalSystemDto {
        ExternalSystemDto {
            external_id: id.to_string(),
            name: format!("Server {id}"),
            status: None,
            ip_address: None,
            linked_system_id: Some(3),
        }
    }

    #[test]
    fn netcup_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let devices = vec![sample_device("111")];

        write_netcup_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &devices).unwrap();
        let loaded = read_netcup_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].external_id, "111");
        assert_eq!(loaded.devices[0].linked_system_id, Some(3));
    }

    #[test]
    fn netcup_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_netcup_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn netcup_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_netcup_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_netcup_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_device("111")],
        )
        .unwrap();

        let loaded = read_netcup_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
    }
}
