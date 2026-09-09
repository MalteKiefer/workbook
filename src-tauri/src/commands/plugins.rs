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
    /// First address from Ninja's `ipAddresses` array, with `publicIP` as a
    /// fallback (see `plugin::ninja::map_ninja_device`).
    pub ip_address: Option<String>,
    /// Direct link to the device dashboard in the Ninja web UI, constructed
    /// from the connection's `base_url` and the external device ID.
    pub ninja_url: String,
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
