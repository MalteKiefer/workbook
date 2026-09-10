//! Real plugin implementation for Kaseya VSA (self-hosted/single-tenant RMM
//! -- each customer runs their own VSA server), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Kaseya-VSA-Plugin". Ninth
//! integration, structurally closest to Tactical RMM
//! (`plugin::tacticalrmm`): self-hosted with a user-supplied `base_url` (no
//! fixed path prefix assumed, see the base-URL note below), a single static
//! secret for authentication (like Level.io/Snipe-IT/Tactical RMM -- no
//! OAuth2 grant like NinjaOne), and mapping granularity at the top tenancy
//! level ("Organization") only, not any finer subdivision -- exactly the
//! same principle as Tactical RMM's Client-level mapping.
//!
//! IMPORTANT, read before trusting any fact below at face value: Kaseya's
//! public documentation for its current product ("VSA X", also still
//! sometimes called "VSA 10") is noticeably thinner and partly
//! self-contradictory compared to every other integration in this codebase.
//! Every point below is marked either CONFIRMED (found consistently in
//! Kaseya's own VSA X admin docs) or an explicit RULING/OPEN RISK where the
//! documentation itself disagrees or is silent -- nothing here is quietly
//! presented as more certain than it actually is.
//!
//! - **Authentication (CONFIRMED)**: HTTP Basic Auth, header
//!   `Authorization: Basic <base64("{token_id}:{token_secret}")>`. Verified
//!   against Kaseya's own VSA X admin documentation (Configuration -> API
//!   Access -> Third Party Tokens -> Create Token, which also offers
//!   optional IP allow-listing/expiration on the token itself -- neither of
//!   which this plugin needs to know about, they're configured entirely on
//!   the Kaseya side). Two secret values are needed (Token ID + Token
//!   Secret), so -- exactly like `plugin::ninja::NinjaCredentials` --
//!   `PluginCredentials.secret` holds a small JSON object
//!   (`KaseyaCredentials`), not a single opaque string like Level.io/
//!   Snipe-IT/Tactical RMM. Note for whoever builds the Pulseway
//!   integration in parallel: Pulseway's auth is documented as the exact
//!   same shape (`base64("{token_id}:{token_secret}")` Basic Auth) -- this
//!   module was NOT written depending on that plugin existing, it's just
//!   worth knowing the pattern repeats.
//! - **Base URL -- DOCUMENTED AMBIGUITY, ruling made below (NOT silently
//!   resolved)**: Kaseya's own current documentation disagrees with itself
//!   across two sources on what the API root actually is -- one page says
//!   `{server_domain}/api`, another says `{server_name}/api/v3/`. This could
//!   not be resolved from the fetched documentation alone (no way to tell
//!   which is current/correct without a real VSA X instance to probe).
//!   **Ruling**: exactly like `plugin::tacticalrmm`'s own "no fixed path
//!   prefix" handling, `KaseyaConnectionMeta.base_url` is a free-text field
//!   the user supplies as their VSA instance's COMPLETE, already-working API
//!   root URL -- this module never appends or assumes any path suffix
//!   (`/api`, `/api/v3`, or otherwise). Every request below is built as
//!   `{base_url}{path}` after trimming a trailing slash, nothing more. The
//!   frontend connection form carries a short German hint next to the field
//!   pointing this out explicitly (see `KaseyaPluginSection.tsx`). Whoever
//!   configures the first real connection against a live VSA X server should
//!   treat this as the number one thing to re-verify.
//! - **Tenancy entities (CONFIRMED existence, field names beyond Id/Name
//!   UNCONFIRMED)**: "Organizations" (`GET {base_url}/organizations`) and
//!   "Groups" (`GET {base_url}/groups`) both exist as documented VSA X API
//!   entities. This plugin maps at the ORGANIZATION level only (like
//!   Tactical RMM maps at the Client level, not the finer Site level) --
//!   `KaseyaOrgMapping` links an Organization to a local customer; Groups
//!   are never independently mapped or listed by this plugin at all, only
//!   referenced (as a bare, unresolved `GroupId`) on a device for
//!   informational display, see the Devices note below. Organization field
//!   names beyond `Id`/`Name` are NOT confirmed for VSA X specifically --
//!   `map_organization` therefore only ever reads `value["Id"]`/
//!   `value["Name"]`, exactly as tolerant/defensive as Tactical RMM's own
//!   `map_client`: an entry with no usable `Id` is skipped, not treated as a
//!   fatal error, and a missing `Name` falls back to the `Id` itself.
//! - **Devices (CONFIRMED fields; IP/online-status explicitly NOT
//!   confirmed, see below)**: `GET {base_url}/devices`, documented as
//!   filterable by `Identifier`, `Name`, `GroupId`, `SiteId`,
//!   `OrganizationId` (query filters this plugin doesn't currently use --
//!   it always fetches the full, paginated list). Confirmed fields on a
//!   device object: `Identifier` (a device GUID string -- used as
//!   `ExternalSystem::external_id`/`KaseyaDevice::external_id`), `Name`,
//!   `GroupId`, `OrganizationId` (a genuine numeric/string foreign key, NOT
//!   a name-join workaround like Tactical RMM's `client_name` -- devices are
//!   therefore joined to organizations by ID, the same way NinjaOne's
//!   `organizationId` works, not the way Tactical RMM's agents are joined),
//!   `IsAgentInstalled` (bool), `IsMdmEnrolled` (bool). `OrganizationId`
//!   itself is wrapped in `Option` on `KaseyaDevice` because -- per the
//!   field-name doubt below -- even ITS presence on every device object
//!   isn't something this integration blindly trusts; a device with no
//!   resolvable organization membership is still kept (never dropped), just
//!   surfaced by `commands::kaseya` under an explicit "Nicht zugeordnet"
//!   bucket rather than silently disappearing.
//!   **IP address and online/offline status fields are explicitly NOT
//!   confirmed for VSA X** and are deliberately absent from `KaseyaDevice`
//!   entirely -- not guessed at, not added as an always-`None` field dressed
//!   up as a real property. This mirrors Tactical RMM's own "honest
//!   omission" principle (see that module's docs on why it has no
//!   `tacticalrmm_url` field) and Snipe-IT's verified always-`None`
//!   `hostname`/`ip_address`. Richer, unconfirmed per-device data is left
//!   entirely to whatever `get_system_details`/`GET {base_url}/devices/{id}`
//!   returns as raw, unmodified JSON -- exactly like
//!   `plugin::tacticalrmm::get_system_details`.
//! - **A real, credible risk, stated explicitly rather than silently
//!   trusted**: the VSA X field names found during research (`Identifier`,
//!   `GroupId`, `OrganizationId`, `IsAgentInstalled`, `IsMdmEnrolled`) are
//!   suspiciously identical to another vendor's (Pulseway's) schema. This
//!   may genuinely be correct (a shared API tooling/white-label backend
//!   between the two products would not be unheard of in the RMM space), or
//!   it may be a research artifact (e.g. a documentation aggregator mixing
//!   up two products, or a stale copy-paste in whatever source was fetched).
//!   Both `map_organization` and `map_device` below are written
//!   DEFENSIVELY specifically because of this doubt: tolerant of missing or
//!   renamed fields, skipping a malformed entry rather than panicking or
//!   failing the whole call -- the same tolerant style as
//!   `plugin::tacticalrmm::map_client`/`map_agent` -- so that if the real
//!   field names turn out to differ, the result is a degraded-but-working
//!   sync (fewer/emptier fields populated) instead of a hard failure.
//!   Whoever configures the first real connection against a live VSA X
//!   server should treat re-verifying these exact field names as the SECOND
//!   thing to check, right after the base-URL ambiguity above.
//! - **Pagination (CONFIRMED mechanism, envelope shape only partly
//!   confirmed)**: OData-style query parameters (`$top`/`$skip`/`$filter`/
//!   `$orderby`/`$count`) are documented for VSA X's list endpoints,
//!   applied here uniformly to both `/organizations` and `/devices`.
//!   `NextQueryLink` is documented to appear in a list response once total
//!   results exceed 5000 (the same convention this codebase's Datto RMM/
//!   Pulseway plugins would use, if/when built -- not depended on here).
//!   The exact JSON key holding the item array itself, and the exact key
//!   holding a total-result count, are NOT named anywhere in the fetched
//!   documentation -- only `NextQueryLink` itself is a named, confirmed
//!   response field. `parse_page` therefore tries a small set of plausible
//!   envelope shapes defensively (a bare top-level array with no envelope
//!   at all, or an object with the items under one of `"Result"`/`"value"`/
//!   `"Items"`/`"data"`, with a total count under one of `"Count"`/
//!   `"TotalCount"`/`"@odata.count"`), rather than assuming one specific
//!   shape is THE correct one. `fetch_all_pages` implements a real
//!   pagination loop: while `NextQueryLink` is present in a page, it is
//!   followed verbatim as the next request URL; once it stops appearing,
//!   the loop still continues by incrementing `$skip` as long as a
//!   recognized total count says more items remain -- covering both
//!   documented pagination signals, exactly as specified. A hard iteration
//!   cap (`MAX_PAGES`) guards against an infinite loop if a future response
//!   shape confuses this parsing -- an implementation safety valve, not
//!   itself a documented API fact.
//! - **Single device detail (CONFIRMED)**: `GET {base_url}/devices/{id}`,
//!   returns one device object -- passed straight through as raw JSON by
//!   `get_system_details`, unmodified, exactly like
//!   `plugin::tacticalrmm::get_system_details`/`plugin::ninja`'s equivalent.
//! - **Rate limits (CONFIRMED, not enforced here)**: documented as roughly
//!   3600 requests/hour on the standard tier (some individual endpoints
//!   apparently have their own, lower limits, and repeated failed requests
//!   are documented to trip a separate, stricter lockout tier). No retry/
//!   backoff logic is implemented for this -- matches every other plugin in
//!   this codebase, none of which implement rate-limit handling either; this
//!   app calls plugin methods rarely/manually, not in a hot loop.
//! - **Web dashboard deep-link -- deliberately absent**: no reachable VSA X
//!   documentation describes a stable, base-URL-derivable web dashboard
//!   link for a single device -- so, exactly like Tactical RMM's missing
//!   `tacticalrmm_url`, there is no `kaseya_url` field/DTO property here.
//!   An honest omission, not a guess dressed up as a feature.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the same dependency as
//!   every other real plugin in this codebase.
//! - **Credential encoding**: two secret values (Token ID + Token Secret),
//!   JSON-encoded into `PluginCredentials.secret` and parsed back out here
//!   -- see `KaseyaCredentials`/`parse_credentials`, structurally identical
//!   to `plugin::ninja::NinjaCredentials`/`parse_credentials`.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Non-secret metadata of a Kaseya VSA connection, as stored in
/// `config.toml` (`Config::kaseya_connections`). The Token ID/Secret pair
/// belongs, per the credential principle, exclusively in the OS keyring,
/// never here. Deliberately WITHOUT `customer_id` -- a connection is one
/// VSA server instance, not a local customer; which VSA "Organization"
/// within this instance corresponds to which local customer is tracked
/// granularly in `KaseyaOrgMapping`/`Config::kaseya_org_mappings`, exactly
/// the same principle as `plugin::tacticalrmm::TacticalRmmConnectionMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaseyaConnectionMeta {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

/// Mapping of a single Kaseya VSA "Organization" (within a connection) to a
/// local customer. Lives in `Config::kaseya_org_mappings`, not in
/// `KaseyaConnectionMeta` -- a connection can see multiple organizations,
/// each of which can be mapped independently (or left unmapped).
/// `organization_name` is stored alongside `organization_id` so a UI list
/// can display a readable name without another live call against Kaseya --
/// analogous to `plugin::tacticalrmm::TacticalRmmClientMapping`/
/// `plugin::ninja::NinjaOrgMapping`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaseyaOrgMapping {
    pub connection_id: String,
    pub organization_id: String,
    pub organization_name: String,
    pub customer_id: i64,
}

/// An organization reported by `GET /organizations`. Separate from
/// `ExternalSystem` (those are devices) -- its own, small shape type,
/// analogous to `plugin::tacticalrmm::TacticalRmmClient`/
/// `plugin::ninja::NinjaOrganization`. Only `id`/`name` are read (see module
/// docs on why nothing else is trusted here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaseyaOrganization {
    pub id: String,
    pub name: String,
}

/// A single device from `GET /devices`, carrying only the fields this
/// module treats as confirmed (see module docs) -- deliberately NOT
/// carrying an IP address or online/offline status field, unlike e.g.
/// `plugin::ninja::NinjaDevice`/`plugin::tacticalrmm::TacticalRmmAgent`.
/// `organization_id`/`organization_name`/`group_id` are all `Option`, not
/// because Kaseya's schema is known to omit them sometimes, but because
/// this integration doesn't fully trust their guaranteed presence on every
/// device object (see the field-name-similarity doubt in the module docs) --
/// a device missing them is still kept, never dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaseyaDevice {
    pub external_id: String,
    pub name: String,
    pub organization_id: Option<String>,
    pub organization_name: Option<String>,
    pub group_id: Option<String>,
    pub is_agent_installed: bool,
    pub is_mdm_enrolled: bool,
}

/// The two secret values a Kaseya VSA connection needs to authenticate.
/// `PluginCredentials.secret` is, per the trait contract, a single opaque
/// string the plugin interprets itself -- here encoded as JSON
/// (`serde_json::to_string`/`from_str`), structurally identical to
/// `plugin::ninja::NinjaCredentials`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaseyaCredentials {
    pub token_id: String,
    pub token_secret: String,
}

impl KaseyaCredentials {
    /// Builds the `Authorization: Basic <base64(...)>` header VALUE (without
    /// the leading `"Authorization: "` -- just what goes on the right-hand
    /// side), per the confirmed auth scheme documented in the module docs.
    pub fn basic_auth_header(&self) -> String {
        use base64::prelude::*;
        let raw = format!("{}:{}", self.token_id, self.token_secret);
        format!("Basic {}", BASE64_STANDARD.encode(raw))
    }
}

/// A plugin object for exactly one configured Kaseya VSA connection. `id`
/// here is already the fully qualified identifier
/// (`"kaseya:<connection_id>"`), so that `Plugin::id()` works unmodified as
/// the `plugin_id`/keyring account (see trait documentation in
/// `plugin::mod`).
pub struct KaseyaPlugin {
    id: String,
    base_url: String,
}

impl KaseyaPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live fetch of this connection's organization list, paginated (see
    /// module docs). Separate from the `Plugin` trait method `list_systems`,
    /// because organizations are not devices and the generic trait has no
    /// room for that -- analogous to
    /// `plugin::tacticalrmm::TacticalRmmPlugin::list_clients`.
    pub fn list_organizations(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<KaseyaOrganization>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let auth_header = creds.basic_auth_header();
        let items = fetch_all_pages(&agent, &self.base_url, "/organizations", &auth_header)?;
        Ok(map_organizations(&items))
    }

    /// Live fetch of all this connection's devices, paginated (see module
    /// docs). Richer than the trait method `list_systems`, which
    /// deliberately stays with the narrow, plugin-agnostic `ExternalSystem`
    /// type.
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<KaseyaDevice>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let auth_header = creds.basic_auth_header();
        let items = fetch_all_pages(&agent, &self.base_url, "/devices", &auth_header)?;
        Ok(map_devices(&items))
    }
}

/// Checks a Token ID/Secret pair against Kaseya VSA, without persisting
/// anything -- a single, lightweight real call (`GET /organizations?$top=1`,
/// deliberately requesting just one page of one item, not the full
/// paginated list) so a typo in the base URL/token is noticed before this
/// app ever writes credentials to the keyring. For
/// `commands::kaseya::test_kaseya_connection`, analogous to
/// `plugin::tacticalrmm::test_credentials`.
pub fn test_credentials(
    base_url: &str,
    token_id: &str,
    token_secret: &str,
) -> Result<(), PluginError> {
    let creds = KaseyaCredentials {
        token_id: token_id.to_string(),
        token_secret: token_secret.to_string(),
    };
    let agent = build_agent();
    let url = format!("{}/organizations?$top=1", base_url.trim_end_matches('/'));
    fetch_json(&agent, &url, &creds.basic_auth_header())?;
    Ok(())
}

impl Plugin for KaseyaPlugin {
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
                // No confirmed hostname field on a Kaseya VSA device (see
                // module docs) -- always `None`, analogous to
                // `plugin::snipeit`'s verified always-`None` `hostname`.
                hostname: None,
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
        let url = format!(
            "{}/devices/{external_id}",
            self.base_url.trim_end_matches('/')
        );
        fetch_json(&agent, &url, &creds.basic_auth_header())
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Kaseya VSA's API doesn't need to know anything about a local link
        // -- purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("KaseyaPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn parse_credentials(credentials: &PluginCredentials) -> Result<KaseyaCredentials, PluginError> {
    serde_json::from_str(&credentials.secret)
        .map_err(|e| PluginError::Authentication(format!("Kaseya-Zugangsdaten ungültig: {e}")))
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

/// Issues an authenticated `GET {url}` and returns the parsed JSON body.
/// Auth header is `Authorization: Basic ...` (see module docs).
fn fetch_json(
    agent: &Agent,
    url: &str,
    auth_header: &str,
) -> Result<serde_json::Value, PluginError> {
    let mut response = agent
        .get(url)
        .header("Authorization", auth_header)
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// One page of a paginated VSA X list response, already normalized to a flat
/// item array plus an optional `NextQueryLink`/total-count -- see
/// `parse_page` and the module docs on why several envelope shapes are
/// tried defensively instead of assuming one is correct.
struct KaseyaPage {
    items: Vec<serde_json::Value>,
    next_query_link: Option<String>,
    total_count: Option<u64>,
}

/// Plausible JSON keys that might hold the item array in a VSA X list
/// response -- NONE of these are confirmed by the fetched documentation
/// (only `NextQueryLink` itself is a named, confirmed field, see module
/// docs); tried in order, first match wins.
const ITEMS_KEYS: [&str; 4] = ["Result", "value", "Items", "data"];

/// Plausible JSON keys that might hold a total result count -- same caveat
/// as `ITEMS_KEYS`.
const COUNT_KEYS: [&str; 3] = ["Count", "TotalCount", "@odata.count"];

/// Normalizes one raw JSON response body from a VSA X list endpoint into a
/// `KaseyaPage`. Pure function, no network access -- testable directly with
/// hardcoded JSON (see tests below), independent of the actual HTTP
/// pagination loop in `fetch_all_pages`. Handles three shapes: a bare
/// top-level JSON array (no envelope, no pagination info -- the whole
/// dataset in the array itself), a JSON object with one of `ITEMS_KEYS`
/// holding the array (plus optionally `NextQueryLink`/one of `COUNT_KEYS`),
/// or any other/unrecognized shape (treated as an empty page rather than an
/// error -- consistent with this module's overall "skip/degrade rather than
/// panic" principle given the field-name doubt documented above).
fn parse_page(json: &serde_json::Value) -> KaseyaPage {
    if let Some(array) = json.as_array() {
        return KaseyaPage {
            items: array.clone(),
            next_query_link: None,
            total_count: None,
        };
    }
    let items = ITEMS_KEYS
        .iter()
        .find_map(|key| json.get(*key).and_then(|v| v.as_array()))
        .cloned()
        .unwrap_or_default();
    let next_query_link = json
        .get("NextQueryLink")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let total_count = COUNT_KEYS
        .iter()
        .find_map(|key| json.get(*key).and_then(|v| v.as_u64()));
    KaseyaPage {
        items,
        next_query_link,
        total_count,
    }
}

/// Requested page size for the initial `$top`/`$skip`-driven request of a
/// paginated list -- a reasonable, safely-under-5000 implementation choice
/// (see module docs on the documented 5000-item `NextQueryLink` threshold),
/// NOT itself a documented/confirmed API constant.
const PAGE_SIZE: u32 = 1000;

/// Hard cap on pagination iterations, purely an implementation safety valve
/// against an infinite loop if a future/unexpected response shape confuses
/// `parse_page` -- not a documented API fact.
const MAX_PAGES: u32 = 200;

/// Fetches every page of a paginated VSA X list endpoint (`/organizations`
/// or `/devices`) and returns the combined, flat item array. Implements the
/// real pagination loop described in the module docs: while a page carries
/// `NextQueryLink`, that URL is followed verbatim for the next request;
/// once it stops appearing, the loop still continues by incrementing
/// `$skip` as long as a recognized total count says more items remain.
fn fetch_all_pages(
    agent: &Agent,
    base_url: &str,
    path: &str,
    auth_header: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let trimmed = base_url.trim_end_matches('/');
    let mut items = Vec::new();
    let mut next_url = Some(format!(
        "{trimmed}{path}?$top={PAGE_SIZE}&$skip=0&$count=true"
    ));

    for _ in 0..MAX_PAGES {
        let Some(url) = next_url.take() else {
            break;
        };
        let json = fetch_json(agent, &url, auth_header)?;
        let page = parse_page(&json);
        let received = page.items.len();
        items.extend(page.items);

        next_url = if let Some(link) = page.next_query_link {
            Some(link)
        } else if received > 0 {
            match page.total_count {
                Some(total) if (items.len() as u64) < total => Some(format!(
                    "{trimmed}{path}?$top={PAGE_SIZE}&$skip={}&$count=true",
                    items.len()
                )),
                _ => None,
            }
        } else {
            None
        };
    }

    Ok(items)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Kaseya-VSA-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Kaseya-VSA-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// A single, possibly Number-or-String-typed ID field, tolerant of both
/// shapes -- the same flexible-ID convention already used by
/// `plugin::tacticalrmm::map_client`/`plugin::ninja::map_organization`.
fn flexible_id(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Maps the already-paginated, flattened item list from `/organizations`
/// into `KaseyaOrganization` values. Pure function, testable with hardcoded
/// JSON, no network access. Only reads `Id`/`Name` (see module docs on why
/// nothing else is trusted here) -- an entry without a usable `Id` is
/// skipped rather than treated as a fatal error, analogous to
/// `plugin::tacticalrmm::map_client`.
fn map_organizations(items: &[serde_json::Value]) -> Vec<KaseyaOrganization> {
    items.iter().filter_map(map_organization).collect()
}

fn map_organization(value: &serde_json::Value) -> Option<KaseyaOrganization> {
    let id = flexible_id(&value["Id"])?;
    let name = value["Name"].as_str().unwrap_or(id.as_str()).to_string();
    Some(KaseyaOrganization { id, name })
}

/// Maps the already-paginated, flattened item list from `/devices` into
/// `KaseyaDevice` values. Pure function, testable with hardcoded JSON, no
/// network access, analogous to `map_organizations`. An entry without a
/// usable `Identifier` is skipped -- without it there's no `external_id` to
/// track the device by at all, mirroring
/// `plugin::tacticalrmm::map_agent`'s "skip without `agent_id`" rule.
/// Everything else (organization membership, group, install/MDM flags) is
/// read defensively and simply absent/`false` when missing, per the
/// field-name doubt documented above -- a malformed or renamed field never
/// drops the whole device.
fn map_devices(items: &[serde_json::Value]) -> Vec<KaseyaDevice> {
    items.iter().filter_map(map_device).collect()
}

fn map_device(value: &serde_json::Value) -> Option<KaseyaDevice> {
    let external_id = value["Identifier"].as_str()?.to_string();
    let name = value["Name"]
        .as_str()
        .unwrap_or(external_id.as_str())
        .to_string();
    let organization_id = flexible_id(&value["OrganizationId"]);
    let organization_name = value["OrganizationName"].as_str().map(str::to_string);
    let group_id = flexible_id(&value["GroupId"]);
    let is_agent_installed = value["IsAgentInstalled"].as_bool().unwrap_or(false);
    let is_mdm_enrolled = value["IsMdmEnrolled"].as_bool().unwrap_or(false);
    Some(KaseyaDevice {
        external_id,
        name,
        organization_id,
        organization_name,
        group_id,
        is_agent_installed,
        is_mdm_enrolled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_page_reads_a_bare_top_level_array_with_no_envelope() {
        let json = serde_json::json!([{"Id": 1}, {"Id": 2}]);
        let page = parse_page(&json);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next_query_link, None);
        assert_eq!(page.total_count, None);
    }

    #[test]
    fn parse_page_reads_result_envelope_with_next_query_link() {
        let json = serde_json::json!({
            "Result": [{"Id": 1}],
            "NextQueryLink": "https://vsa.example.com/api/devices?$skip=1000",
        });
        let page = parse_page(&json);
        assert_eq!(page.items.len(), 1);
        assert_eq!(
            page.next_query_link.as_deref(),
            Some("https://vsa.example.com/api/devices?$skip=1000")
        );
    }

    #[test]
    fn parse_page_reads_value_envelope_with_a_total_count() {
        let json = serde_json::json!({
            "value": [{"Id": 1}, {"Id": 2}],
            "Count": 2,
        });
        let page = parse_page(&json);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next_query_link, None);
        assert_eq!(page.total_count, Some(2));
    }

    #[test]
    fn parse_page_treats_unrecognized_object_shape_as_an_empty_page() {
        let json = serde_json::json!({"something": "unexpected"});
        let page = parse_page(&json);
        assert!(page.items.is_empty());
        assert_eq!(page.next_query_link, None);
        assert_eq!(page.total_count, None);
    }

    #[test]
    fn maps_organizations_json_array_into_kaseya_organizations() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {"Id": 1, "Name": "ACME GmbH"},
                {"Id": "2", "Name": "Contoso AG"}
            ]"#,
        )
        .unwrap();
        let items = json.as_array().unwrap();

        let organizations = map_organizations(items);

        assert_eq!(organizations.len(), 2);
        assert_eq!(
            organizations[0],
            KaseyaOrganization {
                id: "1".to_string(),
                name: "ACME GmbH".to_string()
            }
        );
        assert_eq!(organizations[1].id, "2");
        assert_eq!(organizations[1].name, "Contoso AG");
    }

    #[test]
    fn organization_falls_back_to_id_when_no_name_field_present() {
        let items = vec![serde_json::json!({"Id": 9})];
        let organizations = map_organizations(&items);
        assert_eq!(organizations[0].name, "9");
    }

    #[test]
    fn organization_skips_entries_without_a_usable_id() {
        let items = vec![serde_json::json!({"Name": "OhneId"})];
        let organizations = map_organizations(&items);
        assert!(organizations.is_empty());
    }

    #[test]
    fn maps_devices_json_array_with_organization_group_and_flags() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {
                    "Identifier": "abc-123",
                    "Name": "SRV-01",
                    "OrganizationId": 1,
                    "GroupId": "grp-5",
                    "IsAgentInstalled": true,
                    "IsMdmEnrolled": false
                },
                {
                    "Identifier": "def-456",
                    "Name": "WS-07",
                    "OrganizationId": "2",
                    "IsAgentInstalled": false,
                    "IsMdmEnrolled": true
                }
            ]"#,
        )
        .unwrap();
        let items = json.as_array().unwrap();

        let devices = map_devices(items);

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "abc-123");
        assert_eq!(devices[0].name, "SRV-01");
        assert_eq!(devices[0].organization_id.as_deref(), Some("1"));
        assert_eq!(devices[0].group_id.as_deref(), Some("grp-5"));
        assert!(devices[0].is_agent_installed);
        assert!(!devices[0].is_mdm_enrolled);
        assert_eq!(devices[1].organization_id.as_deref(), Some("2"));
        assert_eq!(devices[1].group_id, None);
        assert!(!devices[1].is_agent_installed);
        assert!(devices[1].is_mdm_enrolled);
    }

    #[test]
    fn device_skips_entries_without_identifier() {
        let items = vec![serde_json::json!({"Name": "OhneId"})];
        let devices = map_devices(&items);
        assert!(devices.is_empty());
    }

    #[test]
    fn device_falls_back_to_identifier_when_no_name_present() {
        let items = vec![serde_json::json!({"Identifier": "abc-123"})];
        let devices = map_devices(&items);
        assert_eq!(devices[0].name, "abc-123");
    }

    #[test]
    fn device_has_no_organization_when_field_is_entirely_absent() {
        let items = vec![serde_json::json!({"Identifier": "abc-123", "Name": "SRV"})];
        let devices = map_devices(&items);
        assert_eq!(devices[0].organization_id, None);
        assert_eq!(devices[0].organization_name, None);
        assert_eq!(devices[0].group_id, None);
    }

    #[test]
    fn device_flags_default_to_false_when_missing() {
        let items = vec![serde_json::json!({"Identifier": "abc-123", "Name": "SRV"})];
        let devices = map_devices(&items);
        assert!(!devices[0].is_agent_installed);
        assert!(!devices[0].is_mdm_enrolled);
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
    fn basic_auth_header_base64_encodes_token_id_and_secret() {
        let creds = KaseyaCredentials {
            token_id: "abc".to_string(),
            token_secret: "xyz".to_string(),
        };
        // base64("abc:xyz") == "YWJjOnh5eg=="
        assert_eq!(creds.basic_auth_header(), "Basic YWJjOnh5eg==");
    }

    #[test]
    fn parses_credentials_from_json_secret() {
        let creds = PluginCredentials {
            secret: r#"{"token_id":"abc","token_secret":"xyz"}"#.into(),
        };
        let parsed = parse_credentials(&creds).unwrap();
        assert_eq!(parsed.token_id, "abc");
        assert_eq!(parsed.token_secret, "xyz");
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
    fn plugin_id_returns_configured_connection_id() {
        let plugin = KaseyaPlugin::new(
            "kaseya:acme-123".to_string(),
            "https://vsa.example.com/api".to_string(),
        );
        assert_eq!(plugin.id(), "kaseya:acme-123");
    }
}
