//! Tauri commands for the Hetzner Cloud plugin integration (see
//! `plugin::hetzner` and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers
//! following exactly the same pattern as `commands::level` -- Hetzner Cloud
//! has no organization/tenant concept (see the `plugin::hetzner` module
//! documentation), so a Hetzner "connection" here corresponds directly to
//! exactly one local customer (`HetznerConnectionMeta.customer_id`). No
//! mapping-table commands like `map_ninja_organization` are needed -- every
//! synced server automatically belongs to the connection's customer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::hetzner::{test_credentials, HetznerConnectionMeta, HetznerPlugin, HetznerServer};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct HetznerConnectionDto {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    /// ONLY `public_net.ipv4.ip` -- see `plugin::hetzner::extract_ipv4_address`.
    /// NEVER the IPv6 field (a /64 CIDR subnet, not a single host address).
    pub ip_address: Option<String>,
    pub status: Option<String>,
    pub platform: Option<String>,
    pub location: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`.
    pub linked_system_id: Option<i64>,
}

/// Snapshot of the last `sync_hetzner_connection` run, cached under
/// `data_dir/plugin-cache/hetzner-<connection_id>.json` (see
/// `write_hetzner_cache`/`read_hetzner_cache`), so `get_cached_hetzner_sync`
/// works without network access. Simpler than `CachedNinjaSyncDto` -- no
/// organization grouping, since Hetzner has no concept of organizations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedHetznerSyncDto {
    pub synced_at_utc: String,
    pub devices: Vec<ExternalSystemDto>,
}

fn to_dto(meta: &HetznerConnectionMeta) -> HetznerConnectionDto {
    HetznerConnectionDto {
        id: meta.id.clone(),
        customer_id: meta.customer_id,
        label: meta.label.clone(),
    }
}

/// The fully-qualified `plugin_id` value for a Hetzner connection -- both
/// the key store account and `external_refs.plugin_id`, analogous to
/// `commands::level::plugin_id_for`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("hetzner:{connection_id}")
}

/// Generates a stable, low-collision connection ID from a user label,
/// exactly following the pattern of `commands::level::generate_connection_id`.
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
        slug.push_str("hetzner");
    }
    slug
}

fn find_connection(
    config: &Config,
    connection_id: &str,
) -> Result<HetznerConnectionMeta, AppError> {
    config
        .hetzner_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Hetzner-Verbindung {connection_id} nicht gefunden"
            ))
        })
}

/// Builds the runnable plugin object plus its associated credentials (the
/// API token) from the key store, based on a connection metadata row.
/// Unlike Ninja, the Hetzner API token is already the full secret string --
/// no JSON encoding needed, since Hetzner only needs a single secret value.
fn build_plugin(
    meta: &HetznerConnectionMeta,
) -> Result<(HetznerPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Hetzner-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        HetznerPlugin::new(plugin_id),
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

/// The same `data_dir/plugin-cache/` directory as `commands::level` -- a
/// shared folder for all plugin cache files, just with a different file
/// name prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn hetzner_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("hetzner-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::level::write_level_cache`.
fn write_hetzner_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    devices: &[ExternalSystemDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedHetznerSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        devices: devices.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache).map_err(|e| {
        AppError::Plugin(format!("Hetzner-Cache konnte nicht kodiert werden: {e}"))
    })?;
    std::fs::write(hetzner_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_hetzner_cache`.
/// `Ok(None)` if this connection was never synced -- not an error case,
/// analogous to `commands::level::read_level_cache`.
fn read_hetzner_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedHetznerSyncDto>, AppError> {
    let path = hetzner_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedHetznerSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Hetzner-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(
    server: HetznerServer,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: server.external_id,
        name: server.name,
        hostname: server.hostname,
        ip_address: server.ip_address,
        status: server.status,
        platform: server.platform,
        location: server.location,
        linked_system_id,
    }
}

#[tauri::command]
pub fn test_hetzner_connection(api_token: String) -> Result<(), AppError> {
    test_credentials(&api_token)?;
    Ok(())
}

#[tauri::command]
pub fn list_hetzner_connections(
    state: State<AppState>,
) -> Result<Vec<HetznerConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.hetzner_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_hetzner_connection(
    state: State<AppState>,
    customer_id: i64,
    label: String,
    api_token: String,
) -> Result<HetznerConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    plugin::secrets::store_secret(&plugin_id, &api_token)?;

    let meta = HetznerConnectionMeta {
        id,
        customer_id,
        label,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.hetzner_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_hetzner_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.hetzner_connections.len();
    config.hetzner_connections.retain(|c| c.id != id);
    if config.hetzner_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Hetzner-Verbindung {id} nicht gefunden"
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

    let cache_path = hetzner_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Hetzner-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn sync_hetzner_connection(
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

    let servers = plugin.list_servers(&credentials)?;

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, across ALL systems of
    // this connection's customer -- unlike Ninja, no "mapped/unmapped" case
    // distinction is needed, every Hetzner connection always has exactly
    // one `customer_id`.
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
    write_hetzner_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_hetzner_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedHetznerSyncDto>, AppError> {
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    read_hetzner_cache(&data_dir, &connection_id)
}

#[tauri::command]
pub fn link_system_to_hetzner(
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
pub fn unlink_system_from_hetzner(
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
pub fn get_hetzner_system_details(
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
    fn to_external_system_dto_carries_hetzner_fields_through() {
        let server = HetznerServer {
            external_id: "42".to_string(),
            name: "web-01".to_string(),
            hostname: Some("web-01".to_string()),
            ip_address: Some("203.0.113.5".to_string()),
            status: Some("running".to_string()),
            platform: Some("cx22".to_string()),
            location: Some("fsn1".to_string()),
        };
        let dto = to_external_system_dto(server, Some(3));
        assert_eq!(dto.external_id, "42");
        assert_eq!(dto.status.as_deref(), Some("running"));
        assert_eq!(dto.platform.as_deref(), Some("cx22"));
        assert_eq!(dto.location.as_deref(), Some("fsn1"));
        assert_eq!(dto.linked_system_id, Some(3));
    }

    #[test]
    fn to_external_system_dto_leaves_optional_fields_none_when_absent() {
        let server = HetznerServer {
            external_id: "7".to_string(),
            name: "web-02".to_string(),
            hostname: None,
            ip_address: None,
            status: None,
            platform: None,
            location: None,
        };
        let dto = to_external_system_dto(server, None);
        assert_eq!(dto.status, None);
        assert_eq!(dto.platform, None);
        assert_eq!(dto.location, None);
        assert_eq!(dto.linked_system_id, None);
    }

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("ACME  Kunde GmbH!!"), "acme-kunde-gmbh");
    }

    #[test]
    fn slugify_falls_back_when_label_has_no_alphanumerics() {
        assert_eq!(slugify("***"), "hetzner");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_hetzner_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "hetzner:acme-123");
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
        config.hetzner_connections.push(HetznerConnectionMeta {
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
            hostname: Some(format!("Server {id}")),
            ip_address: Some("203.0.113.5".to_string()),
            status: Some("running".to_string()),
            platform: Some("cx22".to_string()),
            location: Some("fsn1".to_string()),
            linked_system_id: Some(3),
        }
    }

    #[test]
    fn hetzner_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let devices = vec![sample_device("42")];

        write_hetzner_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &devices).unwrap();
        let loaded = read_hetzner_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].external_id, "42");
        assert_eq!(loaded.devices[0].ip_address.as_deref(), Some("203.0.113.5"));
        assert_eq!(loaded.devices[0].status.as_deref(), Some("running"));
    }

    #[test]
    fn hetzner_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_hetzner_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn hetzner_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_hetzner_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_hetzner_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_device("42")],
        )
        .unwrap();

        let loaded = read_hetzner_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
    }
}
