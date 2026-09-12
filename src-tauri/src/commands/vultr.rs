//! Tauri commands for the Vultr plugin integration (see `plugin::vultr` and
//! `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers following exactly the same
//! pattern as `commands::level` -- a Vultr "connection" (one API key)
//! corresponds directly to exactly one local customer
//! (`VultrConnectionMeta.customer_id`). No granular organization/site
//! mapping layer is needed (see the `plugin::vultr` module documentation,
//! "Tenancy") -- every synced instance automatically belongs to the
//! connection's customer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::vultr::{test_credentials, VultrConnectionMeta, VultrInstance, VultrPlugin};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct VultrConnectionDto {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub ip_address: Option<String>,
    /// See `plugin::vultr::VultrInstance::ipv6_address` -- the empty-string-
    /// means-absent handling already happened server-side, this is never
    /// `Some("")`.
    pub ipv6_address: Option<String>,
    /// Vultr's `power_status`, not its own `status` field -- see the
    /// `plugin::vultr` module documentation.
    pub status: Option<String>,
    pub platform: Option<String>,
    pub operating_system: Option<String>,
    pub region: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`.
    pub linked_system_id: Option<i64>,
}

/// Snapshot of the last `sync_vultr_connection` run, cached under
/// `data_dir/plugin-cache/vultr-<connection_id>.json` (see
/// `write_vultr_cache`/`read_vultr_cache`), so `get_cached_vultr_sync` works
/// without network access. Analogous to `CachedLevelSyncDto` -- no grouping,
/// Vultr has no organization/site concept at all (see the `plugin::vultr`
/// module documentation).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedVultrSyncDto {
    pub synced_at_utc: String,
    pub instances: Vec<ExternalSystemDto>,
}

fn to_dto(meta: &VultrConnectionMeta) -> VultrConnectionDto {
    VultrConnectionDto {
        id: meta.id.clone(),
        customer_id: meta.customer_id,
        label: meta.label.clone(),
    }
}

/// The fully-qualified `plugin_id` value for a Vultr connection -- both the
/// key store account and `external_refs.plugin_id`, analogous to
/// `commands::level::plugin_id_for`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("vultr:{connection_id}")
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
        slug.push_str("vultr");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<VultrConnectionMeta, AppError> {
    config
        .vultr_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("Vultr-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Builds the runnable plugin object plus its associated credentials (the
/// API key) from the key store, based on a connection metadata row. Like
/// Level, the Vultr API key is already the full secret string -- no JSON
/// encoding needed, since Vultr only needs a single secret value.
fn build_plugin(meta: &VultrConnectionMeta) -> Result<(VultrPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Vultr-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((VultrPlugin::new(plugin_id), PluginCredentials { secret }))
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

/// The same `data_dir/plugin-cache/` directory every other plugin module
/// uses -- a shared folder for all plugin cache files, just with a different
/// file name prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn vultr_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("vultr-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::level::write_level_cache`.
fn write_vultr_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    instances: &[ExternalSystemDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedVultrSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        instances: instances.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Vultr-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(vultr_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_vultr_cache`.
/// `Ok(None)` if this connection was never synced -- not an error case,
/// analogous to `commands::level::read_level_cache`.
fn read_vultr_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedVultrSyncDto>, AppError> {
    let path = vultr_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedVultrSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Vultr-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(
    instance: VultrInstance,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: instance.external_id,
        name: instance.name,
        ip_address: instance.ip_address,
        ipv6_address: instance.ipv6_address,
        status: instance.status,
        platform: instance.platform,
        operating_system: instance.operating_system,
        region: instance.region,
        linked_system_id,
    }
}

#[tauri::command]
pub fn test_vultr_connection(api_key: String) -> Result<(), AppError> {
    test_credentials(&api_key)?;
    Ok(())
}

#[tauri::command]
pub fn list_vultr_connections(state: State<AppState>) -> Result<Vec<VultrConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.vultr_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_vultr_connection(
    state: State<AppState>,
    customer_id: i64,
    label: String,
    api_key: String,
) -> Result<VultrConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    plugin::secrets::store_secret(&plugin_id, &api_key)?;

    let meta = VultrConnectionMeta {
        id,
        customer_id,
        label,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.vultr_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_vultr_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.vultr_connections.len();
    config.vultr_connections.retain(|c| c.id != id);
    if config.vultr_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Vultr-Verbindung {id} nicht gefunden"
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

    let cache_path = vultr_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Vultr-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn sync_vultr_connection(
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

    let instances = plugin.list_instances(&credentials)?;

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, across ALL systems of
    // this connection's customer -- unlike Ninja, no "mapped/unmapped" case
    // distinction is needed, every Vultr connection always has exactly one
    // `customer_id`.
    let mut linked_by_external_id: HashMap<String, i64> = HashMap::new();
    for system in db::systems::list_by_customer(&conn, customer_id, true)? {
        for reference in db::external_refs::list_for_system(&conn, system.id)? {
            if reference.plugin_id == plugin_id {
                linked_by_external_id.insert(reference.external_id, system.id);
            }
        }
    }

    let mut result = Vec::with_capacity(instances.len());
    for instance in instances {
        let linked_system_id = linked_by_external_id.get(&instance.external_id).copied();
        if let Some(system_id) = linked_system_id {
            let payload = plugin.get_system_details(&credentials, &instance.external_id)?;
            db::external_refs::upsert(
                &conn,
                system_id,
                &plugin_id,
                &instance.external_id,
                &payload.to_string(),
                &tz,
            )?;
        }
        result.push(to_external_system_dto(instance, linked_system_id));
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_vultr_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_vultr_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedVultrSyncDto>, AppError> {
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    read_vultr_cache(&data_dir, &connection_id)
}

#[tauri::command]
pub fn link_system_to_vultr(
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
pub fn unlink_system_from_vultr(
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
pub fn get_vultr_system_details(
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
    fn to_external_system_dto_carries_fields_through() {
        let instance = VultrInstance {
            external_id: "inst-1".to_string(),
            name: "my-server-01".to_string(),
            ip_address: Some("203.0.113.10".to_string()),
            ipv6_address: Some("2001:db8::1".to_string()),
            status: Some("running".to_string()),
            platform: Some("vc2-2c-4gb".to_string()),
            operating_system: Some("Debian 12 x64".to_string()),
            region: Some("ewr".to_string()),
            linked_system_id: None,
        };
        let dto = to_external_system_dto(instance, Some(3));
        assert_eq!(dto.external_id, "inst-1");
        assert_eq!(dto.ip_address.as_deref(), Some("203.0.113.10"));
        assert_eq!(dto.ipv6_address.as_deref(), Some("2001:db8::1"));
        assert_eq!(dto.status.as_deref(), Some("running"));
        assert_eq!(dto.platform.as_deref(), Some("vc2-2c-4gb"));
        assert_eq!(dto.operating_system.as_deref(), Some("Debian 12 x64"));
        assert_eq!(dto.region.as_deref(), Some("ewr"));
        assert_eq!(dto.linked_system_id, Some(3));
    }

    #[test]
    fn to_external_system_dto_leaves_linked_system_id_none_when_unlinked() {
        let instance = VultrInstance {
            external_id: "inst-2".to_string(),
            name: "srv-02".to_string(),
            ip_address: None,
            ipv6_address: None,
            status: None,
            platform: None,
            operating_system: None,
            region: None,
            linked_system_id: None,
        };
        let dto = to_external_system_dto(instance, None);
        assert_eq!(dto.operating_system, None);
        assert_eq!(dto.linked_system_id, None);
    }

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("ACME  Kunde GmbH!!"), "acme-kunde-gmbh");
    }

    #[test]
    fn slugify_falls_back_when_label_has_no_alphanumerics() {
        assert_eq!(slugify("***"), "vultr");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_vultr_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "vultr:acme-123");
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
        config.vultr_connections.push(VultrConnectionMeta {
            id: "acme-1".to_string(),
            customer_id: 7,
            label: "ACME".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
        assert_eq!(found.customer_id, 7);
    }

    fn sample_instance(id: &str) -> ExternalSystemDto {
        ExternalSystemDto {
            external_id: id.to_string(),
            name: format!("Instance {id}"),
            ip_address: Some("203.0.113.10".to_string()),
            ipv6_address: Some("2001:db8::1".to_string()),
            status: Some("running".to_string()),
            platform: Some("vc2-2c-4gb".to_string()),
            operating_system: Some("Debian 12 x64".to_string()),
            region: Some("ewr".to_string()),
            linked_system_id: Some(3),
        }
    }

    #[test]
    fn vultr_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let instances = vec![sample_instance("inst-1")];

        write_vultr_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &instances).unwrap();
        let loaded = read_vultr_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.instances.len(), 1);
        assert_eq!(loaded.instances[0].external_id, "inst-1");
        assert_eq!(
            loaded.instances[0].ip_address.as_deref(),
            Some("203.0.113.10")
        );
        assert_eq!(
            loaded.instances[0].ipv6_address.as_deref(),
            Some("2001:db8::1")
        );
    }

    #[test]
    fn vultr_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_vultr_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn vultr_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_vultr_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_vultr_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_instance("inst-1")],
        )
        .unwrap();

        let loaded = read_vultr_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.instances.len(), 1);
    }
}
