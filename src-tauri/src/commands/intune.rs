//! Tauri commands for the Microsoft Intune plugin integration (see
//! `plugin::intune` and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers
//! following exactly the same pattern as `commands::level` -- Intune, like
//! Level.io, has no organization/tenant concept from this app's point of
//! view (one Azure AD/Entra ID tenant is one organization, see the
//! `plugin::intune` module documentation), so an Intune "connection" here
//! corresponds directly to exactly one local customer
//! (`IntuneConnectionMeta.customer_id`). No granular organization mapping
//! layer like `NinjaOrgMapping`/`map_ninja_organization` is needed -- every
//! synced device automatically belongs to the connection's customer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::intune::{
    test_credentials, IntuneConnectionMeta, IntuneCredentials, IntuneDevice, IntunePlugin,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct IntuneConnectionDto {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    /// ALWAYS `None` -- Microsoft Graph's managed-device response has no IP
    /// address field at all (verified against Microsoft's own official
    /// schema, see `plugin::intune` module documentation). Not an
    /// oversight -- the same honestly verified absence Snipe-IT's
    /// `hostname`/`ip_address` already document (see
    /// `docs/PLUGIN_ARCHITECTURE.md`, "Snipe-IT-Plugin" section).
    pub ip_address: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`.
    pub linked_system_id: Option<i64>,
    pub operating_system: Option<String>,
    pub os_version: Option<String>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub compliance_state: Option<String>,
    pub last_sync_date_time: Option<String>,
    pub user_principal_name: Option<String>,
}

/// Snapshot of the last `sync_intune_connection` run, cached under
/// `data_dir/plugin-cache/intune-<connection_id>.json` (see
/// `write_intune_cache`/`read_intune_cache`), so `get_cached_intune_sync`
/// works without network access. Simpler than `CachedNinjaSyncDto` -- no
/// organization grouping, analogous to `CachedLevelSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedIntuneSyncDto {
    pub synced_at_utc: String,
    pub devices: Vec<ExternalSystemDto>,
}

fn to_dto(meta: &IntuneConnectionMeta) -> IntuneConnectionDto {
    IntuneConnectionDto {
        id: meta.id.clone(),
        customer_id: meta.customer_id,
        label: meta.label.clone(),
    }
}

/// The fully-qualified `plugin_id` value for an Intune connection -- both
/// the key store account and `external_refs.plugin_id`, analogous to
/// `commands::level::plugin_id_for`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("intune:{connection_id}")
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
        slug.push_str("intune");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<IntuneConnectionMeta, AppError> {
    config
        .intune_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("Intune-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Builds the runnable plugin object plus its associated credentials (the
/// JSON-encoded `IntuneCredentials`) from the key store, based on a
/// connection metadata row. Unlike Level, the secret string here is a JSON
/// object with three fields, not a single raw API key -- but exactly like
/// Ninja, this function itself does not decode it; it just loads the opaque
/// string and hands it through as `PluginCredentials`. Decoding happens
/// inside `plugin::intune` (`parse_credentials`) when the plugin object is
/// actually used.
fn build_plugin(
    meta: &IntuneConnectionMeta,
) -> Result<(IntunePlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Intune-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((IntunePlugin::new(plugin_id), PluginCredentials { secret }))
}

/// Best-effort, analogous to `commands::level::delete_keyring_secret_best_effort`.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The same `data_dir/plugin-cache/` directory as `commands::level`/
/// `commands::plugins` -- a shared folder for all plugin cache files, just
/// with a different file name prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn intune_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("intune-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::level::write_level_cache`.
fn write_intune_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    devices: &[ExternalSystemDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedIntuneSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        devices: devices.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Intune-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(intune_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_intune_cache`.
/// `Ok(None)` if this connection was never synced -- not an error case,
/// analogous to `commands::level::read_level_cache`.
fn read_intune_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedIntuneSyncDto>, AppError> {
    let path = intune_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedIntuneSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Intune-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(
    device: IntuneDevice,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: device.external_id,
        name: device.name,
        hostname: device.hostname,
        ip_address: device.ip_address,
        linked_system_id,
        operating_system: device.operating_system,
        os_version: device.os_version,
        serial_number: device.serial_number,
        manufacturer: device.manufacturer,
        model: device.model,
        compliance_state: device.compliance_state,
        last_sync_date_time: device.last_sync_date_time,
        user_principal_name: device.user_principal_name,
    }
}

#[tauri::command]
pub fn test_intune_connection(
    tenant_id: String,
    client_id: String,
    client_secret: String,
) -> Result<(), AppError> {
    test_credentials(&tenant_id, &client_id, &client_secret)?;
    Ok(())
}

#[tauri::command]
pub fn list_intune_connections(
    state: State<AppState>,
) -> Result<Vec<IntuneConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.intune_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_intune_connection(
    state: State<AppState>,
    customer_id: i64,
    label: String,
    tenant_id: String,
    client_id: String,
    client_secret: String,
) -> Result<IntuneConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    let secret_json = serde_json::to_string(&IntuneCredentials {
        tenant_id,
        client_id,
        client_secret,
    })
    .map_err(|e| {
        AppError::Plugin(format!(
            "Intune-Zugangsdaten konnten nicht kodiert werden: {e}"
        ))
    })?;
    plugin::secrets::store_secret(&plugin_id, &secret_json)?;

    let meta = IntuneConnectionMeta {
        id,
        customer_id,
        label,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.intune_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_intune_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.intune_connections.len();
    config.intune_connections.retain(|c| c.id != id);
    if config.intune_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Intune-Verbindung {id} nicht gefunden"
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

    let cache_path = intune_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Intune-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn sync_intune_connection(
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
    // this connection's customer -- unlike Ninja, no "mapped/unmapped" case
    // distinction is needed, every Intune connection always has exactly one
    // `customer_id`.
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
    write_intune_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_intune_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedIntuneSyncDto>, AppError> {
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    read_intune_cache(&data_dir, &connection_id)
}

#[tauri::command]
pub fn link_system_to_intune(
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
pub fn unlink_system_from_intune(
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
pub fn get_intune_system_details(
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
    fn to_external_system_dto_carries_intune_fields_through() {
        let device = IntuneDevice {
            external_id: "dev-1".to_string(),
            name: "Server 01".to_string(),
            hostname: Some("Server 01".to_string()),
            ip_address: None,
            operating_system: Some("Windows".to_string()),
            os_version: Some("10.0.19045".to_string()),
            serial_number: Some("SN-001".to_string()),
            manufacturer: Some("Contoso".to_string()),
            model: Some("Surface".to_string()),
            compliance_state: Some("compliant".to_string()),
            last_sync_date_time: Some("2026-09-01T10:00:00Z".to_string()),
            user_principal_name: Some("user@contoso.com".to_string()),
        };
        let dto = to_external_system_dto(device, Some(3));
        assert_eq!(dto.linked_system_id, Some(3));
        assert_eq!(dto.ip_address, None);
        assert_eq!(dto.compliance_state.as_deref(), Some("compliant"));
        assert_eq!(dto.operating_system.as_deref(), Some("Windows"));
    }

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("ACME  Kunde GmbH!!"), "acme-kunde-gmbh");
    }

    #[test]
    fn slugify_falls_back_when_label_has_no_alphanumerics() {
        assert_eq!(slugify("***"), "intune");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_intune_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "intune:acme-123");
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
        config.intune_connections.push(IntuneConnectionMeta {
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
            name: format!("Device {id}"),
            hostname: Some(format!("Device {id}")),
            ip_address: None,
            linked_system_id: Some(3),
            operating_system: Some("Windows".to_string()),
            os_version: Some("10.0.19045".to_string()),
            serial_number: Some("SN-001".to_string()),
            manufacturer: Some("Contoso".to_string()),
            model: Some("Surface".to_string()),
            compliance_state: Some("compliant".to_string()),
            last_sync_date_time: Some("2026-09-01T10:00:00Z".to_string()),
            user_principal_name: Some("user@contoso.com".to_string()),
        }
    }

    #[test]
    fn intune_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let devices = vec![sample_device("dev-1")];

        write_intune_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &devices).unwrap();
        let loaded = read_intune_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].external_id, "dev-1");
        assert_eq!(loaded.devices[0].ip_address, None);
        assert_eq!(
            loaded.devices[0].compliance_state.as_deref(),
            Some("compliant")
        );
    }

    #[test]
    fn intune_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_intune_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn intune_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_intune_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_intune_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_device("dev-1")],
        )
        .unwrap();

        let loaded = read_intune_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
    }
}
