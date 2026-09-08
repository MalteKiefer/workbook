//! Reine Lesezugriffs-Aggregation über alle konfigurierten RMM-/Asset-
//! Management-Plugin-Verbindungen (Ninja, Level, Snipe-IT) für einen
//! gegebenen lokalen Kunden: liefert alle Geräte/Assets, die (a) laut den
//! zuletzt synchronisierten Plugin-Caches zu diesem Kunden gehören und (b)
//! noch mit keinem lokalen System verknüpft sind.
//!
//! Hintergrund: das System-Feld beim Anlegen eines Wartungseintrags
//! (`EntryEditor.tsx`/`QuickCapture.tsx`) durchsucht bisher nur bereits
//! existierende lokale `systems`-Zeilen -- Plugin-Geräte, die noch nie über
//! die Plugins-Seite manuell verknüpft wurden, sind dort unsichtbar. Dieses
//! Modul stellt die dafür nötige Auflistung bereit, rein offline (nur
//! `Config` + bereits auf der Platte liegende JSON-Cache-Dateien, kein
//! Netzwerkzugriff), damit sie bei jedem Tastendruck/Kundenwechsel im
//! Frontend aufgerufen werden kann, ohne auf Netzwerklatenz zu warten.
//!
//! Wiederverwendet bewusst die echten, bereits `Deserialize`-fähigen
//! Cache-DTOs der drei Plugin-Module (`commands::plugins::CachedNinjaSyncDto`,
//! `commands::level::CachedLevelSyncDto`, `commands::snipeit::CachedSnipeitSyncDto`),
//! statt die JSON-Form hier ein zweites Mal zu definieren -- so bleibt dieses
//! Modul automatisch synchron, falls sich eine dieser Formen künftig ändert.
//!
//! Die Cache-PFAD-Konvention (`data_dir/plugin-cache/<plugin>-<connection_id>.json`)
//! und das Lesen selbst (`std::fs::read_to_string` + `serde_json::from_str`)
//! sind hier trotzdem inline nachgebaut statt die `read_*_cache`-Hilfsfunktionen
//! der drei Module direkt aufzurufen: die sind dort bewusst privat (`fn`, nicht
//! `pub fn`), und dieses Änderungspaket darf `commands/plugins.rs`,
//! `commands/level.rs` und `commands/snipeit.rs` nicht anfassen (Aufteilung
//! mit einer parallel arbeitenden Änderung, die genau diese Dateien besitzt).
//! Ein `pub(crate)`-Aufweichen dort hätte diese Grenze verletzt; der
//! Pfad-Konvention und den echten DTO-Typen folgend nachzubauen ist die
//! einzige Option, die diese Grenze respektiert, ohne die JSON-FORM selbst zu
//! duplizieren -- dupliziert wird nur die triviale Ein-Zeiler-Pfadkonstruktion/
//! der Datei-Read selbst, exakt wie es die drei Plugin-Module ohnehin schon je
//! einmal für sich selbst tun (`plugin_cache_dir` ist zwischen allen dreien
//! bereits identisch dupliziert).

use std::path::{Path, PathBuf};

use tauri::State;

use crate::commands::level::CachedLevelSyncDto;
use crate::commands::plugins::CachedNinjaSyncDto;
use crate::commands::snipeit::CachedSnipeitSyncDto;
use crate::config::Config;
use crate::{AppError, AppState};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UnlinkedExternalSystemDto {
    pub plugin: String,
    pub connection_id: String,
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
}

/// Dasselbe `data_dir/plugin-cache/`-Verzeichnis, das `commands::plugins`/
/// `commands::level`/`commands::snipeit` jeweils für sich selbst verwenden.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn read_ninja_cache_file(data_dir: &Path, connection_id: &str) -> Result<Option<CachedNinjaSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("ninja-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedNinjaSyncDto =
        serde_json::from_str(&text).map_err(|e| AppError::Plugin(format!("Ninja-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_level_cache_file(data_dir: &Path, connection_id: &str) -> Result<Option<CachedLevelSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("level-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedLevelSyncDto =
        serde_json::from_str(&text).map_err(|e| AppError::Plugin(format!("Level-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_snipeit_cache_file(data_dir: &Path, connection_id: &str) -> Result<Option<CachedSnipeitSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("snipeit-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedSnipeitSyncDto =
        serde_json::from_str(&text).map_err(|e| AppError::Plugin(format!("Snipe-IT-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

/// Ninja: eine Organisation zählt für `customer_id`, wenn
/// `config.ninja_org_mappings` gerade JETZT eine passende Zeile für
/// (Verbindung, Organisation) hat -- nicht das evtl. veraltete, zum letzten
/// Sync-Zeitpunkt in der Cache-Datei eingefrorene `customer_id`-Feld. Sonst
/// würde ein frisches Zuordnen einer Organisation über die Plugins-Seite
/// hier erst nach dem nächsten manuellen Sync sichtbar -- genau das Problem,
/// das `commands::plugins::get_cached_ninja_sync` für denselben Cache aus
/// demselben Grund bereits löst (dortiger Kommentar), und genau der Zweck
/// dieser Funktion (Geräte "automatisch" auftauchen lassen) würde sonst für
/// gerade erst zugeordnete Organisationen verfehlt.
fn collect_ninja(config: &Config, data_dir: &Path, customer_id: i64, out: &mut Vec<UnlinkedExternalSystemDto>) -> Result<(), AppError> {
    for connection in &config.ninja_connections {
        let Some(cache) = read_ninja_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .ninja_org_mappings
                .iter()
                .find(|m| m.connection_id == connection.id && m.organization_id == group.organization_id)
                .map(|m| m.customer_id);
            if mapped_customer_id != Some(customer_id) {
                continue;
            }
            for device in &group.devices {
                if device.linked_system_id.is_some() {
                    continue;
                }
                out.push(UnlinkedExternalSystemDto {
                    plugin: "ninja".to_string(),
                    connection_id: connection.id.clone(),
                    external_id: device.external_id.clone(),
                    name: device.name.clone(),
                    hostname: device.hostname.clone(),
                    ip_address: device.ip_address.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Level: keine separate Organisations-/Zuordnungsebene -- eine Verbindung
/// gehört direkt zu genau einem Kunden (`LevelConnectionMeta.customer_id`),
/// siehe `commands::level`-Moduldokumentation. Kein Staleness-Problem wie bei
/// Ninja/Snipe-IT: `customer_id` ist ein direktes Verbindungsfeld, nicht in
/// einer Cache-Datei eingefroren.
fn collect_level(config: &Config, data_dir: &Path, customer_id: i64, out: &mut Vec<UnlinkedExternalSystemDto>) -> Result<(), AppError> {
    for connection in &config.level_connections {
        if connection.customer_id != customer_id {
            continue;
        }
        let Some(cache) = read_level_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for device in &cache.devices {
            if device.linked_system_id.is_some() {
                continue;
            }
            out.push(UnlinkedExternalSystemDto {
                plugin: "level".to_string(),
                connection_id: connection.id.clone(),
                external_id: device.external_id.clone(),
                name: device.name.clone(),
                hostname: device.hostname.clone(),
                ip_address: device.ip_address.clone(),
            });
        }
    }
    Ok(())
}

/// Snipe-IT: exakt dasselbe Muster wie Ninja (`collect_ninja`), inklusive
/// Neu-Abgleich gegen aktuelle `config.snipeit_company_mappings` statt des
/// eingefrorenen Cache-Werts. `hostname`/`ip_address` sind bei Snipe-IT-
/// Assets praktisch immer `None` (Snipe-IT kennt diese Felder nicht nativ,
/// siehe `plugin::snipeit`-Moduldokumentation) -- `name` ist trotzdem
/// sinnvoll gefüllt, weil Snipe-ITs eigene Geräte-Mapping-Logik
/// (`plugin::snipeit`) bereits selbst von `name` über `asset_tag` und
/// `serial` bis zur externen ID zurückfällt, bevor der Wert überhaupt in den
/// Cache gelangt.
fn collect_snipeit(config: &Config, data_dir: &Path, customer_id: i64, out: &mut Vec<UnlinkedExternalSystemDto>) -> Result<(), AppError> {
    for connection in &config.snipeit_connections {
        let Some(cache) = read_snipeit_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .snipeit_company_mappings
                .iter()
                .find(|m| m.connection_id == connection.id && m.company_id == group.company_id)
                .map(|m| m.customer_id);
            if mapped_customer_id != Some(customer_id) {
                continue;
            }
            for device in &group.devices {
                if device.linked_system_id.is_some() {
                    continue;
                }
                out.push(UnlinkedExternalSystemDto {
                    plugin: "snipeit".to_string(),
                    connection_id: connection.id.clone(),
                    external_id: device.external_id.clone(),
                    name: device.name.clone(),
                    hostname: device.hostname.clone(),
                    ip_address: device.ip_address.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Reine Kernlogik, ohne `State<AppState>` -- testbar mit einer
/// hartkodierten `Config` plus einem `tempfile::tempdir()`, analog zu
/// `commands::plugins::group_devices_by_organization`. Der
/// `#[tauri::command]`-Wrapper unten holt lediglich eine geklonte `Config`
/// (geklont, damit der Config-Mutex nicht während der Datei-I/O gehalten
/// bleibt -- siehe `commands::plugins::get_cached_ninja_sync`, das genau
/// dasselbe für denselben Zweck tut) aus `State<AppState>` und reicht sie
/// hier durch. Reine Funktion -- kein Netzwerk-, kein Datenbankzugriff,
/// nur `Config` (bereits im Speicher) plus bereits auf der Platte liegende
/// JSON-Dateien.
pub fn list_unlinked_external_systems_for_customer_pure(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
) -> Result<Vec<UnlinkedExternalSystemDto>, AppError> {
    let mut result = Vec::new();
    collect_ninja(config, data_dir, customer_id, &mut result)?;
    collect_level(config, data_dir, customer_id, &mut result)?;
    collect_snipeit(config, data_dir, customer_id, &mut result)?;
    Ok(result)
}

#[tauri::command]
pub fn list_unlinked_external_systems_for_customer(
    state: State<AppState>,
    customer_id: i64,
) -> Result<Vec<UnlinkedExternalSystemDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet").clone();
    list_unlinked_external_systems_for_customer_pure(&config, &config.data_dir, customer_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::commands::level::ExternalSystemDto as LevelExternalSystemDto;
    use crate::commands::plugins::{ExternalSystemDto as NinjaExternalSystemDto, NinjaOrgDeviceGroupDto};
    use crate::commands::snipeit::{ExternalSystemDto as SnipeitExternalSystemDto, SnipeitCompanyDeviceGroupDto};
    use crate::plugin::level::LevelConnectionMeta;
    use crate::plugin::ninja::{NinjaConnectionMeta, NinjaOrgMapping};
    use crate::plugin::snipeit::{SnipeitCompanyMapping, SnipeitConnectionMeta};

    fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    fn ninja_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("ninja-{connection_id}.json"))
    }

    fn level_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("level-{connection_id}.json"))
    }

    fn snipeit_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("snipeit-{connection_id}.json"))
    }

    fn ninja_device(external_id: &str, linked_system_id: Option<i64>) -> NinjaExternalSystemDto {
        NinjaExternalSystemDto {
            external_id: external_id.to_string(),
            name: format!("Ninja Device {external_id}"),
            hostname: Some(format!("{external_id}.local")),
            ip_address: Some("10.0.0.5".to_string()),
            ninja_url: format!("https://eu.ninjarmm.com/#/deviceDashboard/{external_id}/overview"),
            linked_system_id,
        }
    }

    fn ninja_config_with_connection(connection_id: &str, organization_id: &str, mapped_customer_id: Option<i64>) -> Config {
        let mut config = Config::default();
        config.ninja_connections.push(NinjaConnectionMeta {
            id: connection_id.to_string(),
            label: "ACME Ninja".to_string(),
            base_url: "https://eu.ninjarmm.com".to_string(),
        });
        if let Some(customer_id) = mapped_customer_id {
            config.ninja_org_mappings.push(NinjaOrgMapping {
                connection_id: connection_id.to_string(),
                organization_id: organization_id.to_string(),
                organization_name: "ACME Hauptsitz".to_string(),
                customer_id,
            });
        }
        config
    }

    #[test]
    fn ninja_device_appears_when_its_organization_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let config = ninja_config_with_connection("conn-1", "org-1", Some(42));
        let cache = CachedNinjaSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![NinjaOrgDeviceGroupDto {
                organization_id: "org-1".to_string(),
                organization_name: "ACME Hauptsitz".to_string(),
                // Bewusst veraltet/`None` gelassen -- die Zuordnung kommt aus
                // `config.ninja_org_mappings`, nicht aus diesem eingefrorenen
                // Feld (siehe Kommentar an `collect_ninja`).
                customer_id: None,
                devices: vec![ninja_device("dev-1", None)],
            }],
        };
        write_json(&ninja_cache_path(dir.path(), "conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "ninja");
        assert_eq!(result[0].connection_id, "conn-1");
        assert_eq!(result[0].external_id, "dev-1");
        assert_eq!(result[0].hostname.as_deref(), Some("dev-1.local"));
    }

    #[test]
    fn already_linked_ninja_device_is_excluded() {
        let dir = tempdir().unwrap();
        let config = ninja_config_with_connection("conn-1", "org-1", Some(42));
        let cache = CachedNinjaSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![NinjaOrgDeviceGroupDto {
                organization_id: "org-1".to_string(),
                organization_name: "ACME Hauptsitz".to_string(),
                customer_id: Some(42),
                devices: vec![ninja_device("dev-1", Some(7))],
            }],
        };
        write_json(&ninja_cache_path(dir.path(), "conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn ninja_device_mapped_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        // Organisation ist Kunde 99 zugeordnet, wir fragen nach Kunde 42.
        let config = ninja_config_with_connection("conn-1", "org-1", Some(99));
        let cache = CachedNinjaSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![NinjaOrgDeviceGroupDto {
                organization_id: "org-1".to_string(),
                organization_name: "ACME Hauptsitz".to_string(),
                customer_id: Some(99),
                devices: vec![ninja_device("dev-1", None)],
            }],
        };
        write_json(&ninja_cache_path(dir.path(), "conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn unmapped_ninja_organization_is_excluded() {
        let dir = tempdir().unwrap();
        // Verbindung existiert, aber die Organisation ist (noch) gar keinem
        // Kunden zugeordnet -- `mapped_customer_id` bleibt `None`.
        let config = ninja_config_with_connection("conn-1", "org-1", None);
        let cache = CachedNinjaSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![NinjaOrgDeviceGroupDto {
                organization_id: "org-1".to_string(),
                organization_name: "ACME Hauptsitz".to_string(),
                customer_id: None,
                devices: vec![ninja_device("dev-1", None)],
            }],
        };
        write_json(&ninja_cache_path(dir.path(), "conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn ninja_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        // Keine Cache-Datei für "conn-1" geschrieben -- simuliert "noch nie
        // synchronisiert".
        let config = ninja_config_with_connection("conn-1", "org-1", Some(42));

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn level_device_appears_when_its_connection_is_bound_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.level_connections.push(LevelConnectionMeta {
            id: "level-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Level".to_string(),
        });
        let cache = CachedLevelSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![LevelExternalSystemDto {
                external_id: "lvl-1".to_string(),
                name: "Level Device 1".to_string(),
                hostname: Some("lvl-1.local".to_string()),
                ip_address: Some("10.0.0.9".to_string()),
                linked_system_id: None,
                group_id: Some("grp-1".to_string()),
                group_name: Some("Werkstatt".to_string()),
            }],
        };
        write_json(&level_cache_path(dir.path(), "level-conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "level");
        assert_eq!(result[0].connection_id, "level-conn-1");
        assert_eq!(result[0].external_id, "lvl-1");
    }

    #[test]
    fn already_linked_level_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.level_connections.push(LevelConnectionMeta {
            id: "level-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Level".to_string(),
        });
        let cache = CachedLevelSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![LevelExternalSystemDto {
                external_id: "lvl-1".to_string(),
                name: "Level Device 1".to_string(),
                hostname: None,
                ip_address: None,
                linked_system_id: Some(3),
                group_id: None,
                group_name: None,
            }],
        };
        write_json(&level_cache_path(dir.path(), "level-conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn level_connection_bound_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.level_connections.push(LevelConnectionMeta {
            id: "level-conn-1".to_string(),
            customer_id: 99,
            label: "ACME Level".to_string(),
        });
        let cache = CachedLevelSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![LevelExternalSystemDto {
                external_id: "lvl-1".to_string(),
                name: "Level Device 1".to_string(),
                hostname: None,
                ip_address: None,
                linked_system_id: None,
                group_id: None,
                group_name: None,
            }],
        };
        write_json(&level_cache_path(dir.path(), "level-conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn level_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.level_connections.push(LevelConnectionMeta {
            id: "level-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Level".to_string(),
        });

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn snipeit_device_appears_when_its_company_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "snipeit-conn-1".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });
        config.snipeit_company_mappings.push(SnipeitCompanyMapping {
            connection_id: "snipeit-conn-1".to_string(),
            company_id: "company-1".to_string(),
            company_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedSnipeitSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![SnipeitCompanyDeviceGroupDto {
                company_id: "company-1".to_string(),
                company_name: "ACME GmbH".to_string(),
                customer_id: None,
                devices: vec![SnipeitExternalSystemDto {
                    external_id: "asset-1".to_string(),
                    name: "AT-0001".to_string(),
                    hostname: None,
                    ip_address: None,
                    asset_tag: Some("AT-0001".to_string()),
                    serial: Some("SN-0001".to_string()),
                    snipeit_url: "https://assets.example.com/hardware/asset-1".to_string(),
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&snipeit_cache_path(dir.path(), "snipeit-conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "snipeit");
        assert_eq!(result[0].name, "AT-0001");
        assert_eq!(result[0].hostname, None);
    }

    #[test]
    fn already_linked_snipeit_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "snipeit-conn-1".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });
        config.snipeit_company_mappings.push(SnipeitCompanyMapping {
            connection_id: "snipeit-conn-1".to_string(),
            company_id: "company-1".to_string(),
            company_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedSnipeitSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![SnipeitCompanyDeviceGroupDto {
                company_id: "company-1".to_string(),
                company_name: "ACME GmbH".to_string(),
                customer_id: Some(5),
                devices: vec![SnipeitExternalSystemDto {
                    external_id: "asset-1".to_string(),
                    name: "AT-0001".to_string(),
                    hostname: None,
                    ip_address: None,
                    asset_tag: Some("AT-0001".to_string()),
                    serial: Some("SN-0001".to_string()),
                    snipeit_url: "https://assets.example.com/hardware/asset-1".to_string(),
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&snipeit_cache_path(dir.path(), "snipeit-conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn snipeit_company_mapped_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "snipeit-conn-1".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });
        config.snipeit_company_mappings.push(SnipeitCompanyMapping {
            connection_id: "snipeit-conn-1".to_string(),
            company_id: "company-1".to_string(),
            company_name: "ACME GmbH".to_string(),
            customer_id: 99,
        });
        let cache = CachedSnipeitSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![SnipeitCompanyDeviceGroupDto {
                company_id: "company-1".to_string(),
                company_name: "ACME GmbH".to_string(),
                customer_id: Some(99),
                devices: vec![SnipeitExternalSystemDto {
                    external_id: "asset-1".to_string(),
                    name: "AT-0001".to_string(),
                    hostname: None,
                    ip_address: None,
                    asset_tag: Some("AT-0001".to_string()),
                    serial: None,
                    snipeit_url: "https://assets.example.com/hardware/asset-1".to_string(),
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&snipeit_cache_path(dir.path(), "snipeit-conn-1"), &cache);

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn snipeit_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "snipeit-conn-1".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });
        config.snipeit_company_mappings.push(SnipeitCompanyMapping {
            connection_id: "snipeit-conn-1".to_string(),
            company_id: "company-1".to_string(),
            company_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn results_from_multiple_plugins_are_combined_for_the_same_customer() {
        let dir = tempdir().unwrap();
        let mut config = ninja_config_with_connection("conn-1", "org-1", Some(42));
        config.level_connections.push(LevelConnectionMeta {
            id: "level-conn-1".to_string(),
            customer_id: 42,
            label: "ACME Level".to_string(),
        });

        write_json(
            &ninja_cache_path(dir.path(), "conn-1"),
            &CachedNinjaSyncDto {
                synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
                groups: vec![NinjaOrgDeviceGroupDto {
                    organization_id: "org-1".to_string(),
                    organization_name: "ACME Hauptsitz".to_string(),
                    customer_id: Some(42),
                    devices: vec![ninja_device("dev-1", None)],
                }],
            },
        );
        write_json(
            &level_cache_path(dir.path(), "level-conn-1"),
            &CachedLevelSyncDto {
                synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
                devices: vec![LevelExternalSystemDto {
                    external_id: "lvl-1".to_string(),
                    name: "Level Device 1".to_string(),
                    hostname: None,
                    ip_address: None,
                    linked_system_id: None,
                    group_id: None,
                    group_name: None,
                }],
            },
        );

        let mut result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();
        result.sort_by(|a, b| a.plugin.cmp(&b.plugin));

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].plugin, "level");
        assert_eq!(result[1].plugin, "ninja");
    }
}
