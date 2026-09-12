//! Tauri commands for the Datto RMM plugin integration (see
//! `plugin::dattormm` and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers
//! following exactly the same pattern as `commands::jamf` -- a Datto RMM
//! "connection" is a user-created record (pod-specific base URL + API
//! Key/API Secret Key) for exactly one Datto RMM account, NOT for exactly
//! one local customer. A single account can see multiple "Sites" (e.g.
//! because the user setting up the connection is themselves an MSP who runs
//! several of their own customers as separate Datto RMM sites). Which site
//! corresponds to which local customer (if any) is a separate, granular
//! mapping (`DattoRmmSiteMapping`/`Config::dattormm_site_mappings`),
//! maintained by this module via `map_dattormm_site`/`unmap_dattormm_site`.
//! The fully qualified identifier `"dattormm:<connection_id>"` serves both
//! as the keyring account (`plugin::secrets`) and as
//! `external_refs.plugin_id`, so the existing one-row-per-(system_id,
//! plugin_id) upsert semantics keep working unchanged.
//!
//! IMPORTANT, verified difference from `commands::tacticalrmm`'s
//! `group_agents_by_client`: Datto RMM's device list carries a GENUINE
//! foreign key to its site (`siteUid`, matching `Site.uid`, see
//! `plugin::dattormm` module docs) -- `group_devices_by_site` below therefore
//! joins by ID, exactly like `commands::plugins::group_devices_by_organization`/
//! `commands::jamf::group_devices_by_site`, NOT by name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::dattormm::{
    test_credentials, DattoRmmConnectionMeta, DattoRmmCredentials, DattoRmmDevice, DattoRmmPlugin,
    DattoRmmSite, DattoRmmSiteMapping,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct DattoRmmConnectionDto {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    /// `"online"`/`"offline"`, passed through verbatim from
    /// `plugin::dattormm` -- see its module docs on the bool -> string
    /// conversion.
    pub status: Option<String>,
    /// One of Datto RMM's own `deviceClass` values, passed through verbatim.
    pub platform: Option<String>,
    /// A genuine, confirmed web-dashboard deep link for this exact device
    /// (see `plugin::dattormm` module docs) -- `None` only if Datto RMM
    /// itself didn't report one for this device, never silently dropped.
    pub portal_url: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// devices of an unmapped site -- without a `customer_id` there's no
    /// meaningful way to cross-reference against `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DattoRmmSiteDto {
    pub uid: String,
    pub name: String,
    pub portal_url: Option<String>,
    /// `None` as long as this site hasn't been mapped to a local customer
    /// yet (`Config::dattormm_site_mappings` has no matching row for this
    /// connection+site).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DattoRmmSiteDeviceGroupDto {
    pub site_uid: String,
    pub site_name: String,
    /// A genuine, confirmed web-dashboard deep link to this exact site (see
    /// `plugin::dattormm` module docs) -- `None` only if Datto RMM itself
    /// didn't report one, or for the synthetic leftover group of devices
    /// whose `site_uid` matched no known site (see `group_devices_by_site`).
    pub portal_url: Option<String>,
    /// `None` if this site isn't mapped to a local customer (yet) -- in this
    /// case the frontend shows "not mapped" and disables linking this
    /// group's devices.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_dattormm_connection` run, cached under
/// `data_dir/plugin-cache/dattormm-<connection_id>.json` (see
/// `write_dattormm_cache`/`read_dattormm_cache`), so
/// `get_cached_dattormm_sync` works without network access -- exactly the
/// same pattern as `commands::jamf::CachedJamfSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedDattoRmmSyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<DattoRmmSiteDeviceGroupDto>,
}

fn to_dto(meta: &DattoRmmConnectionMeta) -> DattoRmmConnectionDto {
    DattoRmmConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// The fully qualified `plugin_id` value for a Datto RMM connection -- both
/// the keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("dattormm:{connection_id}")
}

/// Generates a stable, low-collision connection ID from a user label,
/// following exactly the pattern of
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
        slug.push_str("dattormm");
    }
    slug
}

fn find_connection(
    config: &Config,
    connection_id: &str,
) -> Result<DattoRmmConnectionMeta, AppError> {
    config
        .dattormm_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Datto-RMM-Verbindung {connection_id} nicht gefunden"
            ))
        })
}

/// Builds the runnable plugin object from a connection metadata row, plus
/// the associated credentials (API Key/API Secret Key, JSON-encoded) from
/// the keyring.
fn build_plugin(
    meta: &DattoRmmConnectionMeta,
) -> Result<(DattoRmmPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Datto-RMM-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        DattoRmmPlugin::new(plugin_id, meta.base_url.clone()),
        PluginCredentials { secret },
    ))
}

/// Best-effort, analogous to
/// `commands::plugins::delete_keyring_secret_best_effort`.
fn delete_keyring_secret_best_effort(plugin_id: &str) -> Result<(), keyring::Error> {
    let entry = keyring::Entry::new("wartungsdoku", plugin_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The same `data_dir/plugin-cache/` directory every other plugin in this
/// codebase uses for itself.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn dattormm_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("dattormm-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::jamf::write_jamf_cache`.
fn write_dattormm_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[DattoRmmSiteDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedDattoRmmSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache).map_err(|e| {
        AppError::Plugin(format!("Datto-RMM-Cache konnte nicht kodiert werden: {e}"))
    })?;
    std::fs::write(dattormm_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_dattormm_cache`.
/// `Ok(None)` if this connection has never been synced -- not an error case,
/// analogous to `commands::jamf::read_jamf_cache`.
fn read_dattormm_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedDattoRmmSyncDto>, AppError> {
    let path = dattormm_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedDattoRmmSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Datto-RMM-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_external_system_dto(
    device: DattoRmmDevice,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        external_id: device.external_id,
        name: device.name,
        hostname: device.hostname,
        ip_address: device.ip_address,
        status: device.status,
        platform: device.platform,
        portal_url: device.portal_url,
        linked_system_id,
    }
}

/// Intermediate result of the pure grouping logic: a site along with its
/// devices (still as `DattoRmmDevice`, not as a DTO) and -- if mapped -- the
/// local `customer_id`. Analogous to `commands::jamf::SiteGroup`/
/// `commands::plugins::OrgGroup`.
struct SiteGroup {
    site_uid: String,
    site_name: String,
    portal_url: Option<String>,
    customer_id: Option<i64>,
    devices: Vec<DattoRmmDevice>,
}

/// Groups devices by site and enriches each group with the configured
/// `customer_id` mapping (if any). Pure function -- no network access, no
/// database access -- so it's testable with hardcoded
/// `DattoRmmSite`/`DattoRmmDevice`/`DattoRmmSiteMapping` values, analogous to
/// `commands::jamf::group_devices_by_site`/
/// `commands::plugins::group_devices_by_organization`. Unlike
/// `commands::tacticalrmm::group_agents_by_client`, this joins by the
/// GENUINE `site_uid` foreign key (see `plugin::dattormm` module docs), not
/// by name. A site with no devices at all still appears as a group (empty
/// `devices` list), so a future UI can show it for mapping. Devices whose
/// `site_uid` matches no known site (shouldn't normally happen, but not
/// impossible -- e.g. a site deleted between fetching sites and devices in
/// the same sync run) are not silently dropped, but appended as their own
/// leftover group, keyed by the raw `site_uid` (used as both the synthetic
/// ID and, absent a real site name, the display name too) with
/// `portal_url: None` -- there's no site data at all for this group, so
/// nothing honest to link to.
fn group_devices_by_site(
    sites: &[DattoRmmSite],
    devices: &[DattoRmmDevice],
    mappings: &[DattoRmmSiteMapping],
    connection_id: &str,
) -> Vec<SiteGroup> {
    let mapped_customer_id = |site_uid: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.site_uid == site_uid)
            .map(|m| m.customer_id)
    };

    let mut devices_by_site: HashMap<String, Vec<DattoRmmDevice>> = HashMap::new();
    for device in devices {
        devices_by_site
            .entry(device.site_uid.clone())
            .or_default()
            .push(device.clone());
    }

    let mut groups = Vec::with_capacity(sites.len());
    for site in sites {
        let site_devices = devices_by_site.remove(&site.uid).unwrap_or_default();
        groups.push(SiteGroup {
            site_uid: site.uid.clone(),
            site_name: site.name.clone(),
            portal_url: site.portal_url.clone(),
            customer_id: mapped_customer_id(&site.uid),
            devices: site_devices,
        });
    }

    let mut leftover_site_uids: Vec<String> = devices_by_site.keys().cloned().collect();
    leftover_site_uids.sort();
    for site_uid in leftover_site_uids {
        if let Some(site_devices) = devices_by_site.remove(&site_uid) {
            groups.push(SiteGroup {
                customer_id: mapped_customer_id(&site_uid),
                site_name: site_uid.clone(),
                site_uid,
                portal_url: None,
                devices: site_devices,
            });
        }
    }

    groups
}

#[tauri::command]
pub fn test_dattormm_connection(
    base_url: String,
    api_key: String,
    api_secret_key: String,
) -> Result<(), AppError> {
    test_credentials(&base_url, &api_key, &api_secret_key)?;
    Ok(())
}

#[tauri::command]
pub fn list_dattormm_connections(
    state: State<AppState>,
) -> Result<Vec<DattoRmmConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.dattormm_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_dattormm_connection(
    state: State<AppState>,
    label: String,
    base_url: String,
    api_key: String,
    api_secret_key: String,
) -> Result<DattoRmmConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    let secret_json = serde_json::to_string(&DattoRmmCredentials {
        api_key,
        api_secret_key,
    })
    .map_err(|e| AppError::Plugin(format!("Zugangsdaten konnten nicht kodiert werden: {e}")))?;
    plugin::secrets::store_secret(&plugin_id, &secret_json)?;

    let meta = DattoRmmConnectionMeta {
        id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.dattormm_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_dattormm_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.dattormm_connections.len();
    config.dattormm_connections.retain(|c| c.id != id);
    if config.dattormm_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Datto-RMM-Verbindung {id} nicht gefunden"
        )));
    }
    // Cleanup: site mappings for this connection are meaningless without the
    // connection and would otherwise be left behind as orphaned data.
    config
        .dattormm_site_mappings
        .retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = dattormm_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Datto-RMM-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

/// Live fetch of a connection's site list. Analogous to
/// `commands::jamf::list_jamf_sites`/
/// `commands::tacticalrmm::list_tacticalrmm_clients`, kept for symmetry and
/// a possible initial-setup use case -- normal frontend operation doesn't
/// need this command (cache-first, see
/// `get_cached_dattormm_sync`/`sync_dattormm_connection`).
#[tauri::command]
pub fn list_dattormm_sites(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<DattoRmmSiteDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<DattoRmmSiteMapping> = config
            .dattormm_site_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings)
    };

    let sites = plugin.list_sites(&credentials)?;
    Ok(sites
        .into_iter()
        .map(|site| {
            let mapped_customer_id = mappings
                .iter()
                .find(|m| m.site_uid == site.uid)
                .map(|m| m.customer_id);
            DattoRmmSiteDto {
                uid: site.uid,
                name: site.name,
                portal_url: site.portal_url,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_dattormm_site(
    state: State<AppState>,
    connection_id: String,
    site_uid: String,
    site_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .dattormm_site_mappings
        .retain(|m| !(m.connection_id == connection_id && m.site_uid == site_uid));
    config.dattormm_site_mappings.push(DattoRmmSiteMapping {
        connection_id,
        site_uid,
        site_name,
        customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_dattormm_site(
    state: State<AppState>,
    connection_id: String,
    site_uid: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Not an error if no matching mapping exists -- the result (no mapping
    // left) is the same, analogous to `db::external_refs::delete`. Existing
    // `external_refs` links are left untouched: unmapping a site is
    // deliberately not an automatic unlinking of its already-linked devices.
    config
        .dattormm_site_mappings
        .retain(|m| !(m.connection_id == connection_id && m.site_uid == site_uid));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_dattormm_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<DattoRmmSiteDeviceGroupDto>, AppError> {
    let (plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<DattoRmmSiteMapping> = config
            .dattormm_site_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings, config.data_dir.clone())
    };

    let sites = plugin.list_sites(&credentials)?;
    let devices = plugin.list_devices(&credentials)?;
    let groups = group_devices_by_site(&sites, &devices, &mappings, &connection_id);

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, built ONCE from ALL
    // external_refs of this plugin -- exactly the pattern
    // `commands::jamf::sync_jamf_connection`/`commands::plugins::sync_ninja_connection`
    // use: a device stays "linked" in the UI even if its site mapping was
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

        result.push(DattoRmmSiteDeviceGroupDto {
            site_uid: group.site_uid,
            site_name: group.site_name,
            portal_url: group.portal_url,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_dattormm_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_dattormm_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedDattoRmmSyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<DattoRmmSiteMapping> = config
            .dattormm_site_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_dattormm_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-resolved here against the CURRENT site
    // mappings instead of using the value frozen into the cache file during
    // the last `sync_dattormm_connection` run -- otherwise mapping or
    // unmapping a site would only become visible after the next live sync,
    // even though this exact command is meant to show the frontend the
    // current mapping state without network access (analogous to
    // `commands::jamf::get_cached_jamf_sync`).
    if let Some(cache) = cached.as_mut() {
        for group in &mut cache.groups {
            group.customer_id = mappings
                .iter()
                .find(|m| m.site_uid == group.site_uid)
                .map(|m| m.customer_id);
        }
    }
    Ok(cached)
}

#[tauri::command]
pub fn link_system_to_dattormm(
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
pub fn unlink_system_from_dattormm(
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
pub fn get_dattormm_system_details(
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
        assert_eq!(slugify("***"), "dattormm");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_dattormm_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "dattormm:acme-123");
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
        config.dattormm_connections.push(DattoRmmConnectionMeta {
            id: "acme-1".to_string(),
            label: "ACME".to_string(),
            base_url: "https://merlot-api.centrastage.net".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    fn sample_site(uid: &str, name: &str) -> DattoRmmSite {
        DattoRmmSite {
            uid: uid.to_string(),
            name: name.to_string(),
            portal_url: Some(format!("https://merlot.centrastage.net/site/{uid}")),
        }
    }

    fn sample_device(uid: &str, site_uid: &str) -> DattoRmmDevice {
        DattoRmmDevice {
            external_id: uid.to_string(),
            name: format!("Device {uid}"),
            hostname: None,
            ip_address: None,
            status: None,
            platform: None,
            operating_system: None,
            site_uid: site_uid.to_string(),
            site_name: String::new(),
            portal_url: None,
        }
    }

    #[test]
    fn groups_devices_under_their_site_by_uid() {
        let sites = vec![
            sample_site("site-1", "ACME"),
            sample_site("site-2", "Contoso"),
        ];
        let devices = vec![
            sample_device("dev-1", "site-1"),
            sample_device("dev-2", "site-1"),
            sample_device("dev-3", "site-2"),
        ];

        let groups = group_devices_by_site(&sites, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].site_uid, "site-1");
        assert_eq!(groups[0].devices.len(), 2);
        assert_eq!(
            groups[0].portal_url.as_deref(),
            Some("https://merlot.centrastage.net/site/site-1")
        );
        assert_eq!(groups[1].site_uid, "site-2");
        assert_eq!(groups[1].devices.len(), 1);
    }

    #[test]
    fn site_without_devices_still_appears_as_empty_group() {
        let sites = vec![sample_site("site-1", "ACME")];
        let groups = group_devices_by_site(&sites, &[], &[], "conn-1");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].devices.is_empty());
    }

    #[test]
    fn mapped_site_carries_its_customer_id() {
        let sites = vec![sample_site("site-1", "ACME")];
        let mappings = vec![DattoRmmSiteMapping {
            connection_id: "conn-1".to_string(),
            site_uid: "site-1".to_string(),
            site_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_site(&sites, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, Some(42));
    }

    #[test]
    fn unmapped_site_has_no_customer_id() {
        let sites = vec![sample_site("site-1", "ACME")];
        let groups = group_devices_by_site(&sites, &[], &[], "conn-1");
        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn mapping_for_a_different_connection_is_ignored() {
        let sites = vec![sample_site("site-1", "ACME")];
        let mappings = vec![DattoRmmSiteMapping {
            connection_id: "other-connection".to_string(),
            site_uid: "site-1".to_string(),
            site_name: "ACME".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_site(&sites, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn devices_for_an_unlisted_site_uid_form_their_own_leftover_group() {
        let sites = vec![sample_site("site-1", "ACME")];
        let devices = vec![sample_device("dev-9", "orphan-site")];

        let groups = group_devices_by_site(&sites, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups.iter().find(|g| g.site_uid == "orphan-site").unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.site_name, "orphan-site");
        assert_eq!(leftover.portal_url, None);
    }

    fn sample_group() -> DattoRmmSiteDeviceGroupDto {
        DattoRmmSiteDeviceGroupDto {
            site_uid: "site-1".to_string(),
            site_name: "ACME".to_string(),
            portal_url: Some("https://merlot.centrastage.net/site/site-1".to_string()),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "dev-1".to_string(),
                name: "SRV-01".to_string(),
                hostname: Some("SRV-01".to_string()),
                ip_address: Some("10.0.0.5".to_string()),
                status: Some("online".to_string()),
                platform: Some("device".to_string()),
                portal_url: Some("https://merlot.centrastage.net/device/dev-1".to_string()),
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn dattormm_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_dattormm_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_dattormm_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].site_uid, "site-1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "dev-1");
        assert_eq!(
            loaded.groups[0].devices[0].status.as_deref(),
            Some("online")
        );
        assert_eq!(
            loaded.groups[0].devices[0].portal_url.as_deref(),
            Some("https://merlot.centrastage.net/device/dev-1")
        );
    }

    #[test]
    fn dattormm_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_dattormm_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn dattormm_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_dattormm_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_dattormm_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_dattormm_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
