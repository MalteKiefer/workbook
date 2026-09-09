//! Tauri commands for the Snipe-IT plugin integration (see
//! `plugin::snipeit` and `docs/PLUGIN_ARCHITECTURE.md`). Thin wrappers
//! following exactly the same pattern as `commands::plugins` (NinjaOne) -- a
//! Snipe-IT "connection" is a user-created record (base URL + personal
//! access token) for exactly one Snipe-IT instance, NOT for exactly one
//! local customer. A single instance can manage assets for multiple
//! companies (e.g. because the user setting up the connection is themselves
//! an MSP who runs several of their own customers as separate companies in
//! Snipe-IT). Which company corresponds to which local customer (if any) is
//! a separate, granular mapping
//! (`SnipeitCompanyMapping`/`Config::snipeit_company_mappings`), maintained
//! by this module via `map_snipeit_company`/`unmap_snipeit_company`. The
//! fully qualified identifier `"snipeit:<connection_id>"` serves both as the
//! keyring account (`plugin::secrets`) and as `external_refs.plugin_id`, so
//! the existing one-row-per-(system_id,plugin_id) upsert semantics keep
//! working unchanged.

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
    /// Always `None` -- Snipe-IT provides no hostname field on the core
    /// asset object, see the `plugin::snipeit` module documentation. Kept as
    /// a field anyway so this DTO type structurally matches its Ninja/Level
    /// counterpart.
    pub hostname: Option<String>,
    /// Always `None`, for the same reason as `hostname`.
    pub ip_address: Option<String>,
    /// Snipe-IT's own primary identification field for an asset.
    pub asset_tag: Option<String>,
    /// Snipe-IT's serial number field.
    pub serial: Option<String>,
    /// Direct link to the asset detail page in Snipe-IT's own web UI
    /// (`{base_url}/hardware/{id}`), constructed from the connection's
    /// `base_url` and the external asset ID -- verified against Snipe-IT's
    /// `routes/web/hardware.php` (see the `plugin::snipeit` module
    /// documentation), not a made-up URL scheme.
    pub snipeit_url: String,
    /// `Some(id)` if some local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`. Always `None` for
    /// assets of an unmapped company -- without a `customer_id` there's no
    /// meaningful way to cross-reference against `external_refs`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SnipeitCompanyDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this company hasn't been mapped to a local
    /// customer yet (`Config::snipeit_company_mappings` has no matching row
    /// for this connection+company).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnipeitCompanyDeviceGroupDto {
    pub company_id: String,
    pub company_name: String,
    /// `None` if this company is not (yet) mapped to a local customer --
    /// in that case the frontend shows "not mapped" and disables linking
    /// this group's assets.
    pub customer_id: Option<i64>,
    pub devices: Vec<ExternalSystemDto>,
}

/// Snapshot of the last `sync_snipeit_connection` run, cached under
/// `data_dir/plugin-cache/snipeit-<connection_id>.json` (see
/// `write_snipeit_cache`/`read_snipeit_cache`) so `get_cached_snipeit_sync`
/// works without network access -- exactly the same pattern as
/// `commands::plugins::CachedNinjaSyncDto`.
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

/// The fully qualified `plugin_id` value for a Snipe-IT connection --
/// both the keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("snipeit:{connection_id}")
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

/// Builds the runnable plugin object from a connection metadata row, plus
/// the associated credentials (the personal access token) from the keyring.
/// Unlike Ninja, the Snipe-IT token is already the complete secret string --
/// no JSON encoding needed, since Snipe-IT only needs a single secret value
/// (like Level.io).
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

/// The same `data_dir/plugin-cache/` directory as `commands::plugins`/
/// `commands::level` -- one shared folder for all plugin cache files, just
/// with a different filename prefix per plugin.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn snipeit_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("snipeit-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::plugins::write_ninja_cache`.
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

/// Reads back a snapshot previously written via `write_snipeit_cache`.
/// `Ok(None)` if this connection has never been synced -- not an error case,
/// analogous to `commands::plugins::read_ninja_cache`.
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

/// Intermediate result of the pure grouping logic: a company along with its
/// assets (still as `SnipeitDevice`, not as a DTO) and -- if mapped -- the
/// local `customer_id`. Analogous to `commands::plugins::OrgGroup`.
struct CompanyGroup {
    company_id: String,
    company_name: String,
    customer_id: Option<i64>,
    devices: Vec<SnipeitDevice>,
}

/// Groups assets by company and enriches each group with the configured
/// `customer_id` mapping (if any). A pure function -- no network, no
/// database access -- so it's testable with hardcoded
/// `SnipeitCompany`/`SnipeitDevice`/`SnipeitCompanyMapping` values,
/// analogous to `commands::plugins::group_devices_by_organization`. A
/// company with no assets at all still shows up as a group (empty `devices`
/// list) so a future UI can display it for mapping. Assets whose
/// `company_id` doesn't match any company reported by `companies` --
/// including `UNASSIGNED_COMPANY_ID` for assets with no company assignment
/// in Snipe-IT itself -- are not silently dropped but appended as their own
/// group; for `UNASSIGNED_COMPANY_ID` with a readable name instead of the
/// raw sentinel ID.
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
    // Cleanup: company mappings for this connection are meaningless without
    // the connection and would otherwise be left behind as orphaned data.
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

/// Live fetch of a connection's company list. Analogous to
/// `commands::plugins::list_ninja_organizations`, kept for symmetry and a
/// possible initial-setup use case -- normal frontend operation doesn't need
/// this command (cache-first, see
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
    // Not an error if no matching mapping exists -- the result (no mapping
    // left) is the same, analogous to `db::external_refs::delete`. Existing
    // `external_refs` links are left untouched: unmapping a company is
    // deliberately not an automatic unlinking of its already-linked assets.
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
        // Reverse index external-ID -> local system_id, only built for mapped
        // companies -- without a `customer_id` there's no meaningful set of
        // local systems to cross-reference against.
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
    // `customer_id` per group is re-resolved here against the CURRENT
    // company mappings instead of using the value frozen into the cache file
    // during the last `sync_snipeit_connection` run -- otherwise mapping or
    // unmapping a company would only become visible after the next live
    // sync, even though this exact command is meant to show the frontend the
    // current mapping state without network access (analogous to
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
