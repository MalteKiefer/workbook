//! Tauri commands for the Jamf Pro plugin integration (see `plugin::jamf`
//! and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers following exactly the
//! same pattern as `commands::plugins` (NinjaOne) -- a Jamf "connection" is
//! a user-created record (base URL + OAuth2 client ID/secret) for exactly
//! one Jamf Pro server, NOT for exactly one local customer. A single Jamf
//! Pro server can delegate inventory across multiple "Sites" (e.g. because
//! the user setting up the connection is themselves an MSP or runs a
//! multi-campus organization, each site standing in for a separate local
//! customer). Which site corresponds to which local customer (if any) is a
//! separate, granular mapping (`JamfSiteMapping`/`Config::jamf_site_mappings`),
//! maintained by this module via `map_jamf_site`/`unmap_jamf_site`. The
//! fully qualified identifier `"jamf:<connection_id>"` serves both as the
//! keyring account (`plugin::secrets`) and as `external_refs.plugin_id`, so
//! the existing one-row-per-(system_id,plugin_id) upsert semantics keep
//! working unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::jamf::{
    test_credentials, JamfConnectionMeta, JamfCredentials, JamfDevice, JamfPlugin, JamfSite,
    JamfSiteMapping,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct JamfConnectionDto {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalSystemDto {
    pub external_id: String,
    pub name: String,
    /// Always `Some(general.name)` in practice -- Jamf has no field distinct
    /// from a computer's display name in this API, see the `plugin::jamf`
    /// module documentation.
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub serial_number: Option<String>,
    pub asset_tag: Option<String>,
    pub operating_system: Option<String>,
    /// Direct link to the computer's detail page in Jamf Pro's own web
    /// console (`{base_url}/computers.html?id={id}`), the well-documented
    /// classic Jamf Pro web app URL pattern -- constructed from the
    /// connection's `base_url` and the external computer ID, not a made-up
    /// URL scheme.
    pub jamf_url: String,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// devices of an unmapped site -- without a `customer_id` there's no
    /// meaningful way to cross-reference against `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct JamfSiteDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this site hasn't been mapped to a local customer
    /// yet (`Config::jamf_site_mappings` has no matching row for this
    /// connection+site).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JamfSiteDeviceGroupDto {
    pub site_id: String,
    pub site_name: String,
    /// `None` if this site isn't mapped to a local customer (yet) -- in this
    /// case the frontend shows "not mapped" and disables linking this
    /// group's devices.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_jamf_connection` run, cached under
/// `data_dir/plugin-cache/jamf-<connection_id>.json` (see
/// `write_jamf_cache`/`read_jamf_cache`), so `get_cached_jamf_sync` works
/// without network access -- exactly the same pattern as
/// `commands::plugins::CachedNinjaSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedJamfSyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<JamfSiteDeviceGroupDto>,
}

fn to_dto(meta: &JamfConnectionMeta) -> JamfConnectionDto {
    JamfConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        base_url: meta.base_url.clone(),
    }
}

/// The fully qualified `plugin_id` value for a Jamf connection -- both the
/// keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("jamf:{connection_id}")
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
        slug.push_str("jamf");
    }
    slug
}

fn find_connection(config: &Config, connection_id: &str) -> Result<JamfConnectionMeta, AppError> {
    config
        .jamf_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("Jamf-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Builds the runnable plugin object plus its associated credentials from
/// the key store, based on a connection metadata row.
fn build_plugin(meta: &JamfConnectionMeta) -> Result<(JamfPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Jamf-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        JamfPlugin::new(plugin_id, meta.base_url.clone()),
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

/// The same `data_dir/plugin-cache/` directory that `commands::plugins`/
/// `commands::level`/`commands::snipeit` each use for themselves.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn jamf_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("jamf-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::plugins::write_ninja_cache`.
fn write_jamf_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[JamfSiteDeviceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedJamfSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Jamf-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(jamf_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_jamf_cache`.
/// `Ok(None)` if this connection has never been synced -- not an error case,
/// analogous to `commands::plugins::read_ninja_cache`.
fn read_jamf_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedJamfSyncDto>, AppError> {
    let path = jamf_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedJamfSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Jamf-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn jamf_device_url(base_url: &str, external_id: &str) -> String {
    format!(
        "{}/computers.html?id={external_id}",
        base_url.trim_end_matches('/')
    )
}

fn to_external_system_dto(
    base_url: &str,
    device: JamfDevice,
    linked_system_id: Option<i64>,
) -> ExternalSystemDto {
    ExternalSystemDto {
        jamf_url: jamf_device_url(base_url, &device.external_id),
        external_id: device.external_id,
        name: device.name,
        hostname: device.hostname,
        ip_address: device.ip_address,
        serial_number: device.serial_number,
        asset_tag: device.asset_tag,
        operating_system: device.operating_system,
        linked_system_id,
    }
}

/// Intermediate result of the pure grouping logic: a site along with its
/// devices (still as `JamfDevice`, not as a DTO) and -- if mapped -- the
/// local `customer_id`. Analogous to `commands::plugins::OrgGroup`.
struct SiteGroup {
    site_id: String,
    site_name: String,
    customer_id: Option<i64>,
    devices: Vec<JamfDevice>,
}

/// Groups devices by site and enriches each group with the configured
/// `customer_id` mapping (if any). Pure function -- no network access, no
/// database access -- so it's testable with hardcoded
/// `JamfSite`/`JamfDevice`/`JamfSiteMapping` values, analogous to
/// `commands::plugins::group_devices_by_organization`. A site with no
/// devices at all still appears as a group (empty `devices` list), so a
/// future UI can show it for mapping. Devices whose `site_id` doesn't match
/// any site reported by `sites` (shouldn't normally happen, but a computer
/// could in principle carry a stale/deleted site ID) are not silently
/// dropped, but appended as their own group under the raw site ID -- exactly
/// the same convention as `group_devices_by_organization`.
fn group_devices_by_site(
    sites: &[JamfSite],
    devices: &[JamfDevice],
    mappings: &[JamfSiteMapping],
    connection_id: &str,
) -> Vec<SiteGroup> {
    let mapped_customer_id = |site_id: &str| -> Option<i64> {
        mappings
            .iter()
            .find(|m| m.connection_id == connection_id && m.site_id == site_id)
            .map(|m| m.customer_id)
    };

    let mut devices_by_site: HashMap<String, Vec<JamfDevice>> = HashMap::new();
    for device in devices {
        devices_by_site
            .entry(device.site_id.clone())
            .or_default()
            .push(device.clone());
    }

    let mut groups = Vec::with_capacity(sites.len());
    for site in sites {
        let site_devices = devices_by_site.remove(&site.id).unwrap_or_default();
        groups.push(SiteGroup {
            site_id: site.id.clone(),
            site_name: site.name.clone(),
            customer_id: mapped_customer_id(&site.id),
            devices: site_devices,
        });
    }

    let mut leftover_site_ids: Vec<String> = devices_by_site.keys().cloned().collect();
    leftover_site_ids.sort();
    for site_id in leftover_site_ids {
        if let Some(site_devices) = devices_by_site.remove(&site_id) {
            groups.push(SiteGroup {
                customer_id: mapped_customer_id(&site_id),
                site_name: site_id.clone(),
                site_id,
                devices: site_devices,
            });
        }
    }

    groups
}

#[tauri::command]
pub fn test_jamf_connection(
    base_url: String,
    client_id: String,
    client_secret: String,
) -> Result<(), AppError> {
    test_credentials(&base_url, &client_id, &client_secret)?;
    Ok(())
}

#[tauri::command]
pub fn list_jamf_connections(state: State<AppState>) -> Result<Vec<JamfConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.jamf_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_jamf_connection(
    state: State<AppState>,
    label: String,
    base_url: String,
    client_id: String,
    client_secret: String,
) -> Result<JamfConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    let secret_json = serde_json::to_string(&JamfCredentials {
        client_id,
        client_secret,
    })
    .map_err(|e| AppError::Plugin(format!("Zugangsdaten konnten nicht kodiert werden: {e}")))?;
    plugin::secrets::store_secret(&plugin_id, &secret_json)?;

    let meta = JamfConnectionMeta {
        id,
        label,
        base_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.jamf_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_jamf_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.jamf_connections.len();
    config.jamf_connections.retain(|c| c.id != id);
    if config.jamf_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Jamf-Verbindung {id} nicht gefunden"
        )));
    }
    // Cleanup: site mappings for this connection are meaningless without the
    // connection and would otherwise be left behind as orphaned data.
    config.jamf_site_mappings.retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = jamf_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Jamf-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_jamf_sites(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<JamfSiteDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<JamfSiteMapping> = config
            .jamf_site_mappings
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
                .find(|m| m.site_id == site.id)
                .map(|m| m.customer_id);
            JamfSiteDto {
                id: site.id,
                name: site.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_jamf_site(
    state: State<AppState>,
    connection_id: String,
    site_id: String,
    site_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .jamf_site_mappings
        .retain(|m| !(m.connection_id == connection_id && m.site_id == site_id));
    config.jamf_site_mappings.push(JamfSiteMapping {
        connection_id,
        site_id,
        site_name,
        customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_jamf_site(
    state: State<AppState>,
    connection_id: String,
    site_id: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Not an error if no matching mapping exists -- the result (no mapping
    // left) is the same, analogous to `db::external_refs::delete`. Existing
    // `external_refs` links are left untouched: unmapping a site is
    // deliberately not an automatic unlinking of its already-linked devices.
    config
        .jamf_site_mappings
        .retain(|m| !(m.connection_id == connection_id && m.site_id == site_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_jamf_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<JamfSiteDeviceGroupDto>, AppError> {
    let (base_url, plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<JamfSiteMapping> = config
            .jamf_site_mappings
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

    let sites = plugin.list_sites(&credentials)?;
    let devices = plugin.list_computers(&credentials)?;
    let groups = group_devices_by_site(&sites, &devices, &mappings, &connection_id);

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, built from ALL
    // external_refs of this plugin -- ONCE for the whole connection, not
    // rebuilt per group and not restricted to the systems of the group's
    // `customer_id`, exactly the same reasoning (and the same bug this
    // avoids) as `commands::plugins::sync_ninja_connection` -- see its
    // comment and `db::external_refs::list_for_plugin`'s own doc comment.
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

        result.push(JamfSiteDeviceGroupDto {
            site_id: group.site_id,
            site_name: group.site_name,
            customer_id: group.customer_id,
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_jamf_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_jamf_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedJamfSyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<JamfSiteMapping> = config
            .jamf_site_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_jamf_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-joined against the CURRENT site mappings
    // here rather than trusting the value frozen into the cache file at the
    // last `sync_jamf_connection` run -- otherwise mapping/unmapping a site
    // would only be reflected after the next live sync, exactly the same
    // reasoning as `commands::plugins::get_cached_ninja_sync`.
    if let Some(cache) = cached.as_mut() {
        for group in &mut cache.groups {
            group.customer_id = mappings
                .iter()
                .find(|m| m.site_id == group.site_id)
                .map(|m| m.customer_id);
        }
    }
    Ok(cached)
}

#[tauri::command]
pub fn link_system_to_jamf(
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
pub fn unlink_system_from_jamf(
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
pub fn get_jamf_system_details(
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
        assert_eq!(slugify("***"), "jamf");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_jamf_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "jamf:acme-123");
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
        config.jamf_connections.push(JamfConnectionMeta {
            id: "acme-1".to_string(),
            label: "ACME".to_string(),
            base_url: "https://acme.jamfcloud.com".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    #[test]
    fn jamf_device_url_trims_trailing_slash_and_builds_computers_html_link() {
        let url = jamf_device_url("https://acme.jamfcloud.com/", "101");
        assert_eq!(url, "https://acme.jamfcloud.com/computers.html?id=101");
    }

    fn sample_site(id: &str, name: &str) -> JamfSite {
        JamfSite {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn sample_device(id: &str, site_id: &str) -> JamfDevice {
        JamfDevice {
            external_id: id.to_string(),
            name: format!("Device {id}"),
            hostname: None,
            ip_address: None,
            serial_number: None,
            asset_tag: None,
            operating_system: None,
            site_id: site_id.to_string(),
        }
    }

    #[test]
    fn groups_devices_under_their_site() {
        let sites = vec![
            sample_site("1", "Hauptsitz"),
            sample_site("2", "Zweigstelle"),
        ];
        let devices = vec![
            sample_device("101", "1"),
            sample_device("102", "1"),
            sample_device("201", "2"),
        ];

        let groups = group_devices_by_site(&sites, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].site_id, "1");
        assert_eq!(groups[0].devices.len(), 2);
        assert_eq!(groups[1].site_id, "2");
        assert_eq!(groups[1].devices.len(), 1);
    }

    #[test]
    fn site_without_devices_still_appears_as_empty_group() {
        let sites = vec![sample_site("1", "Hauptsitz")];
        let groups = group_devices_by_site(&sites, &[], &[], "conn-1");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].devices.is_empty());
    }

    #[test]
    fn mapped_site_carries_its_customer_id() {
        let sites = vec![sample_site("1", "Hauptsitz")];
        let mappings = vec![JamfSiteMapping {
            connection_id: "conn-1".to_string(),
            site_id: "1".to_string(),
            site_name: "Hauptsitz".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_site(&sites, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, Some(42));
    }

    #[test]
    fn unmapped_site_has_no_customer_id() {
        let sites = vec![sample_site("1", "Hauptsitz")];
        let groups = group_devices_by_site(&sites, &[], &[], "conn-1");
        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn mapping_for_a_different_connection_is_ignored() {
        let sites = vec![sample_site("1", "Hauptsitz")];
        let mappings = vec![JamfSiteMapping {
            connection_id: "other-connection".to_string(),
            site_id: "1".to_string(),
            site_name: "Hauptsitz".to_string(),
            customer_id: 42,
        }];

        let groups = group_devices_by_site(&sites, &[], &mappings, "conn-1");

        assert_eq!(groups[0].customer_id, None);
    }

    #[test]
    fn devices_for_an_unlisted_site_form_their_own_leftover_group() {
        let sites = vec![sample_site("1", "Hauptsitz")];
        let devices = vec![sample_device("999", "orphan-site")];

        let groups = group_devices_by_site(&sites, &devices, &[], "conn-1");

        assert_eq!(groups.len(), 2);
        let leftover = groups.iter().find(|g| g.site_id == "orphan-site").unwrap();
        assert_eq!(leftover.devices.len(), 1);
        assert_eq!(leftover.site_name, "orphan-site");
    }

    fn sample_group() -> JamfSiteDeviceGroupDto {
        JamfSiteDeviceGroupDto {
            site_id: "1".to_string(),
            site_name: "Hauptsitz".to_string(),
            customer_id: Some(7),
            devices: vec![ExternalSystemDto {
                external_id: "101".to_string(),
                name: "MBP-Anna".to_string(),
                hostname: Some("MBP-Anna".to_string()),
                ip_address: Some("10.0.0.5".to_string()),
                serial_number: Some("C02XXXXX".to_string()),
                asset_tag: Some("AT-0001".to_string()),
                operating_system: Some("macOS 14.5".to_string()),
                jamf_url: "https://acme.jamfcloud.com/computers.html?id=101".to_string(),
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn jamf_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_jamf_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_jamf_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].site_id, "1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "101");
        assert_eq!(
            loaded.groups[0].devices[0].serial_number.as_deref(),
            Some("C02XXXXX")
        );
    }

    #[test]
    fn jamf_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_jamf_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn jamf_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_jamf_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_jamf_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_jamf_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
