//! Real plugin implementation for Jamf Pro (Apple device management), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Jamf-Pro-Plugin". Fourth real
//! integration after NinjaOne (`plugin::ninja`), Level.io (`plugin::level`)
//! and Snipe-IT (`plugin::snipeit`). Structure and signatures follow
//! `plugin::dummy::DummyPlugin`, but talk over real HTTPS (crate `ureq`,
//! synchronous, no async runtime) to Jamf Pro's public REST API.
//!
//! - **Authentication**: OAuth2 client-credentials grant, structurally
//!   identical to `plugin::ninja` -- `POST {base_url}/api/oauth/token`,
//!   `Content-Type: application/x-www-form-urlencoded`, body
//!   `grant_type=client_credentials&client_id=<id>&client_secret=<secret>`
//!   (verified against Jamf's own developer docs,
//!   <https://developer.jamf.com>). Unlike NinjaOne there's no `scope`
//!   parameter. The access token is deliberately not cached across multiple
//!   calls -- same "fetch fresh per method call" simplicity choice already
//!   made for `plugin::ninja` (see its module docs), for the same reason:
//!   this app calls plugin methods rarely/manually, never in a hot loop.
//! - **Self-hosted/cloud-hosted**: like NinjaOne/Snipe-IT (and unlike
//!   Level.io's fixed `BASE_URL` constant), a Jamf connection needs a
//!   user-supplied base URL (`JamfConnectionMeta.base_url`, e.g.
//!   `https://yourserver.jamfcloud.com`).
//! - **Sites (multi-site delegation)**: `GET {base_url}/api/v1/sites`
//!   (verified against Jamf's own developer docs,
//!   <https://developer.jamf.com/jamf-pro/reference/get_v1-sites>) returns a
//!   plain JSON array of `{"id": "...", "name": "...", "divisionId": ...}`
//!   objects, NOT paginated -- structurally identical to NinjaOne's
//!   `GET /v2/organizations`. A single Jamf Pro server can delegate
//!   inventory across multiple "Sites" (e.g. an MSP or a multi-campus
//!   organization) -- hence exactly the same granular mapping principle as
//!   `plugin::ninja::NinjaOrgMapping`: `JamfSiteMapping { connection_id,
//!   site_id, site_name, customer_id }`, `JamfConnectionMeta` itself
//!   deliberately WITHOUT `customer_id`.
//! - **Computers**: `GET {base_url}/api/v1/computers-inventory?section=GENERAL&section=HARDWARE`,
//!   page/page-size-paginated (NOT cursor-based like NinjaOne/Level.io, NOT
//!   offset-based like Snipe-IT -- its own third pagination style, verified
//!   against Jamf's own developer docs,
//!   <https://developer.jamf.com/jamf-pro/reference/get_v1-computers-inventory>:
//!   `page` (0-based) / `page-size` query parameters). Response envelope
//!   verified via the same source: `{"totalCount": <number>, "results":
//!   [...]}`. `list_computers` walks all pages internally (up to
//!   `MAX_PAGES` pages of `PAGE_SIZE` computers each, protection against a
//!   misbehaving remote end, exactly the same principle as
//!   `plugin::snipeit::fetch_all_hardware`) and returns a single,
//!   already-merged list -- the caller sees nothing of Jamf's pagination.
//! - **Site membership/identifying fields**: each computer's
//!   `general.site.id`/`general.site.name` assigns it to exactly one site
//!   (verified via Jamf's own response schema for
//!   `GET /api/v1/computers-inventory`) -- the field this module maps
//!   against `JamfSiteMapping`, analogous to NinjaOne's `organizationId`.
//!   `general.name` is Jamf's own display name for a computer AND doubles as
//!   the hostname-equivalent identifying field (this API has no separate
//!   "hostname" field on a macOS computer distinct from `general.name`) --
//!   used both as `JamfDevice.name` and `JamfDevice.hostname`, matching
//!   NinjaOne/Level.io's hostname-based "link to existing system" matching
//!   convention (see `commands::jamf`/`JamfPluginSection.tsx`). IP address
//!   comes from `general.lastIpAddress`, with `general.lastReportedIpV4` as
//!   a fallback (both verified against Jamf's own response schema).
//! - **Computer detail**: `GET {base_url}/api/v1/computers-inventory/{id}`
//!   with the same `section=GENERAL&section=HARDWARE` query parameters as
//!   the list call (verified endpoint, deliberately preferred over the
//!   sibling `computers-inventory-detail/{id}` endpoint -- both are flagged
//!   as deprecated in Jamf's current OpenAPI spec with no documented
//!   replacement at time of writing, and Jamf has a long history of such
//!   "deprecated" endpoints remaining live for years, e.g. its entire
//!   classic API; this one is preferred because it returns the SAME
//!   `general`/`hardware` section shape as the list call above instead of
//!   every section Jamf knows about, which keeps
//!   `JamfPluginSection.tsx`'s external-field compare panel predictable).
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the exact same dependency
//!   as `plugin::ninja`/`plugin::level`/`plugin::snipeit`. Connection/
//!   timeout/non-2xx errors map to `PluginError::Unreachable`, HTTP 401/403
//!   to `PluginError::Authentication`, unexpected JSON shapes (including a
//!   failed token exchange) to `PluginError::UnexpectedResponse`/
//!   `Authentication` -- same conventions as the other three plugins.
//! - **Credential encoding**: like NinjaOne, Jamf needs two secret values
//!   (`client_id`, `client_secret`); `PluginCredentials.secret` is, per the
//!   trait contract, a single opaque string -- encoded as JSON here exactly
//!   like `plugin::ninja::NinjaCredentials`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Protection against a misbehaving remote end (e.g. `totalCount` that never
/// decreases): more than `MAX_PAGES * PAGE_SIZE` computers per call are not
/// fetched. Same value/rationale as `plugin::snipeit::MAX_PAGES`.
const MAX_PAGES: usize = 50;
/// Sent explicitly as `page-size` on every page of `GET
/// /api/v1/computers-inventory` (verified default is smaller) -- keeps
/// request counts bounded for a large inventory.
const PAGE_SIZE: u32 = 100;

/// Non-secret metadata of a Jamf connection, as stored in `config.toml`
/// (`Config::jamf_connections`). Client ID/secret belong, per the credential
/// principle, exclusively in the OS keyring, never here. Deliberately
/// WITHOUT `customer_id` -- a connection is a Jamf Pro server, not a local
/// customer; which Jamf "Site" within this server corresponds to which local
/// customer is tracked granularly in `JamfSiteMapping`/
/// `Config::jamf_site_mappings` -- exactly the same principle as
/// `plugin::ninja::NinjaConnectionMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JamfConnectionMeta {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

/// Mapping of a single Jamf "Site" (within a connection) to a local
/// customer. Lives in `Config::jamf_site_mappings`, not in
/// `JamfConnectionMeta` -- a connection can see multiple sites, each of
/// which can be mapped independently (or left unmapped). `site_name` is
/// stored in addition to `site_id` so a UI list can display a readable name
/// without another live call against Jamf -- analogous to
/// `plugin::ninja::NinjaOrgMapping`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JamfSiteMapping {
    pub connection_id: String,
    pub site_id: String,
    pub site_name: String,
    pub customer_id: i64,
}

/// A site reported by `GET /api/v1/sites`. Separate from `ExternalSystem`
/// (those are computers) -- its own, small shape type, analogous to
/// `plugin::ninja::NinjaOrganization`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JamfSite {
    pub id: String,
    pub name: String,
}

/// A single computer from `GET /api/v1/computers-inventory` or
/// `GET /api/v1/computers-inventory/{id}`, enriched with site membership
/// (`general.site.id`) -- needed by `commands::jamf::sync_jamf_connection`
/// for grouping by site and for display, which the generic, plugin-agnostic
/// `ExternalSystem` type from `plugin::mod` deliberately does not provide
/// (it stays the narrow, trait-generic minimum that `DummyPlugin` must also
/// be able to satisfy). Hence its own, Jamf-specific type instead of
/// extending `ExternalSystem` -- exactly the same principle as
/// `plugin::ninja::NinjaDevice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JamfDevice {
    pub external_id: String,
    pub name: String,
    /// Always `Some(general.name)` when a name is present -- Jamf has no
    /// field distinct from `general.name` for a macOS computer in this API,
    /// see module documentation. Kept as `Option` for structural parity with
    /// `plugin::ninja::NinjaDevice`/`plugin::level::LevelDevice`/
    /// `plugin::snipeit::SnipeitDevice`.
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub serial_number: Option<String>,
    pub asset_tag: Option<String>,
    /// Jamf's own free-text OS description, combined from the nested
    /// `operatingSystem.name` + `operatingSystem.version` fields (e.g.
    /// `"macOS 14.5"`) -- requires the `section=OPERATING_SYSTEM` query
    /// parameter added above; without it this object is absent from the
    /// response and this is simply `None`.
    pub operating_system: Option<String>,
    pub site_id: String,
}

/// The two secret values a Jamf connection needs to authenticate.
/// `PluginCredentials.secret` is, per the trait contract, a single opaque
/// string that the plugin interprets itself -- here encoded as JSON exactly
/// like `plugin::ninja::NinjaCredentials`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JamfCredentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// A plugin object for exactly one configured Jamf connection. `id` here is
/// already the fully qualified identifier (`"jamf:<connection_id>"`), so
/// that `Plugin::id()` works unmodified as the `plugin_id`/keyring account
/// (see trait documentation in `plugin::mod`).
pub struct JamfPlugin {
    id: String,
    base_url: String,
}

impl JamfPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live fetch of this connection's site list (`GET /api/v1/sites`, not
    /// paginated, see module documentation). Separate from the `Plugin`
    /// trait method `list_systems`, because sites are not computers and the
    /// generic trait has no room for that -- analogous to
    /// `plugin::ninja::NinjaPlugin::list_organizations`.
    pub fn list_sites(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<JamfSite>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        let json = fetch_json(&agent, &self.base_url, "/api/v1/sites", &token)?;
        map_sites_response(&json)
    }

    /// Live fetch of all this connection's computers, with pagination walked
    /// internally (see module documentation). Richer than the trait method
    /// `list_systems`, which deliberately stays with the narrow,
    /// plugin-agnostic `ExternalSystem` type (no `site_id`/`serial_number`/
    /// `asset_tag` field there).
    pub fn list_computers(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<JamfDevice>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        let raw = fetch_all_computers(&agent, &self.base_url, &token)?;
        Ok(map_computers_response(&raw))
    }
}

/// Checks a client ID/secret pair against Jamf Pro (OAuth2 client-credentials
/// grant), without persisting anything -- the access token is discarded
/// after a successful fetch. For `commands::jamf::test_jamf_connection`, so
/// users notice typos in the base URL/credentials before actually creating a
/// connection (writing credentials to the keyring) -- exactly the same
/// pattern as `plugin::ninja::test_credentials`.
pub fn test_credentials(
    base_url: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<(), PluginError> {
    let creds = JamfCredentials {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
    };
    let agent = build_agent();
    fetch_access_token(&agent, base_url, &creds)?;
    Ok(())
}

impl Plugin for JamfPlugin {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn list_systems(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<ExternalSystem>, PluginError> {
        let devices = self.list_computers(credentials)?;
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
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        fetch_computer_detail(&agent, &self.base_url, external_id, &token)
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Jamf's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("JamfPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn parse_credentials(credentials: &PluginCredentials) -> Result<JamfCredentials, PluginError> {
    serde_json::from_str(&credentials.secret)
        .map_err(|e| PluginError::Authentication(format!("Jamf-Zugangsdaten ungültig: {e}")))
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

fn fetch_access_token(
    agent: &Agent,
    base_url: &str,
    creds: &JamfCredentials,
) -> Result<String, PluginError> {
    let url = format!("{}/api/oauth/token", base_url.trim_end_matches('/'));
    let mut response = agent
        .post(&url)
        .send_form([
            ("grant_type", "client_credentials"),
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
        ])
        .map_err(map_ureq_error)?;
    let token: TokenResponse = response.body_mut().read_json().map_err(map_ureq_error)?;
    Ok(token.access_token)
}

fn fetch_json(
    agent: &Agent,
    base_url: &str,
    path: &str,
    bearer_token: &str,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{}{path}", base_url.trim_end_matches('/'));
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {bearer_token}"))
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

fn fetch_computers_page(
    agent: &Agent,
    base_url: &str,
    bearer_token: &str,
    page: u32,
    page_size: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!(
        "{}/api/v1/computers-inventory",
        base_url.trim_end_matches('/')
    );
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {bearer_token}"))
        .query("section", "GENERAL")
        .query("section", "HARDWARE")
        .query("section", "OPERATING_SYSTEM")
        .query("page", page.to_string())
        .query("page-size", page_size.to_string())
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

fn fetch_computer_detail(
    agent: &Agent,
    base_url: &str,
    external_id: &str,
    bearer_token: &str,
) -> Result<serde_json::Value, PluginError> {
    let url = format!(
        "{}/api/v1/computers-inventory/{external_id}",
        base_url.trim_end_matches('/')
    );
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {bearer_token}"))
        .query("section", "GENERAL")
        .query("section", "HARDWARE")
        .query("section", "OPERATING_SYSTEM")
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// Fetches all pages of `GET /api/v1/computers-inventory` and returns the
/// raw computer objects (not yet mapped to `JamfDevice`) as a single, merged
/// list. Breaks off early if a page returns fewer than `PAGE_SIZE` rows
/// (last page) OR the already-fetched row count reaches `totalCount`, or if
/// `MAX_PAGES` is reached (protection against a misbehaving remote end) --
/// exactly the same pattern as `plugin::snipeit::fetch_all_hardware`.
fn fetch_all_computers(
    agent: &Agent,
    base_url: &str,
    bearer_token: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    for page in 0..MAX_PAGES as u32 {
        let page_json = fetch_computers_page(agent, base_url, bearer_token, page, PAGE_SIZE)?;
        let (rows, total) = parse_computers_page(&page_json)?;
        let got = rows.len();
        all.extend(rows);
        if got < PAGE_SIZE as usize || (all.len() as u64) >= total {
            break;
        }
    }
    Ok(all)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Jamf-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Jamf-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extracts rows and total count from a single Jamf computers-inventory page
/// response (`{"totalCount": <number>, "results": [...]}`, verified via
/// Jamf's own developer docs, see module documentation). Pure function,
/// testable with hardcoded JSON, no real network access needed. If
/// `totalCount` is missing/has an unexpected shape -> falls back to this
/// page's actual row count (then `fetch_all_computers` breaks off after this
/// one page, instead of running into an infinite loop) -- same fallback
/// convention as `plugin::snipeit::parse_hardware_page`.
fn parse_computers_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, u64), PluginError> {
    let rows = json["results"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse(
            "Erwartete 'results'-Liste in Jamf-Computer-Antwort".to_string(),
        )
    })?;
    let total = json["totalCount"].as_u64().unwrap_or(rows.len() as u64);
    Ok((rows.clone(), total))
}

/// Maps the JSON list returned by `GET /api/v1/sites` to `JamfSite` values.
/// Pure function, testable on its own with hardcoded JSON, analogous to
/// `plugin::ninja::map_organizations_response`.
fn map_sites_response(json: &serde_json::Value) -> Result<Vec<JamfSite>, PluginError> {
    let array = json.as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete JSON-Liste von Sites".to_string())
    })?;
    Ok(array.iter().filter_map(map_site).collect())
}

/// A single site object from Jamf's `/api/v1/sites` response (`{"id":
/// "...", "name": "...", "divisionId": ...}`, per Jamf's own API
/// specification -- `id` is already a string, unlike NinjaOne's numeric
/// `id`). If `id` is missing or has an unexpected shape -> the site is
/// skipped instead of failing the whole call, analogous to
/// `plugin::ninja::map_organization`.
fn map_site(value: &serde_json::Value) -> Option<JamfSite> {
    let id = match &value["id"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let name = value["name"].as_str().unwrap_or(id.as_str()).to_string();
    Some(JamfSite { id, name })
}

/// Maps a (already merged across all pages) list of raw Jamf computer
/// objects to `JamfDevice` values. Pure function, testable with hardcoded
/// JSON -- Jamf's list and single-computer detail endpoints return computer
/// objects in the same `general`/`hardware` section shape (see module
/// documentation).
fn map_computers_response(rows: &[serde_json::Value]) -> Vec<JamfDevice> {
    rows.iter().filter_map(map_computer).collect()
}

/// A single computer object from Jamf's `computers-inventory` response
/// (`{"id": "...", "udid": "...", "general": {"name": ..., "lastIpAddress":
/// ..., "lastReportedIpV4": ..., "serialNumber": ..., "assetTag": ...,
/// "site": {"id": ..., "name": ...}}, "hardware": {"serialNumber": ...,
/// "macAddress": ...}}`, per Jamf's own API specification, see module
/// documentation). A computer without a usable `id` OR without a usable
/// `general.site.id` is skipped -- without site membership it can't be
/// meaningfully grouped, and a single broken computer object shouldn't make
/// the whole list unusable, exactly the same convention as
/// `plugin::ninja::map_ninja_device` (which likewise skips a device without
/// a usable `organizationId`).
fn map_computer(value: &serde_json::Value) -> Option<JamfDevice> {
    let external_id = match &value["id"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let general = &value["general"];
    let site_id = match &general["site"]["id"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    // `general.name` is both the display name AND the hostname-equivalent
    // identifying field for a Jamf computer (see module documentation) --
    // there is no separate hostname field to fall back to/from.
    let name_str = general["name"].as_str();
    let name = name_str.unwrap_or(external_id.as_str()).to_string();
    let hostname = name_str.map(str::to_string);
    let ip_address = general["lastIpAddress"]
        .as_str()
        .or_else(|| general["lastReportedIpV4"].as_str())
        .map(str::to_string);
    let serial_number = general["serialNumber"]
        .as_str()
        .or_else(|| value["hardware"]["serialNumber"].as_str())
        .map(str::to_string);
    let asset_tag = general["assetTag"].as_str().map(str::to_string);
    let os_name = value["operatingSystem"]["name"].as_str();
    let os_version = value["operatingSystem"]["version"].as_str();
    let operating_system = match (os_name, os_version) {
        (Some(name), Some(version)) => Some(format!("{name} {version}")),
        (Some(name), None) => Some(name.to_string()),
        (None, _) => None,
    };
    Some(JamfDevice {
        external_id,
        name,
        hostname,
        ip_address,
        serial_number,
        asset_tag,
        operating_system,
        site_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_computers_page_with_results_and_total_count() {
        let json = serde_json::json!({
            "totalCount": 2,
            "results": [{"id": "1"}, {"id": "2"}]
        });
        let (rows, total) = parse_computers_page(&json).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(total, 2);
    }

    #[test]
    fn parse_computers_page_rejects_missing_results_field() {
        let json = serde_json::json!({"totalCount": 0});
        let result = parse_computers_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn parse_computers_page_total_defaults_to_results_length_when_missing() {
        let json = serde_json::json!({"results": [{"id": "1"}]});
        let (rows, total) = parse_computers_page(&json).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn maps_sites_json_array_into_jamf_sites() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {"id": "1", "name": "Hauptsitz"},
                {"id": "2", "name": "Zweigstelle"}
            ]"#,
        )
        .unwrap();

        let sites = map_sites_response(&json).unwrap();

        assert_eq!(sites.len(), 2);
        assert_eq!(
            sites[0],
            JamfSite {
                id: "1".to_string(),
                name: "Hauptsitz".to_string()
            }
        );
        assert_eq!(
            sites[1],
            JamfSite {
                id: "2".to_string(),
                name: "Zweigstelle".to_string()
            }
        );
    }

    #[test]
    fn sites_rejects_non_array_json() {
        let json = serde_json::json!({"not": "an array"});
        let result = map_sites_response(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn site_falls_back_to_id_when_no_name_field_present() {
        let json = serde_json::json!([{"id": "9"}]);
        let sites = map_sites_response(&json).unwrap();
        assert_eq!(sites[0].name, "9");
    }

    #[test]
    fn site_skips_entries_without_a_usable_id() {
        let json = serde_json::json!([{"name": "OhneId"}]);
        let sites = map_sites_response(&json).unwrap();
        assert!(sites.is_empty());
    }

    #[test]
    fn maps_computers_json_array_with_site_and_ip() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {
                    "id": "101",
                    "udid": "ABC-123",
                    "general": {
                        "name": "MBP-Anna",
                        "lastIpAddress": "10.0.0.5",
                        "lastReportedIpV4": "10.0.0.6",
                        "serialNumber": "C02XXXXX",
                        "assetTag": "AT-0001",
                        "site": {"id": "1", "name": "Hauptsitz"}
                    },
                    "hardware": {"serialNumber": "C02XXXXX", "macAddress": "AA:BB:CC:DD:EE:FF"},
                    "operatingSystem": {"name": "macOS", "version": "14.5"}
                },
                {
                    "id": "202",
                    "general": {
                        "name": "MBP-Ben",
                        "lastReportedIpV4": "10.0.0.9",
                        "site": {"id": "2", "name": "Zweigstelle"}
                    }
                }
            ]"#,
        )
        .unwrap();

        let devices = map_computers_response(json.as_array().unwrap());

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "101");
        assert_eq!(devices[0].site_id, "1");
        assert_eq!(devices[0].name, "MBP-Anna");
        assert_eq!(devices[0].hostname.as_deref(), Some("MBP-Anna"));
        assert_eq!(devices[0].ip_address.as_deref(), Some("10.0.0.5"));
        assert_eq!(devices[0].serial_number.as_deref(), Some("C02XXXXX"));
        assert_eq!(devices[0].asset_tag.as_deref(), Some("AT-0001"));
        assert_eq!(devices[0].operating_system.as_deref(), Some("macOS 14.5"));
        assert_eq!(devices[1].external_id, "202");
        assert_eq!(devices[1].site_id, "2");
        assert_eq!(devices[1].ip_address.as_deref(), Some("10.0.0.9"));
        assert_eq!(devices[1].asset_tag, None);
        assert_eq!(devices[1].operating_system, None);
    }

    #[test]
    fn computer_rejects_missing_id_by_skipping_it() {
        let json = serde_json::json!([{"general": {"name": "OhneId", "site": {"id": "1"}}}]);
        let devices = map_computers_response(json.as_array().unwrap());
        assert!(devices.is_empty());
    }

    #[test]
    fn computer_skips_devices_without_site_id() {
        let json = serde_json::json!([{"id": "5", "general": {"name": "OhneSite"}}]);
        let devices = map_computers_response(json.as_array().unwrap());
        assert!(devices.is_empty());
    }

    #[test]
    fn computer_ip_falls_back_to_last_reported_ipv4() {
        let json = serde_json::json!([{
            "id": "5",
            "general": {"name": "Fallback", "lastReportedIpV4": "192.168.1.9", "site": {"id": "1"}}
        }]);
        let devices = map_computers_response(json.as_array().unwrap());
        assert_eq!(devices[0].ip_address.as_deref(), Some("192.168.1.9"));
    }

    #[test]
    fn computer_has_no_ip_address_when_neither_field_present() {
        let json =
            serde_json::json!([{"id": "5", "general": {"name": "OhneIp", "site": {"id": "1"}}}]);
        let devices = map_computers_response(json.as_array().unwrap());
        assert_eq!(devices[0].ip_address, None);
    }

    #[test]
    fn computer_serial_number_falls_back_to_hardware_section() {
        let json = serde_json::json!([{
            "id": "5",
            "general": {"name": "SN-Fallback", "site": {"id": "1"}},
            "hardware": {"serialNumber": "HW-SERIAL-1"}
        }]);
        let devices = map_computers_response(json.as_array().unwrap());
        assert_eq!(devices[0].serial_number.as_deref(), Some("HW-SERIAL-1"));
    }

    #[test]
    fn parses_credentials_from_json_secret() {
        let creds = PluginCredentials {
            secret: r#"{"client_id":"abc","client_secret":"xyz"}"#.into(),
        };
        let parsed = parse_credentials(&creds).unwrap();
        assert_eq!(parsed.client_id, "abc");
        assert_eq!(parsed.client_secret, "xyz");
    }

    #[test]
    fn rejects_malformed_credentials_json() {
        let creds = PluginCredentials {
            secret: "not json".into(),
        };
        let result = parse_credentials(&creds);
        assert!(matches!(result, Err(PluginError::Authentication(_))));
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
        let plugin = JamfPlugin::new(
            "jamf:acme-123".to_string(),
            "https://acme.jamfcloud.com".to_string(),
        );
        assert_eq!(plugin.id(), "jamf:acme-123");
    }
}
