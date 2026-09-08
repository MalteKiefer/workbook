//! Tauri-Kommandos für die Level.io-Plugin-Integration (siehe
//! `plugin::level` und `docs/PLUGIN_ARCHITECTURE.md`). Dünne Wrapper nach
//! genau demselben Muster wie `commands::plugins` (NinjaOne) -- aber
//! einfacher: Level.io hat kein Organisations-/Mandanten-Konzept (siehe
//! `plugin::level`-Moduldokumentation), daher entspricht eine Level-
//! "Verbindung" hier direkt genau einem lokalen Kunden
//! (`LevelConnectionMeta.customer_id`). Keine granulare
//! Organisations-Zuordnungsebene wie `NinjaOrgMapping`/
//! `map_ninja_organization` nötig -- jedes synchronisierte Gerät gehört
//! automatisch zum Kunden der Verbindung.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::level::{test_credentials, LevelConnectionMeta, LevelDevice, LevelPlugin};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct LevelConnectionDto {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    /// Erste Adresse aus dem `network_interfaces`-Array der ersten
    /// Schnittstelle (siehe `plugin::level::extract_ip_address`). Level hat
    /// -- anders als Ninja -- kein per-Gerät-Dashboard-URL-Feld, das sich
    /// verlässlich aus öffentlicher Doku konstruieren ließe, daher gibt es
    /// hier bewusst kein `level_url`-Gegenstück zu Ninjas `ninja_url`.
    pub ip_address: Option<String>,
    /// `Some(id)`, wenn irgendein lokales System bereits mit dieser externen
    /// ID für diese Verbindung verknüpft ist (`external_refs`-Zeile mit
    /// passendem `plugin_id`/`external_id`), sonst `None`.
    pub linked_system_id: Option<i64>,
    /// Levels rohe, nullable Gruppen-ID (`None` bedeutet "ungrouped", kein
    /// Fehlerfall -- siehe `plugin::level`-Moduldokumentation, Abschnitt
    /// "Gruppen"). Anders als bei Ninjas Organisationen gibt es dafür keine
    /// separate Kunden-Zuordnungsebene -- reine Anzeige-Gruppierung, die das
    /// Frontend (`LevelPluginSection.tsx`) aus der weiterhin flachen
    /// Geräteliste bildet.
    pub group_id: Option<String>,
    /// Der über `plugin::level::build_group_lookup` serverseitig aufgelöste
    /// Klartextname zu `group_id`. `None`, wenn `group_id` selbst `None` ist,
    /// ODER wenn `group_id` gesetzt ist, aber keine passende Gruppe gefunden
    /// wurde (z. B. eine inzwischen gelöschte Gruppe) -- das Frontend fällt
    /// in letzterem Fall auf `Gruppe {group_id}` zurück.
    pub group_name: Option<String>,
}

/// Momentaufnahme des letzten `sync_level_connection`-Laufs, unter
/// `data_dir/plugin-cache/level-<connection_id>.json` zwischengespeichert
/// (siehe `write_level_cache`/`read_level_cache`), damit
/// `get_cached_level_sync` ohne Netzwerkzugriff funktioniert. Einfacher als
/// `CachedNinjaSyncDto` -- keine Organisations-Gruppierung, da Level keine
/// Organisationen kennt.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedLevelSyncDto {
    pub synced_at_utc: String,
    pub devices: Vec<ExternalSystemDto>,
}

fn to_dto(meta: &LevelConnectionMeta) -> LevelConnectionDto {
    LevelConnectionDto { id: meta.id.clone(), customer_id: meta.customer_id, label: meta.label.clone() }
}

/// Der vollqualifizierte `plugin_id`-Wert für eine Level-Verbindung -- sowohl
/// Schlüsselspeicher-Konto als auch `external_refs.plugin_id`, analog zu
/// `commands::plugins::plugin_id_for`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("level:{connection_id}")
}

/// Erzeugt aus einem Nutzer-Label eine stabile, kollisionsarme
/// Verbindungs-ID, exakt nach dem Muster von
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
        slug.push_str("level");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<LevelConnectionMeta, AppError> {
    config
        .level_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("Level-Verbindung {connection_id} nicht gefunden")))
}

/// Baut aus einer Verbindungs-Metadatenzeile das lauffähige Plugin-Objekt
/// plus die dazugehörigen Zugangsdaten (den API-Key) aus dem
/// Schlüsselspeicher. Anders als bei Ninja ist der Level-API-Key bereits der
/// vollständige Secret-String -- keine JSON-Kodierung nötig, da Level nur
/// einen einzigen Geheimwert braucht.
fn build_plugin(meta: &LevelConnectionMeta) -> Result<(LevelPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Level-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((LevelPlugin::new(plugin_id), PluginCredentials { secret }))
}

/// Bestes Bemühen, analog zu
/// `commands::plugins::delete_keyring_secret_best_effort`.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Dasselbe `data_dir/plugin-cache/`-Verzeichnis wie `commands::plugins`
/// (Ninja) -- ein gemeinsamer Ordner für alle Plugin-Zwischenspeicher-
/// Dateien, nur mit unterschiedlichem Dateiname-Präfix je Plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn level_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("level-{connection_id}.json"))
}

/// Schreibt eine Momentaufnahme des Sync-Ergebnisses als JSON-Datei, analog
/// zu `commands::plugins::write_ninja_cache`.
fn write_level_cache(data_dir: &Path, connection_id: &str, synced_at_utc: &str, devices: &[ExternalSystemDto]) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedLevelSyncDto { synced_at_utc: synced_at_utc.to_string(), devices: devices.to_vec() };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Level-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(level_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Liest eine zuvor über `write_level_cache` geschriebene Momentaufnahme
/// zurück. `Ok(None)`, wenn für diese Verbindung noch nie synchronisiert
/// wurde -- kein Fehlerfall, analog zu
/// `commands::plugins::read_ninja_cache`.
fn read_level_cache(data_dir: &Path, connection_id: &str) -> Result<Option<CachedLevelSyncDto>, AppError> {
    let path = level_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedLevelSyncDto =
        serde_json::from_str(&text).map_err(|e| AppError::Plugin(format!("Level-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(device: LevelDevice, linked_system_id: Option<i64>) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: device.external_id,
        name: device.name,
        hostname: device.hostname,
        ip_address: device.ip_address,
        linked_system_id,
        group_id: device.group_id,
        group_name: device.group_name,
    }
}

#[tauri::command]
pub fn test_level_connection(api_key: String) -> Result<(), AppError> {
    test_credentials(&api_key)?;
    Ok(())
}

#[tauri::command]
pub fn list_level_connections(state: State<AppState>) -> Result<Vec<LevelConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.level_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_level_connection(state: State<AppState>, customer_id: i64, label: String, api_key: String) -> Result<LevelConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    plugin::secrets::store_secret(&plugin_id, &api_key)?;

    let meta = LevelConnectionMeta { id, customer_id, label };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.level_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_level_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.level_connections.len();
    config.level_connections.retain(|c| c.id != id);
    if config.level_connections.len() == before {
        return Err(AppError::NotFound(format!("Level-Verbindung {id} nicht gefunden")));
    }
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = level_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!("Level-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}", cache_path.display());
        }
    }
    Ok(())
}

#[tauri::command]
pub fn sync_level_connection(state: State<AppState>, connection_id: String) -> Result<Vec<ExternalSystemDto>, AppError> {
    let (customer_id, plugin, credentials, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        (meta.customer_id, plugin, credentials, config.data_dir.clone())
    };

    let devices = plugin.list_devices(&credentials)?;

    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Umkehr-Index externe-ID -> lokale system_id, über ALLE Systeme des
    // Kunden dieser Verbindung -- anders als bei Ninja braucht es keine
    // Fallunterscheidung "zugeordnet/unzugeordnet", jede Level-Verbindung hat
    // immer genau eine `customer_id`.
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
            db::external_refs::upsert(&conn, system_id, &plugin_id, &device.external_id, &payload.to_string(), &tz)?;
        }
        result.push(to_external_system_dto(device, linked_system_id));
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_level_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_level_sync(state: State<AppState>, connection_id: String) -> Result<Option<CachedLevelSyncDto>, AppError> {
    let data_dir = state.config.lock().expect("Config-Mutex vergiftet").data_dir.clone();
    read_level_cache(&data_dir, &connection_id)
}

#[tauri::command]
pub fn link_system_to_level(state: State<AppState>, system_id: i64, connection_id: String, external_id: String) -> Result<(), AppError> {
    let (plugin, credentials) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        build_plugin(&meta)?
    };

    let payload = plugin.get_system_details(&credentials, &external_id)?;
    plugin.link_system(system_id, &external_id)?;

    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    db::external_refs::upsert(&conn, system_id, plugin.id(), &external_id, &payload.to_string(), &tz)?;
    Ok(())
}

#[tauri::command]
pub fn unlink_system_from_level(state: State<AppState>, system_id: i64, connection_id: String) -> Result<(), AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    db::external_refs::delete(&conn, system_id, &plugin_id_for(&connection_id))
}

#[tauri::command]
pub fn get_level_system_details(state: State<AppState>, connection_id: String, external_id: String) -> Result<serde_json::Value, AppError> {
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
    fn to_external_system_dto_carries_group_fields_through() {
        let device = LevelDevice {
            external_id: "dev-1".to_string(),
            name: "Server 01".to_string(),
            hostname: Some("srv-01.local".to_string()),
            ip_address: Some("10.0.0.5".to_string()),
            group_id: Some("grp-1".to_string()),
            group_name: Some("Werkstatt".to_string()),
        };
        let dto = to_external_system_dto(device, Some(3));
        assert_eq!(dto.group_id.as_deref(), Some("grp-1"));
        assert_eq!(dto.group_name.as_deref(), Some("Werkstatt"));
        assert_eq!(dto.linked_system_id, Some(3));
    }

    #[test]
    fn to_external_system_dto_leaves_group_fields_none_when_ungrouped() {
        let device = LevelDevice {
            external_id: "dev-2".to_string(),
            name: "Server 02".to_string(),
            hostname: None,
            ip_address: None,
            group_id: None,
            group_name: None,
        };
        let dto = to_external_system_dto(device, None);
        assert_eq!(dto.group_id, None);
        assert_eq!(dto.group_name, None);
    }

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("ACME  Kunde GmbH!!"), "acme-kunde-gmbh");
    }

    #[test]
    fn slugify_falls_back_when_label_has_no_alphanumerics() {
        assert_eq!(slugify("***"), "level");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_level_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "level:acme-123");
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
        config.level_connections.push(LevelConnectionMeta {
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
            hostname: Some(format!("{id}.local")),
            ip_address: Some("10.0.0.5".to_string()),
            linked_system_id: Some(3),
            group_id: Some("grp-1".to_string()),
            group_name: Some("Werkstatt".to_string()),
        }
    }

    #[test]
    fn level_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let devices = vec![sample_device("dev-1")];

        write_level_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &devices).unwrap();
        let loaded = read_level_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].external_id, "dev-1");
        assert_eq!(loaded.devices[0].ip_address.as_deref(), Some("10.0.0.5"));
        assert_eq!(loaded.devices[0].group_id.as_deref(), Some("grp-1"));
        assert_eq!(loaded.devices[0].group_name.as_deref(), Some("Werkstatt"));
    }

    #[test]
    fn level_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_level_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn level_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_level_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_level_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &[sample_device("dev-1")]).unwrap();

        let loaded = read_level_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.devices.len(), 1);
    }
}
