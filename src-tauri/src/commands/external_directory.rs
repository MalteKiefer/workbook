//! Pure read-access aggregation across all configured RMM/asset management
//! plugin connections (Ninja, Level, Snipe-IT, Tactical RMM) for a given
//! local customer: returns all devices/assets that (a) belong to this
//! customer according to the most recently synced plugin caches, and (b) are
//! not yet linked to any local system.
//!
//! Background: the system field when creating a maintenance entry
//! (`EntryEditor.tsx`/`QuickCapture.tsx`) has so far only searched existing
//! local `systems` rows -- plugin devices that were never manually linked via
//! the Plugins page are invisible there. This module provides the listing
//! needed for that, purely offline (only `Config` plus JSON cache files
//! already on disk, no network access), so it can be called on every
//! keystroke/customer switch in the frontend without waiting on network
//! latency.
//!
//! Deliberately reuses the real, already `Deserialize`-capable cache DTOs of
//! the four plugin modules (`commands::plugins::CachedNinjaSyncDto`,
//! `commands::level::CachedLevelSyncDto`, `commands::snipeit::CachedSnipeitSyncDto`,
//! `commands::tacticalrmm::CachedTacticalRmmSyncDto`), instead of defining
//! the JSON shape here a second time -- that way this module stays
//! automatically in sync if one of those shapes ever changes.
//!
//! The cache path convention (`data_dir/plugin-cache/<plugin>-<connection_id>.json`)
//! and the reading itself (`std::fs::read_to_string` + `serde_json::from_str`)
//! are nonetheless rebuilt inline here instead of calling the plugin modules'
//! `read_*_cache` helper functions directly: those are deliberately private
//! there (`fn`, not `pub fn`), and this change set is not allowed to touch
//! `commands/plugins.rs`, `commands/level.rs`, `commands/snipeit.rs`, or
//! `commands/tacticalrmm.rs` (split with changes being worked on in parallel
//! that own exactly those files). Loosening the visibility to `pub(crate)`
//! there would have violated that boundary; rebuilding it following the path
//! convention and the real DTO types is the only option that respects that
//! boundary without duplicating the JSON shape itself -- only the trivial
//! one-liner path construction/the file read itself is duplicated, exactly
//! as the plugin modules already each do for themselves (`plugin_cache_dir`
//! is already identically duplicated across all of them).

use std::path::{Path, PathBuf};

use tauri::State;

use crate::commands::level::CachedLevelSyncDto;
use crate::commands::plugins::CachedNinjaSyncDto;
use crate::commands::snipeit::CachedSnipeitSyncDto;
use crate::commands::tacticalrmm::CachedTacticalRmmSyncDto;
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

/// The same `data_dir/plugin-cache/` directory that `commands::plugins`/
/// `commands::level`/`commands::snipeit` each use for themselves.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn read_ninja_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedNinjaSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("ninja-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedNinjaSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Ninja-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_level_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedLevelSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("level-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedLevelSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Level-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_snipeit_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedSnipeitSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("snipeit-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedSnipeitSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Snipe-IT-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_tacticalrmm_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedTacticalRmmSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("tacticalrmm-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedTacticalRmmSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Tactical-RMM-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

/// Ninja: an organization counts for `customer_id` when
/// `config.ninja_org_mappings` has a matching row for (connection,
/// organization) right NOW -- not the potentially stale `customer_id` field
/// frozen in the cache file at the last sync time. Otherwise, freshly mapping
/// an organization via the Plugins page would only become visible here after
/// the next manual sync -- exactly the problem that
/// `commands::plugins::get_cached_ninja_sync` already solves for the same
/// cache for the same reason (see the comment there), and exactly the
/// purpose of this function (making devices show up "automatically") would
/// otherwise be missed for organizations that were just mapped.
fn collect_ninja(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.ninja_connections {
        let Some(cache) = read_ninja_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .ninja_org_mappings
                .iter()
                .find(|m| {
                    m.connection_id == connection.id && m.organization_id == group.organization_id
                })
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

/// Level: no separate organization/mapping layer -- a connection belongs
/// directly to exactly one customer (`LevelConnectionMeta.customer_id`), see
/// the `commands::level` module documentation. No staleness problem like
/// with Ninja/Snipe-IT: `customer_id` is a direct connection field, not
/// frozen in a cache file.
fn collect_level(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
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

/// Snipe-IT: exactly the same pattern as Ninja (`collect_ninja`), including
/// re-checking against the current `config.snipeit_company_mappings` instead
/// of the frozen cache value. `hostname`/`ip_address` are practically always
/// `None` for Snipe-IT assets (Snipe-IT doesn't natively know these fields,
/// see the `plugin::snipeit` module documentation) -- `name` is nonetheless
/// meaningfully populated, because Snipe-IT's own device mapping logic
/// (`plugin::snipeit`) already falls back itself from `name` through
/// `asset_tag` and `serial` down to the external ID, before the value even
/// reaches the cache.
fn collect_snipeit(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
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

/// Tactical RMM: structurally the same pattern as Ninja/Snipe-IT
/// (`collect_ninja`/`collect_snipeit`) -- re-checking against the current
/// `config.tacticalrmm_client_mappings` instead of the frozen cache value,
/// for the same freshness reason. `hostname`/`ip_address` are genuinely
/// populated for Tactical RMM agents (unlike Snipe-IT), see the
/// `plugin::tacticalrmm` module documentation.
fn collect_tacticalrmm(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.tacticalrmm_connections {
        let Some(cache) = read_tacticalrmm_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .tacticalrmm_client_mappings
                .iter()
                .find(|m| m.connection_id == connection.id && m.client_id == group.client_id)
                .map(|m| m.customer_id);
            if mapped_customer_id != Some(customer_id) {
                continue;
            }
            for device in &group.devices {
                if device.linked_system_id.is_some() {
                    continue;
                }
                out.push(UnlinkedExternalSystemDto {
                    plugin: "tacticalrmm".to_string(),
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

/// Pure core logic, without `State<AppState>` -- testable with a hardcoded
/// `Config` plus a `tempfile::tempdir()`, analogous to
/// `commands::plugins::group_devices_by_organization`. The
/// `#[tauri::command]` wrapper below merely takes a cloned `Config` (cloned
/// so the config mutex isn't held during file I/O -- see
/// `commands::plugins::get_cached_ninja_sync`, which does exactly the same
/// for the same reason) out of `State<AppState>` and passes it through here.
/// Pure function -- no network access, no database access, only `Config`
/// (already in memory) plus JSON files already on disk.
pub fn list_unlinked_external_systems_for_customer_pure(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
) -> Result<Vec<UnlinkedExternalSystemDto>, AppError> {
    let mut result = Vec::new();
    collect_ninja(config, data_dir, customer_id, &mut result)?;
    collect_level(config, data_dir, customer_id, &mut result)?;
    collect_snipeit(config, data_dir, customer_id, &mut result)?;
    collect_tacticalrmm(config, data_dir, customer_id, &mut result)?;
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
    use crate::commands::plugins::{
        ExternalSystemDto as NinjaExternalSystemDto, NinjaOrgDeviceGroupDto,
    };
    use crate::commands::snipeit::{
        ExternalSystemDto as SnipeitExternalSystemDto, SnipeitCompanyDeviceGroupDto,
    };
    use crate::commands::tacticalrmm::{
        ExternalSystemDto as TacticalRmmExternalSystemDto, TacticalRmmClientDeviceGroupDto,
    };
    use crate::plugin::level::LevelConnectionMeta;
    use crate::plugin::ninja::{NinjaConnectionMeta, NinjaOrgMapping};
    use crate::plugin::snipeit::{SnipeitCompanyMapping, SnipeitConnectionMeta};
    use crate::plugin::tacticalrmm::{TacticalRmmClientMapping, TacticalRmmConnectionMeta};

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

    fn tacticalrmm_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("tacticalrmm-{connection_id}.json"))
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

    fn ninja_config_with_connection(
        connection_id: &str,
        organization_id: &str,
        mapped_customer_id: Option<i64>,
    ) -> Config {
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
                // Deliberately left stale/`None` -- the mapping comes from
                // `config.ninja_org_mappings`, not from this frozen field
                // (see the comment on `collect_ninja`).
                customer_id: None,
                devices: vec![ninja_device("dev-1", None)],
            }],
        };
        write_json(&ninja_cache_path(dir.path(), "conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();

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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn ninja_device_mapped_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        // Organization is mapped to customer 99, we're asking about customer 42.
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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn unmapped_ninja_organization_is_excluded() {
        let dir = tempdir().unwrap();
        // Connection exists, but the organization isn't mapped to any
        // customer (yet) -- `mapped_customer_id` stays `None`.
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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn ninja_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        // No cache file written for "conn-1" -- simulates "never synced".
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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

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

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

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
    fn tacticalrmm_device_appears_when_its_client_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config
            .tacticalrmm_connections
            .push(TacticalRmmConnectionMeta {
                id: "trmm-conn-1".to_string(),
                label: "ACME Tactical RMM".to_string(),
                base_url: "https://api.rmm.example.com".to_string(),
            });
        config
            .tacticalrmm_client_mappings
            .push(TacticalRmmClientMapping {
                connection_id: "trmm-conn-1".to_string(),
                client_id: "client-1".to_string(),
                client_name: "ACME GmbH".to_string(),
                customer_id: 5,
            });
        let cache = CachedTacticalRmmSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![TacticalRmmClientDeviceGroupDto {
                client_id: "client-1".to_string(),
                client_name: "ACME GmbH".to_string(),
                // Deliberately left stale/`None` -- the mapping comes from
                // `config.tacticalrmm_client_mappings`, not from this frozen
                // field (see the comment on `collect_tacticalrmm`).
                customer_id: None,
                devices: vec![TacticalRmmExternalSystemDto {
                    external_id: "agent-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: Some("SRV-01".to_string()),
                    ip_address: Some("10.0.0.5".to_string()),
                    status: Some("online".to_string()),
                    platform: Some("windows".to_string()),
                    site_name: Some("Hauptsitz".to_string()),
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&tacticalrmm_cache_path(dir.path(), "trmm-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "tacticalrmm");
        assert_eq!(result[0].connection_id, "trmm-conn-1");
        assert_eq!(result[0].external_id, "agent-1");
        assert_eq!(result[0].hostname.as_deref(), Some("SRV-01"));
    }

    #[test]
    fn already_linked_tacticalrmm_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config
            .tacticalrmm_connections
            .push(TacticalRmmConnectionMeta {
                id: "trmm-conn-1".to_string(),
                label: "ACME Tactical RMM".to_string(),
                base_url: "https://api.rmm.example.com".to_string(),
            });
        config
            .tacticalrmm_client_mappings
            .push(TacticalRmmClientMapping {
                connection_id: "trmm-conn-1".to_string(),
                client_id: "client-1".to_string(),
                client_name: "ACME GmbH".to_string(),
                customer_id: 5,
            });
        let cache = CachedTacticalRmmSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![TacticalRmmClientDeviceGroupDto {
                client_id: "client-1".to_string(),
                client_name: "ACME GmbH".to_string(),
                customer_id: Some(5),
                devices: vec![TacticalRmmExternalSystemDto {
                    external_id: "agent-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: None,
                    ip_address: None,
                    status: None,
                    platform: None,
                    site_name: None,
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&tacticalrmm_cache_path(dir.path(), "trmm-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn tacticalrmm_client_mapped_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config
            .tacticalrmm_connections
            .push(TacticalRmmConnectionMeta {
                id: "trmm-conn-1".to_string(),
                label: "ACME Tactical RMM".to_string(),
                base_url: "https://api.rmm.example.com".to_string(),
            });
        config
            .tacticalrmm_client_mappings
            .push(TacticalRmmClientMapping {
                connection_id: "trmm-conn-1".to_string(),
                client_id: "client-1".to_string(),
                client_name: "ACME GmbH".to_string(),
                customer_id: 99,
            });
        let cache = CachedTacticalRmmSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![TacticalRmmClientDeviceGroupDto {
                client_id: "client-1".to_string(),
                client_name: "ACME GmbH".to_string(),
                customer_id: Some(99),
                devices: vec![TacticalRmmExternalSystemDto {
                    external_id: "agent-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: None,
                    ip_address: None,
                    status: None,
                    platform: None,
                    site_name: None,
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&tacticalrmm_cache_path(dir.path(), "trmm-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn tacticalrmm_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config
            .tacticalrmm_connections
            .push(TacticalRmmConnectionMeta {
                id: "trmm-conn-1".to_string(),
                label: "ACME Tactical RMM".to_string(),
                base_url: "https://api.rmm.example.com".to_string(),
            });
        config
            .tacticalrmm_client_mappings
            .push(TacticalRmmClientMapping {
                connection_id: "trmm-conn-1".to_string(),
                client_id: "client-1".to_string(),
                client_name: "ACME GmbH".to_string(),
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

        let mut result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();
        result.sort_by(|a, b| a.plugin.cmp(&b.plugin));

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].plugin, "level");
        assert_eq!(result[1].plugin, "ninja");
    }
}
