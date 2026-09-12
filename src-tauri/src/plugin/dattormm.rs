//! Real plugin implementation for Datto RMM (cloud-hosted, multi-pod RMM,
//! part of Kaseya, <https://www.datto.com/product/rmm/>), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Datto-RMM-Plugin". Ninth real
//! integration after NinjaOne (`plugin::ninja`), Level.io (`plugin::level`),
//! Snipe-IT (`plugin::snipeit`), Microsoft Intune (`plugin::intune`), Iru
//! (`plugin::iru`), Jamf Pro (`plugin::jamf`), Apple Business Manager
//! (`plugin::abm`) and Tactical RMM (`plugin::tacticalrmm`). Structurally
//! closest to Jamf Pro: a genuine "Site" grouping level with a real
//! string foreign key directly on each device (`siteUid`, matching
//! `Site.uid`) -- unlike Tactical RMM, which has to join agents to clients by
//! NAME because its agent list API carries no client ID at all (see
//! `plugin::tacticalrmm` module docs). Datto RMM's hierarchy is flatter than
//! Tactical RMM's Client -> Site -> Agent, though: just Site -> Device, no
//! higher "Client" grouping level -- so mapping happens directly at the Site
//! level, analogous to how Jamf Pro maps at its own Site level. Self-hosted-
//! style configuration (a user-supplied `base_url`, like NinjaOne/Snipe-IT/
//! Tactical RMM/Jamf Pro), because Datto RMM is split across six independent
//! regional API hosts ("pods") with no way to derive one from the others.
//!
//! Every fact below comes from Datto RMM's own official help documentation
//! and a live-fetched OpenAPI 3.1 specification for its public REST API --
//! not guessed from third-party prose:
//!
//! - **Authentication**: an OAuth2-STYLE token exchange, but NOT a generic
//!   OAuth2 client-credentials grant like `plugin::ninja`/`plugin::jamf`, and
//!   NOT a bare tenant-and-secret exchange like `plugin::intune` either --
//!   `POST {base_url}/auth/oauth/token`, HTTP Basic Auth using Datto's own
//!   FIXED, publicly documented OAuth client (`OAUTH_CLIENT_ID` =
//!   `"public-client"`, `OAUTH_CLIENT_SECRET` = `"public"` -- NOT something a
//!   user generates, the same constant pair for every Datto RMM customer) as
//!   the Basic-Auth username/password, body
//!   `application/x-www-form-urlencoded` with
//!   `grant_type=password&username=<API Key>&password=<API Secret Key>` --
//!   the user's own API Key/API Secret Key pair (generated in Datto RMM's
//!   web UI: Setup -> Users -> click user -> Generate API Keys) go in as
//!   `username`/`password`, deliberately NOT `client_id`/`client_secret` --
//!   an easy mistake to make by pattern-matching `plugin::intune`'s OAuth2
//!   client-credentials grant too literally (see `oauth_token_form`, which
//!   exists specifically so this exact shape has its own unit test). Response
//!   200 + JSON `{"access_token": "<JWT>", ...}` on success, 400 on a bad key
//!   pair. Used as `Authorization: Bearer <access_token>` on every subsequent
//!   call. Tokens expire after 100 hours -- like every other plugin here (see
//!   `plugin::intune` module docs), no refresh-token handling: a fresh token
//!   is fetched per sync.
//! - **Base URL -- multi-pod**: Datto RMM runs across six regional pods
//!   (Pinotage, Merlot, Concord, Vidal, Zinfandel, Syrah), each with its own
//!   API hostname (e.g. `https://merlot-api.centrastage.net`). A user finds
//!   their own pod's exact API URL on their own Datto RMM user page
//!   (populated after generating API keys) -- there is no way to derive it
//!   automatically, hence `DattoRmmConnectionMeta.base_url` is a plain
//!   user-supplied `String`, like `TacticalRmmConnectionMeta`/
//!   `JamfConnectionMeta`, NOT a fixed constant like `plugin::intune`'s
//!   `GRAPH_BASE_URL`/`plugin::level`'s `BASE_URL`. API version `v2` sits in
//!   the path (`{base_url}/api/v2/...`).
//! - **Tenancy entity -- "Site"**: `GET {base_url}/api/v2/account/sites`
//!   (query params `page`, `max`, `siteName`). Response is a wrapped envelope
//!   `{"pageDetails": {...}, "sites": [...]}` (unlike Tactical RMM's bare,
//!   unpaginated array). `uid` (a string) is the real identifier used
//!   everywhere else -- used as the mapping key here, NOT the numeric `id`.
//!   Mapping happens at exactly this level -- Datto RMM's flat grouping
//!   level, simpler than Tactical RMM's Client -> Site hierarchy, there's no
//!   higher level to worry about.
//! - **Devices**: `GET {base_url}/api/v2/account/devices` (all devices,
//!   account-wide -- preferred here over the site-scoped
//!   `GET {base_url}/api/v2/site/{siteUid}/devices` for a single sync pass,
//!   grouping client-side by the `siteUid`/`siteName` fields present
//!   directly on each device). Response: `{"pageDetails": {...}, "devices":
//!   [...]}`. `uid` (string) is the real identifier, used as
//!   `external_id`/`DattoRmmDevice::external_id`; the numeric `id` is not
//!   used, same convention as `Site`. `intIpAddress`/`extIpAddress` are both
//!   plain strings -- `extract_ip_address` prefers `intIpAddress`, falls
//!   back to `extIpAddress`, the same "primary + fallback" convention as
//!   `plugin::tacticalrmm::extract_ip_address`/`plugin::ninja`, just adapted
//!   to Datto RMM's field names. `online` is a real JSON boolean (not a
//!   string enum like Tactical RMM's `status`) -- converted here to
//!   `"online"`/`"offline"` strings for display consistency with every other
//!   plugin in this codebase (a deliberate choice, not a fixed API
//!   constraint -- Datto's own field really is a bool). `deviceClass`
//!   (`"device"`/`"printer"`/`"esxihost"`/`"rmmnetworkdevice"`/`"unknown"`)
//!   is surfaced as-is, a free-form string like Tactical RMM's `plat` -- no
//!   Rust enum, so a future additional value doesn't break parsing. `siteUid`
//!   (string) is a GENUINE foreign key directly on the device, matching
//!   `Site.uid` 1:1 -- unlike Tactical RMM, no name-join workaround is needed
//!   here (see `commands::dattormm::group_devices_by_site`, which is
//!   therefore ID-based, analogous to
//!   `commands::plugins::group_devices_by_organization`/
//!   `commands::jamf::group_devices_by_site`, NOT
//!   `commands::tacticalrmm::group_agents_by_client`'s name-based join).
//!   `portalUrl` is a real, confirmed web-dashboard deep link, present on
//!   both `Site` and `Device` -- unlike Tactical RMM's verified ABSENCE of
//!   one (see `plugin::tacticalrmm` module docs), Datto RMM genuinely
//!   provides one per site/device, surfaced here as `Option<String>` on both
//!   `DattoRmmSite`/`DattoRmmDevice` and rendered in
//!   `DattoRmmPluginSection.tsx`.
//! - **Extra field beyond the brief's literal `DattoRmmDevice` shape**:
//!   `hostname: Option<String>` is kept separate from `name` (which falls
//!   back to `external_id` when `hostname` is absent), mirroring
//!   `plugin::tacticalrmm::TacticalRmmAgent`'s `hostname`/`name` split --
//!   without it, `DattoRmmPluginSection.tsx`'s "link to existing system"
//!   hostname-match heuristic would risk comparing a local system's hostname
//!   against a Datto RMM device UID whenever a device happens to have no
//!   `hostname` of its own.
//! - **Pagination**: query params `page` (0-indexed) and `max` (capped at
//!   250/page -- `PAGE_SIZE` uses the cap). Every list response carries
//!   `pageDetails: {count, totalCount, prevPageUrl, nextPageUrl}`;
//!   `nextPageUrl` is `null` on the last page -- `fetch_all_pages` loops
//!   while it's non-null, following that EXACT URL for the next request (not
//!   re-deriving `page`/`max` itself), the same "follow the server's own
//!   continuation URL" principle as `plugin::intune`'s `@odata.nextLink`
//!   walk, just under a different JSON key. Capped at `MAX_PAGES` pages,
//!   protection against a misbehaving remote end, the same defensive
//!   convention as `plugin::intune::MAX_PAGES`/`plugin::level::MAX_PAGES`.
//! - **Single-device detail**: `GET {base_url}/api/v2/device/{deviceUid}`
//!   (by the string `uid`) -- used for `Plugin::get_system_details`, raw JSON
//!   pass-through, same convention as every other plugin here.
//! - **Rate limits**: 600 query requests/60s, 100 write requests/60s
//!   (account-wide, rolling window); 429 near the limit, 403 + a temporary IP
//!   block on persistent breach. No retry/backoff logic here -- matches
//!   every other plugin in this codebase.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the same dependency as every
//!   other plugin here.
//! - **Credential encoding**: Datto RMM needs two secret values (API Key,
//!   API Secret Key) -- `PluginCredentials.secret` is, per the trait
//!   contract, a single opaque string that the plugin interprets itself --
//!   here encoded as JSON (`serde_json::to_string`/`from_str`), exactly like
//!   `plugin::ninja::NinjaCredentials`, just with different field names
//!   (`api_key`/`api_secret_key` instead of `client_id`/`client_secret`,
//!   because that's what they actually are -- they are used as
//!   `username`/`password` in the OAuth request body, NOT as
//!   `client_id`/`client_secret`, see the authentication bullet above).

use std::time::Duration;

use base64::prelude::*;
use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Datto RMM's own FIXED, publicly documented OAuth client used as the
/// Basic-Auth username for the token exchange -- NOT something a user
/// generates, the same constant for every Datto RMM customer (see module
/// docs). Deliberately NOT put into `DattoRmmCredentials` -- it isn't a
/// per-connection secret.
const OAUTH_CLIENT_ID: &str = "public-client";
/// Datto RMM's own FIXED, publicly documented OAuth client secret used as
/// the Basic-Auth password for the token exchange (see `OAUTH_CLIENT_ID`).
const OAUTH_CLIENT_SECRET: &str = "public";

/// Maximum page size Datto RMM's list endpoints accept (see module docs on
/// pagination) -- always requested, to minimize the number of round trips.
const PAGE_SIZE: u32 = 250;

/// Protection against a misbehaving remote end (a `nextPageUrl` that never
/// stops appearing): more than `MAX_PAGES` pages are not fetched, the same
/// defensive convention as `plugin::intune::MAX_PAGES`/
/// `plugin::level::MAX_PAGES`.
const MAX_PAGES: usize = 50;

/// Non-secret metadata of a Datto RMM connection, as stored in `config.toml`
/// (`Config::dattormm_connections`). The API Key/API Secret Key pair
/// belongs, per the credential principle, exclusively in the OS keyring,
/// never here. Deliberately WITHOUT `customer_id` -- a connection is one
/// Datto RMM pod account, not a local customer; which Datto RMM "Site"
/// within this account corresponds to which local customer is tracked
/// granularly in `DattoRmmSiteMapping`/`Config::dattormm_site_mappings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DattoRmmConnectionMeta {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

/// Mapping of a single Datto RMM "Site" (within a connection) to a local
/// customer. Lives in `Config::dattormm_site_mappings`, not in
/// `DattoRmmConnectionMeta` -- a connection can see multiple sites, each of
/// which can be mapped independently (or left unmapped). `site_name` is
/// stored alongside `site_uid` so a UI list can display a readable name
/// without another live call against Datto RMM -- analogous to
/// `plugin::tacticalrmm::TacticalRmmClientMapping`/
/// `plugin::jamf::JamfSiteMapping`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DattoRmmSiteMapping {
    pub connection_id: String,
    pub site_uid: String,
    pub site_name: String,
    pub customer_id: i64,
}

/// A site reported by `GET /api/v2/account/sites`. Separate from
/// `ExternalSystem` (those are devices) -- its own, small shape type,
/// analogous to `plugin::tacticalrmm::TacticalRmmClient`/
/// `plugin::jamf::JamfSite`. `portal_url` is a genuine, confirmed
/// web-dashboard deep link (see module docs) -- unlike Tactical RMM's
/// verified absence of one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DattoRmmSite {
    pub uid: String,
    pub name: String,
    pub portal_url: Option<String>,
}

/// A single device from `GET /api/v2/account/devices`, enriched with the
/// verified extra fields `plugin::mod::ExternalSystem` deliberately does not
/// provide (IP address, status, platform, site membership, dashboard link --
/// and, beyond the brief's literal shape, `hostname`, see module docs).
/// Needed by `commands::dattormm::sync_dattormm_connection` for grouping by
/// site and for display, analogous to `plugin::tacticalrmm::TacticalRmmAgent`/
/// `plugin::jamf::JamfDevice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DattoRmmDevice {
    pub external_id: String,
    pub name: String,
    /// See the module documentation's "Extra field beyond the brief's
    /// literal `DattoRmmDevice` shape" note.
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    /// `"online"`/`"offline"`, converted from Datto RMM's own real JSON
    /// boolean `online` field (see module docs) -- kept as a free-form
    /// string, not a bool, for display consistency with every other plugin
    /// in this codebase.
    pub status: Option<String>,
    /// One of Datto RMM's own verified `deviceClass` values
    /// (`"device"`/`"printer"`/`"esxihost"`/`"rmmnetworkdevice"`/
    /// `"unknown"`), passed through as a free-form string -- no Rust enum,
    /// see module docs.
    pub platform: Option<String>,
    /// Datto RMM's own free-text OS description (e.g. `"Windows Server
    /// 2022"`), verified present on the same `GET /api/v2/account/devices`
    /// response `map_device` already parses -- distinct from `platform`
    /// above, which is only the coarse `deviceClass` family
    /// (`"device"`/`"printer"`/etc.).
    pub operating_system: Option<String>,
    /// The GENUINE foreign key to `DattoRmmSite::uid` (see module docs) --
    /// used as the join key in `commands::dattormm::group_devices_by_site`,
    /// no name-based workaround needed (unlike Tactical RMM).
    pub site_uid: String,
    pub site_name: String,
    /// A genuine, confirmed web-dashboard deep link for this exact device
    /// (see module docs) -- unlike Tactical RMM's verified absence of one.
    pub portal_url: Option<String>,
}

/// The two secret values a Datto RMM connection needs to authenticate --
/// the user's own API Key/API Secret Key pair (see module docs).
/// `PluginCredentials.secret` is, per the trait contract, a single opaque
/// string that the plugin interprets itself -- here encoded as JSON
/// (`serde_json::to_string`/`from_str`), exactly like
/// `plugin::ninja::NinjaCredentials`. Named `api_key`/`api_secret_key`
/// (matching what they actually are), even though the OAuth token request
/// sends them as `username`/`password` -- see `oauth_token_form`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DattoRmmCredentials {
    pub api_key: String,
    pub api_secret_key: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// A plugin object for exactly one configured Datto RMM connection. `id`
/// here is already the fully qualified identifier
/// (`"dattormm:<connection_id>"`), so that `Plugin::id()` works unmodified as
/// the `plugin_id`/keyring account (see trait documentation in
/// `plugin::mod`).
pub struct DattoRmmPlugin {
    id: String,
    base_url: String,
}

impl DattoRmmPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live fetch of this connection's site list
    /// (`GET /api/v2/account/sites`), walking all pages internally (see
    /// module docs). Separate from the `Plugin` trait method `list_systems`,
    /// because sites are not devices and the generic trait has no room for
    /// that -- analogous to `plugin::tacticalrmm::list_clients`/
    /// `plugin::jamf::list_sites`.
    pub fn list_sites(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<DattoRmmSite>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(
            &agent,
            &self.base_url,
            &creds.api_key,
            &creds.api_secret_key,
        )?;
        let raw = fetch_all_sites_json(&agent, &self.base_url, &token)?;
        Ok(map_sites(&raw))
    }

    /// Live fetch of all this connection's devices (account-wide, see
    /// module docs), walking all pages internally. Richer than the trait
    /// method `list_systems`, which deliberately stays with the narrow,
    /// plugin-agnostic `ExternalSystem` type.
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<DattoRmmDevice>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(
            &agent,
            &self.base_url,
            &creds.api_key,
            &creds.api_secret_key,
        )?;
        let raw = fetch_all_devices_json(&agent, &self.base_url, &token)?;
        Ok(map_devices(&raw))
    }
}

/// Checks a base URL/API Key/API Secret Key triple against Datto RMM: the
/// real OAuth token exchange, plus one lightweight call
/// (`GET /api/v2/account/sites?max=1`), without persisting anything. For
/// `commands::dattormm::test_dattormm_connection`, so users notice a typo in
/// the pod-specific base URL/credentials before actually creating a
/// connection (writing credentials to the keyring) -- the same pattern as
/// `plugin::tacticalrmm::test_credentials`/`plugin::jamf::test_credentials`.
pub fn test_credentials(
    base_url: &str,
    api_key: &str,
    api_secret_key: &str,
) -> Result<(), PluginError> {
    let agent = build_agent();
    let token = fetch_access_token(&agent, base_url, api_key, api_secret_key)?;
    let url = format!(
        "{}/api/v2/account/sites?max=1",
        base_url.trim_end_matches('/')
    );
    fetch_json(&agent, &url, &token)?;
    Ok(())
}

impl Plugin for DattoRmmPlugin {
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
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(
            &agent,
            &self.base_url,
            &creds.api_key,
            &creds.api_secret_key,
        )?;
        let url = format!(
            "{}/api/v2/device/{external_id}",
            self.base_url.trim_end_matches('/')
        );
        fetch_json(&agent, &url, &token)
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Datto RMM's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("DattoRmmPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn parse_credentials(credentials: &PluginCredentials) -> Result<DattoRmmCredentials, PluginError> {
    serde_json::from_str(&credentials.secret)
        .map_err(|e| PluginError::Authentication(format!("Datto-RMM-Zugangsdaten ungültig: {e}")))
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

/// The Datto RMM OAuth token request's form body, as an ordered list of
/// (key, value) pairs -- passed directly into `send_form`. Extracted as its
/// own, pure function so its EXACT shape
/// (`grant_type=password&username=<API Key>&password=<API Secret Key>` --
/// specifically NOT `client_id`/`client_secret`, an easy mistake when
/// pattern-matching `plugin::intune`'s OAuth2 client-credentials grant too
/// literally) can be verified by a unit test without a real HTTP call, see
/// module docs.
fn oauth_token_form<'a>(api_key: &'a str, api_secret_key: &'a str) -> [(&'static str, &'a str); 3] {
    [
        ("grant_type", "password"),
        ("username", api_key),
        ("password", api_secret_key),
    ]
}

/// The `Authorization: Basic ...` header value for the OAuth token request,
/// built from Datto RMM's own FIXED, publicly documented OAuth client (see
/// `OAUTH_CLIENT_ID`/`OAUTH_CLIENT_SECRET`) -- extracted as its own function
/// so it, too, has a dedicated unit test independent of a real HTTP call.
fn oauth_basic_auth_header() -> String {
    let encoded = BASE64_STANDARD.encode(format!("{OAUTH_CLIENT_ID}:{OAUTH_CLIENT_SECRET}"));
    format!("Basic {encoded}")
}

/// Exchanges an API Key/API Secret Key pair for a bearer access token (see
/// module docs on the exact, verified `grant_type=password` shape -- NOT a
/// generic OAuth2 client-credentials grant).
fn fetch_access_token(
    agent: &Agent,
    base_url: &str,
    api_key: &str,
    api_secret_key: &str,
) -> Result<String, PluginError> {
    let url = format!("{}/auth/oauth/token", base_url.trim_end_matches('/'));
    let mut response = agent
        .post(&url)
        .header("Authorization", oauth_basic_auth_header())
        .send_form(oauth_token_form(api_key, api_secret_key))
        .map_err(map_ureq_error)?;
    let token: TokenResponse = response.body_mut().read_json().map_err(map_ureq_error)?;
    Ok(token.access_token)
}

/// Issues an authenticated `GET {url}` and returns the parsed JSON body.
/// `url` is always already-complete here (either built from `base_url` +
/// path, or a `nextPageUrl` handed back by Datto RMM itself, see
/// `fetch_all_pages`).
fn fetch_json(
    agent: &Agent,
    url: &str,
    bearer_token: &str,
) -> Result<serde_json::Value, PluginError> {
    let mut response = agent
        .get(url)
        .header("Authorization", format!("Bearer {bearer_token}"))
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// Extracts one page's array (under `field_name`, e.g. `"sites"`/`"devices"`)
/// and continuation URL (`pageDetails.nextPageUrl`, `null` on the last page,
/// see module docs) from a single Datto RMM list response. Pure function,
/// testable with hardcoded JSON, no real network access needed.
fn parse_page(
    json: &serde_json::Value,
    field_name: &str,
) -> Result<(Vec<serde_json::Value>, Option<String>), PluginError> {
    let data = json[field_name].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse(format!(
            "Erwartete '{field_name}'-Liste in Datto-RMM-Antwort"
        ))
    })?;
    let next_page_url = json["pageDetails"]["nextPageUrl"]
        .as_str()
        .map(str::to_string);
    Ok((data.clone(), next_page_url))
}

/// Walks every page of a Datto RMM list endpoint via `fetch_page`, merging
/// the `field_name` arrays into a single list. Deliberately generic over
/// `fetch_page` (rather than hardcoding a real HTTP call) so the pagination-
/// combining logic itself -- follow `nextPageUrl` exactly as given, stop
/// when it's `null`, never re-derive `page`/`max` -- is testable as a pure
/// function over pre-fetched, hardcoded JSON pages, without a real HTTP call
/// (see the `tests` module below). Capped at `MAX_PAGES` (see module docs).
fn fetch_all_pages<F>(
    start_url: String,
    field_name: &str,
    mut fetch_page: F,
) -> Result<Vec<serde_json::Value>, PluginError>
where
    F: FnMut(&str) -> Result<serde_json::Value, PluginError>,
{
    let mut all = Vec::new();
    let mut url = start_url;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_page(&url)?;
        let (data, next_url) = parse_page(&page_json, field_name)?;
        all.extend(data);
        match next_url {
            Some(next) => url = next,
            None => break,
        }
    }
    Ok(all)
}

fn fetch_all_sites_json(
    agent: &Agent,
    base_url: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let start_url = format!(
        "{}/api/v2/account/sites?page=0&max={PAGE_SIZE}",
        base_url.trim_end_matches('/')
    );
    fetch_all_pages(start_url, "sites", |url| fetch_json(agent, url, token))
}

fn fetch_all_devices_json(
    agent: &Agent,
    base_url: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let start_url = format!(
        "{}/api/v2/account/devices?page=0&max={PAGE_SIZE}",
        base_url.trim_end_matches('/')
    );
    fetch_all_pages(start_url, "devices", |url| fetch_json(agent, url, token))
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Datto-RMM-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Datto-RMM-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Maps already-merged (across all pages) raw site objects to `DattoRmmSite`
/// values. Pure function, testable with hardcoded JSON, analogous to
/// `plugin::tacticalrmm::map_clients_response`.
fn map_sites(raw: &[serde_json::Value]) -> Vec<DattoRmmSite> {
    raw.iter().filter_map(map_site).collect()
}

/// A single site object from Datto RMM's `/api/v2/account/sites` response
/// (`{"id": <number>, "uid": "...", "accountUid": "...", "name": "...",
/// "portalUrl": "..."}`, see module docs). If `uid` is missing or not a
/// string -> the site is skipped instead of failing the whole call,
/// analogous to `plugin::tacticalrmm::map_client`.
fn map_site(value: &serde_json::Value) -> Option<DattoRmmSite> {
    let uid = value["uid"].as_str()?.to_string();
    let name = value["name"].as_str().unwrap_or(uid.as_str()).to_string();
    let portal_url = value["portalUrl"].as_str().map(str::to_string);
    Some(DattoRmmSite {
        uid,
        name,
        portal_url,
    })
}

/// Maps already-merged (across all pages) raw device objects to
/// `DattoRmmDevice` values. Pure function, testable with hardcoded JSON,
/// analogous to `map_sites`.
fn map_devices(raw: &[serde_json::Value]) -> Vec<DattoRmmDevice> {
    raw.iter().filter_map(map_device).collect()
}

/// A single device object from Datto RMM's `/api/v2/account/devices`
/// response (see module docs for the verified field list). A device without
/// a usable `uid` OR without a usable `siteUid` is skipped -- without site
/// membership it can't be meaningfully grouped, and a single broken device
/// object shouldn't make the whole list unusable, analogous to
/// `plugin::tacticalrmm::map_agent`'s "skip without client_name" rule.
fn map_device(value: &serde_json::Value) -> Option<DattoRmmDevice> {
    let external_id = value["uid"].as_str()?.to_string();
    let site_uid = value["siteUid"].as_str()?.to_string();
    let site_name = value["siteName"].as_str().unwrap_or_default().to_string();
    let hostname = value["hostname"].as_str().map(str::to_string);
    let name = hostname.clone().unwrap_or_else(|| external_id.clone());
    let ip_address = extract_ip_address(value);
    let status = value["online"]
        .as_bool()
        .map(|online| if online { "online" } else { "offline" }.to_string());
    let platform = value["deviceClass"].as_str().map(str::to_string);
    let operating_system = value["operatingSystem"].as_str().map(str::to_string);
    let portal_url = value["portalUrl"].as_str().map(str::to_string);
    Some(DattoRmmDevice {
        external_id,
        name,
        hostname,
        ip_address,
        status,
        platform,
        operating_system,
        site_uid,
        site_name,
        portal_url,
    })
}

/// Extracts a display IP address for a device. `intIpAddress`/
/// `extIpAddress` are both plain strings (verified, see module docs) --
/// `intIpAddress` is preferred, `extIpAddress` is the fallback, the same
/// "primary + fallback" convention as
/// `plugin::tacticalrmm::extract_ip_address`/`plugin::ninja`. An empty
/// string is treated the same as an absent field.
fn extract_ip_address(value: &serde_json::Value) -> Option<String> {
    let non_empty = |field: &str| -> Option<String> {
        value[field]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    non_empty("intIpAddress").or_else(|| non_empty("extIpAddress"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_sites_json_array_into_dattormm_sites() {
        let raw = vec![
            serde_json::json!({
                "id": 1,
                "uid": "site-a-uid",
                "accountUid": "acct-uid",
                "name": "ACME Hauptsitz",
                "portalUrl": "https://merlot.centrastage.net/site/1"
            }),
            serde_json::json!({"id": 2, "uid": "site-b-uid", "name": "ACME Zweigstelle"}),
        ];

        let sites = map_sites(&raw);

        assert_eq!(sites.len(), 2);
        assert_eq!(
            sites[0],
            DattoRmmSite {
                uid: "site-a-uid".to_string(),
                name: "ACME Hauptsitz".to_string(),
                portal_url: Some("https://merlot.centrastage.net/site/1".to_string()),
            }
        );
        assert_eq!(sites[1].uid, "site-b-uid");
        assert_eq!(sites[1].name, "ACME Zweigstelle");
        assert_eq!(sites[1].portal_url, None);
    }

    #[test]
    fn site_falls_back_to_uid_when_no_name_field_present() {
        let raw = vec![serde_json::json!({"uid": "site-x"})];
        let sites = map_sites(&raw);
        assert_eq!(sites[0].name, "site-x");
    }

    #[test]
    fn site_skips_entries_without_a_usable_uid() {
        let raw = vec![serde_json::json!({"name": "OhneUid"})];
        let sites = map_sites(&raw);
        assert!(sites.is_empty());
    }

    #[test]
    fn maps_devices_json_array_with_site_status_platform_and_portal_url() {
        let raw = vec![
            serde_json::json!({
                "uid": "dev-1-uid",
                "id": 101,
                "hostname": "SRV-01",
                "intIpAddress": "10.0.0.5",
                "extIpAddress": "203.0.113.9",
                "online": true,
                "operatingSystem": "Windows Server 2022",
                "deviceClass": "device",
                "siteId": 1,
                "siteUid": "site-a-uid",
                "siteName": "ACME Hauptsitz",
                "portalUrl": "https://merlot.centrastage.net/device/101"
            }),
            serde_json::json!({
                "uid": "dev-2-uid",
                "online": false,
                "deviceClass": "printer",
                "siteUid": "site-b-uid",
                "siteName": "ACME Zweigstelle"
            }),
        ];

        let devices = map_devices(&raw);

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "dev-1-uid");
        assert_eq!(devices[0].name, "SRV-01");
        assert_eq!(devices[0].hostname.as_deref(), Some("SRV-01"));
        assert_eq!(devices[0].ip_address.as_deref(), Some("10.0.0.5"));
        assert_eq!(devices[0].status.as_deref(), Some("online"));
        assert_eq!(devices[0].platform.as_deref(), Some("device"));
        assert_eq!(
            devices[0].operating_system.as_deref(),
            Some("Windows Server 2022")
        );
        assert_eq!(devices[0].site_uid, "site-a-uid");
        assert_eq!(devices[0].site_name, "ACME Hauptsitz");
        assert_eq!(
            devices[0].portal_url.as_deref(),
            Some("https://merlot.centrastage.net/device/101")
        );

        assert_eq!(devices[1].external_id, "dev-2-uid");
        // No hostname present -> name falls back to the external ID, but
        // `hostname` itself stays `None` (see module docs on why this
        // distinction matters for the frontend's match heuristic).
        assert_eq!(devices[1].name, "dev-2-uid");
        assert_eq!(devices[1].hostname, None);
        assert_eq!(devices[1].status.as_deref(), Some("offline"));
        assert_eq!(devices[1].portal_url, None);
    }

    #[test]
    fn device_skips_entries_without_uid() {
        let raw = vec![serde_json::json!({"hostname": "OhneUid", "siteUid": "site-a"})];
        let devices = map_devices(&raw);
        assert!(devices.is_empty());
    }

    #[test]
    fn device_skips_entries_without_site_uid() {
        let raw = vec![serde_json::json!({"uid": "dev-1", "hostname": "OhneSite"})];
        let devices = map_devices(&raw);
        assert!(devices.is_empty());
    }

    #[test]
    fn device_status_is_none_when_online_field_missing() {
        let raw = vec![serde_json::json!({"uid": "dev-3", "siteUid": "site-a"})];
        let devices = map_devices(&raw);
        assert_eq!(devices[0].status, None);
    }

    #[test]
    fn extract_ip_address_prefers_int_ip_address() {
        let value = serde_json::json!({"intIpAddress": "10.0.0.5", "extIpAddress": "203.0.113.9"});
        assert_eq!(extract_ip_address(&value).as_deref(), Some("10.0.0.5"));
    }

    #[test]
    fn extract_ip_address_falls_back_to_ext_ip_address_when_int_missing() {
        let value = serde_json::json!({"extIpAddress": "203.0.113.9"});
        assert_eq!(extract_ip_address(&value).as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn extract_ip_address_treats_empty_string_as_absent() {
        let value = serde_json::json!({"intIpAddress": "", "extIpAddress": "203.0.113.9"});
        assert_eq!(extract_ip_address(&value).as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn extract_ip_address_is_none_when_neither_field_present() {
        let value = serde_json::json!({});
        assert_eq!(extract_ip_address(&value), None);
    }

    #[test]
    fn parse_page_extracts_array_and_next_page_url() {
        let json = serde_json::json!({
            "pageDetails": {"count": 2, "totalCount": 5, "nextPageUrl": "https://x/page2"},
            "sites": [{"uid": "a"}, {"uid": "b"}]
        });
        let (data, next) = parse_page(&json, "sites").unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(next.as_deref(), Some("https://x/page2"));
    }

    #[test]
    fn parse_page_returns_none_next_on_last_page() {
        let json = serde_json::json!({"pageDetails": {"nextPageUrl": null}, "devices": []});
        let (data, next) = parse_page(&json, "devices").unwrap();
        assert!(data.is_empty());
        assert_eq!(next, None);
    }

    #[test]
    fn parse_page_rejects_missing_field_array() {
        let json = serde_json::json!({"pageDetails": {}});
        let result = parse_page(&json, "sites");
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn fetch_all_pages_combines_multiple_pages_via_next_page_url() {
        let pages = [
            serde_json::json!({
                "items": [{"n": 1}, {"n": 2}],
                "pageDetails": {"nextPageUrl": "https://example/page2"}
            }),
            serde_json::json!({
                "items": [{"n": 3}],
                "pageDetails": {"nextPageUrl": null}
            }),
        ];
        let call_count = std::cell::Cell::new(0usize);

        let result = fetch_all_pages("https://example/page1".to_string(), "items", |url| {
            let idx = call_count.get();
            call_count.set(idx + 1);
            let expected_url = if idx == 0 {
                "https://example/page1"
            } else {
                "https://example/page2"
            };
            assert_eq!(url, expected_url);
            Ok(pages[idx].clone())
        })
        .unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(call_count.get(), 2);
    }

    #[test]
    fn fetch_all_pages_stops_after_max_pages_guard() {
        let call_count = std::cell::Cell::new(0usize);
        let result = fetch_all_pages("start".to_string(), "items", |_url| {
            call_count.set(call_count.get() + 1);
            Ok(serde_json::json!({
                "items": [{"n": call_count.get()}],
                "pageDetails": {"nextPageUrl": "next"}
            }))
        })
        .unwrap();

        assert_eq!(result.len(), MAX_PAGES);
        assert_eq!(call_count.get(), MAX_PAGES);
    }

    #[test]
    fn oauth_token_form_has_exact_grant_type_password_shape() {
        let form = oauth_token_form("APIKEY123", "SECRET456");
        assert_eq!(
            form,
            [
                ("grant_type", "password"),
                ("username", "APIKEY123"),
                ("password", "SECRET456"),
            ]
        );
    }

    #[test]
    fn oauth_basic_auth_header_uses_fixed_public_client_credentials() {
        let header = oauth_basic_auth_header();
        assert!(header.starts_with("Basic "));
        let encoded = header.trim_start_matches("Basic ");
        let decoded = BASE64_STANDARD.decode(encoded).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "public-client:public");
    }

    #[test]
    fn parses_credentials_from_json_secret() {
        let creds = PluginCredentials {
            secret: r#"{"api_key":"APIKEY123","api_secret_key":"SECRET456"}"#.into(),
        };
        let parsed = parse_credentials(&creds).unwrap();
        assert_eq!(parsed.api_key, "APIKEY123");
        assert_eq!(parsed.api_secret_key, "SECRET456");
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
        let plugin = DattoRmmPlugin::new(
            "dattormm:acme-123".to_string(),
            "https://merlot-api.centrastage.net".to_string(),
        );
        assert_eq!(plugin.id(), "dattormm:acme-123");
    }
}
