//! Tauri commands for the Pulseway plugin integration (see
//! `plugin::pulseway` and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers
//! following exactly the same pattern as `commands::tacticalrmm` -- a
//! Pulseway "connection" is a user-created record (base URL + Token ID/
//! Secret) for exactly one Pulseway account (cloud tenant or self-hosted
//! Enterprise Server instance), NOT for exactly one local customer. A single
//! connection can manage devices for multiple organizations (e.g. because
//! the user setting up the connection is themselves an MSP who runs several
//! of their own customers as separate Pulseway "Organizations"). Which
//! organization corresponds to which local customer (if any) is a separate,
//! granular mapping (`PulsewayOrgMapping`/`Config::pulseway_org_mappings`),
//! maintained by this module via `map_pulseway_organization`/
//! `unmap_pulseway_organization`. The fully qualified identifier
//! `"pulseway:<connection_id>"` serves both as the keyring account
//! (`plugin::secrets`) and as `external_refs.plugin_id`, so the existing
//! one-row-per-(system_id,plugin_id) upsert semantics keep working
//! unchanged.
//!
//! IMPORTANT, verified difference from `commands::tacticalrmm`'s
//! `group_agents_by_client`: Pulseway's device list DOES carry a genuine
//! numeric `OrganizationId` foreign key (see `plugin::pulseway` module
//! docs) -- so `group_devices_by_organization` below joins devices to
//! organizations by ID, exactly like `commands::plugins::
//! group_devices_by_organization` does for NinjaOne, NOT Tactical RMM's
//! name-based workaround.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::pulseway::{
    test_credentials, PulsewayCredentials, PulsewayDevice, PulsewayOrgMapping,
    PulsewayOrganization, PulsewayPlugin,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct PulsewayConnectionDto {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    /// Pulseway's own API reference documents `Name` AS the device's
    /// hostname (see `plugin::pulseway` module docs) -- there is no separate
    /// hostname field to carry through, so this is always `Some(name)`,
    /// exposed as its own field anyway (rather than making the frontend
    /// reuse `name`) so `matchKeyForDevice` here works exactly like every
    /// other plugin section's hostname-based "link to existing system"
    /// heuristic.
    pub hostname: Option<String>,
    /// Informational only -- NOT part of the organization-mapping join
    /// (that's `organization_id`, a real numeric foreign key, see module
    /// docs); just the device's own Pulseway Site name for display, since
    /// mapping granularity deliberately stops at the Organization level.
    pub site_name: Option<String>,
    /// Informational only, same rationale as `site_name` -- Pulseway's
    /// "Group" level, one further subdivision below Site, also not part of
    /// the mapping granularity.
    pub group_name: Option<String>,
    pub is_agent_installed: bool,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// devices of an unmapped organization -- without a `customer_id`
    /// there's no meaningful way to cross-reference against
    /// `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PulsewayOrganizationDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this organization hasn't been mapped to a local
    /// customer yet (`Config::pulseway_org_mappings` has no matching row for
    /// this connection+organization).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PulsewayOrgDeviceGroupDto {
    pub organization_id: String,
    pub organization_name: String,
    /// `None` if this organization isn't mapped to a local customer (yet) --
    /// in this case the frontend shows "not mapped" and disables linking
    /// this group's devices.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_pulseway_connection` run, cached under
/// `data_dir/plugin-cache/pulseway-<connection_id>.json` (see
/// `write_pulseway_cache`/`read_pulseway_cache`), so
/// `get_cached_pulseway_sync` works without network access -- exactly the
/// same pattern as `commands::tacticalrmm::CachedTacticalRmmSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedPulsewaySyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<PulsewayOrgDeviceGroupDto>,
}

fn to_dto(meta: &crate::plugin::pulseway::PulsewayConnectionMeta) -> PulsewayConnectionDto {
    PulsewayConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// The fully qualified `plugin_id` value for a Pulseway connection -- both
/// the keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("pulseway:{connection_id}")
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
        slug.push_str("pulseway");
    }
    slug
}

fn find_connection(
    config: &Config,
    connection_id: &str,
) -> Result<crate::plugin::pulseway::PulsewayConnectionMeta, AppError> {
    config
        .pulseway_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Pulseway-Verbindung {connection_id} nicht gefunden"
            ))
        })
}

/// Builds the runnable plugin object from a connection metadata row, plus
/// the associated credentials (Token ID/Secret, JSON-encoded, see
/// `plugin::pulseway::PulsewayCredentials`) from the keyring. Like Ninja,
/// the Pulseway secret is a small JSON object, not a single raw string
/// (unlike Tactical RMM/Level.io/Snipe-IT).
fn build_plugin(
    meta: &crate::plugin::pulseway::PulsewayConnectionMeta,
) -> Result<(PulsewayPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Pulseway-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        PulsewayPlugin::new(plugin_id, meta.base_url.clone()),
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

/// The same `data_dir/plugin-cache/` directory as every other plugin's
/// commands module -- one shared folder for all plugin cache files, just
/// with a different filename prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn pulseway_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("pulseway-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::tacticalrmm::write_tacticalrmm_cache`.
fn write_pulseway_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[PulsewayOrgDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedPulsewaySyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache).map_err(|e| {
        AppError::Plugin(format!("Pulseway-Cache konnte nicht kodiert werden: {e}"))
    })?;
    std::fs::write(pulseway_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_pulseway_cache`.
/// `Ok(None)` if this connection has never been synced -- not an error case,
/// analogous to `commands::tacticalrmm::read_tacticalrmm_cache`.
fn read_pulseway_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedPulsewaySyncDto>, AppError> {
    let path = pulseway_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedPulsewaySyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Pulseway-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(
    device: PulsewayDevice,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: device.external_id,
        hostname: Some(device.name.clone()),
        name: device.name,
        site_name: device.site_name,
        group_name: device.group_name,
        is_agent_installed: device.is_agent_installed,
        linked_system_id,
    }
}

/// Intermediate result of the pure grouping logic: an organization along
/// with its devices (still as `PulsewayDevice`, not as a DTO) and -- if
/// mapped -- the local `customer_id`. Analogous to
/// `commands::plugins::OrgGroup`/`commands::tacticalrmm::ClientGroup`.
struct OrgGroup {
    organization_id: String,
    organization_name: String,
    customer_id: Option<i64>,
    devices: Vec<PulsewayDevice>,
}

/// Groups devices by organization and enriches each group with the
/// configured `customer_id` mapping (if any). A pure function -- no network,
/// no database access -- so it's testable with hardcoded
/// `PulsewayOrganization`/`PulsewayDevice`/`PulsewayOrgMapping` values,
/// structurally analogous to `commands::plugins::group_devices_by_organization`
/// -- joined by ID, NOT Tactical RMM's name-based workaround, because
/// Pulseway's device list DOES carry a genuine numeric `OrganizationId`
/// foreign key (see `plugin::pulseway` module docs). An organization with no
/// devices at all still shows up as a group (empty `devices` list) so a
/// future UI can display it for mapping. Devices whose `organization_id`
/// doesn't match any organization reported by `organizations` (shouldn't
/// normally happen, but not impossible -- e.g. an organization deleted
/// between fetching organizations and devices in the same sync run) are not
/// silently dropped, but appended as their own leftover group -- using the
/// device's own `organization_name` (Pulseway devices carry this as a
/// redundant, per-device string, unlike NinjaOne) as the display name
/// instead of the bare ID, a small improvement over
/// `commands::plugins::group_devices_by_organization`'s leftover handling.
fn group_devices_by_organization(
    organizations: &[PulsewayOrganization],
    devices: &[PulsewayDevice],
    mappings: &[PulsewayOrgMapping],
    connection_id: &str,
) -> Vec<OrgGroup> {
    let mapped_customer_id = |organization_id: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.organization_id == organization_id)
            .map(|m| m.customer_id)
    };

    let mut devices_by_org: HashMap<String, Vec<PulsewayDevice>> = HashMap::new();
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
            let organization_name = org_devices
                .first()
                .map(|d| d.organization_name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| organization_id.clone());
            groups.push(OrgGroup {
                customer_id: mapped_customer_id(&organization_id),
                organization_name,
                organization_id,
                devices: org_devices,
            });
        }
    }

    groups
}

#[tauri::command]
pub fn test_pulseway_connection(
    base_url: String,
    token_id: String,
    token_secret: String,
) -> Result<(), AppError> {
    test_credentials(&base_url, &token_id, &token_secret)?;
    Ok(())
}

#[tauri::command]
pub fn list_pulseway_connections(
    state: State<AppState>,
) -> Result<Vec<PulsewayConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.pulseway_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_pulseway_connection(
    state: State<AppState>,
    label: String,
    base_url: String,
    token_id: String,
    token_secret: String,
) -> Result<PulsewayConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    let secret_json = serde_json::to_string(&PulsewayCredentials {
        token_id,
        token_secret,
    })
    .map_err(|e| AppError::Plugin(format!("Zugangsdaten konnten nicht kodiert werden: {e}")))?;
    plugin::secrets::store_secret(&plugin_id, &secret_json)?;

    let meta = crate::plugin::pulseway::PulsewayConnectionMeta {
        id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.pulseway_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_pulseway_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.pulseway_connections.len();
    config.pulseway_connections.retain(|c| c.id != id);
    if config.pulseway_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Pulseway-Verbindung {id} nicht gefunden"
        )));
    }
    // Cleanup: organization mappings for this connection are meaningless
    // without the connection and would otherwise be left behind as orphaned
    // data.
    config
        .pulseway_org_mappings
        .retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = pulseway_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Pulseway-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
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
/// `get_cached_pulseway_sync`/`sync_pulseway_connection`).
#[tauri::command]
pub fn list_pulseway_organizations(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<PulsewayOrganizationDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<PulsewayOrgMapping> = config
            .pulseway_org_mappings
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
            PulsewayOrganizationDto {
                id: org.id,
                name: org.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_pulseway_organization(
    state: State<AppState>,
    connection_id: String,
    organization_id: String,
    organization_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .pulseway_org_mappings
        .retain(|m| !(m.connection_id == connection_id && m.organization_id == organization_id));
    config.pulseway_org_mappings.push(PulsewayOrgMapping {
        connection_id,
        organization_id,
        organization_name,
        customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_pulseway_organization(
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
        .pulseway_org_mappings
        .retain(|m| !(m.connection_id == connection_id && m.organization_id == organization_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_pulseway_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<PulsewayOrgDeviceGroupDto>, AppError> {
    let (plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<PulsewayOrgMapping> = config
            .pulseway_org_mappings
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
    // `commands::tacticalrmm::sync_tacticalrmm_connection` uses: a device
    // stays "linked" in the UI even if its organization mapping was
    // corrected to a different customer AFTER the link was made, because
    // `db::external_refs::list_for_plugin` searches independent of the
    // currently-mapped customer.
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
                // Only ALREADY-linked devices get the extra per-device
                // detail call (`GET /devices/{id}`) -- calling this for
                // every device during a sync would be an N+1 pattern against
                // Pulseway's ~3600-requests/hour rate limit (see
                // `plugin::pulseway` module docs), so it's deliberately
                // never done for unlinked devices.
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

        result.push(PulsewayOrgDeviceGroupDto {
            organization_id: group.organization_id,
            organization_name: group.organization_name,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_pulseway_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_pulseway_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedPulsewaySyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<PulsewayOrgMapping> = config
            .pulseway_org_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_pulseway_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-resolved here against the CURRENT
    // organization mappings instead of using the value frozen into the
    // cache file during the last `sync_pulseway_connection` run --
    // otherwise mapping or unmapping an organization would only become
    // visible after the next live sync, even though this exact command is
    // meant to show the frontend the current mapping state without network
    // access (analogous to `commands::tacticalrmm::get_cached_tacticalrmm_sync`).
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
pub fn link_system_to_pulseway(
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
pub fn unlink_system_from_pulseway(
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
pub fn get_pulseway_system_details(
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
        assert_eq!(slugify("***"), "pulseway");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_pulseway_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "pulseway:acme-123");
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
        config
            .pulseway_connections
            .push(crate::plugin::pulseway::PulsewayConnectionMeta {
                id: "acme-1".to_string(),
                label: "ACME".to_string(),
                base_url: "https://api.pulseway.com/v3".to_string(),
            });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    fn sample_org(id: &str, name: &str) -> PulsewayOrganization {
        PulsewayOrganization {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn sample_device(id: &str, organization_id: &str, organization_name: &str) -> PulsewayDevice {
        PulsewayDevice {
            external_id: id.to_string(),
            name: format!("Device {id}"),
            organization_id: organization_id.to_string(),
            organization_name: organization_name.to_string(),
            site_name: None,
            group_name: None,
            is_agent_installed: true,
        }
    }

    #[test]
    fn groups_devices_under_their_organization_by_id() {
        let organizations = vec![sample_org("1", "ACME"), sample_org("2", "Contoso")];
        let devices = vec![
            sample_device("d1", "1", "ACME"),
            sample_device("d2", "1", "ACME"),
            sample_device("d3", "2", "Contoso"),
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
        let mappings = vec![PulsewayOrgMapping {
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
        let mappings = vec![PulsewayOrgMapping {
            connection_id: "other-connection".to_string(),
            organization_id: "1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_organization(&organizations, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn devices_for_an_unlisted_organization_id_form_their_own_leftover_group() {
        let organizations = vec![sample_org("1", "ACME")];
        let devices = vec![sample_device("d9", "99", "Orphan Org")];

        let groups = group_devices_by_organization(&organizations, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups.iter().find(|g| g.organization_id == "99").unwrap();
        assert_eq!(leftover.devices.len(), 1);
        // The leftover group's display name is taken from the device's own
        // OrganizationName field (see doc comment on
        // group_devices_by_organization), NOT the raw ID.
        assert_eq!(leftover.organization_name, "Orphan Org");
    }

    #[test]
    fn leftover_group_falls_back_to_raw_id_when_organization_name_is_empty() {
        let organizations = vec![sample_org("1", "ACME")];
        let devices = vec![sample_device("d9", "99", "")];

        let groups = group_devices_by_organization(&organizations, &devices, &[], "conn-1");

        let leftover = groups.iter().find(|g| g.organization_id == "99").unwrap();
        assert_eq!(leftover.organization_name, "99");
    }

    fn sample_group() -> PulsewayOrgDeviceGroupDto {
        PulsewayOrgDeviceGroupDto {
            organization_id: "1".to_string(),
            organization_name: "ACME".to_string(),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "3f9c1e2a-1111".to_string(),
                name: "SRV-01".to_string(),
                hostname: Some("SRV-01".to_string()),
                site_name: Some("Hauptsitz".to_string()),
                group_name: Some("Server".to_string()),
                is_agent_installed: true,
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn pulseway_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_pulseway_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_pulseway_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].organization_id, "1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "3f9c1e2a-1111");
        assert!(loaded.groups[0].devices[0].is_agent_installed);
    }

    #[test]
    fn pulseway_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_pulseway_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn pulseway_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_pulseway_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_pulseway_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_pulseway_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
