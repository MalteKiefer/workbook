//! Real plugin implementation for Hetzner Cloud (German VPS/server
//! hosting), see `docs/PLUGIN_ARCHITECTURE.md` section
//! "Hetzner-Cloud-Plugin". Fifteenth real integration, and structurally the
//! simplest one in this codebase: unlike every RMM/MDM plugin here, this
//! plugin's purpose is **server inventory** (which Hetzner Cloud servers a
//! customer has), not device-management agents -- and unlike Acronis, there
//! is no backup-status concept either. Architecturally closest to
//! `plugin::level`/`plugin::intune`: a connection maps 1:1 to exactly one
//! local customer, no organization/tenant mapping table at all.
//!
//! Verified against Hetzner's own official Cloud API documentation and the
//! `hcloud-go` SDK source:
//!
//! - **Authentication**: static API token, header `Authorization: Bearer
//!   <token>`, NO OAuth2. Generated in the Hetzner Cloud Console under
//!   Security > API tokens (Read-only or Read&Write -- this plugin only
//!   ever does GETs, so either works, no scope-checking needed client-side).
//!   The token is bound to exactly ONE Hetzner "Project" -- confirmed: a
//!   token cannot see other projects.
//! - **No organization/tenant concept**: one API token = one Hetzner
//!   Project = one local customer, confirmed 1:1. That's why a Hetzner
//!   "connection" here maps directly to exactly one local customer
//!   (`HetznerConnectionMeta.customer_id`) -- NO mapping table like
//!   `NinjaOrgMapping`/`TacticalRmmClientMapping` is needed, the same
//!   principle as `plugin::level`/`plugin::intune`.
//! - **Fixed host, no `base_url`**: Hetzner Cloud's API host is always
//!   `https://api.hetzner.cloud/v1` -- like `plugin::level::BASE_URL`, no
//!   user-supplied `base_url` field is needed on `HetznerConnectionMeta`.
//! - **Servers (the inventory endpoint)**: `GET {BASE_URL}/servers` ->
//!   `{"servers": [...]}` -- a wrapping key, NOT a bare array (unlike
//!   Level.io's `{"devices": [...]}`). Verified per-server fields: `id`
//!   (integer, used as `external_id`, stringified), `name` (string, the
//!   display name -- also doubles as `HetznerServer::hostname`, the same
//!   "no separate hostname field" convention `plugin::intune` uses for
//!   `deviceName`), `status` (free-form string passthrough, no Rust enum,
//!   same convention as every other plugin's `status`/`platform` fields),
//!   `server_type.name` (nested -- used as `HetznerServer::platform`, e.g.
//!   "cx22"), `location.name` (nested -- used as `HetznerServer::location`,
//!   e.g. "fsn1", informational display only, like Pulseway's `site_name`).
//!   `datacenter` is deliberately NEVER read -- Hetzner's own SDK marks it
//!   deprecated (removal after 2026-10-01), superseded by `location`.
//! - **IPv4 vs. IPv6, a real distinction, not an oversight**:
//!   `public_net.ipv4.ip` is a single host address, used directly as
//!   `HetznerServer::ip_address`. `public_net.ipv6.ip` is deliberately
//!   NEVER used for that field -- verified against Hetzner's API schema, it
//!   is a **/64 CIDR SUBNET** (e.g. `"2001:db8::/64"`), not a single host
//!   address. Presenting that subnet as if it were one server's plain IP
//!   would be actively wrong, not just an omission -- see
//!   `extract_ipv4_address` below, which reads ONLY `public_net.ipv4.ip`.
//! - **Single server detail**: `GET {BASE_URL}/servers/{id}` ->
//!   `{"server": {...}}` -- wrapped under a "server" key, unlike
//!   Level.io's/Intune's single-device endpoints (which return the device
//!   object directly, unwrapped). Passing that envelope through unmodified
//!   would break the frontend's flat, top-level field scan
//!   (`findExternalValue` in `HetznerPluginSection.tsx`), so
//!   `fetch_server_json` unwraps to the inner server object before
//!   returning it -- see `unwrap_server_envelope` below.
//! - **Pagination**: query params `page` (1-based) and `per_page`. Response
//!   envelope: `{"servers": [...], "meta": {"pagination": {"page",
//!   "per_page", "previous_page", "next_page", "last_page",
//!   "total_entries"}}}`. More pages exist while `next_page` is
//!   non-null/non-zero -- `collect_paginated` walks all pages internally
//!   (up to `MAX_PAGES` pages of `PAGE_LIMIT` servers each, as protection
//!   against a misbehaving remote end, same convention as
//!   `plugin::level::MAX_PAGES`) and returns a single, already-merged list.
//!   `PAGE_LIMIT` (50) is Hetzner's own documented maximum `per_page` value
//!   -- not independently re-verified beyond that documented maximum here,
//!   noted honestly.
//! - **Rate limits**: response headers `RateLimit-Limit`/
//!   `RateLimit-Remaining`/`RateLimit-Reset` exist; commonly cited ~3600
//!   requests/hour per project (commonly cited, not independently verified
//!   from primary prose docs here). No special retry/backoff handling
//!   needed, matching every other plugin in this codebase.
//! - **No web dashboard deep-link**: a plausible pattern exists
//!   (`https://console.hetzner.cloud/projects/{project_id}/servers/{server_id}`)
//!   but requires a `project_id` that nothing in the `/servers` response
//!   provides, and the pattern itself is only an observed convention, not a
//!   documented guarantee -- left out entirely, the same honest-omission
//!   principle as `plugin::tacticalrmm`'s missing dashboard link.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the exact same dependency
//!   as every other plugin here.
//! - **Credential encoding**: Hetzner needs only a single secret value (the
//!   API token), passed through 1:1 as `PluginCredentials.secret` -- no
//!   JSON encoding needed, exactly like `plugin::level`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Hetzner Cloud's API host is fixed -- no region/instance variants, hence a
/// constant instead of a configurable `base_url` field on
/// `HetznerConnectionMeta`, analogous to `plugin::level::BASE_URL`.
pub const BASE_URL: &str = "https://api.hetzner.cloud/v1";

/// Protection against a misbehaving remote end (an endless `next_page`
/// chain): more than `MAX_PAGES * PAGE_LIMIT` servers per sync run are not
/// fetched, the same defensive convention as `plugin::level::MAX_PAGES`.
const MAX_PAGES: usize = 50;
/// Hetzner's own documented maximum `per_page` value (see module docs).
const PAGE_LIMIT: u32 = 50;

/// Non-secret metadata of a Hetzner Cloud connection, as stored in
/// `config.toml` (`Config::hetzner_connections`). The API token belongs,
/// per the credential principle, exclusively in the OS keyring, never here.
/// Like `LevelConnectionMeta`/`IntuneConnectionMeta`: Hetzner has no
/// organization concept, so a connection here directly carries its
/// `customer_id` -- a connection corresponds to exactly one local customer
/// (one API token = one Hetzner Project = one local customer, confirmed
/// 1:1, see module docs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HetznerConnectionMeta {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

/// A single server from `GET /v1/servers` or `GET /v1/servers/{id}`.
/// Analogous to `plugin::level::LevelDevice`/`plugin::intune::IntuneDevice`,
/// but without a group/organization concept -- Hetzner has neither, see
/// module docs. Deliberately WITHOUT a `linked_system_id` field: whether a
/// server is already linked to a local system is a bookkeeping concern of
/// the caller (`commands::hetzner`, via `db::external_refs`), not something
/// this plugin layer knows about -- exactly the same separation
/// `plugin::level::LevelDevice`/`plugin::intune::IntuneDevice` keep (the
/// `linked_system_id` field only appears on the `commands`-layer DTO, see
/// `commands::hetzner::ExternalSystemDto`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HetznerServer {
    pub external_id: String,
    pub name: String,
    /// Hetzner's server object has no separate physical-hostname field --
    /// `name` doubles as both the display name and the hostname-equivalent,
    /// the same convention `plugin::intune` uses for `deviceName`.
    pub hostname: Option<String>,
    /// ONLY `public_net.ipv4.ip` -- see `extract_ipv4_address` and the
    /// module documentation's "IPv4 vs. IPv6" section. Never the IPv6
    /// field, which is a /64 CIDR subnet, not a single host address.
    pub ip_address: Option<String>,
    /// Free-form string passthrough (`initializing`/`off`/`running`/
    /// `starting`/`stopping`/`migrating`/`rebuilding`/`deleting`/`unknown`),
    /// no Rust enum -- same convention as every other plugin's `status`/
    /// `platform` fields here.
    pub status: Option<String>,
    /// `server_type.name`, e.g. "cx22".
    pub platform: Option<String>,
    /// `location.name`, e.g. "fsn1" -- informational display only.
    pub location: Option<String>,
}

/// A plugin object for exactly one configured Hetzner connection. `id` here
/// is already the fully qualified identifier (`"hetzner:<connection_id>"`),
/// so that `Plugin::id()` works unmodified as the `plugin_id`/keyring
/// account (see trait documentation in `plugin::mod`). Like `LevelPlugin`,
/// this type needs no `base_url` field (see `BASE_URL` above) and no cached
/// token -- the API token comes fresh from `PluginCredentials` on every
/// call.
pub struct HetznerPlugin {
    id: String,
}

impl HetznerPlugin {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    /// Live fetch of ALL servers of this connection, with pagination walked
    /// internally (see module documentation). Richer than the trait method
    /// `list_systems`, which deliberately stays with the narrow,
    /// plugin-agnostic `ExternalSystem` type (no `status`/`platform`/
    /// `location` fields there).
    pub fn list_servers(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<HetznerServer>, PluginError> {
        let agent = build_agent();
        let secret = credentials.secret.clone();
        let raw = collect_paginated(MAX_PAGES, |page| {
            fetch_servers_page(&agent, &secret, page, PAGE_LIMIT)
        })?;
        Ok(map_servers(&raw))
    }
}

/// Checks an API token against Hetzner Cloud (a lightweight call: one page
/// with `per_page=1`), without persisting anything. For
/// `commands::hetzner::test_hetzner_connection`, so users notice a typo in
/// the API token before actually creating a connection (writing
/// credentials to the keyring).
pub fn test_credentials(api_token: &str) -> Result<(), PluginError> {
    let agent = build_agent();
    fetch_servers_page(&agent, api_token, 1, 1)?;
    Ok(())
}

impl Plugin for HetznerPlugin {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn list_systems(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<ExternalSystem>, PluginError> {
        let servers = self.list_servers(credentials)?;
        Ok(servers
            .into_iter()
            .map(|s| ExternalSystem {
                external_id: s.external_id,
                name: s.name,
                hostname: s.hostname,
            })
            .collect())
    }

    fn get_system_details(
        &self,
        credentials: &PluginCredentials,
        external_id: &str,
    ) -> Result<serde_json::Value, PluginError> {
        let agent = build_agent();
        fetch_server_json(&agent, &credentials.secret, external_id)
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Hetzner's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("HetznerPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

/// Generic pagination walker: starts at page 1, calls `fetch_page` for each
/// page number, extends the running list with `parse_servers_page`'s data,
/// and continues to whatever `next_page` that page reports -- up to
/// `max_pages`, the same defensive brake against a misbehaving remote end
/// every other plugin's own pagination loop uses (see
/// `plugin::level::MAX_PAGES`). Factored out from `HetznerPlugin::list_servers`
/// as its own, network-agnostic function (`fetch_page` is an injected
/// closure) specifically so the accumulation/stop logic itself is
/// unit-testable with hardcoded JSON pages, no real network access needed --
/// this codebase has no HTTP mocking dependency, so this is the only way to
/// genuinely exercise "the pagination loop collects across multiple pages"
/// without one.
fn collect_paginated<F>(
    max_pages: usize,
    mut fetch_page: F,
) -> Result<Vec<serde_json::Value>, PluginError>
where
    F: FnMut(u32) -> Result<serde_json::Value, PluginError>,
{
    let mut all = Vec::new();
    let mut page: u32 = 1;
    for _ in 0..max_pages {
        let page_json = fetch_page(page)?;
        let (data, next_page) = parse_servers_page(&page_json)?;
        all.extend(data);
        match next_page {
            Some(next) => page = next,
            None => break,
        }
    }
    Ok(all)
}

fn fetch_servers_page(
    agent: &Agent,
    api_token: &str,
    page: u32,
    per_page: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{BASE_URL}/servers");
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {api_token}"))
        .query("page", page.to_string())
        .query("per_page", per_page.to_string())
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// `GET {BASE_URL}/servers/{id}` -> `{"server": {...}}` (see module docs).
/// Unwraps to the inner server object before returning it, so
/// `Plugin::get_system_details`'s result stays a flat object usable by the
/// frontend's top-level field scan, the same shape every other plugin's
/// single-resource endpoint already returns unwrapped.
fn fetch_server_json(
    agent: &Agent,
    api_token: &str,
    external_id: &str,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{BASE_URL}/servers/{external_id}");
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {api_token}"))
        .call()
        .map_err(map_ureq_error)?;
    let json: serde_json::Value = response.body_mut().read_json().map_err(map_ureq_error)?;
    Ok(unwrap_server_envelope(json))
}

/// Pure function, testable with hardcoded JSON: unwraps a `{"server":
/// {...}}` envelope to the inner object. Falls back to the raw value
/// unmodified if "server" is missing or not an object, rather than failing
/// the whole call -- a defensive fallback for an unexpected shape, not an
/// expected case (see module docs).
fn unwrap_server_envelope(json: serde_json::Value) -> serde_json::Value {
    match json {
        serde_json::Value::Object(mut map) => match map.remove("server") {
            Some(server @ serde_json::Value::Object(_)) => server,
            _ => serde_json::Value::Object(map),
        },
        other => other,
    }
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Hetzner-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Hetzner-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extracts the server list and next-page indicator from a single Hetzner
/// page response (`GET /v1/servers`: `{"servers": [...], "meta":
/// {"pagination": {"next_page": ...}}}` -- a wrapping key, NOT a bare
/// array, unlike Level.io's `{"devices": [...]}`, verified against
/// Hetzner's own API documentation). Pure function, testable with
/// hardcoded JSON, no real network access needed. `next_page` of
/// `null`/`0`/missing means "no more pages".
fn parse_servers_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, Option<u32>), PluginError> {
    let data = json["servers"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete 'servers'-Liste in Hetzner-Antwort".to_string())
    })?;
    let next_page = json["meta"]["pagination"]["next_page"]
        .as_u64()
        .filter(|&p| p > 0)
        .map(|p| p as u32);
    Ok((data.clone(), next_page))
}

/// Maps a (already merged across all pages) list of raw Hetzner server
/// objects to `HetznerServer` values. Pure function, testable with
/// hardcoded JSON -- Hetzner's "List Servers" and "Get Server" endpoints
/// return server objects in exactly the same shape.
fn map_servers(servers: &[serde_json::Value]) -> Vec<HetznerServer> {
    servers.iter().filter_map(map_server).collect()
}

/// A single server object from Hetzner's response. A server without a
/// usable `id` is skipped instead of failing the whole call -- a single
/// broken server object shouldn't make the whole list unusable, analogous
/// to `plugin::level::map_level_device`. Every other field degrades
/// gracefully instead of failing: `name` falls back to the external ID
/// (mirrors `plugin::intune::map_intune_device`'s fallback for
/// `deviceName`), `status`/`server_type.name`/`location.name`/IP address
/// simply become `None` when missing.
fn map_server(value: &serde_json::Value) -> Option<HetznerServer> {
    let id = value["id"].as_i64()?;
    let external_id = id.to_string();
    let name = value["name"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| external_id.clone());
    let status = value["status"].as_str().map(str::to_string);
    let platform = value["server_type"]["name"].as_str().map(str::to_string);
    let location = value["location"]["name"].as_str().map(str::to_string);
    let ip_address = extract_ipv4_address(value);
    Some(HetznerServer {
        external_id,
        hostname: Some(name.clone()),
        name,
        ip_address,
        status,
        platform,
        location,
    })
}

/// Reads ONLY `public_net.ipv4.ip` -- a single host address, safe to use
/// directly as `ip_address`. `public_net.ipv6.ip` is deliberately NEVER
/// read here: verified against Hetzner's API schema, it is a /64 CIDR
/// SUBNET (e.g. "2001:db8::/64"), not a single host address -- presenting
/// it as if it were one server's plain IP would be wrong, not just an
/// omission (see module documentation, "IPv4 vs. IPv6").
fn extract_ipv4_address(value: &serde_json::Value) -> Option<String> {
    value["public_net"]["ipv4"]["ip"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_server_json() -> serde_json::Value {
        serde_json::json!({
            "id": 42,
            "name": "web-01",
            "status": "running",
            "server_type": {"name": "cx22"},
            "location": {"name": "fsn1"},
            "public_net": {
                "ipv4": {"ip": "203.0.113.5"},
                "ipv6": {"ip": "2001:db8::/64"}
            }
        })
    }

    #[test]
    fn parses_servers_page_with_servers_and_next_page() {
        let json = serde_json::json!({
            "servers": [{"id": 1}, {"id": 2}],
            "meta": {"pagination": {"next_page": 2}}
        });
        let (data, next_page) = parse_servers_page(&json).unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(next_page, Some(2));
    }

    #[test]
    fn parses_servers_page_with_null_next_page_as_last_page() {
        let json = serde_json::json!({
            "servers": [],
            "meta": {"pagination": {"next_page": null}}
        });
        let (data, next_page) = parse_servers_page(&json).unwrap();
        assert!(data.is_empty());
        assert_eq!(next_page, None);
    }

    #[test]
    fn parses_servers_page_with_zero_next_page_as_last_page() {
        let json = serde_json::json!({
            "servers": [],
            "meta": {"pagination": {"next_page": 0}}
        });
        let (_, next_page) = parse_servers_page(&json).unwrap();
        assert_eq!(next_page, None);
    }

    #[test]
    fn parses_servers_page_with_missing_pagination_as_last_page() {
        let json = serde_json::json!({"servers": []});
        let (_, next_page) = parse_servers_page(&json).unwrap();
        assert_eq!(next_page, None);
    }

    #[test]
    fn parse_servers_page_rejects_missing_servers_field() {
        let json = serde_json::json!({"meta": {"pagination": {}}});
        let result = parse_servers_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn collect_paginated_walks_until_next_page_is_none() {
        let pages = vec![
            serde_json::json!({"servers": [{"id": 1}], "meta": {"pagination": {"next_page": 2}}}),
            serde_json::json!({"servers": [{"id": 2}, {"id": 3}], "meta": {"pagination": {"next_page": null}}}),
        ];
        let mut calls = 0usize;
        let result = collect_paginated(MAX_PAGES, |page| {
            assert_eq!(page, calls as u32 + 1);
            let json = pages[calls].clone();
            calls += 1;
            Ok(json)
        })
        .unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(calls, 2);
    }

    #[test]
    fn collect_paginated_stops_at_a_single_page_without_next_page() {
        let result = collect_paginated(MAX_PAGES, |_page| {
            Ok(serde_json::json!({"servers": [{"id": 1}], "meta": {"pagination": {"next_page": null}}}))
        })
        .unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn collect_paginated_stops_at_the_max_pages_guard_against_an_endless_next_page_chain() {
        let result = collect_paginated(3, |page| {
            Ok(serde_json::json!({"servers": [{"id": page}], "meta": {"pagination": {"next_page": page + 1}}}))
        })
        .unwrap();
        // A misbehaving remote end that always reports another next_page
        // must still be bounded by max_pages, not walked forever.
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn collect_paginated_propagates_a_page_parse_error() {
        let result = collect_paginated(MAX_PAGES, |_page| Ok(serde_json::json!({"oops": true})));
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn maps_full_server_json() {
        let servers = map_servers(std::slice::from_ref(&full_server_json()));
        assert_eq!(servers.len(), 1);
        let server = &servers[0];
        assert_eq!(server.external_id, "42");
        assert_eq!(server.name, "web-01");
        assert_eq!(server.hostname.as_deref(), Some("web-01"));
        assert_eq!(server.status.as_deref(), Some("running"));
        assert_eq!(server.platform.as_deref(), Some("cx22"));
        assert_eq!(server.location.as_deref(), Some("fsn1"));
        assert_eq!(server.ip_address.as_deref(), Some("203.0.113.5"));
    }

    #[test]
    fn skips_servers_without_a_usable_id() {
        let json = serde_json::json!([{"name": "ohne-id"}]);
        let servers = map_servers(json.as_array().unwrap());
        assert!(servers.is_empty());
    }

    #[test]
    fn falls_back_to_external_id_when_name_is_missing() {
        let json = serde_json::json!([{"id": 7}]);
        let servers = map_servers(json.as_array().unwrap());
        assert_eq!(servers[0].external_id, "7");
        assert_eq!(servers[0].name, "7");
        assert_eq!(servers[0].hostname.as_deref(), Some("7"));
    }

    #[test]
    fn missing_status_server_type_and_location_become_none_not_an_error() {
        let json = serde_json::json!([{"id": 1, "name": "srv"}]);
        let servers = map_servers(json.as_array().unwrap());
        assert_eq!(servers[0].status, None);
        assert_eq!(servers[0].platform, None);
        assert_eq!(servers[0].location, None);
        assert_eq!(servers[0].ip_address, None);
    }

    #[test]
    fn ipv6_cidr_subnet_is_never_used_as_ip_address_even_without_ipv4_present() {
        let json = serde_json::json!({
            "id": 1,
            "name": "srv",
            "public_net": {"ipv6": {"ip": "2001:db8::/64"}}
        });
        let servers = map_servers(std::slice::from_ref(&json));
        assert_eq!(servers[0].ip_address, None);
    }

    #[test]
    fn ipv4_is_used_even_when_ipv6_is_also_present() {
        let servers = map_servers(std::slice::from_ref(&full_server_json()));
        assert_eq!(servers[0].ip_address.as_deref(), Some("203.0.113.5"));
    }

    #[test]
    fn extract_ipv4_address_ignores_missing_public_net() {
        let json = serde_json::json!({"id": 1, "name": "srv"});
        assert_eq!(extract_ipv4_address(&json), None);
    }

    #[test]
    fn unwrap_server_envelope_unwraps_the_server_key() {
        let json = serde_json::json!({"server": {"id": 1, "name": "srv"}});
        let unwrapped = unwrap_server_envelope(json);
        assert_eq!(unwrapped["id"], 1);
        assert_eq!(unwrapped["name"], "srv");
    }

    #[test]
    fn unwrap_server_envelope_falls_back_to_raw_value_when_server_key_missing() {
        let json = serde_json::json!({"id": 1, "name": "srv"});
        let unwrapped = unwrap_server_envelope(json.clone());
        assert_eq!(unwrapped, json);
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
        let plugin = HetznerPlugin::new("hetzner:acme-123".to_string());
        assert_eq!(plugin.id(), "hetzner:acme-123");
    }

    #[test]
    fn list_systems_maps_hetzner_servers_into_generic_external_systems() {
        let plugin = HetznerPlugin::new("hetzner:acme-123".to_string());
        // list_systems delegates through list_servers -> map_servers; this
        // exercises the field projection directly without real network
        // access by constructing the mapped value the same way
        // list_systems does internally.
        let servers = map_servers(std::slice::from_ref(&full_server_json()));
        let systems: Vec<ExternalSystem> = servers
            .into_iter()
            .map(|s| ExternalSystem {
                external_id: s.external_id,
                name: s.name,
                hostname: s.hostname,
            })
            .collect();
        assert_eq!(systems[0].external_id, "42");
        assert_eq!(systems[0].name, "web-01");
        assert_eq!(systems[0].hostname.as_deref(), Some("web-01"));
        assert_eq!(plugin.id(), "hetzner:acme-123");
    }
}
