//! Tauri commands for the NinjaOne plugin integration (see `plugin::ninja`
//! and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers following the pattern
//! of `commands::export`: just take `State<AppState>` here, grab a pooled
//! connection/the config mutex, and pass through to the actual logic
//! (plugin trait, `db::external_refs`, `Config`).
//!
//! A Ninja "connection" is a user-created record (base URL + credentials)
//! for exactly one Ninja tenant -- NOT for exactly one local customer. A
//! Ninja tenant itself models multiple "organizations" (e.g. because the
//! user creating the connection is themselves an MSP and manages several of
//! their own customers as separate organizations in Ninja). Which
//! organization corresponds to which local customer (if any) is a separate,
//! granular mapping (`NinjaOrgMapping`/`Config::ninja_org_mappings`) that
//! this module maintains via `map_ninja_organization`/
//! `unmap_ninja_organization`. The fully-qualified identifier
//! `"ninja:<connection_id>"` serves both as the key store account
//! (`plugin::secrets`) and as `external_refs.plugin_id`, so the existing
//! one-row-per-(system_id,plugin_id) upsert semantics keep working
//! unchanged.
//!
//! Besides the Ninja commands, this module also hosts the one genuinely
//! cross-cutting plugin command: `sync_all_plugins_for_customer`, which
//! syncs every connection of every plugin that is relevant to one local
//! customer by calling the plugins' own `sync_<plugin>_connection`
//! functions. It lives here rather than in a module of its own for the same
//! reason `NinjaOrgDeviceGroupDto`/`ExternalSystemDto` do: it belongs to no
//! single plugin. Its offline read-only counterpart (same 17 plugins, but
//! from the on-disk sync caches instead of the network) is
//! `commands::external_directory::list_unlinked_external_systems_for_customer`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tauri::State;

use crate::commands::abm::sync_abm_connection;
use crate::commands::acronis::sync_acronis_connection;
use crate::commands::action1::sync_action1_connection;
use crate::commands::atera::sync_atera_connection;
use crate::commands::dattormm::sync_dattormm_connection;
use crate::commands::hetzner::sync_hetzner_connection;
use crate::commands::intune::sync_intune_connection;
use crate::commands::iru::sync_iru_connection;
use crate::commands::jamf::sync_jamf_connection;
use crate::commands::kaseya::sync_kaseya_connection;
use crate::commands::level::sync_level_connection;
use crate::commands::netcup::sync_netcup_connection;
use crate::commands::pulseway::sync_pulseway_connection;
use crate::commands::snipeit::sync_snipeit_connection;
use crate::commands::tacticalrmm::sync_tacticalrmm_connection;
use crate::commands::vultr::sync_vultr_connection;
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
    /// First address from Ninja's `ipAddresses` array, with `publicIP` as a
    /// fallback (see `plugin::ninja::map_ninja_device`).
    pub ip_address: Option<String>,
    /// Direct link to the device dashboard in the Ninja web UI, constructed
    /// from the connection's `base_url` and the external device ID.
    pub ninja_url: String,
    /// See `plugin::ninja::NinjaDevice::node_class`.
    pub node_class: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// devices of an unmapped organization -- without a `customer_id`
    /// there's no meaningful way to cross-reference against
    /// `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NinjaOrganizationDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this organization hasn't been mapped to a local
    /// customer yet (`Config::ninja_org_mappings` has no matching row for
    /// this connection+organization).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NinjaOrgDeviceGroupDto {
    pub organization_id: String,
    pub organization_name: String,
    /// `None` if this organization isn't mapped to a local customer (yet) --
    /// in this case a future frontend should show "unmapped" and disable
    /// linking the devices in this group.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_ninja_connection` run, cached under
/// `data_dir/plugin-cache/ninja-<connection_id>.json` (see
/// `write_ninja_cache`/`read_ninja_cache`), so `get_cached_ninja_sync` works
/// without network access. Needs `Deserialize` (for reading back) in
/// addition to `Serialize` (for writing) -- both are needed for the cache
/// file round trip.
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

/// The fully-qualified `plugin_id` value for a Ninja connection -- both the
/// key store account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("ninja:{connection_id}")
}

/// Generates a stable, low-collision connection ID from a user label: a
/// URL-/filename-safe slug of the label plus a millisecond timestamp
/// suffix. No extra `uuid` crate needed -- these IDs are generated rarely
/// (interactively, "create connection"), never in a hot loop, a timestamp
/// is enough of a uniqueness guarantee.
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

/// Builds the runnable plugin object plus its associated credentials from
/// the key store, based on a connection metadata row.
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

/// `plugin::secrets` deliberately only offers `store_secret`/`load_secret`
/// (see the credential principle in `docs/PLUGIN_ARCHITECTURE.md`) -- no
/// delete, because that module is out of scope for this change. For the
/// rare "remove connection" case, a direct, local access with the same
/// service name (`"wartungsdoku"`) is enough, purely best-effort: a missing
/// or non-deletable key store account must not fail the entire
/// `remove_ninja_connection` action.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Directory for plugin-specific cache files (`data_dir/plugin-cache/`).
/// Currently only used for Ninja sync snapshots, but deliberately not named
/// `ninja-cache` -- a future additional plugin can share the same folder.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn ninja_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("ninja-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, for later offline
/// reading via `read_ninja_cache`/`get_cached_ninja_sync`. Overwrites any
/// older snapshot that may exist for the same connection. Extracted into
/// its own function (instead of inline code in `sync_ninja_connection`), so
/// it can be tested in isolation with `tempfile::tempdir()`, analogous to
/// `Config::save`/`Config::load_or_default`.
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

/// Reads back a snapshot previously written via `write_ninja_cache`.
/// `Ok(None)` if this connection was never synced (file doesn't exist) --
/// not an error case. Extracted into its own function (instead of inline
/// code in the Tauri command), so it can be tested in isolation without
/// `State<AppState>`.
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
        node_class: device.node_class,
        linked_system_id,
    }
}

/// Intermediate result of the pure grouping logic: an organization along
/// with its devices (still as `NinjaDevice`, not as a DTO) and -- if mapped
/// -- the local `customer_id`. Kept separate from `NinjaOrgDeviceGroupDto`,
/// because the latter already expects finished `ExternalSystemDto` values
/// (including `ninja_url`/`linked_system_id`), which can only be built
/// after this grouping (`ninja_url` needs `base_url`, `linked_system_id`
/// needs a database access).
struct OrgGroup {
    organization_id: String,
    organization_name: String,
    customer_id: Option<i64>,
    devices: Vec<NinjaDevice>,
}

/// Groups devices by organization and enriches each group with the
/// configured `customer_id` mapping (if any). Pure function -- no network
/// access, no database access -- so it's testable with hardcoded
/// `NinjaOrganization`/`NinjaDevice`/`NinjaOrgMapping` values. An
/// organization with no devices at all still appears as a group (empty
/// `devices` list), so a future UI can show it for mapping. Devices whose
/// `organizationId` doesn't match any organization reported by
/// `organizations` (shouldn't happen per Ninja's data model) are not
/// silently dropped, but appended as their own group under the raw
/// organization ID.
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
    // Cleanup: organization mappings for this connection are meaningless
    // without the connection and would otherwise be left behind as dead data.
    config.ninja_org_mappings.retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    // Also best-effort: a leftover cache file for a removed connection is
    // just dead weight, but its absence is harmless (creating a new
    // connection with the same ID is practically impossible, see
    // `generate_connection_id`).
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
    // Not an error if no matching mapping exists -- the result (no mapping
    // present anymore) is the same, analogous to `db::external_refs::delete`.
    // Existing `external_refs` links are left untouched: unmapping an
    // organization is deliberately not an automatic unlinking of its
    // already-linked devices.
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

    // Reverse index external-id -> local system_id, built from ALL
    // external_refs of this plugin -- ONCE for the whole connection, not
    // rebuilt per group and not restricted to the systems of the group's
    // `customer_id`. Previously this searched per group only within
    // `list_by_customer(group.customer_id)`; that made an actually linked
    // device incorrectly appear as "not linked" (linked_system_id: None) as
    // soon as its organization was mapped to a DIFFERENT customer AFTER
    // linking (e.g. because the original mapping was a mistake and got
    // corrected) -- the linked system then sits under the old customer, not
    // under `group.customer_id`, so it was never found.
    // `db::external_refs::list_for_plugin` deliberately searches
    // independent of customer (see its doc comment), matching the fact
    // that `unmap_ninja_organization` deliberately does NOT touch links
    // when only the mapping changes.
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

/// One of the 17 plugin integrations `sync_all_plugins_for_customer` knows
/// about. Deliberately an enum rather than the plugin's name as a plain
/// string: the match in `sync_customer_target` is then exhaustive and
/// compiler-checked, so adding an 18th plugin cannot silently fall through
/// into a "nothing to sync" branch -- the missing arm is a build error.
///
/// Splits the same way `Config` does: the first ten variants are org-mapped
/// (one connection can serve several customers, which of its remote
/// organizations/sites/tenants/companies belongs to which local customer
/// lives in a separate `<plugin>_..._mappings` list), the last seven are
/// direct-customer (the connection itself carries `customer_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncPlugin {
    Ninja,
    Atera,
    TacticalRmm,
    DattoRmm,
    Action1,
    Jamf,
    Kaseya,
    Pulseway,
    Snipeit,
    Acronis,
    Level,
    Intune,
    Iru,
    Abm,
    Hetzner,
    Netcup,
    Vultr,
}

impl SyncPlugin {
    /// Human-readable product name for display in the result rows, NOT the
    /// internal `plugin_id` prefix (`"ninja"`, `"atera"`, ... -- those are
    /// what `commands::external_directory::UnlinkedExternalSystemDto::plugin`
    /// carries, because the frontend dispatches on them). Spelled the way
    /// the rest of the app already spells these products in user-facing
    /// text, e.g. the "Snipe-IT-Verbindung ... nicht gefunden" errors.
    pub fn display_name(self) -> &'static str {
        match self {
            SyncPlugin::Ninja => "Ninja",
            SyncPlugin::Atera => "Atera",
            SyncPlugin::TacticalRmm => "Tactical RMM",
            SyncPlugin::DattoRmm => "Datto RMM",
            SyncPlugin::Action1 => "Action1",
            SyncPlugin::Jamf => "Jamf",
            SyncPlugin::Kaseya => "Kaseya",
            SyncPlugin::Pulseway => "Pulseway",
            SyncPlugin::Snipeit => "Snipe-IT",
            SyncPlugin::Acronis => "Acronis",
            SyncPlugin::Level => "Level",
            SyncPlugin::Intune => "Intune",
            SyncPlugin::Iru => "Iru",
            SyncPlugin::Abm => "ABM",
            SyncPlugin::Hetzner => "Hetzner",
            SyncPlugin::Netcup => "netcup",
            SyncPlugin::Vultr => "Vultr",
        }
    }
}

/// One connection that `sync_all_plugins_for_customer` is going to sync,
/// resolved out of `Config` before any network access happens (and thus
/// before the config mutex is released again).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerSyncTarget {
    pub plugin: SyncPlugin,
    pub connection_id: String,
    pub connection_label: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CustomerPluginSyncResultDto {
    /// Human-readable plugin name for display, e.g. "Ninja", "Atera",
    /// "Hetzner" -- not the internal plugin_id prefix.
    pub plugin: String,
    pub connection_label: String,
    /// `Some(count)` on success, `None` on failure (see `error`).
    pub device_count: Option<i64>,
    /// `Some(message)` on failure, `None` on success.
    pub error: Option<String>,
}

/// Appends the connections of ONE org-mapped plugin that are relevant to the
/// customer whose mappings produced `mapped_connection_ids`.
///
/// Two things this deliberately handles rather than trusting the config:
/// the same connection appearing several times in `mapped_connection_ids`
/// (a connection with two of its organizations mapped to the SAME customer
/// must be synced exactly once, not twice -- one sync already returns the
/// devices of ALL of that connection's organizations), and a mapping whose
/// `connection_id` no longer exists in the plugin's own connection list
/// (leftover/hand-edited config -- skipped silently, never a panic, matching
/// the "a single bad record must not break the whole operation" philosophy
/// of `commands::external_directory`).
fn push_org_mapped_targets<'a>(
    plugin: SyncPlugin,
    mapped_connection_ids: impl IntoIterator<Item = &'a str>,
    connections: &[(&str, &str)],
    out: &mut Vec<CustomerSyncTarget>,
) {
    let mut seen: HashSet<&str> = HashSet::new();
    for connection_id in mapped_connection_ids {
        if !seen.insert(connection_id) {
            continue;
        }
        let Some((_, label)) = connections.iter().find(|(id, _)| *id == connection_id) else {
            continue;
        };
        out.push(CustomerSyncTarget {
            plugin,
            connection_id: connection_id.to_string(),
            connection_label: (*label).to_string(),
        });
    }
}

/// Appends the connections of ONE direct-customer plugin. No dedup and no
/// connection lookup needed here: `connections` already IS the plugin's own
/// connection list filtered down to this customer, so every entry is a
/// distinct connection that carries its own label.
fn push_direct_targets<'a>(
    plugin: SyncPlugin,
    connections: impl IntoIterator<Item = (&'a str, &'a str)>,
    out: &mut Vec<CustomerSyncTarget>,
) {
    for (connection_id, label) in connections {
        out.push(CustomerSyncTarget {
            plugin,
            connection_id: connection_id.to_string(),
            connection_label: label.to_string(),
        });
    }
}

/// Every plugin connection relevant to `customer_id`, across all 17
/// integrations. Pure function over `Config` -- no network access, no
/// database access, no `State<AppState>` -- so it is directly testable,
/// exactly like `commands::external_directory::list_unlinked_external_systems_for_customer_pure`.
///
/// NOTE on Atera: `AteraCustomerMapping` is the one mapping struct that does
/// NOT follow the `customer_id`-means-local-customer convention of the other
/// nine. Its `customer_id` is Atera's OWN customer id (a `String`), and the
/// local one is `local_customer_id` -- see the `plugin::atera` module docs
/// and `commands::external_directory::collect_atera`, which documents the
/// same deviation.
pub fn sync_targets_for_customer(config: &Config, customer_id: i64) -> Vec<CustomerSyncTarget> {
    let mut targets = Vec::new();

    // --- Org-mapped plugins (10) -------------------------------------
    push_org_mapped_targets(
        SyncPlugin::Ninja,
        config
            .ninja_org_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .ninja_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::Atera,
        config
            .atera_customer_mappings
            .iter()
            .filter(|m| m.local_customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .atera_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::TacticalRmm,
        config
            .tacticalrmm_client_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .tacticalrmm_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::DattoRmm,
        config
            .dattormm_site_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .dattormm_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::Action1,
        config
            .action1_org_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .action1_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::Jamf,
        config
            .jamf_site_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .jamf_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::Kaseya,
        config
            .kaseya_org_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .kaseya_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::Pulseway,
        config
            .pulseway_org_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .pulseway_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::Snipeit,
        config
            .snipeit_company_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .snipeit_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );
    push_org_mapped_targets(
        SyncPlugin::Acronis,
        config
            .acronis_tenant_mappings
            .iter()
            .filter(|m| m.customer_id == customer_id)
            .map(|m| m.connection_id.as_str()),
        &config
            .acronis_connections
            .iter()
            .map(|c| (c.id.as_str(), c.label.as_str()))
            .collect::<Vec<_>>(),
        &mut targets,
    );

    // --- Direct-customer plugins (7) ---------------------------------
    push_direct_targets(
        SyncPlugin::Level,
        config
            .level_connections
            .iter()
            .filter(|c| c.customer_id == customer_id)
            .map(|c| (c.id.as_str(), c.label.as_str())),
        &mut targets,
    );
    push_direct_targets(
        SyncPlugin::Intune,
        config
            .intune_connections
            .iter()
            .filter(|c| c.customer_id == customer_id)
            .map(|c| (c.id.as_str(), c.label.as_str())),
        &mut targets,
    );
    push_direct_targets(
        SyncPlugin::Iru,
        config
            .iru_connections
            .iter()
            .filter(|c| c.customer_id == customer_id)
            .map(|c| (c.id.as_str(), c.label.as_str())),
        &mut targets,
    );
    push_direct_targets(
        SyncPlugin::Abm,
        config
            .abm_connections
            .iter()
            .filter(|c| c.customer_id == customer_id)
            .map(|c| (c.id.as_str(), c.label.as_str())),
        &mut targets,
    );
    push_direct_targets(
        SyncPlugin::Hetzner,
        config
            .hetzner_connections
            .iter()
            .filter(|c| c.customer_id == customer_id)
            .map(|c| (c.id.as_str(), c.label.as_str())),
        &mut targets,
    );
    push_direct_targets(
        SyncPlugin::Netcup,
        config
            .netcup_connections
            .iter()
            .filter(|c| c.customer_id == customer_id)
            .map(|c| (c.id.as_str(), c.label.as_str())),
        &mut targets,
    );
    push_direct_targets(
        SyncPlugin::Vultr,
        config
            .vultr_connections
            .iter()
            .filter(|c| c.customer_id == customer_id)
            .map(|c| (c.id.as_str(), c.label.as_str())),
        &mut targets,
    );

    targets
}

/// Device count of a group-shaped sync result (the 10 org-mapped plugins all
/// return `Vec<Some...GroupDto>`) that is actually attributable to
/// `customer_id`. Always filtering is essential, never blindly summing: the
/// very same connection can also serve OTHER customers through other
/// organizations, and one sync returns all of that connection's groups.
///
/// Takes `(group customer_id, group device count)` pairs rather than the
/// group DTOs themselves, because the ten group DTO types are ten unrelated
/// structs that merely happen to share those two fields -- this way the
/// counting rule exists exactly once instead of ten times.
fn count_devices_for_customer(
    groups: impl IntoIterator<Item = (Option<i64>, usize)>,
    customer_id: i64,
) -> i64 {
    groups
        .into_iter()
        .filter(|(group_customer_id, _)| *group_customer_id == Some(customer_id))
        .map(|(_, device_count)| device_count as i64)
        .sum()
}

/// Syncs exactly ONE target by calling that plugin's existing, already
/// correct `sync_<plugin>_connection` -- a plain Rust call, the
/// `#[tauri::command]` attribute on those functions does not stop other Rust
/// code from calling them -- and reduces its result to a device count for
/// `customer_id`.
///
/// Group-shaped results (the 10 org-mapped plugins) get filtered by group
/// `customer_id`; direct-vec results (the 7 direct-customer plugins) are
/// already scoped to this connection's single customer by construction, so
/// their count is just the vector length.
fn sync_customer_target(
    state: State<AppState>,
    target: &CustomerSyncTarget,
    customer_id: i64,
) -> Result<i64, AppError> {
    let connection_id = target.connection_id.clone();
    match target.plugin {
        // --- Group-shaped results (10) -------------------------------
        SyncPlugin::Ninja => sync_ninja_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        SyncPlugin::Atera => sync_atera_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        SyncPlugin::TacticalRmm => {
            sync_tacticalrmm_connection(state, connection_id).map(|groups| {
                count_devices_for_customer(
                    groups.iter().map(|g| (g.customer_id, g.devices.len())),
                    customer_id,
                )
            })
        }
        SyncPlugin::DattoRmm => sync_dattormm_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        SyncPlugin::Action1 => sync_action1_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        SyncPlugin::Jamf => sync_jamf_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        SyncPlugin::Kaseya => sync_kaseya_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        SyncPlugin::Pulseway => sync_pulseway_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        SyncPlugin::Snipeit => sync_snipeit_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        SyncPlugin::Acronis => sync_acronis_connection(state, connection_id).map(|groups| {
            count_devices_for_customer(
                groups.iter().map(|g| (g.customer_id, g.devices.len())),
                customer_id,
            )
        }),
        // --- Direct-vec results (7) ----------------------------------
        SyncPlugin::Level => {
            sync_level_connection(state, connection_id).map(|devices| devices.len() as i64)
        }
        SyncPlugin::Intune => {
            sync_intune_connection(state, connection_id).map(|devices| devices.len() as i64)
        }
        SyncPlugin::Iru => {
            sync_iru_connection(state, connection_id).map(|devices| devices.len() as i64)
        }
        SyncPlugin::Abm => {
            sync_abm_connection(state, connection_id).map(|devices| devices.len() as i64)
        }
        SyncPlugin::Hetzner => {
            sync_hetzner_connection(state, connection_id).map(|devices| devices.len() as i64)
        }
        SyncPlugin::Netcup => {
            sync_netcup_connection(state, connection_id).map(|devices| devices.len() as i64)
        }
        SyncPlugin::Vultr => {
            sync_vultr_connection(state, connection_id).map(|devices| devices.len() as i64)
        }
    }
}

/// Runs `sync_one` over every target and turns each outcome into its own
/// result row. A failing connection produces a row carrying its `error`
/// instead of a `device_count` and does NOT abort the batch -- the whole
/// point of the command, since one expired API token must not hide the
/// sixteen other plugins' results.
///
/// `sync_one` is a parameter rather than a hardcoded call to
/// `sync_customer_target`, so this batching rule is testable without a
/// `State<AppState>` (which cannot be constructed outside a running Tauri
/// app -- `tauri::State`'s single field is private and its only constructor
/// is the `CommandArg` impl).
fn collect_customer_sync_results(
    targets: Vec<CustomerSyncTarget>,
    mut sync_one: impl FnMut(&CustomerSyncTarget) -> Result<i64, AppError>,
) -> Vec<CustomerPluginSyncResultDto> {
    let mut results = Vec::with_capacity(targets.len());
    for target in targets {
        let outcome = sync_one(&target);
        results.push(match outcome {
            Ok(device_count) => CustomerPluginSyncResultDto {
                plugin: target.plugin.display_name().to_string(),
                connection_label: target.connection_label,
                device_count: Some(device_count),
                error: None,
            },
            Err(e) => CustomerPluginSyncResultDto {
                plugin: target.plugin.display_name().to_string(),
                connection_label: target.connection_label,
                device_count: None,
                error: Some(e.to_string()),
            },
        });
    }
    results
}

/// Syncs every plugin connection actually relevant to `customer_id` -- both
/// org-mapped connections (where at least one of the connection's remote
/// organizations/sites/tenants is mapped to this customer) and
/// direct-customer connections (where the connection itself belongs to this
/// customer). One result row per connection actually synced; a failure on
/// one connection is recorded in its own row's `error` field and does NOT
/// abort the rest -- matching this app's existing bulk operation
/// conventions.
///
/// The config is read ONCE up front and the config mutex released again
/// before any network I/O: every `sync_<plugin>_connection` below locks
/// `state.config` itself, so holding the outer lock across those calls would
/// deadlock on the first target.
#[tauri::command]
pub fn sync_all_plugins_for_customer(
    state: State<AppState>,
    customer_id: i64,
) -> Result<Vec<CustomerPluginSyncResultDto>, AppError> {
    let targets = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        sync_targets_for_customer(&config, customer_id)
    };

    // `tauri::State<'r, T>` is a `&'r T` newtype with a hand-written `Clone`
    // impl (but no `Copy`), see `tauri-2.11.5/src/state.rs` -- so the same
    // state can simply be handed to each nested sync call in turn.
    Ok(collect_customer_sync_results(targets, |target| {
        sync_customer_target(state.clone(), target, customer_id)
    }))
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

    #[test]
    fn to_external_system_dto_carries_node_class_through() {
        let device = NinjaDevice {
            external_id: "101".to_string(),
            name: "Server 01".to_string(),
            hostname: None,
            ip_address: None,
            node_class: Some("WINDOWS_SERVER".to_string()),
            organization_id: "1".to_string(),
        };

        let dto = to_external_system_dto("https://eu.ninjarmm.com", device, None);

        assert_eq!(dto.node_class, Some("WINDOWS_SERVER".to_string()));
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
            node_class: None,
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
                node_class: Some("WINDOWS_SERVER".to_string()),
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

    // --- sync_all_plugins_for_customer ------------------------------
    //
    // The `#[tauri::command]` itself can't be exercised in a unit test:
    // `tauri::State<'r, T>` wraps a private `&'r T` and its only
    // constructor is the `CommandArg` impl inside a running Tauri app. Its
    // two halves are therefore tested directly instead -- which is where
    // all the actual logic lives, exactly like
    // `commands::external_directory::list_unlinked_external_systems_for_customer_pure`
    // is tested rather than its own thin command wrapper:
    // `sync_targets_for_customer` (which connections are relevant, dedup,
    // graceful skipping) and `collect_customer_sync_results`/
    // `count_devices_for_customer` (batching, per-row errors, counting).

    use crate::plugin::abm::AbmConnectionMeta;
    use crate::plugin::acronis::{AcronisConnectionMeta, AcronisTenantMapping};
    use crate::plugin::action1::{Action1ConnectionMeta, Action1OrgMapping};
    use crate::plugin::atera::{AteraConnectionMeta, AteraCustomerMapping};
    use crate::plugin::dattormm::{DattoRmmConnectionMeta, DattoRmmSiteMapping};
    use crate::plugin::hetzner::HetznerConnectionMeta;
    use crate::plugin::intune::IntuneConnectionMeta;
    use crate::plugin::iru::IruConnectionMeta;
    use crate::plugin::jamf::{JamfConnectionMeta, JamfSiteMapping};
    use crate::plugin::kaseya::{KaseyaConnectionMeta, KaseyaOrgMapping};
    use crate::plugin::level::LevelConnectionMeta;
    use crate::plugin::netcup::NetcupConnectionMeta;
    use crate::plugin::pulseway::{PulsewayConnectionMeta, PulsewayOrgMapping};
    use crate::plugin::snipeit::{SnipeitCompanyMapping, SnipeitConnectionMeta};
    use crate::plugin::tacticalrmm::{TacticalRmmClientMapping, TacticalRmmConnectionMeta};
    use crate::plugin::vultr::VultrConnectionMeta;

    /// A config with one Ninja connection whose organization `org_id` is
    /// mapped to `customer_id`.
    fn ninja_config(connection_id: &str, org_id: &str, customer_id: i64) -> Config {
        let mut config = Config::default();
        config.ninja_connections.push(NinjaConnectionMeta {
            id: connection_id.to_string(),
            label: format!("Ninja {connection_id}"),
            base_url: "https://eu.ninjarmm.com".to_string(),
        });
        config.ninja_org_mappings.push(NinjaOrgMapping {
            connection_id: connection_id.to_string(),
            organization_id: org_id.to_string(),
            organization_name: format!("Organisation {org_id}"),
            customer_id,
        });
        config
    }

    fn plugins_of(targets: &[CustomerSyncTarget]) -> Vec<&'static str> {
        targets.iter().map(|t| t.plugin.display_name()).collect()
    }

    #[test]
    fn no_targets_at_all_for_a_customer_without_any_plugin_connections() {
        let targets = sync_targets_for_customer(&Config::default(), 42);
        assert!(targets.is_empty());
    }

    #[test]
    fn connections_of_another_customer_produce_no_targets() {
        // Both plugin architectures present, both pointing at customer 99.
        let mut config = ninja_config("ninja-1", "org-1", 99);
        config.hetzner_connections.push(HetznerConnectionMeta {
            id: "hetzner-1".to_string(),
            customer_id: 99,
            label: "ACME Hetzner".to_string(),
        });

        let targets = sync_targets_for_customer(&config, 42);

        assert!(targets.is_empty());
    }

    #[test]
    fn a_customer_without_targets_syncs_nothing_and_returns_an_empty_result() {
        let targets = sync_targets_for_customer(&Config::default(), 42);
        let mut calls = 0;
        let results = collect_customer_sync_results(targets, |_| {
            calls += 1;
            Ok(1)
        });

        assert!(results.is_empty());
        assert_eq!(calls, 0);
    }

    #[test]
    fn org_mapped_and_direct_customer_plugins_both_become_targets() {
        let mut config = ninja_config("ninja-1", "org-1", 42);
        config.hetzner_connections.push(HetznerConnectionMeta {
            id: "hetzner-1".to_string(),
            customer_id: 42,
            label: "ACME Hetzner".to_string(),
        });

        let targets = sync_targets_for_customer(&config, 42);

        assert_eq!(plugins_of(&targets), vec!["Ninja", "Hetzner"]);
        assert_eq!(targets[0].connection_id, "ninja-1");
        assert_eq!(targets[0].connection_label, "Ninja ninja-1");
        assert_eq!(targets[1].connection_id, "hetzner-1");
        assert_eq!(targets[1].connection_label, "ACME Hetzner");
    }

    #[test]
    fn a_connection_mapped_through_two_organizations_becomes_exactly_one_target() {
        // One Ninja connection, two of its organizations mapped to the SAME
        // customer -- syncing it twice would double-count its devices and
        // hit the Ninja API twice for nothing.
        let mut config = ninja_config("ninja-1", "org-1", 42);
        config.ninja_org_mappings.push(NinjaOrgMapping {
            connection_id: "ninja-1".to_string(),
            organization_id: "org-2".to_string(),
            organization_name: "Organisation org-2".to_string(),
            customer_id: 42,
        });

        let targets = sync_targets_for_customer(&config, 42);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].connection_id, "ninja-1");
    }

    #[test]
    fn two_different_connections_of_the_same_plugin_both_become_targets() {
        // Guards the dedup above from collapsing genuinely distinct
        // connections of one plugin.
        let mut config = ninja_config("ninja-1", "org-1", 42);
        config.ninja_connections.push(NinjaConnectionMeta {
            id: "ninja-2".to_string(),
            label: "Ninja ninja-2".to_string(),
            base_url: "https://eu.ninjarmm.com".to_string(),
        });
        config.ninja_org_mappings.push(NinjaOrgMapping {
            connection_id: "ninja-2".to_string(),
            organization_id: "org-9".to_string(),
            organization_name: "Organisation org-9".to_string(),
            customer_id: 42,
        });

        let targets = sync_targets_for_customer(&config, 42);

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].connection_id, "ninja-1");
        assert_eq!(targets[1].connection_id, "ninja-2");
    }

    #[test]
    fn a_mapping_referencing_a_removed_connection_is_skipped_gracefully() {
        // Leftover/hand-edited config: the mapping survived, its connection
        // didn't. Must be skipped, never panic.
        let mut config = ninja_config("ninja-1", "org-1", 42);
        config.ninja_connections.clear();

        let targets = sync_targets_for_customer(&config, 42);

        assert!(targets.is_empty());
    }

    #[test]
    fn atera_targets_follow_local_customer_id_not_ateras_own_customer_id() {
        // `AteraCustomerMapping` is the one mapping struct where
        // `customer_id` is the REMOTE (Atera) id and `local_customer_id` is
        // the local one -- see `plugin::atera`.
        let mut config = Config::default();
        config.atera_connections.push(AteraConnectionMeta {
            id: "atera-1".to_string(),
            label: "ACME Atera".to_string(),
        });
        config.atera_customer_mappings.push(AteraCustomerMapping {
            connection_id: "atera-1".to_string(),
            customer_id: "7".to_string(),
            customer_name: "ACME Hauptsitz".to_string(),
            local_customer_id: 42,
        });

        assert_eq!(
            plugins_of(&sync_targets_for_customer(&config, 42)),
            ["Atera"]
        );
        // 7 is Atera's own customer id, not a local one -- must not match.
        assert!(sync_targets_for_customer(&config, 7).is_empty());
    }

    /// One connection per plugin, all assigned/mapped to customer 42.
    fn config_with_every_plugin_mapped_to(customer_id: i64) -> Config {
        let mut config = ninja_config("ninja-1", "org-1", customer_id);

        config.atera_connections.push(AteraConnectionMeta {
            id: "atera-1".to_string(),
            label: "ACME Atera".to_string(),
        });
        config.atera_customer_mappings.push(AteraCustomerMapping {
            connection_id: "atera-1".to_string(),
            customer_id: "a-1".to_string(),
            customer_name: "ACME".to_string(),
            local_customer_id: customer_id,
        });

        config
            .tacticalrmm_connections
            .push(TacticalRmmConnectionMeta {
                id: "trmm-1".to_string(),
                label: "ACME Tactical RMM".to_string(),
                base_url: "https://api.rmm.example.com".to_string(),
            });
        config
            .tacticalrmm_client_mappings
            .push(TacticalRmmClientMapping {
                connection_id: "trmm-1".to_string(),
                client_id: "c-1".to_string(),
                client_name: "ACME".to_string(),
                customer_id,
            });

        config.dattormm_connections.push(DattoRmmConnectionMeta {
            id: "datto-1".to_string(),
            label: "ACME Datto RMM".to_string(),
            base_url: "https://pinotage-api.centrastage.net".to_string(),
        });
        config.dattormm_site_mappings.push(DattoRmmSiteMapping {
            connection_id: "datto-1".to_string(),
            site_uid: "s-1".to_string(),
            site_name: "ACME".to_string(),
            customer_id,
        });

        config.action1_connections.push(Action1ConnectionMeta {
            id: "action1-1".to_string(),
            label: "ACME Action1".to_string(),
            base_url: "https://app.action1.com".to_string(),
        });
        config.action1_org_mappings.push(Action1OrgMapping {
            connection_id: "action1-1".to_string(),
            organization_id: "o-1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id,
        });

        config.jamf_connections.push(JamfConnectionMeta {
            id: "jamf-1".to_string(),
            label: "ACME Jamf".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });
        config.jamf_site_mappings.push(JamfSiteMapping {
            connection_id: "jamf-1".to_string(),
            site_id: "s-1".to_string(),
            site_name: "ACME".to_string(),
            customer_id,
        });

        config.kaseya_connections.push(KaseyaConnectionMeta {
            id: "kaseya-1".to_string(),
            label: "ACME Kaseya".to_string(),
            base_url: "https://vsa.example.com".to_string(),
        });
        config.kaseya_org_mappings.push(KaseyaOrgMapping {
            connection_id: "kaseya-1".to_string(),
            organization_id: "o-1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id,
        });

        config.pulseway_connections.push(PulsewayConnectionMeta {
            id: "pulseway-1".to_string(),
            label: "ACME Pulseway".to_string(),
            base_url: "https://api.pulseway.com".to_string(),
        });
        config.pulseway_org_mappings.push(PulsewayOrgMapping {
            connection_id: "pulseway-1".to_string(),
            organization_id: "o-1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id,
        });

        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "snipeit-1".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });
        config.snipeit_company_mappings.push(SnipeitCompanyMapping {
            connection_id: "snipeit-1".to_string(),
            company_id: "c-1".to_string(),
            company_name: "ACME".to_string(),
            customer_id,
        });

        config.acronis_connections.push(AcronisConnectionMeta {
            id: "acronis-1".to_string(),
            label: "ACME Acronis".to_string(),
            datacenter_url: "https://eu2-cloud.acronis.com".to_string(),
        });
        config.acronis_tenant_mappings.push(AcronisTenantMapping {
            connection_id: "acronis-1".to_string(),
            tenant_id: "t-1".to_string(),
            tenant_name: "ACME".to_string(),
            customer_id,
        });

        config.level_connections.push(LevelConnectionMeta {
            id: "level-1".to_string(),
            customer_id,
            label: "ACME Level".to_string(),
        });
        config.intune_connections.push(IntuneConnectionMeta {
            id: "intune-1".to_string(),
            customer_id,
            label: "ACME Intune".to_string(),
        });
        config.iru_connections.push(IruConnectionMeta {
            id: "iru-1".to_string(),
            customer_id,
            label: "ACME Iru".to_string(),
            base_url: "https://acme.api.kandji.io".to_string(),
        });
        config.abm_connections.push(AbmConnectionMeta {
            id: "abm-1".to_string(),
            customer_id,
            label: "ACME ABM".to_string(),
        });
        config.hetzner_connections.push(HetznerConnectionMeta {
            id: "hetzner-1".to_string(),
            customer_id,
            label: "ACME Hetzner".to_string(),
        });
        config.netcup_connections.push(NetcupConnectionMeta {
            id: "netcup-1".to_string(),
            customer_id,
            label: "ACME netcup".to_string(),
        });
        config.vultr_connections.push(VultrConnectionMeta {
            id: "vultr-1".to_string(),
            customer_id,
            label: "ACME Vultr".to_string(),
        });

        config
    }

    #[test]
    fn all_seventeen_plugins_are_represented_in_the_target_building() {
        let config = config_with_every_plugin_mapped_to(42);

        let targets = sync_targets_for_customer(&config, 42);

        assert_eq!(
            plugins_of(&targets),
            vec![
                "Ninja",
                "Atera",
                "Tactical RMM",
                "Datto RMM",
                "Action1",
                "Jamf",
                "Kaseya",
                "Pulseway",
                "Snipe-IT",
                "Acronis",
                "Level",
                "Intune",
                "Iru",
                "ABM",
                "Hetzner",
                "netcup",
                "Vultr",
            ]
        );
    }

    #[test]
    fn every_plugins_connections_disappear_again_for_a_different_customer() {
        let config = config_with_every_plugin_mapped_to(42);
        assert!(sync_targets_for_customer(&config, 99).is_empty());
    }

    #[test]
    fn device_count_ignores_groups_belonging_to_other_customers() {
        // The same connection can serve several customers through several
        // organizations -- only this customer's groups may be counted.
        let groups = vec![(Some(42), 2), (Some(99), 5), (None, 3), (Some(42), 4)];
        assert_eq!(count_devices_for_customer(groups, 42), 6);
    }

    #[test]
    fn device_count_sums_both_organizations_of_one_connection() {
        let groups = vec![(Some(42), 2), (Some(42), 3)];
        assert_eq!(count_devices_for_customer(groups, 42), 5);
    }

    #[test]
    fn device_count_is_zero_when_no_group_belongs_to_the_customer() {
        assert_eq!(count_devices_for_customer(vec![(Some(99), 7)], 42), 0);
    }

    #[test]
    fn a_failing_connection_does_not_stop_the_other_connections_results() {
        let config = {
            let mut config = ninja_config("ninja-1", "org-1", 42);
            config.hetzner_connections.push(HetznerConnectionMeta {
                id: "hetzner-1".to_string(),
                customer_id: 42,
                label: "ACME Hetzner".to_string(),
            });
            config.vultr_connections.push(VultrConnectionMeta {
                id: "vultr-1".to_string(),
                customer_id: 42,
                label: "ACME Vultr".to_string(),
            });
            config
        };
        let targets = sync_targets_for_customer(&config, 42);

        // The middle connection fails the way a real one would (expired
        // token, unreachable host -- both surface as an `AppError` out of
        // `sync_<plugin>_connection`).
        let results = collect_customer_sync_results(targets, |target| {
            if target.connection_id == "hetzner-1" {
                Err(AppError::Plugin(
                    "Hetzner-API antwortete mit Status 401".to_string(),
                ))
            } else {
                Ok(3)
            }
        });

        assert_eq!(results.len(), 3);

        assert_eq!(results[0].plugin, "Ninja");
        assert_eq!(results[0].device_count, Some(3));
        assert!(results[0].error.is_none());

        assert_eq!(results[1].plugin, "Hetzner");
        assert_eq!(results[1].connection_label, "ACME Hetzner");
        assert_eq!(results[1].device_count, None);
        assert!(results[1].error.as_deref().unwrap().contains("Status 401"));

        // The connection AFTER the failing one still ran and reported.
        assert_eq!(results[2].plugin, "Vultr");
        assert_eq!(results[2].device_count, Some(3));
        assert!(results[2].error.is_none());
    }

    #[test]
    fn result_rows_carry_the_connection_label_not_the_connection_id() {
        let mut config = Config::default();
        config.netcup_connections.push(NetcupConnectionMeta {
            id: "netcup-1700000000000".to_string(),
            customer_id: 42,
            label: "ACME netcup".to_string(),
        });

        let results =
            collect_customer_sync_results(sync_targets_for_customer(&config, 42), |_| Ok(11));

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].plugin, "netcup");
        assert_eq!(results[0].connection_label, "ACME netcup");
        assert_eq!(results[0].device_count, Some(11));
    }
}
