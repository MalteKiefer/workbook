//! Real plugin implementation for Snipe-IT (open-source IT asset
//! management), see `docs/PLUGIN_ARCHITECTURE.md` section "Snipe-IT
//! plugin". Third real integration after NinjaOne (`plugin::ninja`) and
//! Level.io (`plugin::level`). Structure and signatures follow
//! `plugin::dummy::DummyPlugin`, but talk over real HTTPS (crate `ureq`,
//! synchronous, no async runtime) to Snipe-IT's public REST API.
//!
//! - **Authentication**: static bearer token in the `Authorization` header
//!   (`Authorization: Bearer <token>`) -- a "Personal Access Token" that the
//!   user generates themselves in Snipe-IT's own web UI (Profile -> API
//!   Tokens). Verified via Snipe-IT's own `routes/api.php` on GitHub
//!   (`POST/GET/DELETE /api/v1/account/personal-access-tokens`). No OAuth2
//!   grant like with NinjaOne, no token exchange -- exactly like Level.io,
//!   just with a `Bearer ` prefix (Level has none).
//! - **Self-hosted**: unlike Level.io (fixed `BASE_URL` constant) and like
//!   NinjaOne, a Snipe-IT connection needs a base URL supplied by the user
//!   (`SnipeitConnectionMeta.base_url`, e.g. `https://assets.example.com`)
//!   -- `API_PATH` (`/api/v1`) is fixedly appended when building every
//!   request URL.
//! - **Companies (multi-tenancy)**: `GET {base_url}/api/v1/companies`
//!   verified via Snipe-IT's `routes/api.php` (index/show/store/update/
//!   destroy/selectlist endpoints, only the list is needed here). A single
//!   Snipe-IT instance can manage assets of multiple companies (e.g. an MSP
//!   running customer inventories in one shared instance) -- hence exactly
//!   the same granular mapping principle as with NinjaOne's "Organizations"
//!   (`NinjaOrgMapping`): `SnipeitCompanyMapping` maps each company
//!   independently to a local customer, `SnipeitConnectionMeta` itself
//!   deliberately carries NO `customer_id`.
//! - **Devices/assets**: `GET {base_url}/api/v1/hardware`, offset-paginated
//!   (NOT cursor-based like NinjaOne/Level.io) -- `limit`/`offset` query
//!   parameters, verified via Snipe-IT's own API reference page
//!   (<https://snipe-it.readme.io/reference/hardware-list>). The default
//!   `limit` value is absurdly low at 2, so `PAGE_LIMIT` (100) is always
//!   sent explicitly here. Response envelope verified via Snipe-IT's own
//!   source code
//!   (`app/Http/Transformers/DatatablesTransformer.php::transformDatatables`):
//!   `{"total": <number>, "rows": [...], "current_page": ..., "per_page":
//!   ..., "total_pages": ..., "prev_page_url": ..., "next_page_url": ...}`
//!   -- so `total`/`rows`, no guessing. `list_devices` walks all pages
//!   internally (up to `MAX_PAGES` pages of `PAGE_LIMIT` assets each,
//!   protection against a misbehaving remote end) and returns a single,
//!   already-merged list -- the caller sees nothing of Snipe-IT's
//!   pagination, exactly the same principle as with NinjaOne/Level.io.
//! - **No hostname/IP address**: Snipe-IT is asset/inventory management,
//!   not RMM monitoring software -- unlike NinjaOne/Level.io, the core
//!   asset object has NO guaranteed hostname/IP address field. Verified via
//!   Snipe-IT's own source code
//!   (`app/Http/Transformers/AssetsTransformer.php::transformAsset`): the
//!   field list there includes `id`, `name`, `asset_tag`, `serial`,
//!   `model`, `company`, `status_label`/`status` and more, but provably
//!   neither `hostname` nor an IP address. `SnipeitDevice.hostname`/
//!   `ip_address` are therefore ALWAYS `None` -- not a guess, but an
//!   honest, verified omission. Instead, Snipe-IT's own natural
//!   identification fields (`asset_tag`, `serial`) are first-class fields
//!   here. Snipe-IT additionally returns a `custom_fields` object per asset
//!   (verified: `$fields_array[$field->name] = {field, value,
//!   field_format, element}`, i.e. keys named after the field name, not a
//!   fixed list) -- since field names are freely configured by each
//!   Snipe-IT administrator, there is deliberately NO brittle guessing
//!   logic here for a "hostname"-like custom field; a clean,
//!   `asset_tag`/`serial`-based model with `hostname`/`ip_address` always
//!   `None` is honest and correct for v1.
//! - **Web UI link**: `{base_url}/hardware/{id}` shows an asset's detail
//!   page in Snipe-IT's own web UI (NOT the API), verified via Snipe-IT's
//!   `routes/web/hardware.php` (`Route::resource('hardware',
//!   AssetsController::class, ...)`, the standard Laravel "show" route is
//!   `hardware/{asset}`). Analogous to NinjaOne's `ninja_url`, there is
//!   therefore a `snipeit_url` counterpart in
//!   `commands::snipeit::ExternalSystemDto` -- not a made-up URL scheme,
//!   but derived from Snipe-IT's own source code.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the exact same dependency
//!   as `plugin::ninja`/`plugin::level` (no second HTTP client in this
//!   codebase).
//! - **Credential encoding**: Snipe-IT needs only a single secret value
//!   (the Personal Access Token), which is passed through 1:1 as
//!   `PluginCredentials.secret` -- like Level.io, no JSON encoding of
//!   multiple values is needed (unlike NinjaOne).

use serde::{Deserialize, Serialize};
use std::time::Duration;
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Fixed API path suffix appended to the user-supplied `base_url` (see
/// module documentation). Snipe-IT is self-hosted -- unlike Level.io (fixed
/// `BASE_URL` constant) -- so no hardcoded host, just this shared path
/// suffix.
const API_PATH: &str = "/api/v1";

/// Protection against a misbehaving remote end (e.g. `total` that never
/// decreases): more than `MAX_PAGES * PAGE_LIMIT` rows per call are not
/// fetched. Chosen higher than Level.io's `MAX_PAGES` (20) because a
/// Snipe-IT instance used by an MSP can realistically manage several
/// thousand assets.
const MAX_PAGES: usize = 50;
/// Snipe-IT's own default `limit` value is, per its own API reference, only
/// 2 -- so this higher value is always sent explicitly on every page here.
const PAGE_LIMIT: u32 = 100;

/// Synthetic placeholder for `SnipeitDevice.company_id` when an asset is,
/// per Snipe-IT itself, assigned to NO company (`"company": null` or the
/// field is missing entirely -- a legitimate case common in Snipe-IT, not
/// an error: many single-tenant instances don't use the company concept at
/// all). Deliberately non-numeric so it can never collide with a real,
/// numeric Snipe-IT company ID. `commands::snipeit` groups assets with this
/// ID as their own, clearly labeled group instead of silently discarding
/// them -- analogous to how `plugin::ninja` handles a device's
/// `organizationId` that doesn't match any known organization.
pub const UNASSIGNED_COMPANY_ID: &str = "unassigned";

/// Non-secret metadata of a Snipe-IT connection, as stored in
/// `config.toml` (`Config::snipeit_connections`). The Personal Access Token
/// belongs, per the credential principle, exclusively in the OS keyring,
/// never here. Deliberately WITHOUT `customer_id` -- a connection is a
/// Snipe-IT instance, not a local customer; which Snipe-IT "Company" within
/// this instance corresponds to which local customer is tracked granularly
/// in `SnipeitCompanyMapping`/`Config::snipeit_company_mappings` -- exactly
/// the same principle as `plugin::ninja::NinjaConnectionMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnipeitConnectionMeta {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

/// Mapping of a single Snipe-IT "Company" (within a connection) to a local
/// customer. Lives in `Config::snipeit_company_mappings`, not in
/// `SnipeitConnectionMeta` -- a connection can see multiple companies, each
/// of which can be mapped independently (or left unmapped). `company_name`
/// is stored in addition to `company_id` so a UI list can display a
/// readable name without another live call against Snipe-IT -- analogous
/// to `plugin::ninja::NinjaOrgMapping`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnipeitCompanyMapping {
    pub connection_id: String,
    pub company_id: String,
    pub company_name: String,
    pub customer_id: i64,
}

/// A company reported by `GET /api/v1/companies`. Separate from
/// `ExternalSystem` (those are assets) -- its own, small shape type,
/// analogous to `plugin::ninja::NinjaOrganization`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnipeitCompany {
    pub id: String,
    pub name: String,
}

/// A single asset from `GET /api/v1/hardware` or
/// `GET /api/v1/hardware/{id}`, enriched with company membership
/// (`company_id`) and Snipe-IT's own natural identification fields
/// (`asset_tag`, `serial`) -- needed by
/// `commands::snipeit::sync_snipeit_connection` for grouping by company and
/// for display, which the generic, plugin-agnostic `ExternalSystem` type
/// from `plugin::mod` deliberately does not provide. `hostname`/
/// `ip_address` are ALWAYS `None` (see module documentation) -- present as
/// fields anyway so this type structurally matches
/// `plugin::ninja::NinjaDevice`/`plugin::level::LevelDevice` and a future UI
/// can handle it uniformly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnipeitDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub asset_tag: Option<String>,
    pub serial: Option<String>,
    pub company_id: String,
}

/// A plugin object for exactly one configured Snipe-IT connection. `id`
/// here is already the fully qualified identifier
/// (`"snipeit:<connection_id>"`), so that `Plugin::id()` works unmodified
/// as the `plugin_id`/keyring account (see trait documentation in
/// `plugin::mod`).
pub struct SnipeitPlugin {
    id: String,
    base_url: String,
}

impl SnipeitPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live fetch of this connection's company list (`GET /api/v1/companies`,
    /// merged internally across all pages, see module documentation).
    /// Separate from the `Plugin` trait method `list_systems`, because
    /// companies are not assets and the generic trait has no room for that
    /// -- analogous to `plugin::ninja::NinjaPlugin::list_organizations`.
    pub fn list_companies(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<SnipeitCompany>, PluginError> {
        let agent = build_agent();
        let raw = fetch_all_companies(&agent, &self.base_url, &credentials.secret)?;
        Ok(map_companies_response(&raw))
    }

    /// Live fetch of all this connection's assets, with pagination walked
    /// internally (see module documentation). Richer than the trait method
    /// `list_systems`, which deliberately stays with the narrow,
    /// plugin-agnostic `ExternalSystem` type (no `asset_tag`/`serial`/
    /// `company_id` field there).
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<SnipeitDevice>, PluginError> {
        let agent = build_agent();
        let raw = fetch_all_hardware(&agent, &self.base_url, &credentials.secret)?;
        Ok(map_hardware_response(&raw))
    }
}

/// Checks a Personal Access Token against Snipe-IT (a lightweight call: one
/// page with `limit=1`), without persisting anything. For
/// `commands::snipeit::test_snipeit_connection`, so users notice a typo in
/// the base URL/token before actually creating a connection (writing
/// credentials to the keyring) -- exactly the same pattern as
/// `plugin::level::test_credentials`.
pub fn test_credentials(base_url: &str, token: &str) -> Result<(), PluginError> {
    let agent = build_agent();
    fetch_hardware_page(&agent, base_url, token, 0, 1)?;
    Ok(())
}

impl Plugin for SnipeitPlugin {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn list_systems(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<ExternalSystem>, PluginError> {
        let devices = self.list_devices(credentials)?;
        Ok(devices
            .into_iter()
            .map(|d| ExternalSystem {
                external_id: d.external_id,
                name: d.name,
                hostname: d.hostname,
            })
            .collect())
    }

    fn get_system_details(
        &self,
        credentials: &PluginCredentials,
        external_id: &str,
    ) -> Result<serde_json::Value, PluginError> {
        let agent = build_agent();
        fetch_json(
            &agent,
            &self.base_url,
            &format!("/hardware/{external_id}"),
            &credentials.secret,
        )
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Snipe-IT's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("SnipeitPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

fn fetch_json(
    agent: &Agent,
    base_url: &str,
    path: &str,
    token: &str,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{}{API_PATH}{path}", base_url.trim_end_matches('/'));
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

fn fetch_hardware_page(
    agent: &Agent,
    base_url: &str,
    token: &str,
    offset: u32,
    limit: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{}{API_PATH}/hardware", base_url.trim_end_matches('/'));
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .query("limit", limit.to_string())
        .query("offset", offset.to_string())
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

fn fetch_companies_page(
    agent: &Agent,
    base_url: &str,
    token: &str,
    offset: u32,
    limit: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{}{API_PATH}/companies", base_url.trim_end_matches('/'));
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .query("limit", limit.to_string())
        .query("offset", offset.to_string())
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// Fetches all pages of `GET /api/v1/hardware` and returns the raw asset
/// objects (not yet mapped to `SnipeitDevice`) as a single, merged list.
/// Breaks off early if a page returns fewer than `PAGE_LIMIT` rows (last
/// page) OR the already-fetched row count reaches `total`, or if
/// `MAX_PAGES` is reached (protection against a misbehaving remote end).
fn fetch_all_hardware(
    agent: &Agent,
    base_url: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut offset: u32 = 0;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_hardware_page(agent, base_url, token, offset, PAGE_LIMIT)?;
        let (rows, total) = parse_hardware_page(&page_json)?;
        let got = rows.len();
        all.extend(rows);
        offset += PAGE_LIMIT;
        if got < PAGE_LIMIT as usize || (all.len() as u64) >= total {
            break;
        }
    }
    Ok(all)
}

/// Fetches all pages of `GET /api/v1/companies`, following exactly the same
/// pattern as `fetch_all_hardware`.
fn fetch_all_companies(
    agent: &Agent,
    base_url: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut offset: u32 = 0;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_companies_page(agent, base_url, token, offset, PAGE_LIMIT)?;
        let (rows, total) = parse_companies_page(&page_json)?;
        let got = rows.len();
        all.extend(rows);
        offset += PAGE_LIMIT;
        if got < PAGE_LIMIT as usize || (all.len() as u64) >= total {
            break;
        }
    }
    Ok(all)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Snipe-IT-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Snipe-IT-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extracts rows and total count from a single Snipe-IT device page
/// response (`GET /api/v1/hardware`: `{"total": <number>, "rows": [...],
/// ...}`, verified via Snipe-IT's own source code
/// (`DatatablesTransformer::transformDatatables`), see module
/// documentation). Pure function, testable with hardcoded JSON, no real
/// network access needed. If `total` is missing/has an unexpected shape ->
/// falls back to this page's actual row count (then `fetch_all_hardware`
/// breaks off after this one page, instead of running into an infinite
/// loop).
fn parse_hardware_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, u64), PluginError> {
    let rows = json["rows"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse(
            "Erwartete 'rows'-Liste in Snipe-IT-Geräte-Antwort".to_string(),
        )
    })?;
    let total = json["total"].as_u64().unwrap_or(rows.len() as u64);
    Ok((rows.clone(), total))
}

/// Same as `parse_hardware_page`, just for `GET /api/v1/companies` --
/// identical response envelope (`DatatablesTransformer` is used by
/// Snipe-IT for both endpoints), but deliberately kept as its own function
/// with its own error message, analogous to
/// `plugin::level::parse_devices_page`/`parse_groups_page`.
fn parse_companies_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, u64), PluginError> {
    let rows = json["rows"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse(
            "Erwartete 'rows'-Liste in Snipe-IT-Firmen-Antwort".to_string(),
        )
    })?;
    let total = json["total"].as_u64().unwrap_or(rows.len() as u64);
    Ok((rows.clone(), total))
}

/// Maps a (already merged across all pages) list of raw Snipe-IT asset
/// objects to `SnipeitDevice` values. Pure function, testable with
/// hardcoded JSON -- Snipe-IT's "List Hardware" and "Show Hardware"
/// endpoints return asset objects in the same shape.
fn map_hardware_response(rows: &[serde_json::Value]) -> Vec<SnipeitDevice> {
    rows.iter().filter_map(map_hardware_asset).collect()
}

/// A single asset object from Snipe-IT's `/hardware` response. Unlike
/// NinjaOne/Level.io, Snipe-IT often has no meaningfully maintained `name`
/// field (assets there are primarily identified via `asset_tag`, `name` is
/// an optional nickname and frequently `null`/empty); the display name
/// therefore falls back from `name` through `asset_tag` and `serial` down
/// to the external ID. `company` is a nested object (`{"id": ..., "name":
/// ...}`) or `null`/missing if the asset isn't assigned to any company -- a
/// legitimate case (see `UNASSIGNED_COMPANY_ID` documentation), not an
/// error. An asset without a usable `id` is skipped instead of failing the
/// whole call -- a single broken asset object shouldn't make the whole
/// list unusable (analogous to
/// `plugin::ninja::map_device`/`plugin::level::map_level_device`).
fn map_hardware_asset(value: &serde_json::Value) -> Option<SnipeitDevice> {
    let external_id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let asset_tag = value["asset_tag"].as_str().map(str::to_string);
    let serial = value["serial"].as_str().map(str::to_string);
    let raw_name = value["name"].as_str().filter(|s| !s.is_empty());
    let name = raw_name
        .or(asset_tag.as_deref())
        .or(serial.as_deref())
        .unwrap_or(external_id.as_str())
        .to_string();
    let company_id = match &value["company"]["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => UNASSIGNED_COMPANY_ID.to_string(),
    };
    Some(SnipeitDevice {
        external_id,
        name,
        hostname: None,
        ip_address: None,
        asset_tag,
        serial,
        company_id,
    })
}

/// Maps the row list returned by `GET /api/v1/companies` to `SnipeitCompany`
/// values. Pure function, testable on its own with hardcoded JSON,
/// analogous to `map_hardware_response`.
fn map_companies_response(rows: &[serde_json::Value]) -> Vec<SnipeitCompany> {
    rows.iter().filter_map(map_company).collect()
}

/// A single company object from Snipe-IT's `/companies` response
/// (`{"id": <number>, "name": "...", ...}`, verified via Snipe-IT's own
/// source code, `CompaniesTransformer::transformCompany`). If `id` is
/// missing or has an unexpected shape -> the company is skipped instead of
/// failing the whole call, analogous to `map_hardware_asset`.
fn map_company(value: &serde_json::Value) -> Option<SnipeitCompany> {
    let id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let name = value["name"].as_str().unwrap_or(id.as_str()).to_string();
    Some(SnipeitCompany { id, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hardware_page_with_rows_and_total() {
        let json = serde_json::json!({
            "total": 2,
            "rows": [{"id": 1}, {"id": 2}]
        });
        let (rows, total) = parse_hardware_page(&json).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(total, 2);
    }

    #[test]
    fn parse_hardware_page_rejects_missing_rows_field() {
        let json = serde_json::json!({"total": 0});
        let result = parse_hardware_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn parse_hardware_page_total_defaults_to_rows_length_when_missing() {
        let json = serde_json::json!({"rows": [{"id": 1}]});
        let (rows, total) = parse_hardware_page(&json).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn parses_companies_page_with_rows_and_total() {
        let json = serde_json::json!({
            "total": 1,
            "rows": [{"id": 5, "name": "ACME GmbH"}]
        });
        let (rows, total) = parse_companies_page(&json).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn parse_companies_page_rejects_missing_rows_field() {
        let json = serde_json::json!({"total": 0});
        let result = parse_companies_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn maps_hardware_json_array_into_snipeit_devices() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {
                    "id": 101,
                    "name": "Bürorechner Anna",
                    "asset_tag": "AT-0001",
                    "serial": "SN-XYZ",
                    "company": {"id": 7, "name": "ACME GmbH"}
                },
                {
                    "id": 202,
                    "asset_tag": "AT-0002"
                }
            ]"#,
        )
        .unwrap();

        let devices = map_hardware_response(json.as_array().unwrap());

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "101");
        assert_eq!(devices[0].name, "Bürorechner Anna");
        assert_eq!(devices[0].asset_tag.as_deref(), Some("AT-0001"));
        assert_eq!(devices[0].serial.as_deref(), Some("SN-XYZ"));
        assert_eq!(devices[0].company_id, "7");
        assert_eq!(devices[0].hostname, None);
        assert_eq!(devices[0].ip_address, None);
        assert_eq!(devices[1].external_id, "202");
        assert_eq!(devices[1].name, "AT-0002");
    }

    #[test]
    fn falls_back_to_serial_when_no_name_or_asset_tag_present() {
        let json = serde_json::json!([{"id": 9, "serial": "SN-ONLY"}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].name, "SN-ONLY");
    }

    #[test]
    fn falls_back_to_external_id_when_nothing_else_present() {
        let json = serde_json::json!([{"id": 9}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].name, "9");
    }

    #[test]
    fn empty_name_string_falls_back_to_asset_tag() {
        let json = serde_json::json!([{"id": 9, "name": "", "asset_tag": "AT-9"}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].name, "AT-9");
    }

    #[test]
    fn skips_devices_without_a_usable_id() {
        let json = serde_json::json!([{"name": "OhneId"}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert!(devices.is_empty());
    }

    #[test]
    fn device_without_company_gets_unassigned_sentinel() {
        let json = serde_json::json!([{"id": 1, "company": null}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].company_id, UNASSIGNED_COMPANY_ID);
    }

    #[test]
    fn device_with_missing_company_field_gets_unassigned_sentinel() {
        let json = serde_json::json!([{"id": 1}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].company_id, UNASSIGNED_COMPANY_ID);
    }

    #[test]
    fn device_with_company_object_gets_its_id() {
        let json = serde_json::json!([{"id": 1, "company": {"id": 42, "name": "ACME"}}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].company_id, "42");
    }

    #[test]
    fn maps_companies_json_array_into_snipeit_companies() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {"id": 1, "name": "ACME Hauptsitz"},
                {"id": 2, "name": "ACME Zweigstelle"}
            ]"#,
        )
        .unwrap();

        let companies = map_companies_response(json.as_array().unwrap());

        assert_eq!(companies.len(), 2);
        assert_eq!(
            companies[0],
            SnipeitCompany {
                id: "1".to_string(),
                name: "ACME Hauptsitz".to_string()
            }
        );
        assert_eq!(
            companies[1],
            SnipeitCompany {
                id: "2".to_string(),
                name: "ACME Zweigstelle".to_string()
            }
        );
    }

    #[test]
    fn company_falls_back_to_id_when_no_name_field_present() {
        let json = serde_json::json!([{"id": 9}]);
        let companies = map_companies_response(json.as_array().unwrap());
        assert_eq!(companies[0].name, "9");
    }

    #[test]
    fn company_skips_entries_without_a_usable_id() {
        let json = serde_json::json!([{"name": "OhneId"}]);
        let companies = map_companies_response(json.as_array().unwrap());
        assert!(companies.is_empty());
    }

    #[test]
    fn maps_401_status_to_authentication_error() {
        let err = map_ureq_error(ureq::Error::StatusCode(401));
        assert!(matches!(err, PluginError::Authentication(_)));
    }

    #[test]
    fn maps_403_status_to_authentication_error() {
        let err = map_ureq_error(ureq::Error::StatusCode(403));
        assert!(matches!(err, PluginError::Authentication(_)));
    }

    #[test]
    fn maps_other_status_codes_to_unreachable_error() {
        let err = map_ureq_error(ureq::Error::StatusCode(500));
        assert!(matches!(err, PluginError::Unreachable(_)));
    }

    #[test]
    fn plugin_id_returns_configured_connection_id() {
        let plugin = SnipeitPlugin::new(
            "snipeit:acme-123".to_string(),
            "https://assets.example.com".to_string(),
        );
        assert_eq!(plugin.id(), "snipeit:acme-123");
    }
}
