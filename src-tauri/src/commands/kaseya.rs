//! Tauri commands for the Kaseya VSA plugin integration (see
//! `plugin::kaseya` and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers
//! following exactly the same pattern as `commands::tacticalrmm` -- a Kaseya
//! "connection" is a user-created record (base URL + Token ID/Secret) for
//! exactly one VSA server instance, NOT for exactly one local customer. A
//! single instance can manage devices for multiple organizations (e.g.
//! because the user setting up the connection is themselves an MSP who runs
//! several of their own customers as separate VSA "Organizations"). Which
//! organization corresponds to which local customer (if any) is a separate,
//! granular mapping (`KaseyaOrgMapping`/`Config::kaseya_org_mappings`),
//! maintained by this module via `map_kaseya_organization`/
//! `unmap_kaseya_organization`. The fully qualified identifier
//! `"kaseya:<connection_id>"` serves both as the keyring account
//! (`plugin::secrets`) and as `external_refs.plugin_id`, so the existing
//! one-row-per-(system_id,plugin_id) upsert semantics keep working
//! unchanged.
//!
//! IMPORTANT, verified difference from `commands::tacticalrmm`'s
//! `group_agents_by_client`: Kaseya VSA's device list carries a genuine
//! numeric/string `OrganizationId` foreign key (see `plugin::kaseya` module
//! docs) -- `group_devices_by_organization` below therefore joins devices to
//! organizations by ID, the same way `commands::plugins::
//! group_devices_by_organization` (NinjaOne) does, NOT by name like Tactical
//! RMM. Unlike NinjaOne, though, `KaseyaDevice.organization_id` is an
//! `Option` -- this integration doesn't fully trust every device object to
//! actually carry it (see the field-name-similarity doubt in the module
//! docs) -- so devices with NO organization id at all are bucketed
//! separately, under a fixed "Nicht zugeordnet" pseudo-group, distinct from
//! the "has an id, but it matches no known organization" leftover case that
//! NinjaOne/Tactical RMM already handle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::kaseya::{
    test_credentials, KaseyaConnectionMeta, KaseyaCredentials, KaseyaDevice, KaseyaOrgMapping,
    KaseyaOrganization, KaseyaPlugin,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

/// Sentinel `organization_id` for devices with NO organization membership at
/// all (`KaseyaDevice.organization_id == None`) -- distinct from a real,
/// just-unmatched organization id (see module docs). Never returned by the
/// real Kaseya API as an actual `Id` (empty string), so this can't collide
/// with a genuine organization.
const UNASSIGNED_ORGANIZATION_ID: &str = "";
const UNASSIGNED_ORGANIZATION_NAME: &str = "Nicht zugeordnet";

#[derive(Debug, Clone, serde::Serialize)]
pub struct KaseyaConnectionDto {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    /// Informational only, mirrors `KaseyaDevice.organization_id` -- NOT
    /// necessarily equal to the enclosing group's `organization_id` for the
    /// "Nicht zugeordnet" bucket (there it's always `None`, see module
    /// docs).
    pub organization_id: Option<String>,
    pub organization_name: Option<String>,
    pub group_id: Option<String>,
    pub is_agent_installed: bool,
    pub is_mdm_enrolled: bool,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// devices of an unmapped organization -- without a `customer_id`
    /// there's no meaningful way to cross-reference against
    /// `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct KaseyaOrganizationDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this organization hasn't been mapped to a local
    /// customer yet (`Config::kaseya_org_mappings` has no matching row for
    /// this connection+organization).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KaseyaOrgDeviceGroupDto {
    pub organization_id: String,
    pub organization_name: String,
    /// `None` if this organization isn't mapped to a local customer (yet) --
    /// in this case the frontend shows "nicht zugeordnet" and disables
    /// linking this group's devices. Always `None` for the synthetic "Nicht
    /// zugeordnet" bucket unless that bucket itself was explicitly mapped
    /// (nothing prevents mapping it like any other group, see module docs).
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_kaseya_connection` run, cached under
/// `data_dir/plugin-cache/kaseya-<connection_id>.json` (see
/// `write_kaseya_cache`/`read_kaseya_cache`), so `get_cached_kaseya_sync`
/// works without network access -- exactly the same pattern as
/// `commands::tacticalrmm::CachedTacticalRmmSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedKaseyaSyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<KaseyaOrgDeviceGroupDto>,
}

fn to_dto(meta: &KaseyaConnectionMeta) -> KaseyaConnectionDto {
    KaseyaConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// The fully qualified `plugin_id` value for a Kaseya connection -- both the
/// keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("kaseya:{connection_id}")
}

/// Generates a stable, low-collision connection ID from a user label,
/// following exactly the pattern of
/// `commands::tacticalrmm::generate_connection_id`.
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
        slug.push_str("kaseya");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<KaseyaConnectionMeta, AppError> {
    config
        .kaseya_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("Kaseya-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Builds the runnable plugin object from a connection metadata row, plus
/// the associated credentials (Token ID + Token Secret, JSON-encoded) from
/// the keyring -- the plugin itself decodes the JSON (see
/// `plugin::kaseya::parse_credentials`), this layer just passes the raw
/// secret string through, exactly like `commands::plugins::build_plugin`
/// (NinjaOne).
fn build_plugin(
    meta: &KaseyaConnectionMeta,
) -> Result<(KaseyaPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Kaseya-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        KaseyaPlugin::new(plugin_id, meta.base_url.clone()),
        PluginCredentials { secret },
    ))
}

/// Best-effort, analogous to
/// `commands::tacticalrmm::delete_keyring_secret_best_effort`.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The same `data_dir/plugin-cache/` directory as every other plugin -- one
/// shared folder for all plugin cache files, just with a different filename
/// prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn kaseya_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("kaseya-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::tacticalrmm::write_tacticalrmm_cache`.
fn write_kaseya_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[KaseyaOrgDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedKaseyaSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Kaseya-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(kaseya_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_kaseya_cache`.
/// `Ok(None)` if this connection has never been synced -- not an error case,
/// analogous to `commands::tacticalrmm::read_tacticalrmm_cache`.
fn read_kaseya_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedKaseyaSyncDto>, AppError> {
    let path = kaseya_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedKaseyaSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Kaseya-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(
    device: KaseyaDevice,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: device.external_id,
        name: device.name,
        organization_id: device.organization_id,
        organization_name: device.organization_name,
        group_id: device.group_id,
        is_agent_installed: device.is_agent_installed,
        is_mdm_enrolled: device.is_mdm_enrolled,
        linked_system_id,
    }
}

/// Intermediate result of the pure grouping logic: an organization along
/// with its devices (still as `KaseyaDevice`, not as a DTO) and -- if mapped
/// -- the local `customer_id`. Analogous to
/// `commands::plugins::OrgGroup`/`commands::tacticalrmm::ClientGroup`.
struct OrgGroup {
    organization_id: String,
    organization_name: String,
    customer_id: Option<i64>,
    devices: Vec<KaseyaDevice>,
}

/// Groups devices by organization and enriches each group with the
/// configured `customer_id` mapping (if any). A pure function -- no
/// network, no database access -- so it's testable with hardcoded
/// `KaseyaOrganization`/`KaseyaDevice`/`KaseyaOrgMapping` values.
///
/// Unlike Tactical RMM's name-based join, this joins by the genuine
/// `organization_id` foreign key (see `plugin::kaseya` module docs),
/// structurally like `commands::plugins::group_devices_by_organization`
/// (NinjaOne) -- with TWO leftover cases, not one:
///
/// 1. A device with `Some(organization_id)` that matches no organization
///    Kaseya reported (shouldn't normally happen, but the field-name doubt
///    documented in `plugin::kaseya` means it's plausible) -- appended as
///    its own leftover group keyed by the raw ID, same principle as
///    NinjaOne's leftover-organization handling.
/// 2. A device with `organization_id: None` entirely -- appended as its own
///    fixed "Nicht zugeordnet" bucket (`UNASSIGNED_ORGANIZATION_ID`/
///    `UNASSIGNED_ORGANIZATION_NAME`), always LAST, distinct from case 1.
///
/// An organization with no devices at all still shows up as a group (empty
/// `devices` list) so a future UI can display it for mapping.
fn group_devices_by_organization(
    organizations: &[KaseyaOrganization],
    devices: &[KaseyaDevice],
    mappings: &[KaseyaOrgMapping],
    connection_id: &str,
) -> Vec<OrgGroup> {
    let mapped_customer_id = |organization_id: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.organization_id == organization_id)
            .map(|m| m.customer_id)
    };

    let mut devices_by_org: HashMap<String, Vec<KaseyaDevice>> = HashMap::new();
    for device in devices {
        let key = device
            .organization_id
            .clone()
            .unwrap_or_else(|| UNASSIGNED_ORGANIZATION_ID.to_string());
        devices_by_org.entry(key).or_default().push(device.clone());
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

    // Case 1: a real (non-empty) organization_id that matched no known
    // organization.
    let mut leftover_org_ids: Vec<String> = devices_by_org
        .keys()
        .filter(|k| k.as_str() != UNASSIGNED_ORGANIZATION_ID)
        .cloned()
        .collect();
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

    // Case 2: devices with NO organization_id at all -- always last, its own
    // fixed bucket, distinct from case 1 above.
    if let Some(unassigned_devices) = devices_by_org.remove(UNASSIGNED_ORGANIZATION_ID) {
        groups.push(OrgGroup {
            organization_id: UNASSIGNED_ORGANIZATION_ID.to_string(),
            organization_name: UNASSIGNED_ORGANIZATION_NAME.to_string(),
            customer_id: mapped_customer_id(UNASSIGNED_ORGANIZATION_ID),
            devices: unassigned_devices,
        });
    }

    groups
}

#[tauri::command]
pub fn test_kaseya_connection(
    base_url: String,
    token_id: String,
    token_secret: String,
) -> Result<(), AppError> {
    test_credentials(&base_url, &token_id, &token_secret)?;
    Ok(())
}

#[tauri::command]
pub fn list_kaseya_connections(
    state: State<AppState>,
) -> Result<Vec<KaseyaConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.kaseya_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_kaseya_connection(
    state: State<AppState>,
    label: String,
    base_url: String,
    token_id: String,
    token_secret: String,
) -> Result<KaseyaConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    let secret_json = serde_json::to_string(&KaseyaCredentials {
        token_id,
        token_secret,
    })
    .map_err(|e| AppError::Plugin(format!("Zugangsdaten konnten nicht kodiert werden: {e}")))?;
    plugin::secrets::store_secret(&plugin_id, &secret_json)?;

    let meta = KaseyaConnectionMeta {
        id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.kaseya_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_kaseya_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.kaseya_connections.len();
    config.kaseya_connections.retain(|c| c.id != id);
    if config.kaseya_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Kaseya-Verbindung {id} nicht gefunden"
        )));
    }
    // Cleanup: organization mappings for this connection are meaningless
    // without the connection and would otherwise be left behind as orphaned
    // data.
    config.kaseya_org_mappings.retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = kaseya_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Kaseya-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

/// Live fetch of a connection's organization list. Analogous to
/// `commands::tacticalrmm::list_tacticalrmm_clients`/
/// `commands::plugins::list_ninja_organizations`, kept for symmetry and a
/// possible initial-setup use case -- normal frontend operation doesn't need
/// this command (cache-first, see
/// `get_cached_kaseya_sync`/`sync_kaseya_connection`).
#[tauri::command]
pub fn list_kaseya_organizations(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<KaseyaOrganizationDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<KaseyaOrgMapping> = config
            .kaseya_org_mappings
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
            KaseyaOrganizationDto {
                id: org.id,
                name: org.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_kaseya_organization(
    state: State<AppState>,
    connection_id: String,
    organization_id: String,
    organization_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .kaseya_org_mappings
        .retain(|m| !(m.connection_id == connection_id && m.organization_id == organization_id));
    config.kaseya_org_mappings.push(KaseyaOrgMapping {
        connection_id,
        organization_id,
        organization_name,
        customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_kaseya_organization(
    state: State<AppState>,
    connection_id: String,
    organization_id: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Not an error if no matching mapping exists -- the result (no mapping
    // left) is the same, analogous to `db::external_refs::delete`. Existing
    // `external_refs` links are left untouched: unmapping an organization is
    // deliberately not an automatic unlinking of its already-linked devices.
    config
        .kaseya_org_mappings
        .retain(|m| !(m.connection_id == connection_id && m.organization_id == organization_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_kaseya_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<KaseyaOrgDeviceGroupDto>, AppError> {
    let (plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<KaseyaOrgMapping> = config
            .kaseya_org_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings, config.data_dir.clone())
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

    // Reverse index external-id -> local system_id, built ONCE from ALL
    // external_refs of this plugin -- exactly the pattern
    // `commands::plugins::sync_ninja_connection`/
    // `commands::tacticalrmm::sync_tacticalrmm_connection` use: a device
    // stays "linked" in the UI even if its organization mapping was
    // corrected to a different customer AFTER the link was made.
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
            device_dtos.push(to_external_system_dto(device, linked_system_id));
        }

        result.push(KaseyaOrgDeviceGroupDto {
            organization_id: group.organization_id,
            organization_name: group.organization_name,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_kaseya_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_kaseya_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedKaseyaSyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<KaseyaOrgMapping> = config
            .kaseya_org_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_kaseya_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-resolved here against the CURRENT
    // organization mappings instead of using the value frozen into the
    // cache file during the last `sync_kaseya_connection` run -- otherwise
    // mapping or unmapping an organization would only become visible after
    // the next live sync, even though this exact command is meant to show
    // the frontend the current mapping state without network access
    // (analogous to `commands::tacticalrmm::get_cached_tacticalrmm_sync`).
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
pub fn link_system_to_kaseya(
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
pub fn unlink_system_from_kaseya(
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
pub fn get_kaseya_system_details(
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
        assert_eq!(slugify("***"), "kaseya");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_kaseya_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "kaseya:acme-123");
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
        config.kaseya_connections.push(KaseyaConnectionMeta {
            id: "acme-1".to_string(),
            label: "ACME".to_string(),
            base_url: "https://vsa.example.com/api".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    fn sample_org(id: &str, name: &str) -> KaseyaOrganization {
        KaseyaOrganization {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn sample_device(id: &str, org_id: Option<&str>) -> KaseyaDevice {
        KaseyaDevice {
            external_id: id.to_string(),
            name: format!("Device {id}"),
            organization_id: org_id.map(str::to_string),
            organization_name: None,
            group_id: None,
            is_agent_installed: true,
            is_mdm_enrolled: false,
        }
    }

    #[test]
    fn groups_devices_under_their_organization_by_id() {
        let organizations = vec![sample_org("1", "ACME"), sample_org("2", "Contoso")];
        let devices = vec![
            sample_device("d1", Some("1")),
            sample_device("d2", Some("1")),
            sample_device("d3", Some("2")),
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
        let organizations = vec![sample_org("1", "ACME")];
        let groups = group_devices_by_organization(&organizations, &[], &[], "conn-1");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].devices.is_empty());
    }

    #[test]
    fn mapped_organization_carries_its_customer_id() {
        let organizations = vec![sample_org("1", "ACME")];
        let mappings = vec![KaseyaOrgMapping {
            connection_id: "conn-1".to_string(),
            organization_id: "1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_organization(&organizations, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, Some(42));
    }

    #[test]
    fn unmapped_organization_has_no_customer_id() {
        let organizations = vec![sample_org("1", "ACME")];
        let groups = group_devices_by_organization(&organizations, &[], &[], "conn-1");
        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn mapping_for_a_different_connection_is_ignored() {
        let organizations = vec![sample_org("1", "ACME")];
        let mappings = vec![KaseyaOrgMapping {
            connection_id: "other-connection".to_string(),
            organization_id: "1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_organization(&organizations, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn devices_with_an_unmatched_organization_id_form_their_own_leftover_group() {
        let organizations = vec![sample_org("1", "ACME")];
        let devices = vec![sample_device("d9", Some("orphan-org"))];

        let groups = group_devices_by_organization(&organizations, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups
            .iter()
            .find(|g| g.organization_id == "orphan-org")
            .unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.organization_name, "orphan-org");
    }

    #[test]
    fn devices_with_no_organization_id_at_all_form_the_nicht_zugeordnet_bucket() {
        let organizations = vec![sample_org("1", "ACME")];
        let devices = vec![
            sample_device("d1", Some("1")),
            sample_device("d10", None),
            sample_device("d11", None),
        ];

        let groups = group_devices_by_organization(&organizations, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let unassigned = groups.last().unwrap();
        assert_eq!(unassigned.organization_id, "");
        assert_eq!(unassigned.organization_name, "Nicht zugeordnet");
        assert_eq!(unassigned.devices.len(), 2);
    }

    #[test]
    fn unassigned_bucket_is_absent_when_every_device_has_an_organization() {
        let organizations = vec![sample_org("1", "ACME")];
        let devices = vec![sample_device("d1", Some("1"))];

        let groups = group_devices_by_organization(&organizations, &devices, &[], "conn-1");

        assert!(groups.iter().all(|g| !g.organization_id.is_empty()));
    }

    fn sample_group() -> KaseyaOrgDeviceGroupDto {
        KaseyaOrgDeviceGroupDto {
            organization_id: "1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "abc123".to_string(),
                name: "SRV-01".to_string(),
                organization_id: Some("1".to_string()),
                organization_name: Some("ACME".to_string()),
                group_id: Some("grp-1".to_string()),
                is_agent_installed: true,
                is_mdm_enrolled: false,
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn kaseya_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_kaseya_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_kaseya_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].organization_id, "1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "abc123");
        assert!(loaded.groups[0].devices[0].is_agent_installed);
    }

    #[test]
    fn kaseya_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_kaseya_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn kaseya_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_kaseya_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_kaseya_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_kaseya_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
