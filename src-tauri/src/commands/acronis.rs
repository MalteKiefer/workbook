//! Tauri commands for the Acronis Cyber Protect Cloud plugin integration
//! (see `plugin::acronis` and `docs/PLUGIN_ARCHITECTURE.md`, section
//! "Acronis-Plugin"). Thin wrappers following exactly the same pattern as
//! `commands::tacticalrmm`: an Acronis "connection" is a user-created
//! record (datacenter URL + OAuth2 client ID/secret) for exactly one
//! Acronis API client, NOT for exactly one local customer. A single API
//! client can see multiple tenants (e.g. because the user setting up the
//! connection is themselves an MSP running several of their own customers
//! as separate Acronis tenants). Which tenant corresponds to which local
//! customer (if any) is a separate, granular mapping
//! (`AcronisTenantMapping`/`Config::acronis_tenant_mappings`), maintained
//! by this module via `map_acronis_tenant`/`unmap_acronis_tenant`. The
//! fully qualified identifier `"acronis:<connection_id>"` serves both as
//! the keyring account (`plugin::secrets`) and as `external_refs.plugin_id`,
//! so the existing one-row-per-(system_id,plugin_id) upsert semantics keep
//! working unchanged.
//!
//! Two DELIBERATE deviations from `commands::tacticalrmm`'s shape, both
//! forced by Acronis's own API shape (see `plugin::acronis` module docs
//! for the full reasoning), flagged here explicitly, not silently copied
//! from the brief this plugin was built from:
//!
//! 1. `sync_acronis_connection` loops ONLY over tenants that are already
//!    MAPPED (`Config::acronis_tenant_mappings`), NOT every tenant the
//!    connection can see. Tactical RMM's/NinjaOne's/Snipe-IT's single
//!    "list every client/agent, then group" call is cheap and connection-
//!    wide; Acronis's resources endpoint is REQUIRED to be tenant-scoped
//!    (`tenant_id` query parameter, see `plugin::acronis` module docs),
//!    and there is no verified "all tenants at once" variant to call
//!    instead. An unmapped tenant therefore shows no resources at all
//!    until it is mapped (unlike Tactical RMM, which shows an unmapped
//!    client's agents too, just without a `customer_id`).
//!    `list_acronis_tenants` remains available (mirroring
//!    `list_tacticalrmm_clients`) so the mapping UI still has a live
//!    candidate list to map FROM.
//! 2. `link_system_to_acronis`/`get_acronis_system_details` need a
//!    `tenant_id` parameter that `link_system_to_tacticalrmm`/
//!    `get_tacticalrmm_system_details` don't need. Acronis has no verified
//!    single-resource detail endpoint, only the tenant-wide, UNFILTERED
//!    `resource_management/v4/resource_statuses` (see `plugin::acronis`
//!    module docs), so `Plugin::get_system_details`
//!    (`(credentials, external_id)`, no tenant parameter) can't be the
//!    real implementation here (`plugin::acronis::AcronisPlugin`'s trait
//!    impl deliberately reports that gap instead of guessing at an
//!    unconfirmed endpoint). This module therefore calls the plugin's
//!    inherent `get_resource_statuses(credentials, tenant_id)` method
//!    directly wherever a payload is needed, using tenant context it
//!    already has: from the tenant mapping being synced/linked, or, for
//!    `get_acronis_system_details`, passed in from the frontend, which
//!    already knows which mapped tenant's resource list the device being
//!    inspected belongs to. The resulting payload is the WHOLE tenant's
//!    raw `resource_statuses` response (an array covering every resource
//!    in that tenant, not just one); `AcronisPluginSection.tsx` finds its
//!    own resource's entry inside `data.items` client-side by matching
//!    `external_id`, exactly the same "raw JSON, caller interprets it"
//!    contract every other plugin section here already follows for its
//!    own `get_*_system_details` payload, just with one extra
//!    client-side lookup step.

use std::path::{Path, PathBuf};

use tauri::State;

use crate::config::Config;
use crate::plugin::acronis::{
    test_credentials, AcronisConnectionMeta, AcronisCredentials, AcronisPlugin, AcronisResource,
    AcronisTenant, AcronisTenantMapping,
};
use crate::plugin::{self, Plugin, PluginCredentials};
use crate::{db, time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AcronisConnectionDto {
    pub id: String,
    pub label: String,
    pub datacenter_url: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AcronisTenantDto {
    pub id: String,
    pub name: String,
    /// `None` as long as this tenant hasn't been mapped to a local customer
    /// yet (`Config::acronis_tenant_mappings` has no matching row for this
    /// connection+tenant). Only `kind == "customer"` tenants ever appear
    /// here: `plugin::acronis::AcronisPlugin::list_tenants` already
    /// filters that (see its own doc comment).
    pub mapped_customer_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AcronisResourceDto {
    pub external_id: String,
    pub name: String,
    /// Acronis Alert Manager `severity`, passed through verbatim
    /// (`"ok"`/`"information"`/`"warning"`/`"error"`/`"critical"`), or
    /// `None` if this resource has no alert-manager entry at all (never
    /// backed up / not protected); see `plugin::acronis` module docs.
    pub backup_status: Option<String>,
    /// `Some(id)` if any local system is already linked to this external ID
    /// for this connection (an `external_refs` row with matching
    /// `plugin_id`/`external_id`), otherwise `None`.
    pub linked_system_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AcronisTenantResourceGroupDto {
    pub tenant_id: String,
    pub tenant_name: String,
    /// `None` if this tenant isn't mapped to a local customer (yet); this
    /// can't actually happen for a group produced by
    /// `sync_acronis_connection` (which only ever loops over mapped
    /// tenants, see module docs), but `get_cached_acronis_sync` still
    /// re-resolves this live against the CURRENT mappings, so a tenant
    /// unmapped after its last sync legitimately shows `None` here.
    pub customer_id: Option<i64>,
    pub devices: Vec<AcronisResourceDto>,
}

/// Snapshot of the last `sync_acronis_connection` run, cached under
/// `data_dir/plugin-cache/acronis-<connection_id>.json` (see
/// `write_acronis_cache`/`read_acronis_cache`), so `get_cached_acronis_sync`
/// works without network access, exactly the same pattern as
/// `commands::tacticalrmm::CachedTacticalRmmSyncDto`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedAcronisSyncDto {
    pub synced_at_utc: String,
    pub groups: Vec<AcronisTenantResourceGroupDto>,
}

fn to_dto(meta: &AcronisConnectionMeta) -> AcronisConnectionDto {
    AcronisConnectionDto {
        id: meta.id.clone(),
        label: meta.label.clone(),
        datacenter_url: meta.datacenter_url.clone(),
    }
}

/// The fully qualified `plugin_id` value for an Acronis connection: both
/// the keyring account and `external_refs.plugin_id`.
fn plugin_id_for(connection_id: &str) -> String {
    format!("acronis:{connection_id}")
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
        slug.push_str("acronis");
    }
    slug
}

fn find_connection(
    config: &Config,
    connection_id: &str,
) -> Result<AcronisConnectionMeta, AppError> {
    config
        .acronis_connections
        .iter()
        .find(|c| c.id == connection_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!("Acronis-Verbindung {connection_id} nicht gefunden"))
        })
}

/// Builds the runnable plugin object from a connection metadata row, plus
/// the associated credentials (client ID/secret) from the keyring. Unlike
/// Tactical RMM's single secret string, Acronis needs the two-value JSON
/// `AcronisCredentials`, but that decoding happens inside
/// `plugin::acronis` itself (`parse_credentials`), so this function, like
/// `commands::intune::build_plugin`, just passes the raw stored JSON string
/// straight through as `PluginCredentials.secret`.
fn build_plugin(
    meta: &AcronisConnectionMeta,
) -> Result<(AcronisPlugin, PluginCredentials), AppError> {
    let plugin_id = plugin_id_for(&meta.id);
    let secret = plugin::secrets::load_secret(&plugin_id)?.ok_or_else(|| {
        AppError::Plugin(format!(
            "Keine Zugangsdaten für Acronis-Verbindung {} im Schlüsselspeicher gefunden",
            meta.id
        ))
    })?;
    Ok((
        AcronisPlugin::new(plugin_id, meta.datacenter_url.clone()),
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
/// commands module: one shared folder, just a different filename prefix.
fn plugin_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("plugin-cache")
}

fn acronis_cache_path(data_dir: &Path, connection_id: &str) -> PathBuf {
    plugin_cache_dir(data_dir).join(format!("acronis-{connection_id}.json"))
}

/// Writes a snapshot of the sync result as a JSON file, analogous to
/// `commands::tacticalrmm::write_tacticalrmm_cache`.
fn write_acronis_cache(
    data_dir: &Path,
    connection_id: &str,
    synced_at_utc: &str,
    groups: &[AcronisTenantResourceGroupDto],
) -> Result<(), AppError> {
    let dir = plugin_cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let cache = CachedAcronisSyncDto {
        synced_at_utc: synced_at_utc.to_string(),
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| AppError::Plugin(format!("Acronis-Cache konnte nicht kodiert werden: {e}")))?;
    std::fs::write(acronis_cache_path(data_dir, connection_id), json)?;
    Ok(())
}

/// Reads back a snapshot previously written via `write_acronis_cache`.
/// `Ok(None)` if this connection has never been synced, not an error
/// case, analogous to `commands::tacticalrmm::read_tacticalrmm_cache`.
fn read_acronis_cache(
    data_dir: &Path,
    connection_id: &str,
) -> Result<Option<CachedAcronisSyncDto>, AppError> {
    let path = acronis_cache_path(data_dir, connection_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let cached: CachedAcronisSyncDto = serde_json::from_str(&text)
        .map_err(|e| AppError::Plugin(format!("Acronis-Cache-Datei ungültig: {e}")))?;
    Ok(Some(cached))
}

fn to_resource_dto(resource: AcronisResource, linked_system_id: Option<i64>) -> AcronisResourceDto {
    AcronisResourceDto {
        external_id: resource.external_id,
        name: resource.name,
        backup_status: resource.backup_status,
        linked_system_id,
    }
}

#[tauri::command]
pub fn test_acronis_connection(
    datacenter_url: String,
    client_id: String,
    client_secret: String,
) -> Result<(), AppError> {
    test_credentials(&datacenter_url, &client_id, &client_secret)?;
    Ok(())
}

#[tauri::command]
pub fn list_acronis_connections(
    state: State<AppState>,
) -> Result<Vec<AcronisConnectionDto>, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(config.acronis_connections.iter().map(to_dto).collect())
}

#[tauri::command]
pub fn add_acronis_connection(
    state: State<AppState>,
    label: String,
    datacenter_url: String,
    client_id: String,
    client_secret: String,
) -> Result<AcronisConnectionDto, AppError> {
    let id = generate_connection_id(&label);
    let plugin_id = plugin_id_for(&id);

    let secret_json = serde_json::to_string(&AcronisCredentials {
        client_id,
        client_secret,
    })
    .map_err(|e| {
        AppError::Plugin(format!(
            "Acronis-Zugangsdaten konnten nicht kodiert werden: {e}"
        ))
    })?;
    plugin::secrets::store_secret(&plugin_id, &secret_json)?;

    let meta = AcronisConnectionMeta {
        id,
        label,
        datacenter_url,
    };

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.acronis_connections.push(meta.clone());
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;

    Ok(to_dto(&meta))
}

#[tauri::command]
pub fn remove_acronis_connection(state: State<AppState>, id: String) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    let before = config.acronis_connections.len();
    config.acronis_connections.retain(|c| c.id != id);
    if config.acronis_connections.len() == before {
        return Err(AppError::NotFound(format!(
            "Acronis-Verbindung {id} nicht gefunden"
        )));
    }
    // Cleanup: tenant mappings for this connection are meaningless without
    // the connection and would otherwise be left behind as orphaned data.
    config
        .acronis_tenant_mappings
        .retain(|m| m.connection_id != id);
    let data_dir = config.data_dir.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    let plugin_id = plugin_id_for(&id);
    if let Err(e) = delete_keyring_secret_best_effort(&plugin_id) {
        eprintln!("Schlüsselspeicher-Eintrag für {plugin_id} konnte nicht entfernt werden (ignoriert): {e}");
    }

    let cache_path = acronis_cache_path(&data_dir, &id);
    if cache_path.exists() {
        if let Err(e) = std::fs::remove_file(&cache_path) {
            eprintln!(
                "Acronis-Cache-Datei {} konnte nicht entfernt werden (ignoriert): {e}",
                cache_path.display()
            );
        }
    }
    Ok(())
}

/// Live fetch of a connection's customer-kind tenant list. Analogous to
/// `commands::tacticalrmm::list_tacticalrmm_clients`, kept for the mapping
/// UI's candidate list. Unlike Tactical RMM, this IS load-bearing here
/// (not just symmetry): `sync_acronis_connection` only ever loops over
/// already-mapped tenants (see module docs), so this live call is the only
/// way the mapping UI can discover which tenants exist to map FROM in the
/// first place.
#[tauri::command]
pub fn list_acronis_tenants(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<AcronisTenantDto>, AppError> {
    let (plugin, credentials, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<AcronisTenantMapping> = config
            .acronis_tenant_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings)
    };

    let tenants: Vec<AcronisTenant> = plugin.list_tenants(&credentials)?;
    Ok(tenants
        .into_iter()
        .map(|tenant| {
            let mapped_customer_id = mappings
                .iter()
                .find(|m| m.tenant_id == tenant.id)
                .map(|m| m.customer_id);
            AcronisTenantDto {
                id: tenant.id,
                name: tenant.name,
                mapped_customer_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn map_acronis_tenant(
    state: State<AppState>,
    connection_id: String,
    tenant_id: String,
    tenant_name: String,
    customer_id: i64,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    find_connection(&config, &connection_id)?;
    config
        .acronis_tenant_mappings
        .retain(|m| !(m.connection_id == connection_id && m.tenant_id == tenant_id));
    config.acronis_tenant_mappings.push(AcronisTenantMapping {
        connection_id,
        tenant_id,
        tenant_name,
        customer_id,
    });
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn unmap_acronis_tenant(
    state: State<AppState>,
    connection_id: String,
    tenant_id: String,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    // Not an error if no matching mapping exists: the result (no mapping
    // left) is the same, analogous to `db::external_refs::delete`. Existing
    // `external_refs` links are left untouched: unmapping a tenant is
    // deliberately not an automatic unlinking of its already-linked
    // resources.
    config
        .acronis_tenant_mappings
        .retain(|m| !(m.connection_id == connection_id && m.tenant_id == tenant_id));
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn sync_acronis_connection(
    state: State<AppState>,
    connection_id: String,
) -> Result<Vec<AcronisTenantResourceGroupDto>, AppError> {
    let (plugin, credentials, mappings, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        let (plugin, credentials) = build_plugin(&meta)?;
        let mappings: Vec<AcronisTenantMapping> = config
            .acronis_tenant_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (plugin, credentials, mappings, config.data_dir.clone())
    };

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let plugin_id = plugin.id().to_string();

    // Reverse index external-id -> local system_id, built ONCE from ALL
    // external_refs of this plugin, exactly the pattern
    // `commands::tacticalrmm::sync_tacticalrmm_connection` uses: a resource
    // stays "linked" in the UI even if its tenant mapping was corrected to
    // a different customer AFTER the link was made.
    let linked_by_external_id: std::collections::HashMap<String, i64> =
        db::external_refs::list_for_plugin(&conn, &plugin_id)?
            .into_iter()
            .map(|reference| (reference.external_id, reference.system_id))
            .collect();

    // Loops ONLY over already-mapped tenants, NOT every tenant the
    // connection can see: see module docs on why (the resources API is
    // required to be tenant-scoped, unlike Tactical RMM's/NinjaOne's
    // connection-wide agent/device list).
    let mut result = Vec::with_capacity(mappings.len());
    for mapping in &mappings {
        let resources = plugin.list_resources_with_status(
            &credentials,
            &mapping.tenant_id,
            &mapping.tenant_name,
        )?;

        // The tenant-wide, unfiltered resource_statuses payload is fetched
        // AT MOST ONCE per tenant per sync run, lazily, only if this
        // tenant actually has a linked resource that needs its cached
        // payload refreshed, not once per resource (see module docs:
        // there is no per-resource endpoint, so every linked resource of
        // this tenant shares the exact same raw payload).
        let mut tenant_details: Option<serde_json::Value> = None;

        let mut device_dtos = Vec::with_capacity(resources.len());
        for resource in resources {
            let linked_system_id = linked_by_external_id.get(&resource.external_id).copied();
            if let Some(system_id) = linked_system_id {
                if tenant_details.is_none() {
                    tenant_details =
                        Some(plugin.get_resource_statuses(&credentials, &mapping.tenant_id)?);
                }
                let payload = tenant_details.as_ref().expect("gerade befüllt");
                db::external_refs::upsert(
                    &conn,
                    system_id,
                    &plugin_id,
                    &resource.external_id,
                    &payload.to_string(),
                    &tz,
                )?;
            }
            device_dtos.push(to_resource_dto(resource, linked_system_id));
        }

        result.push(AcronisTenantResourceGroupDto {
            tenant_id: mapping.tenant_id.clone(),
            tenant_name: mapping.tenant_name.clone(),
            customer_id: Some(mapping.customer_id),
            devices: device_dtos,
        });
    }

    let (synced_at_utc, _) = time::now_with_tz(&tz);
    write_acronis_cache(&data_dir, &connection_id, &synced_at_utc, &result)?;

    Ok(result)
}

#[tauri::command]
pub fn get_cached_acronis_sync(
    state: State<AppState>,
    connection_id: String,
) -> Result<Option<CachedAcronisSyncDto>, AppError> {
    let (data_dir, mappings) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let mappings: Vec<AcronisTenantMapping> = config
            .acronis_tenant_mappings
            .iter()
            .filter(|m| m.connection_id == connection_id)
            .cloned()
            .collect();
        (config.data_dir.clone(), mappings)
    };
    let mut cached = read_acronis_cache(&data_dir, &connection_id)?;
    // `customer_id` per group is re-resolved here against the CURRENT
    // tenant mappings instead of using the value frozen into the cache
    // file during the last `sync_acronis_connection` run, otherwise
    // mapping or unmapping a tenant would only become visible after the
    // next live sync, even though this exact command is meant to show the
    // frontend the current mapping state without network access (analogous
    // to `commands::tacticalrmm::get_cached_tacticalrmm_sync`).
    if let Some(cache) = cached.as_mut() {
        for group in &mut cache.groups {
            group.customer_id = mappings
                .iter()
                .find(|m| m.tenant_id == group.tenant_id)
                .map(|m| m.customer_id);
        }
    }
    Ok(cached)
}

#[tauri::command]
pub fn link_system_to_acronis(
    state: State<AppState>,
    system_id: i64,
    connection_id: String,
    tenant_id: String,
    external_id: String,
) -> Result<(), AppError> {
    let (plugin, credentials) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        build_plugin(&meta)?
    };

    // See module docs: no verified per-resource detail endpoint exists, so
    // the payload persisted here is the whole mapped tenant's raw,
    // unfiltered resource_statuses response, not just this one resource's.
    let payload = plugin.get_resource_statuses(&credentials, &tenant_id)?;
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
pub fn unlink_system_from_acronis(
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

/// Returns the mapped tenant's WHOLE raw `resource_statuses` payload (see
/// module docs). Unlike every other plugin's `get_*_system_details`,
/// this is NOT scoped down to one resource server-side (no verified
/// endpoint exists to do that); `AcronisPluginSection.tsx` finds its own
/// resource's entry inside the returned `items` array client-side by
/// matching `external_id`.
#[tauri::command]
pub fn get_acronis_system_details(
    state: State<AppState>,
    connection_id: String,
    tenant_id: String,
) -> Result<serde_json::Value, AppError> {
    let (plugin, credentials) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let meta = find_connection(&config, &connection_id)?;
        build_plugin(&meta)?
    };
    Ok(plugin.get_resource_statuses(&credentials, &tenant_id)?)
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
        assert_eq!(slugify("***"), "acronis");
    }

    #[test]
    fn generate_connection_id_is_prefixed_with_the_slug() {
        let id = generate_connection_id("ACME Kunde");
        assert!(id.starts_with("acme-kunde-"));
    }

    #[test]
    fn plugin_id_for_uses_the_acronis_prefix() {
        assert_eq!(plugin_id_for("acme-123"), "acronis:acme-123");
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
        config.acronis_connections.push(AcronisConnectionMeta {
            id: "acme-1".to_string(),
            label: "ACME".to_string(),
            datacenter_url: "https://eu2-cloud.acronis.com".to_string(),
        });
        let found = find_connection(&config, "acme-1").unwrap();
        assert_eq!(found.label, "ACME");
    }

    fn sample_resource(external_id: &str, status: Option<&str>) -> AcronisResource {
        AcronisResource {
            external_id: external_id.to_string(),
            name: format!("Resource {external_id}"),
            tenant_id: "tenant-1".to_string(),
            tenant_name: "ACME".to_string(),
            backup_status: status.map(str::to_string),
        }
    }

    #[test]
    fn to_resource_dto_carries_backup_status_and_link_state() {
        let resource = sample_resource("res-1", Some("warning"));
        let dto = to_resource_dto(resource, Some(42));
        assert_eq!(dto.external_id, "res-1");
        assert_eq!(dto.backup_status.as_deref(), Some("warning"));
        assert_eq!(dto.linked_system_id, Some(42));
    }

    #[test]
    fn to_resource_dto_handles_unprotected_and_unlinked_resource() {
        let resource = sample_resource("res-2", None);
        let dto = to_resource_dto(resource, None);
        assert_eq!(dto.backup_status, None);
        assert_eq!(dto.linked_system_id, None);
    }

    fn sample_group() -> AcronisTenantResourceGroupDto {
        AcronisTenantResourceGroupDto {
            tenant_id: "tenant-1".to_string(),
            tenant_name: "ACME".to_string(),
            customer_id: Some(7),
            devices: vec![AcronisResourceDto {
                external_id: "res-1".to_string(),
                name: "SRV-01".to_string(),
                backup_status: Some("ok".to_string()),
                linked_system_id: Some(3),
            }],
        }
    }

    #[test]
    fn acronis_cache_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let groups = vec![sample_group()];

        write_acronis_cache(dir.path(), "conn-1", "2026-09-07T12:00:00.000Z", &groups).unwrap();
        let loaded = read_acronis_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].tenant_id, "tenant-1");
        assert_eq!(loaded.groups[0].devices[0].external_id, "res-1");
        assert_eq!(
            loaded.groups[0].devices[0].backup_status.as_deref(),
            Some("ok")
        );
    }

    #[test]
    fn acronis_cache_returns_none_when_never_synced() {
        let dir = tempdir().unwrap();
        let loaded = read_acronis_cache(dir.path(), "never-synced").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn acronis_cache_overwrites_previous_snapshot_for_the_same_connection() {
        let dir = tempdir().unwrap();
        write_acronis_cache(dir.path(), "conn-1", "2026-09-07T10:00:00.000Z", &[]).unwrap();
        write_acronis_cache(
            dir.path(),
            "conn-1",
            "2026-09-07T12:00:00.000Z",
            &[sample_group()],
        )
        .unwrap();

        let loaded = read_acronis_cache(dir.path(), "conn-1").unwrap().unwrap();

        assert_eq!(loaded.synced_at_utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(loaded.groups.len(), 1);
    }
}
