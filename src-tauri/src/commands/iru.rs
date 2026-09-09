//! Tauri commands for the Iru plugin integration (see `plugin::iru` and
//! `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers following exactly the same
//! pattern as `commands::level` -- Iru has no organization/tenant concept
//! (see the `plugin::iru` module documentation), so an Iru "connection" here
//! corresponds directly to exactly one local customer
//! (`IruConnectionMeta.customer_id`). No granular organization mapping layer
//! like `NinjaOrgMapping`/`map_ninja_organization` or
//! `SnipeitCompanyMapping`/`map_snipeit_company` is needed -- every synced
//! device automatically belongs to the connection's customer. Unlike
//! `commands::level` but like `commands::snipeit`: Iru is subdomain-per-
//! tenant, so a connection also needs a user-supplied `base_url` (see
//! `plugin::iru` module documentation).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::iru::{test_credentials, IruConnectionMeta, IruDevice, IruPlugin};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct IruConnectionDto {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    /// Always `None` -- Iru's device object has no hostname field, see the
    /// `plugin::iru` module documentation. Kept as a field anyway so this
    /// DTO type structurally matches its Ninja/Level/Snipe-IT counterparts.
    pub hostname: Option<String>,
    /// Always `None`, for the same reason as `hostname`.
    pub ip_address: Option<String>,
    /// Iru's own device serial number -- together with `asset_tag`, the
    /// natural identification field used for the "link to existing system"
    /// match-key convention (see `plugin::iru` module documentation).
    pub serial_number: Option<String>,
    pub asset_tag: Option<String>,
    pub model: Option<String>,
    pub platform: Option<String>,
    pub os_version: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`.
    pub linked_system_id: Option<i64>,
}

/// Snapshot of the last `sync_iru_connection` run, cached under
/// `data_dir/plugin-cache/iru-<connection_id>.json` (see
/// `write_iru_cache`/`read_iru_cache`), so `get_cached_iru_sync` works
/// without network access. Simpler than `CachedNinjaSyncDto`/
/// `CachedSnipeitSyncDto` -- no organization/company grouping, since Iru has
/// no concept of organizations, exactly like `commands::level::CachedLevelSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedIruSyncDto {
    pub synced_at_utc: String,
    pub devices: Vec<ExternalSystemDto>,
}

fn to_dto(meta: &IruConnectionMeta) -> IruConnectionDto {
    IruConnectionDto {
        id: meta.id.clone(),
        customer_id: meta.customer_id,
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// The fully-qualified `plugin_id` value for an Iru connection -- both the
/// key store account and `external_refs.plugin_id`, analogous to
/// `commands::level::plugin_id_for`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("iru:{connection_id}")
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
        slug.push_str("iru");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<IruConnectionMeta, AppError> {
    config
        .iru_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("Iru-Verbindung {connection_id} nicht gefunden")))
}

/// Builds the runnable plugin object plus its associated credentials (the
/// bearer token) from the key store, based on a connection metadata row.
/// Unlike Ninja, the Iru token is already the full secret string -- no JSON
/// encoding needed, since Iru only needs a single secret value.
fn build_plugin(meta: &IruConnectionMeta) -> Result<(IruPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Iru-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        IruPlugin::new(plugin_id, meta.base_url.clone()),
        PluginCredentials { secret },
    ))
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

/// The same `data_dir/plugin-cache/` directory as `commands::level`/
/// `commands::snipeit` -- a shared folder for all plugin cache files, just
/// with a different file name prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn iru_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("iru-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::level::write_level_cache`.
fn write_iru_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    devices: &[ExternalSystemDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedIruSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        devices: devices.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Iru-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(iru_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_iru_cache`.
/// `Ok(None)` if this connection was never synced -- not an error case,
/// analogous to `commands::level::read_level_cache`.
fn read_iru_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedIruSyncDto>, AppError> {
    let path = iru_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedIruSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Iru-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(device: IruDevice, linked_system_id: Option<i64>) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: device.external_id,
        name: device.name,
        hostname: device.hostname,
        ip_address: device.ip_address,
        serial_number: device.serial_number,
        asset_tag: device.asset_tag,
        model: device.model,
        platform: device.platform,
        os_version: device.os_version,
        linked_system_id,
    }
}

#[tauri::command]
pub fn test_iru_connection(base_url: String, token: String) -> Result<(), AppError> {
    test_credentials(&base_url, &token)?;
    Ok(())
}

#[tauri::command]
pub fn list_iru_connections(state: State<AppState>) -> Result<Vec<IruConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.iru_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_iru_connection(
    state: State<AppState>,
    customer_id: i64,
    label: String,
    base_url: String,
    token: String,
) -> Result<IruConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    plugin::secrets::store_secret(&plugin_id, &token)?;

    let meta = IruConnectionMeta {
        id,
        customer_id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.iru_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_iru_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.iru_connections.len();
    config.iru_connections.retain(|c| c.id != id);
    if config.iru_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Iru-Verbindung {id} nicht gefunden"
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

    let cache_path = iru_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Iru-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn sync_iru_connection(
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

    let devices = plugin.list_devices(&credentials)?;

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, across ALL systems of
    // this connection's customer -- unlike Ninja/Snipe-IT, no
    // "mapped/unmapped" case distinction is needed, every Iru connection
    // always has exactly one `customer_id`.
    let mut linked_by_external_id: HashMap<String, i64> = HashMap::new();
    for system in db::systems::list_by_customer(&conn, customer_id, true)? {
        for reference in db::external_refs::list_for_system(&conn, system.id)? {
            if reference.plugin_id == plugin_id {
                linked_by_external_id.insert(reference.external_id, system.id);
            }
        }
    }

    let mut result = Vec::with_capacity(devices.len());
    for device in devices {
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
        result.push(to_external_system_dto(device, linked_system_id));
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_iru_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_iru_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedIruSyncDto>, AppError> {
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    read_iru_cache(&data_dir, &connection_id)
}

#[tauri::command]
pub fn link_system_to_iru(
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
pub fn unlink_system_from_iru(
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
pub fn get_iru_system_details(
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
    fn to_external_system_dto_carries_iru_fields_through() {
        let device = IruDevice {
            external_id: "dev-1".to_string(),
            name: "MacBook Air".to_string(),
            hostname: None,
            ip_address: None,
            serial_number: Some("FVHHFKF7Q6L4".to_string()),
            asset_tag: Some("AT-042".to_string()),
            model: Some("MacBook Air (M1, 2020)".to_string()),
            platform: Some("Mac".to_string()),
            os_version: Some("14.4.1".to_string()),
        };
        let dto = to_external_system_dto(device, Some(3));
        assert_eq!(dto.serial_number.as_deref(), Some("FVHHFKF7Q6L4"));
        assert_eq!(dto.asset_tag.as_deref(), Some("AT-042"));
        assert_eq!(dto.model.as_deref(), Some("MacBook Air (M1, 2020)"));
        assert_eq!(dto.hostname, None);
        assert_eq!(dto.ip_address, None);
        assert_eq!(dto.linked_system_id, Some(3));
    }

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("ACME  Kunde GmbH!!"), "acme-kunde-gmbh");
    }

    #[test]
    fn slugify_falls_back_when_label_has_no_alphanumerics() {
        assert_eq!(slugify("***"), "iru");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_iru_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "iru:acme-123");
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
        config.iru_connections.push(IruConnectionMeta {
            id: "acme-1".to_string(),
            customer_id: 7,
            label: "ACME".to_string(),
            base_url: "https://acme.api.kandji.io".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
        assert_eq!(found.customer_id, 7);
        assert_eq!(found.base_url, "https://acme.api.kandji.io");
    }

    fn sample_device(id: &str) -> ExternalSystemDto {
        ExternalSystemDto {
            external_id: id.to_string(),
            name: format!("Device {id}"),
            hostname: None,
            ip_address: None,
            serial_number: Some(format!("SN-{id}")),
            asset_tag: None,
            model: Some("MacBook Air".to_string()),
            platform: Some("Mac".to_string()),
            os_version: Some("14.4.1".to_string()),
            linked_system_id: Some(3),
        }
    }

    #[test]
    fn iru_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let devices = vec![sample_device("dev-1")];

        write_iru_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &devices).unwrap();
        let loaded = read_iru_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].external_id, "dev-1");
        assert_eq!(loaded.devices[0].serial_number.as_deref(), Some("SN-dev-1"));
    }

    #[test]
    fn iru_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_iru_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn iru_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_iru_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_iru_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_device("dev-1")],
        )
        .unwrap();

        let loaded = read_iru_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
    }
}
