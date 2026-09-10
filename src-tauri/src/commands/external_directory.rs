//! Pure read-access aggregation across all configured RMM/asset management
//! plugin connections (Ninja, Level, Snipe-IT, Intune, Iru, Jamf, ABM,
//! Tactical RMM, Atera, Pulseway, Kaseya, Action1, Datto RMM, Acronis, Vultr)
//! for a given local customer: returns all devices/assets/resources/
//! instances that (a) belong to this customer according to the most
//! recently synced plugin caches, and (b) are not yet linked to any local
//! system.
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
//! IMPORTANT, a real gap fixed here: six plugins added after this module was
//! first written (Atera, Pulseway, Kaseya, Action1, Datto RMM, Acronis) were
//! never wired into `list_unlinked_external_systems_for_customer_pure` --
//! their devices/resources were entirely invisible in the Journal entry's
//! System field, even when a customer had dozens of unlinked, correctly
//! mapped devices in one of those plugins. Adding a new plugin's `collect_*`
//! function here is a required step, not optional wiring -- see the six
//! `collect_atera`/`collect_pulseway`/`collect_kaseya`/`collect_action1`/
//! `collect_dattormm`/`collect_acronis` functions below for the pattern (and
//! `collect_vultr`, added together with Vultr itself, for a fresh example of
//! a new plugin done right from the start).
//!
//! Deliberately reuses the real, already `Deserialize`-capable cache DTOs of
//! the plugin modules (`commands::plugins::CachedNinjaSyncDto`,
//! `commands::level::CachedLevelSyncDto`, `commands::snipeit::CachedSnipeitSyncDto`,
//! `commands::intune::CachedIntuneSyncDto`, `commands::iru::CachedIruSyncDto`,
//! `commands::jamf::CachedJamfSyncDto`, `commands::abm::CachedAbmSyncDto`,
//! `commands::tacticalrmm::CachedTacticalRmmSyncDto`,
//! `commands::atera::CachedAteraSyncDto`,
//! `commands::pulseway::CachedPulsewaySyncDto`,
//! `commands::kaseya::CachedKaseyaSyncDto`,
//! `commands::action1::CachedAction1SyncDto`,
//! `commands::dattormm::CachedDattoRmmSyncDto`,
//! `commands::acronis::CachedAcronisSyncDto`,
//! `commands::vultr::CachedVultrSyncDto`), instead of defining the JSON
//! shape here a second time -- that way this module stays automatically in
//! sync if one of those shapes ever changes.
//!
//! The cache path convention (`data_dir/plugin-cache/<plugin>-<connection_id>.json`)
//! and the reading itself (`std::fs::read_to_string` + `serde_json::from_str`)
//! are nonetheless rebuilt inline here instead of calling the plugin modules'
//! `read_*_cache` helper functions directly: those are deliberately private
//! there (`fn`, not `pub fn`), and an earlier change set that introduced this
//! module wasn't allowed to touch `commands/plugins.rs`, `commands/level.rs`,
//! or `commands/snipeit.rs` (split with a change being worked on in parallel
//! that owned exactly those files at the time). Loosening the visibility to
//! `pub(crate)` there would have violated that boundary; rebuilding it
//! following the path convention and the real DTO types is the only option
//! that respects that boundary without duplicating the JSON shape itself --
//! only the trivial one-liner path construction/the file read itself is
//! duplicated, exactly as every plugin module already does for itself
//! (`plugin_cache_dir` is already identically duplicated across all of
//! them). Intune's, Iru's, Jamf's, ABM's, and Tactical RMM's own
//! `collect_*`/`read_*_cache_file` below follow that same established
//! convention rather than reaching into their own `commands` modules'
//! private helpers, for consistency rather than because of that same
//! historical constraint (this module is free to touch
//! `commands/intune.rs`/`commands/iru.rs`/`commands/jamf.rs`/`commands/abm.rs`/`commands/tacticalrmm.rs`,
//! there's just nothing there worth touching).

use std::path::{Path, PathBuf};

use tauri::State;

use crate::commands::abm::CachedAbmSyncDto;
use crate::commands::acronis::CachedAcronisSyncDto;
use crate::commands::action1::CachedAction1SyncDto;
use crate::commands::atera::CachedAteraSyncDto;
use crate::commands::dattormm::CachedDattoRmmSyncDto;
use crate::commands::intune::CachedIntuneSyncDto;
use crate::commands::iru::CachedIruSyncDto;
use crate::commands::jamf::CachedJamfSyncDto;
use crate::commands::kaseya::CachedKaseyaSyncDto;
use crate::commands::level::CachedLevelSyncDto;
use crate::commands::plugins::CachedNinjaSyncDto;
use crate::commands::pulseway::CachedPulsewaySyncDto;
use crate::commands::snipeit::CachedSnipeitSyncDto;
use crate::commands::tacticalrmm::CachedTacticalRmmSyncDto;
use crate::commands::vultr::CachedVultrSyncDto;
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

fn read_iru_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedIruSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("iru-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedIruSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Iru-Cache-Datei ungültig: {e}")))?;
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

fn read_intune_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedIntuneSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("intune-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedIntuneSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Intune-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_jamf_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedJamfSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("jamf-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedJamfSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Jamf-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_abm_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedAbmSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("abm-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedAbmSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("ABM-Cache-Datei ungültig: {e}")))?;
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

fn read_atera_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedAteraSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("atera-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedAteraSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Atera-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_pulseway_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedPulsewaySyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("pulseway-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedPulsewaySyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Pulseway-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_kaseya_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedKaseyaSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("kaseya-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedKaseyaSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Kaseya-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_action1_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedAction1SyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("action1-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedAction1SyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Action1-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_dattormm_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedDattoRmmSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("dattormm-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedDattoRmmSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Datto-RMM-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_acronis_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedAcronisSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("acronis-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedAcronisSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Acronis-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn read_vultr_cache_file(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedVultrSyncDto>, AppError> {
    let path = plugin_cache_dir(data_dir).join(format!("vultr-{connection_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedVultrSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Vultr-Cache-Datei ungültig: {e}")))?;
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

/// Iru: no separate organization/mapping layer -- exactly the same
/// direct-`customer_id`-on-the-connection shape as Level
/// (`collect_level`/`LevelConnectionMeta.customer_id`), see the
/// `commands::iru` module documentation. `hostname`/`ip_address` are ALWAYS
/// `None` for Iru devices (Iru's device object has no such field, see the
/// `plugin::iru` module documentation) -- `name` is nonetheless meaningfully
/// populated, because Iru's own device mapping logic (`plugin::iru`) already
/// falls back itself from `device_name` through `model` and `serial_number`
/// down to the external ID, before the value even reaches the cache.
fn collect_iru(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.iru_connections {
        if connection.customer_id != customer_id {
            continue;
        }
        let Some(cache) = read_iru_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for device in &cache.devices {
            if device.linked_system_id.is_some() {
                continue;
            }
            out.push(UnlinkedExternalSystemDto {
                plugin: "iru".to_string(),
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

/// Intune: exactly the same pattern as Level (`collect_level`) -- a
/// connection belongs directly to exactly one customer
/// (`IntuneConnectionMeta.customer_id`), see the `plugin::intune` module
/// documentation. No staleness problem like with Ninja/Snipe-IT:
/// `customer_id` is a direct connection field, not frozen in a cache file.
/// `hostname` is populated from Intune's `deviceName` (see `plugin::intune`
/// module documentation); `ip_address` is always `None` -- Microsoft Graph's
/// managed-device response has no IP address field at all.
fn collect_intune(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.intune_connections {
        if connection.customer_id != customer_id {
            continue;
        }
        let Some(cache) = read_intune_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for device in &cache.devices {
            if device.linked_system_id.is_some() {
                continue;
            }
            out.push(UnlinkedExternalSystemDto {
                plugin: "intune".to_string(),
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

/// Jamf: exactly the same pattern as Ninja/Snipe-IT (`collect_ninja`/
/// `collect_snipeit`), including re-checking against the current
/// `config.jamf_site_mappings` instead of the frozen cache value, for the
/// same reason (a site freshly mapped via the Plugins page should show up
/// here without requiring another live sync first).
fn collect_jamf(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.jamf_connections {
        let Some(cache) = read_jamf_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .jamf_site_mappings
                .iter()
                .find(|m| m.connection_id == connection.id && m.site_id == group.site_id)
                .map(|m| m.customer_id);
            if mapped_customer_id != Some(customer_id) {
                continue;
            }
            for device in &group.devices {
                if device.linked_system_id.is_some() {
                    continue;
                }
                out.push(UnlinkedExternalSystemDto {
                    plugin: "jamf".to_string(),
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

/// ABM: exactly the same pattern as Level (`collect_level`) -- no separate
/// organization/mapping layer, a connection belongs directly to exactly one
/// customer (`AbmConnectionMeta.customer_id`), see the `plugin::abm` module
/// documentation. `hostname`/`ip_address` are always `None` for ABM devices
/// (ABM is a purchasing/enrollment registry, not RMM telemetry, see
/// `plugin::abm` module docs) -- `name` is nonetheless meaningfully
/// populated, because ABM's own device mapping logic (`plugin::abm`) already
/// falls back itself from `deviceModel` to `serialNumber` down to the
/// external ID, before the value even reaches the cache.
fn collect_abm(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.abm_connections {
        if connection.customer_id != customer_id {
            continue;
        }
        let Some(cache) = read_abm_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for device in &cache.devices {
            if device.linked_system_id.is_some() {
                continue;
            }
            out.push(UnlinkedExternalSystemDto {
                plugin: "abm".to_string(),
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

/// Atera: exactly the same pattern as Ninja/Snipe-IT/Jamf/Tactical RMM
/// (`collect_ninja` etc.) -- re-checking against the current
/// `config.atera_customer_mappings` instead of the frozen cache value, for
/// the same freshness reason. Note the mapping struct's own field names:
/// `AteraCustomerMapping.customer_id` is Atera's OWN customer id (matched
/// against the group's `atera_customer_id`), `local_customer_id` is the
/// LOCAL customer id being asked about here -- not the usual
/// `customer_id`-means-local convention every other mapping struct in this
/// file uses, see `plugin::atera` module docs.
fn collect_atera(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.atera_connections {
        let Some(cache) = read_atera_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .atera_customer_mappings
                .iter()
                .find(|m| {
                    m.connection_id == connection.id && m.customer_id == group.atera_customer_id
                })
                .map(|m| m.local_customer_id);
            if mapped_customer_id != Some(customer_id) {
                continue;
            }
            for device in &group.devices {
                if device.linked_system_id.is_some() {
                    continue;
                }
                out.push(UnlinkedExternalSystemDto {
                    plugin: "atera".to_string(),
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

/// Pulseway: exactly the same pattern as Ninja/Snipe-IT (`collect_ninja`/
/// `collect_snipeit`), re-checking against the current
/// `config.pulseway_org_mappings`. `ip_address` is always `None` -- Pulseway
/// has no such field on its device object at all (only `hostname`, which
/// doubles as the display name), see `plugin::pulseway` module docs.
fn collect_pulseway(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.pulseway_connections {
        let Some(cache) = read_pulseway_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .pulseway_org_mappings
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
                    plugin: "pulseway".to_string(),
                    connection_id: connection.id.clone(),
                    external_id: device.external_id.clone(),
                    name: device.name.clone(),
                    hostname: device.hostname.clone(),
                    ip_address: None,
                });
            }
        }
    }
    Ok(())
}

/// Kaseya: exactly the same pattern as Ninja/Snipe-IT (`collect_ninja`/
/// `collect_snipeit`), re-checking against the current
/// `config.kaseya_org_mappings`. `hostname`/`ip_address` are always `None`
/// -- no confirmed hostname/IP field exists on a Kaseya device at all, see
/// `plugin::kaseya` module docs.
fn collect_kaseya(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.kaseya_connections {
        let Some(cache) = read_kaseya_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .kaseya_org_mappings
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
                    plugin: "kaseya".to_string(),
                    connection_id: connection.id.clone(),
                    external_id: device.external_id.clone(),
                    name: device.name.clone(),
                    hostname: None,
                    ip_address: None,
                });
            }
        }
    }
    Ok(())
}

/// Action1: exactly the same pattern as Ninja/Snipe-IT (`collect_ninja`/
/// `collect_snipeit`), re-checking against the current
/// `config.action1_org_mappings`. `hostname` is always `None` -- Action1's
/// endpoint object has no hostname field (only `device_name`, already used
/// as `name`, and a separate `ip_address` field), see `plugin::action1`
/// module docs.
fn collect_action1(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.action1_connections {
        let Some(cache) = read_action1_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .action1_org_mappings
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
                    plugin: "action1".to_string(),
                    connection_id: connection.id.clone(),
                    external_id: device.external_id.clone(),
                    name: device.name.clone(),
                    hostname: None,
                    ip_address: device.ip_address.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Datto RMM: exactly the same pattern as Ninja/Snipe-IT (`collect_ninja`/
/// `collect_snipeit`), re-checking against the current
/// `config.dattormm_site_mappings`, keyed by `site_uid` (Datto RMM's flat
/// grouping level, see `plugin::dattormm` module docs) rather than an
/// `organization_id`.
fn collect_dattormm(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.dattormm_connections {
        let Some(cache) = read_dattormm_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .dattormm_site_mappings
                .iter()
                .find(|m| m.connection_id == connection.id && m.site_uid == group.site_uid)
                .map(|m| m.customer_id);
            if mapped_customer_id != Some(customer_id) {
                continue;
            }
            for device in &group.devices {
                if device.linked_system_id.is_some() {
                    continue;
                }
                out.push(UnlinkedExternalSystemDto {
                    plugin: "dattormm".to_string(),
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

/// Acronis: same pattern as Ninja/Snipe-IT/Tactical RMM (`collect_ninja`
/// etc.), re-checking against the current `config.acronis_tenant_mappings`,
/// keyed by `tenant_id`. `hostname`/`ip_address` are always `None` --
/// Acronis's `AcronisResourceDto` has neither field at all (this plugin
/// surfaces backup status, not device inventory, see `plugin::acronis`
/// module docs). Unlike every other plugin collected here, Acronis's own
/// cache is only ever populated for tenants that were ALREADY mapped at
/// sync time (`sync_acronis_connection` only loops mapped tenants, see
/// `commands::acronis` module docs) -- so, unlike Ninja/Snipe-IT/Jamf/
/// Tactical RMM, a cached group here can't legitimately belong to a
/// DIFFERENT customer than the one it was synced for; re-checking against
/// the live mapping table is still done anyway, for the same "freshly
/// remapped without a new sync" consistency every other plugin here gets.
fn collect_acronis(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.acronis_connections {
        let Some(cache) = read_acronis_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for group in &cache.groups {
            let mapped_customer_id = config
                .acronis_tenant_mappings
                .iter()
                .find(|m| m.connection_id == connection.id && m.tenant_id == group.tenant_id)
                .map(|m| m.customer_id);
            if mapped_customer_id != Some(customer_id) {
                continue;
            }
            for device in &group.devices {
                if device.linked_system_id.is_some() {
                    continue;
                }
                out.push(UnlinkedExternalSystemDto {
                    plugin: "acronis".to_string(),
                    connection_id: connection.id.clone(),
                    external_id: device.external_id.clone(),
                    name: device.name.clone(),
                    hostname: None,
                    ip_address: None,
                });
            }
        }
    }
    Ok(())
}

/// Vultr: exactly the same pattern as Level/Intune (`collect_level`/
/// `collect_intune`) -- no separate organization/mapping layer, a connection
/// belongs directly to exactly one customer (`VultrConnectionMeta.
/// customer_id`), see the `plugin::vultr` module documentation, "Tenancy".
/// No staleness problem like with Ninja/Snipe-IT: `customer_id` is a direct
/// connection field, not frozen in a cache file. `hostname` is always `None`
/// -- Vultr's instance object has no separate hostname field surfaced here,
/// the display-name fallback chain already folds it into `name` server-side
/// (see `plugin::vultr::map_instance`).
fn collect_vultr(
    config: &Config,
    data_dir: &Path,
    customer_id: i64,
    out: &mut Vec<UnlinkedExternalSystemDto>,
) -> Result<(), AppError> {
    for connection in &config.vultr_connections {
        if connection.customer_id != customer_id {
            continue;
        }
        let Some(cache) = read_vultr_cache_file(data_dir, &connection.id)? else {
            continue;
        };
        for instance in &cache.instances {
            if instance.linked_system_id.is_some() {
                continue;
            }
            out.push(UnlinkedExternalSystemDto {
                plugin: "vultr".to_string(),
                connection_id: connection.id.clone(),
                external_id: instance.external_id.clone(),
                name: instance.name.clone(),
                hostname: None,
                ip_address: instance.ip_address.clone(),
            });
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
    collect_intune(config, data_dir, customer_id, &mut result)?;
    collect_iru(config, data_dir, customer_id, &mut result)?;
    collect_jamf(config, data_dir, customer_id, &mut result)?;
    collect_abm(config, data_dir, customer_id, &mut result)?;
    collect_tacticalrmm(config, data_dir, customer_id, &mut result)?;
    collect_atera(config, data_dir, customer_id, &mut result)?;
    collect_pulseway(config, data_dir, customer_id, &mut result)?;
    collect_kaseya(config, data_dir, customer_id, &mut result)?;
    collect_action1(config, data_dir, customer_id, &mut result)?;
    collect_dattormm(config, data_dir, customer_id, &mut result)?;
    collect_acronis(config, data_dir, customer_id, &mut result)?;
    collect_vultr(config, data_dir, customer_id, &mut result)?;
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

    use crate::commands::abm::ExternalSystemDto as AbmExternalSystemDto;
    use crate::commands::acronis::{AcronisResourceDto, AcronisTenantResourceGroupDto};
    use crate::commands::action1::{
        Action1OrgDeviceGroupDto, ExternalSystemDto as Action1ExternalSystemDto,
    };
    use crate::commands::atera::{
        AteraCustomerDeviceGroupDto, ExternalSystemDto as AteraExternalSystemDto,
    };
    use crate::commands::dattormm::{
        DattoRmmSiteDeviceGroupDto, ExternalSystemDto as DattoRmmExternalSystemDto,
    };
    use crate::commands::intune::ExternalSystemDto as IntuneExternalSystemDto;
    use crate::commands::iru::ExternalSystemDto as IruExternalSystemDto;
    use crate::commands::jamf::{
        ExternalSystemDto as JamfExternalSystemDto, JamfSiteDeviceGroupDto,
    };
    use crate::commands::kaseya::{
        ExternalSystemDto as KaseyaExternalSystemDto, KaseyaOrgDeviceGroupDto,
    };
    use crate::commands::level::ExternalSystemDto as LevelExternalSystemDto;
    use crate::commands::plugins::{
        ExternalSystemDto as NinjaExternalSystemDto, NinjaOrgDeviceGroupDto,
    };
    use crate::commands::pulseway::{
        ExternalSystemDto as PulsewayExternalSystemDto, PulsewayOrgDeviceGroupDto,
    };
    use crate::commands::snipeit::{
        ExternalSystemDto as SnipeitExternalSystemDto, SnipeitCompanyDeviceGroupDto,
    };
    use crate::commands::tacticalrmm::{
        ExternalSystemDto as TacticalRmmExternalSystemDto, TacticalRmmClientDeviceGroupDto,
    };
    use crate::commands::vultr::ExternalSystemDto as VultrExternalSystemDto;
    use crate::plugin::abm::AbmConnectionMeta;
    use crate::plugin::acronis::{AcronisConnectionMeta, AcronisTenantMapping};
    use crate::plugin::action1::{Action1ConnectionMeta, Action1OrgMapping};
    use crate::plugin::atera::{AteraConnectionMeta, AteraCustomerMapping};
    use crate::plugin::dattormm::{DattoRmmConnectionMeta, DattoRmmSiteMapping};
    use crate::plugin::intune::IntuneConnectionMeta;
    use crate::plugin::iru::IruConnectionMeta;
    use crate::plugin::jamf::{JamfConnectionMeta, JamfSiteMapping};
    use crate::plugin::kaseya::{KaseyaConnectionMeta, KaseyaOrgMapping};
    use crate::plugin::level::LevelConnectionMeta;
    use crate::plugin::ninja::{NinjaConnectionMeta, NinjaOrgMapping};
    use crate::plugin::pulseway::{PulsewayConnectionMeta, PulsewayOrgMapping};
    use crate::plugin::snipeit::{SnipeitCompanyMapping, SnipeitConnectionMeta};
    use crate::plugin::tacticalrmm::{TacticalRmmClientMapping, TacticalRmmConnectionMeta};
    use crate::plugin::vultr::VultrConnectionMeta;

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

    fn intune_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("intune-{connection_id}.json"))
    }

    fn iru_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("iru-{connection_id}.json"))
    }

    fn jamf_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("jamf-{connection_id}.json"))
    }

    fn abm_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("abm-{connection_id}.json"))
    }

    fn tacticalrmm_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("tacticalrmm-{connection_id}.json"))
    }

    fn vultr_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("vultr-{connection_id}.json"))
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
    fn iru_device_appears_when_its_connection_is_bound_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.iru_connections.push(IruConnectionMeta {
            id: "iru-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Iru".to_string(),
            base_url: "https://acme.api.kandji.io".to_string(),
        });
        let cache = CachedIruSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![IruExternalSystemDto {
                external_id: "iru-1".to_string(),
                name: "Iru Device 1".to_string(),
                hostname: None,
                ip_address: None,
                serial_number: Some("SN-1".to_string()),
                asset_tag: None,
                model: Some("MacBook Air".to_string()),
                platform: Some("Mac".to_string()),
                os_version: Some("14.4.1".to_string()),
                linked_system_id: None,
            }],
        };
        write_json(&iru_cache_path(dir.path(), "iru-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "iru");
        assert_eq!(result[0].connection_id, "iru-conn-1");
        assert_eq!(result[0].external_id, "iru-1");
        assert_eq!(result[0].hostname, None);
    }

    #[test]
    fn already_linked_iru_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.iru_connections.push(IruConnectionMeta {
            id: "iru-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Iru".to_string(),
            base_url: "https://acme.api.kandji.io".to_string(),
        });
        let cache = CachedIruSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![IruExternalSystemDto {
                external_id: "iru-1".to_string(),
                name: "Iru Device 1".to_string(),
                hostname: None,
                ip_address: None,
                serial_number: None,
                asset_tag: None,
                model: None,
                platform: None,
                os_version: None,
                linked_system_id: Some(3),
            }],
        };
        write_json(&iru_cache_path(dir.path(), "iru-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn iru_connection_bound_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.iru_connections.push(IruConnectionMeta {
            id: "iru-conn-1".to_string(),
            customer_id: 99,
            label: "ACME Iru".to_string(),
            base_url: "https://acme.api.kandji.io".to_string(),
        });
        let cache = CachedIruSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![IruExternalSystemDto {
                external_id: "iru-1".to_string(),
                name: "Iru Device 1".to_string(),
                hostname: None,
                ip_address: None,
                serial_number: None,
                asset_tag: None,
                model: None,
                platform: None,
                os_version: None,
                linked_system_id: None,
            }],
        };
        write_json(&iru_cache_path(dir.path(), "iru-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn iru_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.iru_connections.push(IruConnectionMeta {
            id: "iru-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Iru".to_string(),
            base_url: "https://acme.api.kandji.io".to_string(),
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
    fn intune_device_appears_when_its_connection_is_bound_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.intune_connections.push(IntuneConnectionMeta {
            id: "intune-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Intune".to_string(),
        });
        let cache = CachedIntuneSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![IntuneExternalSystemDto {
                external_id: "intune-1".to_string(),
                name: "Intune Device 1".to_string(),
                hostname: Some("Intune Device 1".to_string()),
                ip_address: None,
                linked_system_id: None,
                operating_system: Some("Windows".to_string()),
                os_version: Some("10.0.19045".to_string()),
                serial_number: Some("SN-001".to_string()),
                manufacturer: Some("Contoso".to_string()),
                model: Some("Surface".to_string()),
                compliance_state: Some("compliant".to_string()),
                last_sync_date_time: Some("2026-09-07T10:00:00Z".to_string()),
                user_principal_name: Some("user@contoso.com".to_string()),
            }],
        };
        write_json(&intune_cache_path(dir.path(), "intune-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "intune");
        assert_eq!(result[0].connection_id, "intune-conn-1");
        assert_eq!(result[0].external_id, "intune-1");
        assert_eq!(result[0].ip_address, None);
    }

    #[test]
    fn already_linked_intune_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.intune_connections.push(IntuneConnectionMeta {
            id: "intune-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Intune".to_string(),
        });
        let cache = CachedIntuneSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![IntuneExternalSystemDto {
                external_id: "intune-1".to_string(),
                name: "Intune Device 1".to_string(),
                hostname: None,
                ip_address: None,
                linked_system_id: Some(3),
                operating_system: None,
                os_version: None,
                serial_number: None,
                manufacturer: None,
                model: None,
                compliance_state: None,
                last_sync_date_time: None,
                user_principal_name: None,
            }],
        };
        write_json(&intune_cache_path(dir.path(), "intune-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn abm_device_appears_when_its_connection_is_bound_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.abm_connections.push(AbmConnectionMeta {
            id: "abm-conn-1".to_string(),
            customer_id: 7,
            label: "ACME ABM".to_string(),
        });
        let cache = CachedAbmSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![AbmExternalSystemDto {
                external_id: "abm-1".to_string(),
                name: "iMac 21.5\"".to_string(),
                hostname: None,
                ip_address: None,
                serial_number: Some("XABC123X0ABC123X0".to_string()),
                device_model: Some("iMac 21.5\"".to_string()),
                linked_system_id: None,
            }],
        };
        write_json(&abm_cache_path(dir.path(), "abm-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "abm");
        assert_eq!(result[0].connection_id, "abm-conn-1");
        assert_eq!(result[0].external_id, "abm-1");
        assert_eq!(result[0].hostname, None);
    }

    #[test]
    fn already_linked_abm_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.abm_connections.push(AbmConnectionMeta {
            id: "abm-conn-1".to_string(),
            customer_id: 7,
            label: "ACME ABM".to_string(),
        });
        let cache = CachedAbmSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![AbmExternalSystemDto {
                external_id: "abm-1".to_string(),
                name: "iMac 21.5\"".to_string(),
                hostname: None,
                ip_address: None,
                serial_number: Some("XABC123X0ABC123X0".to_string()),
                device_model: Some("iMac 21.5\"".to_string()),
                linked_system_id: Some(3),
            }],
        };
        write_json(&abm_cache_path(dir.path(), "abm-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn jamf_device_appears_when_its_site_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.jamf_connections.push(JamfConnectionMeta {
            id: "jamf-conn-1".to_string(),
            label: "ACME Jamf".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });
        config.jamf_site_mappings.push(JamfSiteMapping {
            connection_id: "jamf-conn-1".to_string(),
            site_id: "site-1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 5,
        });
        let cache = CachedJamfSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![JamfSiteDeviceGroupDto {
                site_id: "site-1".to_string(),
                site_name: "ACME Hauptsitz".to_string(),
                // Deliberately left stale/`None` -- the mapping comes from
                // `config.jamf_site_mappings`, not from this frozen field.
                customer_id: None,
                devices: vec![JamfExternalSystemDto {
                    external_id: "mac-1".to_string(),
                    name: "MBP-Anna".to_string(),
                    hostname: Some("MBP-Anna".to_string()),
                    ip_address: Some("10.0.0.5".to_string()),
                    serial_number: Some("C02XXXXX".to_string()),
                    asset_tag: Some("AT-0001".to_string()),
                    jamf_url: "https://acme.jamfcloud.com/computers.html?id=mac-1".to_string(),
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&jamf_cache_path(dir.path(), "jamf-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "jamf");
        assert_eq!(result[0].connection_id, "jamf-conn-1");
        assert_eq!(result[0].external_id, "mac-1");
        assert_eq!(result[0].hostname.as_deref(), Some("MBP-Anna"));
    }

    #[test]
    fn already_linked_jamf_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.jamf_connections.push(JamfConnectionMeta {
            id: "jamf-conn-1".to_string(),
            label: "ACME Jamf".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });
        config.jamf_site_mappings.push(JamfSiteMapping {
            connection_id: "jamf-conn-1".to_string(),
            site_id: "site-1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 5,
        });
        let cache = CachedJamfSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![JamfSiteDeviceGroupDto {
                site_id: "site-1".to_string(),
                site_name: "ACME Hauptsitz".to_string(),
                customer_id: Some(5),
                devices: vec![JamfExternalSystemDto {
                    external_id: "mac-1".to_string(),
                    name: "MBP-Anna".to_string(),
                    hostname: None,
                    ip_address: None,
                    serial_number: None,
                    asset_tag: None,
                    jamf_url: "https://acme.jamfcloud.com/computers.html?id=mac-1".to_string(),
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&jamf_cache_path(dir.path(), "jamf-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
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
    fn intune_connection_bound_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.intune_connections.push(IntuneConnectionMeta {
            id: "intune-conn-1".to_string(),
            customer_id: 99,
            label: "ACME Intune".to_string(),
        });
        let cache = CachedIntuneSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![IntuneExternalSystemDto {
                external_id: "intune-1".to_string(),
                name: "Intune Device 1".to_string(),
                hostname: None,
                ip_address: None,
                linked_system_id: None,
                operating_system: None,
                os_version: None,
                serial_number: None,
                manufacturer: None,
                model: None,
                compliance_state: None,
                last_sync_date_time: None,
                user_principal_name: None,
            }],
        };
        write_json(&intune_cache_path(dir.path(), "intune-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn abm_connection_bound_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.abm_connections.push(AbmConnectionMeta {
            id: "abm-conn-1".to_string(),
            customer_id: 99,
            label: "ACME ABM".to_string(),
        });
        let cache = CachedAbmSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            devices: vec![AbmExternalSystemDto {
                external_id: "abm-1".to_string(),
                name: "iMac 21.5\"".to_string(),
                hostname: None,
                ip_address: None,
                serial_number: None,
                device_model: None,
                linked_system_id: None,
            }],
        };
        write_json(&abm_cache_path(dir.path(), "abm-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn jamf_site_mapped_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.jamf_connections.push(JamfConnectionMeta {
            id: "jamf-conn-1".to_string(),
            label: "ACME Jamf".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });
        config.jamf_site_mappings.push(JamfSiteMapping {
            connection_id: "jamf-conn-1".to_string(),
            site_id: "site-1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 99,
        });
        let cache = CachedJamfSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![JamfSiteDeviceGroupDto {
                site_id: "site-1".to_string(),
                site_name: "ACME Hauptsitz".to_string(),
                customer_id: Some(99),
                devices: vec![JamfExternalSystemDto {
                    external_id: "mac-1".to_string(),
                    name: "MBP-Anna".to_string(),
                    hostname: None,
                    ip_address: None,
                    serial_number: None,
                    asset_tag: None,
                    jamf_url: "https://acme.jamfcloud.com/computers.html?id=mac-1".to_string(),
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&jamf_cache_path(dir.path(), "jamf-conn-1"), &cache);

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
    fn intune_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.intune_connections.push(IntuneConnectionMeta {
            id: "intune-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Intune".to_string(),
        });

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn abm_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.abm_connections.push(AbmConnectionMeta {
            id: "abm-conn-1".to_string(),
            customer_id: 7,
            label: "ACME ABM".to_string(),
        });

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn jamf_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.jamf_connections.push(JamfConnectionMeta {
            id: "jamf-conn-1".to_string(),
            label: "ACME Jamf".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });
        config.jamf_site_mappings.push(JamfSiteMapping {
            connection_id: "jamf-conn-1".to_string(),
            site_id: "site-1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 5,
        });

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
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
        config.jamf_connections.push(JamfConnectionMeta {
            id: "jamf-conn-1".to_string(),
            label: "ACME Jamf".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });
        config.jamf_site_mappings.push(JamfSiteMapping {
            connection_id: "jamf-conn-1".to_string(),
            site_id: "site-1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 42,
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
        write_json(
            &jamf_cache_path(dir.path(), "jamf-conn-1"),
            &CachedJamfSyncDto {
                synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
                groups: vec![JamfSiteDeviceGroupDto {
                    site_id: "site-1".to_string(),
                    site_name: "ACME Hauptsitz".to_string(),
                    customer_id: Some(42),
                    devices: vec![JamfExternalSystemDto {
                        external_id: "mac-1".to_string(),
                        name: "MBP-Anna".to_string(),
                        hostname: Some("MBP-Anna".to_string()),
                        ip_address: None,
                        serial_number: None,
                        asset_tag: None,
                        jamf_url: "https://acme.jamfcloud.com/computers.html?id=mac-1".to_string(),
                        linked_system_id: None,
                    }],
                }],
            },
        );

        let mut result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 42).unwrap();
        result.sort_by(|a, b| a.plugin.cmp(&b.plugin));

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].plugin, "jamf");
        assert_eq!(result[1].plugin, "level");
        assert_eq!(result[2].plugin, "ninja");
    }

    fn atera_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("atera-{connection_id}.json"))
    }

    fn pulseway_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("pulseway-{connection_id}.json"))
    }

    fn kaseya_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("kaseya-{connection_id}.json"))
    }

    fn action1_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("action1-{connection_id}.json"))
    }

    fn dattormm_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("dattormm-{connection_id}.json"))
    }

    fn acronis_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
        plugin_cache_dir(data_dir).join(format!("acronis-{connection_id}.json"))
    }

    #[test]
    fn atera_device_appears_when_its_customer_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.atera_connections.push(AteraConnectionMeta {
            id: "atera-conn-1".to_string(),
            label: "ACME Atera".to_string(),
        });
        config.atera_customer_mappings.push(AteraCustomerMapping {
            connection_id: "atera-conn-1".to_string(),
            customer_id: "atera-cust-1".to_string(),
            customer_name: "ACME GmbH".to_string(),
            local_customer_id: 5,
        });
        let cache = CachedAteraSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![AteraCustomerDeviceGroupDto {
                atera_customer_id: "atera-cust-1".to_string(),
                atera_customer_name: "ACME GmbH".to_string(),
                customer_id: None,
                devices: vec![AteraExternalSystemDto {
                    external_id: "agent-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: Some("SRV-01".to_string()),
                    ip_address: Some("10.0.0.5".to_string()),
                    status: Some("online".to_string()),
                    platform: Some("Windows Server 2022".to_string()),
                    view_url: None,
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&atera_cache_path(dir.path(), "atera-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "atera");
        assert_eq!(result[0].connection_id, "atera-conn-1");
        assert_eq!(result[0].external_id, "agent-1");
        assert_eq!(result[0].hostname.as_deref(), Some("SRV-01"));
    }

    #[test]
    fn already_linked_atera_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.atera_connections.push(AteraConnectionMeta {
            id: "atera-conn-1".to_string(),
            label: "ACME Atera".to_string(),
        });
        config.atera_customer_mappings.push(AteraCustomerMapping {
            connection_id: "atera-conn-1".to_string(),
            customer_id: "atera-cust-1".to_string(),
            customer_name: "ACME GmbH".to_string(),
            local_customer_id: 5,
        });
        let cache = CachedAteraSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![AteraCustomerDeviceGroupDto {
                atera_customer_id: "atera-cust-1".to_string(),
                atera_customer_name: "ACME GmbH".to_string(),
                customer_id: Some(5),
                devices: vec![AteraExternalSystemDto {
                    external_id: "agent-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: None,
                    ip_address: None,
                    status: None,
                    platform: None,
                    view_url: None,
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&atera_cache_path(dir.path(), "atera-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn pulseway_device_appears_when_its_organization_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.pulseway_connections.push(PulsewayConnectionMeta {
            id: "pulseway-conn-1".to_string(),
            label: "ACME Pulseway".to_string(),
            base_url: "https://api.pulseway.com/v3".to_string(),
        });
        config.pulseway_org_mappings.push(PulsewayOrgMapping {
            connection_id: "pulseway-conn-1".to_string(),
            organization_id: "6978".to_string(),
            organization_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedPulsewaySyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![PulsewayOrgDeviceGroupDto {
                organization_id: "6978".to_string(),
                organization_name: "ACME GmbH".to_string(),
                customer_id: None,
                devices: vec![PulsewayExternalSystemDto {
                    external_id: "dev-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: Some("SRV-01".to_string()),
                    site_name: None,
                    group_name: None,
                    is_agent_installed: true,
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&pulseway_cache_path(dir.path(), "pulseway-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "pulseway");
        assert_eq!(result[0].connection_id, "pulseway-conn-1");
        assert_eq!(result[0].external_id, "dev-1");
        assert_eq!(result[0].ip_address, None);
    }

    #[test]
    fn already_linked_pulseway_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.pulseway_connections.push(PulsewayConnectionMeta {
            id: "pulseway-conn-1".to_string(),
            label: "ACME Pulseway".to_string(),
            base_url: "https://api.pulseway.com/v3".to_string(),
        });
        config.pulseway_org_mappings.push(PulsewayOrgMapping {
            connection_id: "pulseway-conn-1".to_string(),
            organization_id: "6978".to_string(),
            organization_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedPulsewaySyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![PulsewayOrgDeviceGroupDto {
                organization_id: "6978".to_string(),
                organization_name: "ACME GmbH".to_string(),
                customer_id: Some(5),
                devices: vec![PulsewayExternalSystemDto {
                    external_id: "dev-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: None,
                    site_name: None,
                    group_name: None,
                    is_agent_installed: true,
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&pulseway_cache_path(dir.path(), "pulseway-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn kaseya_device_appears_when_its_organization_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.kaseya_connections.push(KaseyaConnectionMeta {
            id: "kaseya-conn-1".to_string(),
            label: "ACME Kaseya".to_string(),
            base_url: "https://vsa.example.com/api".to_string(),
        });
        config.kaseya_org_mappings.push(KaseyaOrgMapping {
            connection_id: "kaseya-conn-1".to_string(),
            organization_id: "org-1".to_string(),
            organization_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedKaseyaSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![KaseyaOrgDeviceGroupDto {
                organization_id: "org-1".to_string(),
                organization_name: "ACME GmbH".to_string(),
                customer_id: None,
                devices: vec![KaseyaExternalSystemDto {
                    external_id: "machine-1".to_string(),
                    name: "SRV-01".to_string(),
                    organization_id: Some("org-1".to_string()),
                    organization_name: Some("ACME GmbH".to_string()),
                    group_id: None,
                    is_agent_installed: true,
                    is_mdm_enrolled: false,
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&kaseya_cache_path(dir.path(), "kaseya-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "kaseya");
        assert_eq!(result[0].connection_id, "kaseya-conn-1");
        assert_eq!(result[0].external_id, "machine-1");
        assert_eq!(result[0].hostname, None);
    }

    #[test]
    fn already_linked_kaseya_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.kaseya_connections.push(KaseyaConnectionMeta {
            id: "kaseya-conn-1".to_string(),
            label: "ACME Kaseya".to_string(),
            base_url: "https://vsa.example.com/api".to_string(),
        });
        config.kaseya_org_mappings.push(KaseyaOrgMapping {
            connection_id: "kaseya-conn-1".to_string(),
            organization_id: "org-1".to_string(),
            organization_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedKaseyaSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![KaseyaOrgDeviceGroupDto {
                organization_id: "org-1".to_string(),
                organization_name: "ACME GmbH".to_string(),
                customer_id: Some(5),
                devices: vec![KaseyaExternalSystemDto {
                    external_id: "machine-1".to_string(),
                    name: "SRV-01".to_string(),
                    organization_id: Some("org-1".to_string()),
                    organization_name: Some("ACME GmbH".to_string()),
                    group_id: None,
                    is_agent_installed: true,
                    is_mdm_enrolled: false,
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&kaseya_cache_path(dir.path(), "kaseya-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn action1_device_appears_when_its_organization_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.action1_connections.push(Action1ConnectionMeta {
            id: "action1-conn-1".to_string(),
            label: "ACME Action1".to_string(),
            base_url: "https://app.eu.action1.com/api/3.0".to_string(),
        });
        config.action1_org_mappings.push(Action1OrgMapping {
            connection_id: "action1-conn-1".to_string(),
            organization_id: "org-1".to_string(),
            organization_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedAction1SyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![Action1OrgDeviceGroupDto {
                organization_id: "org-1".to_string(),
                organization_name: "ACME GmbH".to_string(),
                customer_id: None,
                devices: vec![Action1ExternalSystemDto {
                    external_id: "endpoint-1".to_string(),
                    name: "SRV-01".to_string(),
                    ip_address: Some("10.0.0.5".to_string()),
                    status: Some("Connected".to_string()),
                    platform: Some("Windows".to_string()),
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&action1_cache_path(dir.path(), "action1-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "action1");
        assert_eq!(result[0].connection_id, "action1-conn-1");
        assert_eq!(result[0].external_id, "endpoint-1");
        assert_eq!(result[0].ip_address.as_deref(), Some("10.0.0.5"));
    }

    #[test]
    fn already_linked_action1_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.action1_connections.push(Action1ConnectionMeta {
            id: "action1-conn-1".to_string(),
            label: "ACME Action1".to_string(),
            base_url: "https://app.eu.action1.com/api/3.0".to_string(),
        });
        config.action1_org_mappings.push(Action1OrgMapping {
            connection_id: "action1-conn-1".to_string(),
            organization_id: "org-1".to_string(),
            organization_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedAction1SyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![Action1OrgDeviceGroupDto {
                organization_id: "org-1".to_string(),
                organization_name: "ACME GmbH".to_string(),
                customer_id: Some(5),
                devices: vec![Action1ExternalSystemDto {
                    external_id: "endpoint-1".to_string(),
                    name: "SRV-01".to_string(),
                    ip_address: None,
                    status: None,
                    platform: None,
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&action1_cache_path(dir.path(), "action1-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn dattormm_device_appears_when_its_site_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.dattormm_connections.push(DattoRmmConnectionMeta {
            id: "dattormm-conn-1".to_string(),
            label: "ACME Datto RMM".to_string(),
            base_url: "https://merlot-api.centrastage.net".to_string(),
        });
        config.dattormm_site_mappings.push(DattoRmmSiteMapping {
            connection_id: "dattormm-conn-1".to_string(),
            site_uid: "site-uid-1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 5,
        });
        let cache = CachedDattoRmmSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![DattoRmmSiteDeviceGroupDto {
                site_uid: "site-uid-1".to_string(),
                site_name: "ACME Hauptsitz".to_string(),
                portal_url: None,
                customer_id: None,
                devices: vec![DattoRmmExternalSystemDto {
                    external_id: "device-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: Some("SRV-01".to_string()),
                    ip_address: Some("10.0.0.5".to_string()),
                    status: Some("online".to_string()),
                    platform: Some("device".to_string()),
                    portal_url: None,
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&dattormm_cache_path(dir.path(), "dattormm-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "dattormm");
        assert_eq!(result[0].connection_id, "dattormm-conn-1");
        assert_eq!(result[0].external_id, "device-1");
        assert_eq!(result[0].hostname.as_deref(), Some("SRV-01"));
    }

    #[test]
    fn already_linked_dattormm_device_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.dattormm_connections.push(DattoRmmConnectionMeta {
            id: "dattormm-conn-1".to_string(),
            label: "ACME Datto RMM".to_string(),
            base_url: "https://merlot-api.centrastage.net".to_string(),
        });
        config.dattormm_site_mappings.push(DattoRmmSiteMapping {
            connection_id: "dattormm-conn-1".to_string(),
            site_uid: "site-uid-1".to_string(),
            site_name: "ACME Hauptsitz".to_string(),
            customer_id: 5,
        });
        let cache = CachedDattoRmmSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![DattoRmmSiteDeviceGroupDto {
                site_uid: "site-uid-1".to_string(),
                site_name: "ACME Hauptsitz".to_string(),
                portal_url: None,
                customer_id: Some(5),
                devices: vec![DattoRmmExternalSystemDto {
                    external_id: "device-1".to_string(),
                    name: "SRV-01".to_string(),
                    hostname: None,
                    ip_address: None,
                    status: None,
                    platform: None,
                    portal_url: None,
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&dattormm_cache_path(dir.path(), "dattormm-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn acronis_resource_appears_when_its_tenant_is_mapped_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.acronis_connections.push(AcronisConnectionMeta {
            id: "acronis-conn-1".to_string(),
            label: "ACME Acronis".to_string(),
            datacenter_url: "https://eu2-cloud.acronis.com".to_string(),
        });
        config.acronis_tenant_mappings.push(AcronisTenantMapping {
            connection_id: "acronis-conn-1".to_string(),
            tenant_id: "tenant-1".to_string(),
            tenant_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedAcronisSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![AcronisTenantResourceGroupDto {
                tenant_id: "tenant-1".to_string(),
                tenant_name: "ACME GmbH".to_string(),
                customer_id: None,
                devices: vec![AcronisResourceDto {
                    external_id: "resource-1".to_string(),
                    name: "SRV-01".to_string(),
                    backup_status: Some("ok".to_string()),
                    linked_system_id: None,
                }],
            }],
        };
        write_json(&acronis_cache_path(dir.path(), "acronis-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "acronis");
        assert_eq!(result[0].connection_id, "acronis-conn-1");
        assert_eq!(result[0].external_id, "resource-1");
        assert_eq!(result[0].hostname, None);
    }

    #[test]
    fn already_linked_acronis_resource_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.acronis_connections.push(AcronisConnectionMeta {
            id: "acronis-conn-1".to_string(),
            label: "ACME Acronis".to_string(),
            datacenter_url: "https://eu2-cloud.acronis.com".to_string(),
        });
        config.acronis_tenant_mappings.push(AcronisTenantMapping {
            connection_id: "acronis-conn-1".to_string(),
            tenant_id: "tenant-1".to_string(),
            tenant_name: "ACME GmbH".to_string(),
            customer_id: 5,
        });
        let cache = CachedAcronisSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            groups: vec![AcronisTenantResourceGroupDto {
                tenant_id: "tenant-1".to_string(),
                tenant_name: "ACME GmbH".to_string(),
                customer_id: Some(5),
                devices: vec![AcronisResourceDto {
                    external_id: "resource-1".to_string(),
                    name: "SRV-01".to_string(),
                    backup_status: None,
                    linked_system_id: Some(11),
                }],
            }],
        };
        write_json(&acronis_cache_path(dir.path(), "acronis-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 5).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn vultr_instance_appears_when_its_connection_is_bound_to_the_target_customer() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.vultr_connections.push(VultrConnectionMeta {
            id: "vultr-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Vultr".to_string(),
        });
        let cache = CachedVultrSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            instances: vec![VultrExternalSystemDto {
                external_id: "inst-1".to_string(),
                name: "Vultr Instance 1".to_string(),
                ip_address: Some("203.0.113.10".to_string()),
                ipv6_address: None,
                status: Some("running".to_string()),
                platform: Some("vc2-2c-4gb".to_string()),
                region: Some("ewr".to_string()),
                linked_system_id: None,
            }],
        };
        write_json(&vultr_cache_path(dir.path(), "vultr-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].plugin, "vultr");
        assert_eq!(result[0].connection_id, "vultr-conn-1");
        assert_eq!(result[0].external_id, "inst-1");
        assert_eq!(result[0].hostname, None);
        assert_eq!(result[0].ip_address.as_deref(), Some("203.0.113.10"));
    }

    #[test]
    fn already_linked_vultr_instance_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.vultr_connections.push(VultrConnectionMeta {
            id: "vultr-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Vultr".to_string(),
        });
        let cache = CachedVultrSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            instances: vec![VultrExternalSystemDto {
                external_id: "inst-1".to_string(),
                name: "Vultr Instance 1".to_string(),
                ip_address: None,
                ipv6_address: None,
                status: None,
                platform: None,
                region: None,
                linked_system_id: Some(3),
            }],
        };
        write_json(&vultr_cache_path(dir.path(), "vultr-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn vultr_connection_bound_to_a_different_customer_is_excluded() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.vultr_connections.push(VultrConnectionMeta {
            id: "vultr-conn-1".to_string(),
            customer_id: 99,
            label: "ACME Vultr".to_string(),
        });
        let cache = CachedVultrSyncDto {
            synced_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
            instances: vec![VultrExternalSystemDto {
                external_id: "inst-1".to_string(),
                name: "Vultr Instance 1".to_string(),
                ip_address: None,
                ipv6_address: None,
                status: None,
                platform: None,
                region: None,
                linked_system_id: None,
            }],
        };
        write_json(&vultr_cache_path(dir.path(), "vultr-conn-1"), &cache);

        let result =
            list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn vultr_connection_never_synced_is_skipped_gracefully_not_as_an_error() {
        let dir = tempdir().unwrap();
        let mut config = Config::default();
        config.vultr_connections.push(VultrConnectionMeta {
            id: "vultr-conn-1".to_string(),
            customer_id: 7,
            label: "ACME Vultr".to_string(),
        });

        let result = list_unlinked_external_systems_for_customer_pure(&config, dir.path(), 7);

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
