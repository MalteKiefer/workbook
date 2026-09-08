//! Tauri-Kommandos für die Snipe-IT-Plugin-Integration (siehe
//! `plugin::snipeit` und `docs/PLUGIN_ARCHITECTURE.md`). Dünne Wrapper nach
//! genau demselben Muster wie `commands::plugins` (NinjaOne) -- eine
//! Snipe-IT-"Verbindung" ist ein vom Nutzer angelegter Datensatz (Basis-URL +
//! Personal Access Token) für genau eine Snipe-IT-Instanz, NICHT für genau
//! einen lokalen Kunden. Eine einzelne Instanz kann Assets mehrerer Firmen
//! verwalten (z. B. weil der Nutzer, der die Verbindung anlegt, selbst ein
//! MSP ist und mehrere eigene Kunden als getrennte Firmen in Snipe-IT führt).
//! Welche Firma welchem lokalen Kunden entspricht (falls überhaupt), ist eine
//! separate, granulare Zuordnung
//! (`SnipeitCompanyMapping`/`Config::snipeit_company_mappings`), die dieses
//! Modul über `map_snipeit_company`/`unmap_snipeit_company` pflegt. Der
//! vollqualifizierte Bezeichner `"snipeit:<connection_id>"` dient sowohl als
//! Schlüsselspeicher-Konto (`plugin::secrets`) als auch als
//! `external_refs.plugin_id`, sodass die bestehende
//! Ein-Zeile-je-(system_id,plugin_id)-Upsert-Semantik unverändert
//! weiterfunktioniert.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::snipeit::{
    test_credentials, SnipeitCompany, SnipeitCompanyMapping, SnipeitConnectionMeta, SnipeitDevice,
    SnipeitPlugin, UNASSIGNED_COMPANY_ID,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SnipeitConnectionDto {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    /// Immer `None` -- Snipe-IT liefert kein Hostname-Feld auf dem Asset-
    /// Kernobjekt, siehe `plugin::snipeit`-Moduldokumentation. Als Feld
    /// trotzdem vorhanden, damit dieser DTO-Typ strukturell zu Ninjas/Levels
    /// Gegenstück passt.
    pub hostname: Option<String>,
    /// Immer `None`, aus demselben Grund wie `hostname`.
    pub ip_address: Option<String>,
    /// Snipe-ITs eigenes, primäres Identifikationsfeld für ein Asset.
    pub asset_tag: Option<String>,
    /// Snipe-ITs Seriennummer-Feld.
    pub serial: Option<String>,
    /// Direktlink auf die Asset-Detailseite in Snipe-ITs eigener
    /// Weboberfläche (`{base_url}/hardware/{id}`), aus `base_url` der
    /// Verbindung und der externen Asset-ID konstruiert -- verifiziert über
    /// Snipe-ITs `routes/web/hardware.php` (siehe
    /// `plugin::snipeit`-Moduldokumentation), kein erfundenes URL-Schema.
    pub snipeit_url: String,
    /// `Some(id)`, wenn irgendein lokales System bereits mit dieser externen
    /// ID für diese Verbindung verknüpft ist (`external_refs`-Zeile mit
    /// passendem `plugin_id`/`external_id`), sonst `None`. Bei Assets einer
    /// nicht zugeordneten Firma immer `None` -- ohne `customer_id` lässt
    /// sich nicht sinnvoll gegen `external_refs` querverweisen.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SnipeitCompanyDto {
    pub id: String,
    pub name: String,
    /// `None`, solange diese Firma noch keinem lokalen Kunden zugeordnet
    /// wurde (`Config::snipeit_company_mappings` hat keine passende Zeile
    /// für diese Verbindung+Firma).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnipeitCompanyDeviceGroupDto {
    pub company_id: String,
    pub company_name: String,
    /// `None`, wenn diese Firma (noch) keinem lokalen Kunden zugeordnet ist
    /// -- das Frontend zeigt in diesem Fall "nicht zugeordnet" an und
    /// deaktiviert das Verknüpfen der Assets dieser Gruppe.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Momentaufnahme des letzten `sync_snipeit_connection`-Laufs, unter
/// `data_dir/plugin-cache/snipeit-<connection_id>.json` zwischengespeichert
/// (siehe `write_snipeit_cache`/`read_snipeit_cache`), damit
/// `get_cached_snipeit_sync` ohne Netzwerkzugriff funktioniert -- exakt
/// dasselbe Muster wie `commands::plugins::CachedNinjaSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedSnipeitSyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<SnipeitCompanyDeviceGroupDto>,
}

fn to_dto(meta: &SnipeitConnectionMeta) -> SnipeitConnectionDto {
    SnipeitConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// Der vollqualifizierte `plugin_id`-Wert für eine Snipe-IT-Verbindung --
/// sowohl Schlüsselspeicher-Konto als auch `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("snipeit:{connection_id}")
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
        slug.push_str("snipeit");
    }
    slug
}

fn find_connection(
    config: &Config,
    connection_id: &str,
) -> Result<SnipeitConnectionMeta, AppError> {
    config
        .snipeit_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Snipe-IT-Verbindung {connection_id} nicht gefunden"
            ))
        })
}

/// Baut aus einer Verbindungs-Metadatenzeile das lauffähige Plugin-Objekt
/// plus die dazugehörigen Zugangsdaten (den Personal Access Token) aus dem
/// Schlüsselspeicher. Anders als bei Ninja ist der Snipe-IT-Token bereits der
/// vollständige Secret-String -- keine JSON-Kodierung nötig, da Snipe-IT nur
/// einen einzigen Geheimwert braucht (wie Level.io).
fn build_plugin(
    meta: &SnipeitConnectionMeta,
) -> Result<(SnipeitPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Snipe-IT-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        SnipeitPlugin::new(plugin_id, meta.base_url.clone()),
        PluginCredentials { secret },
    ))
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

/// Dasselbe `data_dir/plugin-cache/`-Verzeichnis wie `commands::plugins`/
/// `commands::level` -- ein gemeinsamer Ordner für alle Plugin-Zwischen-
/// speicher-Dateien, nur mit unterschiedlichem Dateiname-Präfix je Plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn snipeit_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("snipeit-{connection_id}.json"))
}

/// Schreibt eine Momentaufnahme des Sync-Ergebnisses als JSON-Datei, analog
/// zu `commands::plugins::write_ninja_cache`.
fn write_snipeit_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[SnipeitCompanyDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedSnipeitSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache).map_err(|e| {
        AppError::Plugin(format!("Snipe-IT-Cache konnte nicht kodiert werden: {e}"))
    })?;
    std::fs::write(snipeit_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Liest eine zuvor über `write_snipeit_cache` geschriebene Momentaufnahme
/// zurück. `Ok(None)`, wenn für diese Verbindung noch nie synchronisiert
/// wurde -- kein Fehlerfall, analog zu `commands::plugins::read_ninja_cache`.
fn read_snipeit_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedSnipeitSyncDto>, AppError> {
    let path = snipeit_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedSnipeitSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Snipe-IT-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn snipeit_device_url(base_url: &str, external_id: &str) -> String {
    format!("{}/hardware/{external_id}", base_url.trim_end_matches('/'))
}

fn to_external_system_dto(
    base_url: &str,
    device: SnipeitDevice,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        snipeit_url: snipeit_device_url(base_url, &device.external_id),
        external_id: device.external_id,
        name: device.name,
        hostname: device.hostname,
        ip_address: device.ip_address,
        asset_tag: device.asset_tag,
        serial: device.serial,
        linked_system_id,
    }
}

/// Zwischenergebnis der reinen Gruppierungslogik: eine Firma samt ihrer
/// Assets (noch als `SnipeitDevice`, nicht als DTO) und -- falls zugeordnet
/// -- der lokalen `customer_id`. Analog zu `commands::plugins::OrgGroup`.
struct CompanyGroup {
    company_id: String,
    company_name: String,
    customer_id: Option<i64>,
    devices: Vec<SnipeitDevice>,
}

/// Gruppiert Assets nach Firma und reichert jede Gruppe um die konfigurierte
/// `customer_id`-Zuordnung an (falls vorhanden). Reine Funktion -- kein
/// Netzwerk-, kein Datenbankzugriff -- deshalb mit hartkodierten
/// `SnipeitCompany`/`SnipeitDevice`/`SnipeitCompanyMapping`-Werten testbar,
/// analog zu `commands::plugins::group_devices_by_organization`. Eine Firma
/// ganz ohne Assets erscheint trotzdem als Gruppe (leere `devices`-Liste),
/// damit eine künftige UI sie zum Zuordnen anzeigen kann. Assets, deren
/// `company_id` auf keine von `companies` gemeldete Firma passt -- inklusive
/// `UNASSIGNED_COMPANY_ID` für Assets ganz ohne Firmenzuordnung in Snipe-IT
/// selbst --, werden nicht stillschweigend verworfen, sondern als eigene
/// Gruppe angehängt; für `UNASSIGNED_COMPANY_ID` mit einem lesbaren Namen
/// statt der rohen Sentinel-ID.
fn group_devices_by_company(
    companies: &[SnipeitCompany],
    devices: &[SnipeitDevice],
    mappings: &[SnipeitCompanyMapping],
    connection_id: &str,
) -> Vec<CompanyGroup> {
    let mapped_customer_id = |company_id: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.company_id == company_id)
            .map(|m| m.customer_id)
    };

    let mut devices_by_company: HashMap<String, Vec<SnipeitDevice>> = HashMap::new();
    for device in devices {
        devices_by_company
            .entry(device.company_id.clone())
            .or_default()
            .push(device.clone());
    }

    let mut groups = Vec::with_capacity(companies.len());
    for company in companies {
        let company_devices = devices_by_company.remove(&company.id).unwrap_or_default();
        groups.push(CompanyGroup {
            company_id: company.id.clone(),
            company_name: company.name.clone(),
            customer_id: mapped_customer_id(&company.id),
            devices: company_devices,
        });
    }

    let mut leftover_ids: Vec<String> = devices_by_company.keys().cloned().collect();
    leftover_ids.sort();
    for company_id in leftover_ids {
        if let Some(company_devices) = devices_by_company.remove(&company_id) {
            let company_name = if company_id == UNASSIGNED_COMPANY_ID {
                "Ohne Firma (Snipe-IT)".to_string()
            } else {
                company_id.clone()
            };
            groups.push(CompanyGroup {
                customer_id: mapped_customer_id(&company_id),
                company_name,
                company_id,
                devices: company_devices,
            });
        }
    }

    groups
}

#[tauri::command]
pub fn test_snipeit_connection(base_url: String, token: String) -> Result<(), AppError> {
    test_credentials(&base_url, &token)?;
    Ok(())
}

#[tauri::command]
pub fn list_snipeit_connections(
    state: State<AppState>,
) -> Result<Vec<SnipeitConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.snipeit_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_snipeit_connection(
    state: State<AppState>,
    label: String,
    base_url: String,
    token: String,
) -> Result<SnipeitConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    plugin::secrets::store_secret(&plugin_id, &token)?;

    let meta = SnipeitConnectionMeta {
        id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.snipeit_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_snipeit_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.snipeit_connections.len();
    config.snipeit_connections.retain(|c| c.id != id);
    if config.snipeit_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Snipe-IT-Verbindung {id} nicht gefunden"
        )));
    }
    // Aufräumen: Firmen-Zuordnungen dieser Verbindung sind ohne die
    // Verbindung bedeutungslos und würden sonst als Datenleiche liegen bleiben.
    config
        .snipeit_company_mappings
        .retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = snipeit_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Snipe-IT-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

/// Live-Abruf der Firmenliste einer Verbindung. Analog zu
/// `commands::plugins::list_ninja_organizations`, gehalten für Symmetrie und
/// einen möglichen Ersteinrichtungs-Anwendungsfall -- die normale Bedienung
/// des Frontends braucht diesen Befehl nicht (Cache-first, siehe
/// `get_cached_snipeit_sync`/`sync_snipeit_connection`).
#[tauri::command]
pub fn list_snipeit_companies(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<SnipeitCompanyDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<SnipeitCompanyMapping> = config
            .snipeit_company_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings)
    };

    let companies = plugin.list_companies(&credentials)?;
    Ok(companies
        .into_iter()
        .map(|company| {
            let mapped_customer_id = mappings
                .iter()
                .find(|m| m.company_id == company.id)
                .map(|m| m.customer_id);
            SnipeitCompanyDto {
                id: company.id,
                name: company.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_snipeit_company(
    state: State<AppState>,
    connection_id: String,
    company_id: String,
    company_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .snipeit_company_mappings
        .retain(|m| !(m.connection_id == connection_id && m.company_id == company_id));
    config.snipeit_company_mappings.push(SnipeitCompanyMapping {
        connection_id,
        company_id,
        company_name,
        customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_snipeit_company(
    state: State<AppState>,
    connection_id: String,
    company_id: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Kein Fehler, wenn keine passende Zuordnung existiert -- das Ergebnis
    // (keine Zuordnung mehr vorhanden) ist dasselbe, analog zu
    // `db::external_refs::delete`. Bestehende `external_refs`-Verknüpfungen
    // bleiben unangetastet: Entzuordnen einer Firma ist bewusst keine
    // automatische Entverknüpfung ihrer bereits verknüpften Assets.
    config
        .snipeit_company_mappings
        .retain(|m| !(m.connection_id == connection_id && m.company_id == company_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_snipeit_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<SnipeitCompanyDeviceGroupDto>, AppError> {
    let (base_url, plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<SnipeitCompanyMapping> = config
            .snipeit_company_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (
            meta.base_url,
            plugin,
            credentials,
            mappings,
            config.data_dir.clone(),
        )
    };

    let companies = plugin.list_companies(&credentials)?;
    let devices = plugin.list_devices(&credentials)?;
    let groups = group_devices_by_company(&companies, &devices, &mappings, &connection_id);

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    let mut result = Vec::with_capacity(groups.len());
    for group in groups {
        // Umkehr-Index externe-ID -> lokale system_id, nur für zugeordnete
        // Firmen aufgebaut -- ohne `customer_id` gibt es keine sinnvolle
        // Menge lokaler Systeme, gegen die man querverweisen könnte.
        let mut linked_by_external_id: HashMap<String, i64> = HashMap::new();
        if let Some(customer_id) = group.customer_id {
            for system in db::systems::list_by_customer(&conn, customer_id, true)? {
                for reference in db::external_refs::list_for_system(&conn, system.id)? {
                    if reference.plugin_id == plugin_id {
                        linked_by_external_id.insert(reference.external_id, system.id);
                    }
                }
            }
        }

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
            device_dtos.push(to_external_system_dto(&base_url, device, linked_system_id));
        }

        result.push(SnipeitCompanyDeviceGroupDto {
            company_id: group.company_id,
            company_name: group.company_name,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_snipeit_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_snipeit_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedSnipeitSyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<SnipeitCompanyMapping> = config
            .snipeit_company_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_snipeit_cache(&data_dir, &connection_id)?;
    // `customer_id` je Gruppe wird hier gegen die AKTUELLEN Firmen-
    // Zuordnungen neu verknüpft statt den in der Cache-Datei beim letzten
    // `sync_snipeit_connection`-Lauf eingefrorenen Wert zu übernehmen --
    // sonst würde ein Zu-/Entzuordnen einer Firma erst nach dem nächsten
    // Live-Sync sichtbar, obwohl genau dieser Befehl dem Frontend einen
    // aktuellen Zuordnungsstand ohne Netzwerkzugriff zeigen soll (analog zu
    // `commands::plugins::get_cached_ninja_sync`).
    if let Some(cache) = cached.as_mut() {
        for group in &mut cache.groups {
            group.customer_id = mappings
                .iter()
                .find(|m| m.company_id == group.company_id)
                .map(|m| m.customer_id);
        }
    }
    Ok(cached)
}

#[tauri::command]
pub fn link_system_to_snipeit(
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
pub fn unlink_system_from_snipeit(
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
pub fn get_snipeit_system_details(
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
        assert_eq!(slugify("***"), "snipeit");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_snipeit_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "snipeit:acme-123");
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
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "acme-1".to_string(),
            label: "ACME".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    #[test]
    fn snipeit_device_url_trims_trailing_slash_and_builds_hardware_link() {
        let url = snipeit_device_url("https://assets.example.com/", "101");
        assert_eq!(url, "https://assets.example.com/hardware/101");
    }

    fn sample_company(id: &str, name: &str) -> SnipeitCompany {
        SnipeitCompany {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn sample_device(id: &str, company_id: &str) -> SnipeitDevice {
        SnipeitDevice {
            external_id: id.to_string(),
            name: format!("Device {id}"),
            hostname: None,
            ip_address: None,
            asset_tag: Some(format!("AT-{id}")),
            serial: None,
            company_id: company_id.to_string(),
        }
    }

    #[test]
    fn groups_devices_under_their_company() {
        let companies = vec![
            sample_company("1", "ACME Hauptsitz"),
            sample_company("2", "ACME Zweigstelle"),
        ];
        let devices = vec![
            sample_device("101", "1"),
            sample_device("102", "1"),
            sample_device("201", "2"),
        ];

        let groups = group_devices_by_company(&companies, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].company_id, "1");
        assert_eq!(groups[0].devices.len(), 2);
        assert_eq!(groups[1].company_id, "2");
        assert_eq!(groups[1].devices.len(), 1);
    }

    #[test]
    fn company_without_devices_still_appears_as_empty_group() {
        let companies = vec![sample_company("1", "ACME Hauptsitz")];
        let groups = group_devices_by_company(&companies, &[], &[], "conn-1");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].devices.is_empty());
    }

    #[test]
    fn mapped_company_carries_its_customer_id() {
        let companies = vec![sample_company("1", "ACME Hauptsitz")];
        let mappings = vec![SnipeitCompanyMapping {
            connection_id: "conn-1".to_string(),
            company_id: "1".to_string(),
            company_name: "ACME Hauptsitz".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_company(&companies, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, Some(42));
    }

    #[test]
    fn unmapped_company_has_no_customer_id() {
        let companies = vec![sample_company("1", "ACME Hauptsitz")];
        let groups = group_devices_by_company(&companies, &[], &[], "conn-1");
        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn mapping_for_a_different_connection_is_ignored() {
        let companies = vec![sample_company("1", "ACME Hauptsitz")];
        let mappings = vec![SnipeitCompanyMapping {
            connection_id: "other-connection".to_string(),
            company_id: "1".to_string(),
            company_name: "ACME Hauptsitz".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_company(&companies, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn devices_for_an_unlisted_company_form_their_own_leftover_group() {
        let companies = vec![sample_company("1", "ACME Hauptsitz")];
        let devices = vec![sample_device("999", "orphan-company")];

        let groups = group_devices_by_company(&companies, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups
            .iter()
            .find(|g| g.company_id == "orphan-company")
            .unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.company_name, "orphan-company");
    }

    #[test]
    fn devices_without_any_company_form_a_friendly_named_leftover_group() {
        let companies = vec![sample_company("1", "ACME Hauptsitz")];
        let devices = vec![sample_device("999", UNASSIGNED_COMPANY_ID)];

        let groups = group_devices_by_company(&companies, &devices, &[], "conn-1");

        let leftover = groups
            .iter()
            .find(|g| g.company_id == UNASSIGNED_COMPANY_ID)
            .unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.company_name, "Ohne Firma (Snipe-IT)");
    }

    fn sample_group() -> SnipeitCompanyDeviceGroupDto {
        SnipeitCompanyDeviceGroupDto {
            company_id: "1".to_string(),
            company_name: "ACME Hauptsitz".to_string(),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "101".to_string(),
                name: "Server 01".to_string(),
                hostname: None,
                ip_address: None,
                asset_tag: Some("AT-101".to_string()),
                serial: Some("SN-101".to_string()),
                snipeit_url: "https://assets.example.com/hardware/101".to_string(),
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn snipeit_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_snipeit_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_snipeit_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].company_id, "1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "101");
        assert_eq!(
            loaded.groups[0].devices[0].asset_tag.as_deref(),
            Some("AT-101")
        );
    }

    #[test]
    fn snipeit_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_snipeit_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn snipeit_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_snipeit_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_snipeit_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_snipeit_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
