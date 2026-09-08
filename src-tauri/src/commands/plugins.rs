//! Tauri-Kommandos für die NinjaOne-Plugin-Integration (siehe
//! `plugin::ninja` und `docs/PLUGIN_ARCHITECTURE.md`). Dünne Wrapper nach dem
//! Muster von `commands::export`: hier nur `State<AppState>` entgegennehmen,
//! eine gepoolte Verbindung/den Config-Mutex holen und in die eigentliche
//! Logik (Plugin-Trait, `db::external_refs`, `Config`) durchreichen.
//!
//! Eine Ninja-"Verbindung" ist ein vom Nutzer angelegter Datensatz (Basis-URL +
//! Zugangsdaten) für genau einen Ninja-Mandanten -- NICHT für genau einen
//! lokalen Kunden. Ein Ninja-Mandant modelliert selbst mehrere
//! "Organizations" (z. B. weil der Nutzer, der die Verbindung anlegt, selbst
//! ein MSP ist und mehrere eigene Kunden als getrennte Organisationen in
//! Ninja führt). Welche Organisation welchem lokalen Kunden entspricht (falls
//! überhaupt), ist eine separate, granulare Zuordnung
//! (`NinjaOrgMapping`/`Config::ninja_org_mappings`), die dieses Modul über
//! `map_ninja_organization`/`unmap_ninja_organization` pflegt. Der
//! vollqualifizierte Bezeichner `"ninja:<connection_id>"` dient sowohl als
//! Schlüsselspeicher-Konto (`plugin::secrets`) als auch als
//! `external_refs.plugin_id`, sodass die bestehende
//! Ein-Zeile-je-(system_id,plugin_id)-Upsert-Semantik unverändert
//! weiterfunktioniert.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::ninja::{
    test_credentials, NinjaConnectionMeta, NinjaCredentials, NinjaDevice, NinjaOrgMapping,
    NinjaOrganization, NinjaPlugin,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct NinjaConnectionDto {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    /// Erste Adresse aus Ninjas `ipAddresses`-Array, mit `publicIP` als
    /// Rückfallebene (siehe `plugin::ninja::map_ninja_device`).
    pub ip_address: Option<String>,
    /// Direktlink auf das Geräte-Dashboard in der Ninja-Weboberfläche, aus
    /// `base_url` der Verbindung und der externen Geräte-ID konstruiert.
    pub ninja_url: String,
    /// `Some(id)`, wenn irgendein lokales System bereits mit dieser externen
    /// ID für diese Verbindung verknüpft ist (`external_refs`-Zeile mit
    /// passendem `plugin_id`/`external_id`), sonst `None`. Bei Geräten einer
    /// nicht zugeordneten Organisation immer `None` -- ohne `customer_id`
    /// lässt sich nicht sinnvoll gegen `external_refs` querverweisen.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NinjaOrganizationDto {
    pub id: String,
    pub name: String,
    /// `None`, solange diese Organisation noch keinem lokalen Kunden
    /// zugeordnet wurde (`Config::ninja_org_mappings` hat keine passende
    /// Zeile für diese Verbindung+Organisation).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NinjaOrgDeviceGroupDto {
    pub organization_id: String,
    pub organization_name: String,
    /// `None`, wenn diese Organisation (noch) keinem lokalen Kunden
    /// zugeordnet ist -- ein künftiges Frontend soll in diesem Fall
    /// "nicht zugeordnet" anzeigen und das Verknüpfen der Geräte dieser
    /// Gruppe deaktivieren.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Momentaufnahme des letzten `sync_ninja_connection`-Laufs, unter
/// `data_dir/plugin-cache/ninja-<connection_id>.json` zwischengespeichert
/// (siehe `write_ninja_cache`/`read_ninja_cache`), damit
/// `get_cached_ninja_sync` ohne Netzwerkzugriff funktioniert. Braucht neben
/// `Serialize` (zum Schreiben) auch `Deserialize` (zum Wiedereinlesen) --
/// beides für den Cache-Datei-Roundtrip nötig.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedNinjaSyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<NinjaOrgDeviceGroupDto>,
}

fn to_dto(meta: &NinjaConnectionMeta) -> NinjaConnectionDto {
    NinjaConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// Der vollqualifizierte `plugin_id`-Wert für eine Ninja-Verbindung -- sowohl
/// Schlüsselspeicher-Konto als auch `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("ninja:{connection_id}")
}

/// Erzeugt aus einem Nutzer-Label eine stabile, kollisionsarme
/// Verbindungs-ID: ein URL-/Dateiname-taugliches Slug des Labels plus ein
/// Millisekunden-Zeitstempel-Suffix. Kein zusätzliches `uuid`-Crate nötig --
/// diese IDs werden selten (interaktiv, "Verbindung anlegen") erzeugt, nie in
/// einer heißen Schleife, ein Zeitstempel reicht als Eindeutigkeitsgarantie.
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
        slug.push_str("ninja");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<NinjaConnectionMeta, AppError> {
    config
        .ninja_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("Ninja-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Baut aus einer Verbindungs-Metadatenzeile das lauffähige Plugin-Objekt
/// plus die dazugehörigen Zugangsdaten aus dem Schlüsselspeicher.
fn build_plugin(meta: &NinjaConnectionMeta) -> Result<(NinjaPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Ninja-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        NinjaPlugin::new(plugin_id, meta.base_url.clone()),
        PluginCredentials { secret },
    ))
}

/// `plugin::secrets` bietet bewusst nur `store_secret`/`load_secret` (siehe
/// Credential-Prinzip in `docs/PLUGIN_ARCHITECTURE.md`) -- kein Löschen, weil
/// dieses Modul außerhalb des Aufgabenbereichs dieser Änderung liegt. Für den
/// seltenen "Verbindung entfernen"-Fall reicht ein direkter, lokaler Zugriff
/// mit demselben Service-Namen (`"wartungsdoku"`), rein bestes Bemühen: ein
/// fehlendes oder nicht löschbares Schlüsselspeicher-Konto darf die gesamte
/// `remove_ninja_connection`-Aktion nicht scheitern lassen.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Verzeichnis für plugin-spezifische Zwischenspeicher-Dateien
/// (`data_dir/plugin-cache/`). Aktuell nur für Ninja-Sync-Momentaufnahmen
/// genutzt, aber bewusst nicht `ninja-cache` genannt -- ein künftiges
/// weiteres Plugin kann denselben Ordner mitbenutzen.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn ninja_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("ninja-{connection_id}.json"))
}

/// Schreibt eine Momentaufnahme des Sync-Ergebnisses als JSON-Datei, für ein
/// späteres Offline-Auslesen über `read_ninja_cache`/`get_cached_ninja_sync`.
/// Überschreibt eine evtl. vorhandene ältere Momentaufnahme für dieselbe
/// Verbindung. Als eigene Funktion herausgezogen (statt Inline-Code in
/// `sync_ninja_connection`), damit sie sich mit `tempfile::tempdir()` isoliert
/// testen lässt, analog zu `Config::save`/`Config::load_or_default`.
fn write_ninja_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[NinjaOrgDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedNinjaSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Ninja-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(ninja_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Liest eine zuvor über `write_ninja_cache` geschriebene Momentaufnahme
/// zurück. `Ok(None)`, wenn für diese Verbindung noch nie synchronisiert
/// wurde (Datei existiert nicht) -- kein Fehlerfall. Als eigene Funktion
/// herausgezogen (statt Inline-Code im Tauri-Kommando), damit sie sich ohne
/// `State<AppState>` isoliert testen lässt.
fn read_ninja_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedNinjaSyncDto>, AppError> {
    let path = ninja_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedNinjaSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Ninja-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn ninja_device_url(base_url: &str, external_id: &str) -> String {
    format!(
        "{}/#/deviceDashboard/{external_id}/overview",
        base_url.trim_end_matches('/')
    )
}

fn to_external_system_dto(
    base_url: &str,
    device: NinjaDevice,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        ninja_url: ninja_device_url(base_url, &device.external_id),
        external_id: device.external_id,
        name: device.name,
        hostname: device.hostname,
        ip_address: device.ip_address,
        linked_system_id,
    }
}

/// Zwischenergebnis der reinen Gruppierungslogik: eine Organisation samt
/// ihrer Geräte (noch als `NinjaDevice`, nicht als DTO) und -- falls
/// zugeordnet -- der lokalen `customer_id`. Getrennt von
/// `NinjaOrgDeviceGroupDto`, weil Letzteres bereits fertige `ExternalSystemDto`-
/// Werte erwartet (inkl. `ninja_url`/`linked_system_id`), die erst nach dieser
/// Gruppierung gebaut werden können (`ninja_url` braucht `base_url`,
/// `linked_system_id` braucht einen Datenbankzugriff).
struct OrgGroup {
    organization_id: String,
    organization_name: String,
    customer_id: Option<i64>,
    devices: Vec<NinjaDevice>,
}

/// Gruppiert Geräte nach Organisation und reichert jede Gruppe um die
/// konfigurierte `customer_id`-Zuordnung an (falls vorhanden). Reine
/// Funktion -- kein Netzwerk-, kein Datenbankzugriff -- deshalb mit
/// hartkodierten `NinjaOrganization`/`NinjaDevice`/`NinjaOrgMapping`-Werten
/// testbar. Eine Organisation ganz ohne Geräte erscheint trotzdem als Gruppe
/// (leere `devices`-Liste), damit eine künftige UI sie zum Zuordnen anzeigen
/// kann. Geräte, deren `organizationId` auf keine von `organizations`
/// gemeldete Organisation passt (sollte laut Ninjas Datenmodell nicht
/// vorkommen), werden nicht stillschweigend verworfen, sondern als eigene
/// Gruppe unter der rohen Organisations-ID angehängt.
fn group_devices_by_organization(
    organizations: &[NinjaOrganization],
    devices: &[NinjaDevice],
    mappings: &[NinjaOrgMapping],
    connection_id: &str,
) -> Vec<OrgGroup> {
    let mapped_customer_id = |organization_id: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.organization_id == organization_id)
            .map(|m| m.customer_id)
    };

    let mut devices_by_org: HashMap<String, Vec<NinjaDevice>> = HashMap::new();
    for device in devices {
        devices_by_org
            .entry(device.organization_id.clone())
            .or_default()
            .push(device.clone());
    }

    let mut groups = Vec::with_capacity(organizations.len());
    for org in organizations {
        let org_devices = devices_by_org.remove(&org.id).unwrap_or_default();
        groups.push(OrgGroup {
            organization_id: org.id.clone(),
            organization_name: org.name.clone(),
            customer_id: mapped_customer_id(&org.id),
            devices: org_devices,
        });
    }

    let mut leftover_org_ids: Vec<String> = devices_by_org.keys().cloned().collect();
    leftover_org_ids.sort();
    for organization_id in leftover_org_ids {
        if let Some(org_devices) = devices_by_org.remove(&organization_id) {
            groups.push(OrgGroup {
                customer_id: mapped_customer_id(&organization_id),
                organization_name: organization_id.clone(),
                organization_id,
                devices: org_devices,
            });
        }
    }

    groups
}

#[tauri::command]
pub fn test_ninja_connection(
    base_url: String,
    client_id: String,
    client_secret: String,
) -> Result<(), AppError> {
    test_credentials(&base_url, &client_id, &client_secret)?;
    Ok(())
}

#[tauri::command]
pub fn list_ninja_connections(state: State<AppState>) -> Result<Vec<NinjaConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.ninja_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_ninja_connection(
    state: State<AppState>,
    label: String,
    base_url: String,
    client_id: String,
    client_secret: String,
) -> Result<NinjaConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    let secret_json = serde_json::to_string(&NinjaCredentials {
        client_id,
        client_secret,
    })
    .map_err(|e| AppError::Plugin(format!("Zugangsdaten konnten nicht kodiert werden: {e}")))?;
    plugin::secrets::store_secret(&plugin_id, &secret_json)?;

    let meta = NinjaConnectionMeta {
        id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.ninja_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_ninja_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.ninja_connections.len();
    config.ninja_connections.retain(|c| c.id != id);
    if config.ninja_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Ninja-Verbindung {id} nicht gefunden"
        )));
    }
    // Aufräumen: Organisations-Zuordnungen dieser Verbindung sind ohne die
    // Verbindung bedeutungslos und würden sonst als Datenleiche liegen bleiben.
    config.ninja_org_mappings.retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    // Ebenfalls bestes Bemühen: eine übrig gebliebene Cache-Datei für eine
    // entfernte Verbindung ist nur totes Gewicht, ihr Fehlen aber unschädlich
    // (Erstellen einer neuen Verbindung mit derselben ID ist praktisch
    // ausgeschlossen, siehe `generate_connection_id`).
    let cache_path = ninja_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Ninja-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_ninja_organizations(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<NinjaOrganizationDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<NinjaOrgMapping> = config
            .ninja_org_mappings
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
            NinjaOrganizationDto {
                id: org.id,
                name: org.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_ninja_organization(
    state: State<AppState>,
    connection_id: String,
    organization_id: String,
    organization_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .ninja_org_mappings
        .retain(|m| !(m.connection_id == connection_id && m.organization_id == organization_id));
    config.ninja_org_mappings.push(NinjaOrgMapping {
        connection_id,
        organization_id,
        organization_name,
        customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_ninja_organization(
    state: State<AppState>,
    connection_id: String,
    organization_id: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Kein Fehler, wenn keine passende Zuordnung existiert -- das Ergebnis
    // (keine Zuordnung mehr vorhanden) ist dasselbe, analog zu
    // `db::external_refs::delete`. Bestehende `external_refs`-Verknüpfungen
    // bleiben unangetastet: Entzuordnen einer Organisation ist bewusst keine
    // automatische Entverknüpfung ihrer bereits verknüpften Geräte.
    config
        .ninja_org_mappings
        .retain(|m| !(m.connection_id == connection_id && m.organization_id == organization_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_ninja_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<NinjaOrgDeviceGroupDto>, AppError> {
    let (base_url, plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<NinjaOrgMapping> = config
            .ninja_org_mappings
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

    let organizations = plugin.list_organizations(&credentials)?;
    let devices = plugin.list_devices(&credentials)?;
    let groups = group_devices_by_organization(&organizations, &devices, &mappings, &connection_id);

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Umkehr-Index externe-ID -> lokale system_id, aus ALLEN external_refs
    // dieses Plugins aufgebaut -- EINMAL für die ganze Verbindung, nicht neu
    // je Gruppe und nicht auf die Systeme des Gruppen-`customer_id`
    // beschränkt. Vorher wurde hier je Gruppe nur innerhalb
    // `list_by_customer(group.customer_id)` gesucht; das ließ ein tatsächlich
    // verknüpftes Gerät fälschlich als "nicht verknüpft" (linked_system_id:
    // None) erscheinen, sobald seine Organisation NACH dem Verknüpfen einem
    // ANDEREN Kunden zugeordnet wurde (z. B. weil die ursprüngliche Zuordnung
    // ein Versehen war und korrigiert wurde) -- das verknüpfte System liegt
    // dann unter dem alten Kunden, nicht unter `group.customer_id`, wurde
    // also nie gefunden. `db::external_refs::list_for_plugin` sucht bewusst
    // kundenunabhängig (siehe deren Doc-Kommentar), passend dazu, dass
    // `unmap_ninja_organization` Verknüpfungen ausdrücklich NICHT antastet,
    // wenn sich nur die Zuordnung ändert.
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
            device_dtos.push(to_external_system_dto(&base_url, device, linked_system_id));
        }

        result.push(NinjaOrgDeviceGroupDto {
            organization_id: group.organization_id,
            organization_name: group.organization_name,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_ninja_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_ninja_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedNinjaSyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<NinjaOrgMapping> = config
            .ninja_org_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_ninja_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-joined against the CURRENT org mappings
    // here rather than trusting the value frozen into the cache file at the
    // last `sync_ninja_connection` run -- otherwise mapping/unmapping an
    // organization would only be reflected after the next live sync, even
    // though the whole point of this command is to let the frontend show an
    // up-to-date mapping state without forcing a network call.
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
pub fn link_system_to_ninja(
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
pub fn unlink_system_from_ninja(
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
pub fn get_ninja_system_details(
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
        assert_eq!(slugify("***"), "ninja");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_ninja_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "ninja:acme-123");
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
        config.ninja_connections.push(NinjaConnectionMeta {
            id: "acme-1".to_string(),
            label: "ACME".to_string(),
            base_url: "https://eu.ninjarmm.com".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    #[test]
    fn ninja_device_url_trims_trailing_slash_and_builds_dashboard_link() {
        let url = ninja_device_url("https://eu.ninjarmm.com/", "101");
        assert_eq!(
            url,
            "https://eu.ninjarmm.com/#/deviceDashboard/101/overview"
        );
    }

    fn sample_org(id: &str, name: &str) -> NinjaOrganization {
        NinjaOrganization {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn sample_device(id: &str, org_id: &str) -> NinjaDevice {
        NinjaDevice {
            external_id: id.to_string(),
            name: format!("Device {id}"),
            hostname: None,
            ip_address: None,
            organization_id: org_id.to_string(),
        }
    }

    #[test]
    fn groups_devices_under_their_organization() {
        let organizations = vec![
            sample_org("1", "ACME Hauptsitz"),
            sample_org("2", "ACME Zweigstelle"),
        ];
        let devices = vec![
            sample_device("101", "1"),
            sample_device("102", "1"),
            sample_device("201", "2"),
        ];

        let groups = group_devices_by_organization(&organizations, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].organization_id, "1");
        assert_eq!(groups[0].devices.len(), 2);
        assert_eq!(groups[1].organization_id, "2");
        assert_eq!(groups[1].devices.len(), 1);
    }

    #[test]
    fn organization_without_devices_still_appears_as_empty_group() {
        let organizations = vec![sample_org("1", "ACME Hauptsitz")];
        let groups = group_devices_by_organization(&organizations, &[], &[], "conn-1");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].devices.is_empty());
    }

    #[test]
    fn mapped_organization_carries_its_customer_id() {
        let organizations = vec![sample_org("1", "ACME Hauptsitz")];
        let mappings = vec![NinjaOrgMapping {
            connection_id: "conn-1".to_string(),
            organization_id: "1".to_string(),
            organization_name: "ACME Hauptsitz".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_organization(&organizations, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, Some(42));
    }

    #[test]
    fn unmapped_organization_has_no_customer_id() {
        let organizations = vec![sample_org("1", "ACME Hauptsitz")];
        let groups = group_devices_by_organization(&organizations, &[], &[], "conn-1");
        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn mapping_for_a_different_connection_is_ignored() {
        let organizations = vec![sample_org("1", "ACME Hauptsitz")];
        let mappings = vec![NinjaOrgMapping {
            connection_id: "other-connection".to_string(),
            organization_id: "1".to_string(),
            organization_name: "ACME Hauptsitz".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_organization(&organizations, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn devices_for_an_unlisted_organization_form_their_own_leftover_group() {
        let organizations = vec![sample_org("1", "ACME Hauptsitz")];
        let devices = vec![sample_device("999", "orphan-org")];

        let groups = group_devices_by_organization(&organizations, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups
            .iter()
            .find(|g| g.organization_id == "orphan-org")
            .unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.organization_name, "orphan-org");
    }

    fn sample_group() -> NinjaOrgDeviceGroupDto {
        NinjaOrgDeviceGroupDto {
            organization_id: "1".to_string(),
            organization_name: "ACME Hauptsitz".to_string(),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "101".to_string(),
                name: "Server 01".to_string(),
                hostname: Some("srv-01.local".to_string()),
                ip_address: Some("10.0.0.5".to_string()),
                ninja_url: "https://eu.ninjarmm.com/#/deviceDashboard/101/overview".to_string(),
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn ninja_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_ninja_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_ninja_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].organization_id, "1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "101");
        assert_eq!(
            loaded.groups[0].devices[0].ip_address.as_deref(),
            Some("10.0.0.5")
        );
    }

    #[test]
    fn ninja_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_ninja_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn ninja_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_ninja_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_ninja_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_ninja_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
