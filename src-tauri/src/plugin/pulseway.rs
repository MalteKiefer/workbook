//! Real plugin implementation for Pulseway (cloud-hosted RMM,
//! <https://www.pulseway.com/>, also offered as a self-hosted "Enterprise
//! Server" variant under the same API shape), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Pulseway-Plugin". Ninth real
//! integration after NinjaOne (`plugin::ninja`), Level.io (`plugin::level`),
//! Snipe-IT (`plugin::snipeit`), Microsoft Intune (`plugin::intune`), Iru
//! (`plugin::iru`), Jamf Pro (`plugin::jamf`), Apple Business Manager
//! (`plugin::abm`) and Tactical RMM (`plugin::tacticalrmm`). Structurally
//! closest to Tactical RMM: self-hosted-capable (a user-supplied
//! `base_url`, defaulting to the fixed cloud host in the frontend form, but
//! always editable, since Pulseway also documents a self-hosted "Enterprise
//! Server" variant at `https://<your-server>/api/v3/`) with a genuine
//! multi-tenant hierarchy within a single connection (Pulseway's
//! "Organization" -> "Site" -> "Group" -> "Device", four levels deep, one
//! more than Tactical RMM's "Client" -> "Site" -> "Agent" or NinjaOne's
//! "Organization" -> "Device") -- hence the same connection+mapping-table
//! pattern as Tactical RMM/NinjaOne (`PulsewayConnectionMeta`/
//! `PulsewayOrgMapping`, mapping granularity at the Organization level ONLY
//! -- deliberately no second mapping layer for Site/Group, mirroring how
//! Tactical RMM stops at Client and NinjaOne stops at Organization).
//! Authentication, however, needs TWO secret values (a Token ID and a Token
//! Secret) like NinjaOne's client ID/secret pair, unlike Tactical RMM's
//! single API key -- see `PulsewayCredentials` below.
//!
//! Facts below are taken from Pulseway's own official API reference at
//! <https://api.pulseway.com>, EXCEPT the exact web-UI path for generating a
//! token, which is sourced from a third-party integration guide, not
//! Pulseway's own primary documentation -- flagged explicitly below as
//! "plausible, not fully confirmed", the same honesty convention
//! `plugin::tacticalrmm` uses for its own unconfirmed facts (see that
//! module's docs on why there's no verified web dashboard link):
//!
//! - **Authentication**: HTTP Basic Auth --
//!   `Authorization: Basic <base64("{token_id}:{token_secret}")>`. The user
//!   generates a Token ID + Token Secret pair in Pulseway's own web UI
//!   (PLAUSIBLE, NOT FULLY CONFIRMED: "Configuration -> API Access -> Third
//!   Party Tokens -> Create Token", per a third-party integration guide, not
//!   Pulseway's own primary docs). `basic_auth_header` builds the header
//!   value from a `PulsewayCredentials` value using the `base64` crate
//!   (already a dependency, 0.23.1, same crate `plugin::abm` already uses
//!   for its own encoding needs).
//! - **Base URL**: the fixed cloud host `https://api.pulseway.com/v3` is the
//!   default (see the frontend form's placeholder), but Pulseway also
//!   documents a self-hosted "Enterprise Server" variant under the same API
//!   shape at `https://<your-server>/api/v3`. Like Tactical RMM/NinjaOne/
//!   Snipe-IT (and unlike Level.io's fixed `BASE_URL` constant), a
//!   connection therefore needs a user-supplied, always-editable
//!   `PulsewayConnectionMeta.base_url` -- the safer, more flexible choice
//!   given Pulseway explicitly supports self-hosting.
//! - **Tenancy hierarchy**: FOUR levels -- Organization -> Site -> Group ->
//!   Device (one level deeper than Tactical RMM's Client -> Site -> Agent).
//!   Mapping stays at the Organization level ONLY, exactly like Tactical RMM
//!   maps at Client (not Site) and NinjaOne maps at Organization (not some
//!   finer subdivision) -- no second mapping layer for Sites/Groups. A
//!   device's Site/Group are still surfaced read-only
//!   (`PulsewayDevice.site_name`/`group_name`) for display, same principle
//!   as `TacticalRmmAgent.site_name`.
//! - **Organizations**: `GET /organizations` returns an envelope --
//!   `{"Data": [...], "Meta": {"ResponseCode": 200, "TotalCount": <n>}}` --
//!   PascalCase envelope keys AND PascalCase item fields (`Id`, `Name`,
//!   `Type`). NOT Tactical RMM's bare, unpaginated array, NOT Snipe-IT's
//!   `{"total", "rows"}` shape -- its own, distinct envelope requiring an
//!   explicit `Data` unwrap (see `map_organizations_pages`).
//! - **Devices**: `GET /devices`, same `Data`/`Meta` envelope. Verified list
//!   item fields: `Identifier` (a string GUID -- used as `external_id`,
//!   analogous to Tactical RMM's `agent_id`), `Name` (the device's hostname
//!   -- Pulseway's own API reference documents this field AS the hostname,
//!   there is no separate `hostname` key), `OrganizationId` (a genuine
//!   NUMERIC foreign key -- a real, verified difference from Tactical RMM,
//!   whose agent list carries no client ID at all and needs a NAME-based
//!   join instead, see `plugin::tacticalrmm` module docs; here
//!   `commands::pulseway::group_devices_by_organization` can join by ID,
//!   exactly like `commands::plugins::group_devices_by_organization` does
//!   for NinjaOne), `OrganizationName`, `SiteName`, `GroupName`,
//!   `IsAgentInstalled` (bool). **Verified gap, deliberate**: the list
//!   endpoint has NO IP address, NO online/offline status, and NO
//!   platform/OS field -- those three only exist on the single-device detail
//!   endpoint (see below). `PulsewayDevice` therefore has NO `ip_address`/
//!   `status`/`platform` fields -- not silently-always-`None` fields that
//!   would misleadingly imply an attempt was made to fetch them, but
//!   genuinely absent from the type, exactly the honesty principle
//!   `plugin::snipeit`'s always-`None` `hostname`/`ip_address` fields
//!   document (and unlike those, this integration doesn't even define the
//!   fields at all, since there's no legacy shape to stay compatible with).
//! - **Single device detail**: `GET /devices/{id}` (`id` = the `Identifier`
//!   GUID from the list) carries the RICH fields the list endpoint lacks --
//!   `IsOnline` (bool), `ComputerType` (string, e.g. `"windows"`),
//!   `ExternalIpAddress` (string), `LocalIpAddresses` (an array of
//!   network-adapter objects, each with `IpV4`/`IpV6`). Used EXCLUSIVELY by
//!   `Plugin::get_system_details` (one call, raw JSON passed straight
//!   through unmodified, exactly like `plugin::tacticalrmm::
//!   get_system_details`) -- deliberately NEVER called per-device during
//!   `list_systems`/a sync run, which would be an N+1 call pattern against a
//!   documented ~3600-requests/hour rate limit (see below). The list/sync
//!   path only ever uses the light `/devices` fields; status/IP/platform
//!   only ever surface when a user views one system's own details.
//! - **Pagination**: OData-style `$top`/`$skip` query parameters, `$count=true`
//!   to get `Meta.TotalCount`. `Meta.NextQueryLink` (a full next-page URL) is
//!   only returned once total results exceed 5000 -- for typical MSP fleet
//!   sizes under that threshold a single large-`$top` request would already
//!   be enough in practice, but `fetch_all_pages` implements a REAL
//!   pagination loop regardless (follow `NextQueryLink` while present, else
//!   increment `$skip` by the page's own item count while the collected
//!   count is still below `Meta.TotalCount`), so a larger deployment isn't
//!   silently truncated -- the same exhaustive-fetch discipline
//!   `plugin::tacticalrmm` applies to its own (unpaginated) endpoints,
//!   adapted here to Pulseway's actually-paginated shape.
//! - **Rate limits**: documented at roughly 3600 requests/hour per token per
//!   endpoint (varies by endpoint); exceeding it returns HTTP 429 with a
//!   `Retry-After` header. No special retry/backoff logic is implemented for
//!   this integration -- matches every existing plugin in this codebase,
//!   none of which implement retry/backoff either; a 429 simply falls into
//!   `map_ureq_error`'s generic "other status -> Unreachable" branch like
//!   any other non-2xx response.
//! - **No web dashboard deep-link**: a real `ExternalUrl` field exists in
//!   Pulseway's API, but only on the separate `/assets`/`/assets/{id}`
//!   resource, NOT on `/devices` -- using it would require an extra
//!   per-device call this plugin doesn't otherwise need (see the N+1/rate
//!   limit note above). An honest omission, analogous to
//!   `plugin::tacticalrmm`'s missing `tacticalrmm_url` (see that module's
//!   docs) -- no `pulseway_url` field/DTO property here either.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the same dependency as
//!   every other real plugin in this codebase.
//! - **Credential encoding**: Pulseway needs TWO secret values (Token ID +
//!   Token Secret), so `PluginCredentials.secret` holds a small JSON object
//!   (`{"token_id":"...","token_secret":"..."}`), parsed with
//!   `serde_json::from_str` inside this module -- exactly
//!   `plugin::ninja::NinjaCredentials`'s pattern (`client_id`/
//!   `client_secret`), NOT Tactical RMM's single-string passthrough.

use base64::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Non-secret metadata of a Pulseway connection, as stored in `config.toml`
/// (`Config::pulseway_connections`). The Token ID/Secret pair belongs, per
/// the credential principle, exclusively in the OS keyring, never here.
/// Deliberately WITHOUT `customer_id` -- a connection is a Pulseway account
/// (cloud tenant or self-hosted Enterprise Server), not a local customer;
/// which Pulseway "Organization" within it corresponds to which local
/// customer is tracked granularly in `PulsewayOrgMapping`/
/// `Config::pulseway_org_mappings` -- exactly the same principle as
/// `plugin::tacticalrmm::TacticalRmmConnectionMeta`/
/// `plugin::ninja::NinjaConnectionMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PulsewayConnectionMeta {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

/// Mapping of a single Pulseway "Organization" (within a connection) to a
/// local customer. Lives in `Config::pulseway_org_mappings`, not in
/// `PulsewayConnectionMeta` -- a connection can see multiple organizations,
/// each mapped independently (or left unmapped). `organization_id` is the
/// real, numeric Pulseway organization ID (as a string, from
/// `GET /organizations`); `organization_name` is stored alongside it so a UI
/// list can display a readable name without another live call against
/// Pulseway -- analogous to `plugin::tacticalrmm::TacticalRmmClientMapping`/
/// `plugin::ninja::NinjaOrgMapping`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PulsewayOrgMapping {
    pub connection_id: String,
    pub organization_id: String,
    pub organization_name: String,
    pub customer_id: i64,
}

/// An organization reported by `GET /organizations` (unwrapped from the
/// `Data`/`Meta` envelope, see module docs). Separate from `ExternalSystem`
/// (those are devices) -- its own, small shape type, analogous to
/// `plugin::tacticalrmm::TacticalRmmClient`/`plugin::ninja::NinjaOrganization`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PulsewayOrganization {
    pub id: String,
    pub name: String,
}

/// A single device from `GET /devices` (unwrapped from the `Data`/`Meta`
/// envelope), enriched with the verified extra fields
/// `plugin::mod::ExternalSystem` deliberately does not provide
/// (`organization_id`/`organization_name` for grouping and display,
/// `site_name`/`group_name` for informational display -- mapping granularity
/// stops at the Organization level, see module docs -- and
/// `is_agent_installed`). Deliberately, verifiably WITHOUT `ip_address`/
/// `status`/`platform` -- Pulseway's `/devices` list response genuinely does
/// not carry those fields, see module docs on the verified gap; they only
/// exist on the single-device detail endpoint used by
/// `Plugin::get_system_details`, never here. Needed by
/// `commands::pulseway::sync_pulseway_connection` for grouping by
/// organization and for display, analogous to
/// `plugin::tacticalrmm::TacticalRmmAgent`/`plugin::ninja::NinjaDevice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PulsewayDevice {
    pub external_id: String,
    pub name: String,
    pub organization_id: String,
    pub organization_name: String,
    pub site_name: Option<String>,
    pub group_name: Option<String>,
    pub is_agent_installed: bool,
}

/// The two secret values a Pulseway connection needs to authenticate.
/// `PluginCredentials.secret` is, per the trait contract, a single opaque
/// string that the plugin interprets itself -- here encoded as JSON
/// (`serde_json::to_string`/`from_str`), exactly
/// `plugin::ninja::NinjaCredentials`'s pattern (see module docs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulsewayCredentials {
    pub token_id: String,
    pub token_secret: String,
}

/// A plugin object for exactly one configured Pulseway connection. `id` here
/// is already the fully qualified identifier (`"pulseway:<connection_id>"`),
/// so that `Plugin::id()` works unmodified as the `plugin_id`/keyring
/// account (see trait documentation in `plugin::mod`).
pub struct PulsewayPlugin {
    id: String,
    base_url: String,
}

impl PulsewayPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live, EXHAUSTIVE fetch of this connection's organization list
    /// (`GET /organizations`, following pagination in full -- see
    /// `fetch_all_pages`). Separate from the `Plugin` trait method
    /// `list_systems`, because organizations are not devices and the generic
    /// trait has no room for that -- analogous to
    /// `plugin::tacticalrmm::TacticalRmmPlugin::list_clients`/
    /// `plugin::ninja::NinjaPlugin::list_organizations`.
    pub fn list_organizations(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<PulsewayOrganization>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let pages = fetch_all_pages(&agent, &self.base_url, "/organizations", &creds)?;
        map_organizations_pages(&pages)
    }

    /// Live, EXHAUSTIVE fetch of all this connection's devices
    /// (`GET /devices`, following pagination in full). Richer than the trait
    /// method `list_systems`, which deliberately stays with the narrow,
    /// plugin-agnostic `ExternalSystem` type (no `organization_id`/
    /// `site_name`/`group_name`/`is_agent_installed` field there).
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<PulsewayDevice>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let pages = fetch_all_pages(&agent, &self.base_url, "/devices", &creds)?;
        map_devices_pages(&pages)
    }
}

/// Checks a Token ID/Secret pair against Pulseway (a lightweight, real call:
/// `GET /organizations?$top=1`, a single-item page -- never fetches the full
/// organization list), without persisting anything. For
/// `commands::pulseway::test_pulseway_connection`, so users notice a typo in
/// the base URL/token before actually creating a connection (writing
/// credentials to the keyring) -- exactly the same pattern as
/// `plugin::tacticalrmm::test_credentials`/`plugin::ninja::test_credentials`.
pub fn test_credentials(
    base_url: &str,
    token_id: &str,
    token_secret: &str,
) -> Result<(), PluginError> {
    let creds = PulsewayCredentials {
        token_id: token_id.to_string(),
        token_secret: token_secret.to_string(),
    };
    let agent = build_agent();
    fetch_page(&agent, base_url, "/organizations", &creds, 1, 0)?;
    Ok(())
}

impl Plugin for PulsewayPlugin {
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
                // Pulseway's own API reference documents `Name` AS the
                // device's hostname (see module docs) -- there is no
                // separate hostname field to fall back from/to, unlike
                // Tactical RMM/NinjaOne.
                hostname: Some(d.name.clone()),
                name: d.name,
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
        fetch_json_at(&agent, &url, &creds)
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Pulseway's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("PulsewayPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn parse_credentials(credentials: &PluginCredentials) -> Result<PulsewayCredentials, PluginError> {
    serde_json::from_str(&credentials.secret)
        .map_err(|e| PluginError::Authentication(format!("Pulseway-Zugangsdaten ungültig: {e}")))
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

/// Builds the `Authorization: Basic <...>` header value from a Token
/// ID/Secret pair -- `base64(token_id ++ ":" ++ token_secret)`, see module
/// docs.
fn basic_auth_header(creds: &PulsewayCredentials) -> String {
    let raw = format!("{}:{}", creds.token_id, creds.token_secret);
    format!("Basic {}", BASE64_STANDARD.encode(raw))
}

/// Issues an authenticated `GET` against a fully-formed URL (either a
/// hand-built `{base_url}{path}?...` URL from `fetch_page`, or a
/// `Meta.NextQueryLink` URL handed back by Pulseway itself) and returns the
/// parsed JSON envelope. Auth header is `Authorization: Basic ...` (see
/// module docs), NOT Tactical RMM's `X-API-KEY`/NinjaOne's `Bearer` token.
fn fetch_json_at(
    agent: &Agent,
    url: &str,
    creds: &PulsewayCredentials,
) -> Result<serde_json::Value, PluginError> {
    let mut response = agent
        .get(url)
        .header("Authorization", basic_auth_header(creds))
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// Fetches exactly one OData-style page (`$top`/`$skip`, `$count=true` for
/// `Meta.TotalCount`, see module docs) of a Pulseway list endpoint.
fn fetch_page(
    agent: &Agent,
    base_url: &str,
    path: &str,
    creds: &PulsewayCredentials,
    top: u32,
    skip: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!(
        "{}{path}?$top={top}&$skip={skip}&$count=true",
        base_url.trim_end_matches('/')
    );
    fetch_json_at(agent, &url, creds)
}

/// Page size requested per `$top` page while walking `$skip`. Large enough
/// that typical MSP fleets (see module docs: pagination only truly kicks in
/// past 5000 total results) finish in one or two requests, small enough to
/// stay a well-behaved API citizen under the ~3600-requests/hour rate limit.
const PAGE_SIZE: u32 = 1000;

/// Exhaustively fetches ALL pages of a paginated Pulseway list endpoint
/// (`/organizations` or `/devices`), returning the raw JSON envelope of each
/// page fetched -- NOT yet combined into typed values, see
/// `map_organizations_pages`/`map_devices_pages` for the pure, separately
/// testable combining step (see module docs on pagination).
///
/// Real pagination loop, per module docs: as long as a page's
/// `Meta.NextQueryLink` is present, that exact URL is followed directly
/// (Pulseway only starts returning it past 5000 total results); once absent,
/// falls back to incrementing `$skip` by the page's own item count while the
/// running total collected is still below `Meta.TotalCount`. An empty page,
/// or a page whose `Meta.TotalCount` can't be read at all, stops the loop
/// rather than risking an infinite request loop against an unexpected
/// response shape.
fn fetch_all_pages(
    agent: &Agent,
    base_url: &str,
    path: &str,
    creds: &PulsewayCredentials,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut pages: Vec<serde_json::Value> = Vec::new();
    let mut skip: u32 = 0;
    let mut collected: u64 = 0;

    loop {
        let page = fetch_page(agent, base_url, path, creds, PAGE_SIZE, skip)?;
        let page_len = page["Data"].as_array().map(|a| a.len()).unwrap_or(0);
        collected += page_len as u64;
        let next_link = page["Meta"]["NextQueryLink"].as_str().map(str::to_string);
        let total_count = page["Meta"]["TotalCount"].as_u64();
        pages.push(page);

        if let Some(link) = next_link {
            // Once Pulseway itself hands back a `NextQueryLink`, follow it
            // to completion unconditionally -- `Meta.TotalCount` is already
            // known to be exceeded at this point (see module docs), so
            // there's nothing left for `collected` to gate here.
            let mut current_link = link;
            loop {
                let next_page = fetch_json_at(agent, &current_link, creds)?;
                let next_link_again = next_page["Meta"]["NextQueryLink"]
                    .as_str()
                    .map(str::to_string);
                pages.push(next_page);
                match next_link_again {
                    Some(link) => current_link = link,
                    None => break,
                }
            }
            break;
        }

        if page_len == 0 {
            break;
        }
        skip += page_len as u32;
        match total_count {
            Some(total) if collected < total => continue,
            _ => break,
        }
    }

    Ok(pages)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Pulseway-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Pulseway-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Combines the `Data` arrays of already-fetched `/organizations` page
/// envelopes into `PulsewayOrganization` values. Deliberately extracted as
/// its own, PURE function (no network access) taking already-fetched pages
/// -- both the per-item mapping AND the multi-page-combining logic are
/// independently testable this way, without any real network access, same
/// principle as `plugin::tacticalrmm::map_clients_response`.
fn map_organizations_pages(
    pages: &[serde_json::Value],
) -> Result<Vec<PulsewayOrganization>, PluginError> {
    let mut result = Vec::new();
    for page in pages {
        let array = page["Data"].as_array().ok_or_else(|| {
            PluginError::UnexpectedResponse("Erwartete Data-Liste von Organisationen".to_string())
        })?;
        result.extend(array.iter().filter_map(map_organization));
    }
    Ok(result)
}

/// A single organization object from Pulseway's `/organizations` response
/// (`{"Id": <number>, "Name": "...", "Type": "..."}`, PascalCase, see module
/// docs). If `Id` is missing or has an unexpected shape -> the organization
/// is skipped instead of failing the whole call, analogous to
/// `plugin::tacticalrmm::map_client`/`plugin::ninja::map_organization`.
fn map_organization(value: &serde_json::Value) -> Option<PulsewayOrganization> {
    let id = match &value["Id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let name = value["Name"].as_str().unwrap_or(id.as_str()).to_string();
    Some(PulsewayOrganization { id, name })
}

/// Combines the `Data` arrays of already-fetched `/devices` page envelopes
/// into `PulsewayDevice` values. Pure function, testable on its own with
/// hardcoded JSON, analogous to `map_organizations_pages`.
fn map_devices_pages(pages: &[serde_json::Value]) -> Result<Vec<PulsewayDevice>, PluginError> {
    let mut result = Vec::new();
    for page in pages {
        let array = page["Data"].as_array().ok_or_else(|| {
            PluginError::UnexpectedResponse("Erwartete Data-Liste von Geräten".to_string())
        })?;
        result.extend(array.iter().filter_map(map_device));
    }
    Ok(result)
}

/// A single device object from Pulseway's `/devices` response (verified
/// field list: `Identifier`, `Name`, `OrganizationId`, `OrganizationName`,
/// `SiteName`, `GroupName`, `IsAgentInstalled` -- see module docs). A device
/// without a usable `Identifier` OR without a numeric `OrganizationId` is
/// skipped -- without organization membership it can't be meaningfully
/// grouped, and a single broken device object shouldn't make the whole list
/// unusable, analogous to `plugin::ninja::map_ninja_device`'s "skip without
/// organizationId" rule.
fn map_device(value: &serde_json::Value) -> Option<PulsewayDevice> {
    let external_id = value["Identifier"].as_str()?.to_string();
    let organization_id = match &value["OrganizationId"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let name = value["Name"]
        .as_str()
        .unwrap_or(external_id.as_str())
        .to_string();
    let organization_name = value["OrganizationName"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let site_name = value["SiteName"].as_str().map(str::to_string);
    let group_name = value["GroupName"].as_str().map(str::to_string);
    let is_agent_installed = value["IsAgentInstalled"].as_bool().unwrap_or(false);
    Some(PulsewayDevice {
        external_id,
        name,
        organization_id,
        organization_name,
        site_name,
        group_name,
        is_agent_installed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn organizations_page(items: &str) -> serde_json::Value {
        serde_json::from_str(&format!(
            r#"{{"Data": {items}, "Meta": {{"ResponseCode": 200, "TotalCount": 2}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn maps_organizations_data_envelope_into_pulseway_organizations() {
        let page = organizations_page(
            r#"[
                {"Id": 6978, "Name": "Acme Corp", "Type": "Customer"},
                {"Id": 6979, "Name": "Contoso AG", "Type": "Customer"}
            ]"#,
        );

        let organizations = map_organizations_pages(&[page]).unwrap();

        assert_eq!(organizations.len(), 2);
        assert_eq!(
            organizations[0],
            PulsewayOrganization {
                id: "6978".to_string(),
                name: "Acme Corp".to_string()
            }
        );
        assert_eq!(organizations[1].id, "6979");
        assert_eq!(organizations[1].name, "Contoso AG");
    }

    #[test]
    fn organizations_rejects_envelope_without_data_key() {
        let page = serde_json::json!({"Meta": {"TotalCount": 0}});
        let result = map_organizations_pages(&[page]);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn organization_falls_back_to_id_when_no_name_field_present() {
        let page = organizations_page(r#"[{"Id": 9}]"#);
        let organizations = map_organizations_pages(&[page]).unwrap();
        assert_eq!(organizations[0].name, "9");
    }

    #[test]
    fn organization_skips_entries_without_a_usable_id() {
        let page = organizations_page(r#"[{"Name": "OhneId"}]"#);
        let organizations = map_organizations_pages(&[page]).unwrap();
        assert!(organizations.is_empty());
    }

    #[test]
    fn organizations_pages_combine_across_multiple_fetched_pages() {
        let page1 = organizations_page(r#"[{"Id": 1, "Name": "Erste"}]"#);
        let page2 = organizations_page(r#"[{"Id": 2, "Name": "Zweite"}]"#);

        let organizations = map_organizations_pages(&[page1, page2]).unwrap();

        assert_eq!(organizations.len(), 2);
        assert_eq!(organizations[0].name, "Erste");
        assert_eq!(organizations[1].name, "Zweite");
    }

    fn devices_page(items: &str, total_count: u64) -> serde_json::Value {
        serde_json::from_str(&format!(
            r#"{{"Data": {items}, "Meta": {{"ResponseCode": 200, "TotalCount": {total_count}}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn maps_devices_data_envelope_with_organization_site_and_group() {
        let page = devices_page(
            r#"[
                {
                    "Identifier": "3f9c1e2a-1111-4a6b-9d3e-abc123",
                    "Name": "SRV-01",
                    "OrganizationId": 6978,
                    "OrganizationName": "Acme Corp",
                    "SiteName": "Hauptsitz",
                    "GroupName": "Server",
                    "IsAgentInstalled": true
                },
                {
                    "Identifier": "3f9c1e2a-2222-4a6b-9d3e-abc456",
                    "Name": "WS-07",
                    "OrganizationId": 6979,
                    "OrganizationName": "Contoso AG",
                    "SiteName": null,
                    "GroupName": null,
                    "IsAgentInstalled": false
                }
            ]"#,
            2,
        );

        let devices = map_devices_pages(&[page]).unwrap();

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "3f9c1e2a-1111-4a6b-9d3e-abc123");
        assert_eq!(devices[0].name, "SRV-01");
        assert_eq!(devices[0].organization_id, "6978");
        assert_eq!(devices[0].organization_name, "Acme Corp");
        assert_eq!(devices[0].site_name.as_deref(), Some("Hauptsitz"));
        assert_eq!(devices[0].group_name.as_deref(), Some("Server"));
        assert!(devices[0].is_agent_installed);
        assert_eq!(devices[1].external_id, "3f9c1e2a-2222-4a6b-9d3e-abc456");
        assert_eq!(devices[1].organization_id, "6979");
        assert_eq!(devices[1].site_name, None);
        assert_eq!(devices[1].group_name, None);
        assert!(!devices[1].is_agent_installed);
    }

    #[test]
    fn devices_rejects_envelope_without_data_key() {
        let page = serde_json::json!({"Meta": {"TotalCount": 0}});
        let result = map_devices_pages(&[page]);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn device_skips_entries_without_identifier() {
        let page = devices_page(r#"[{"Name": "OhneId", "OrganizationId": 1}]"#, 1);
        let devices = map_devices_pages(&[page]).unwrap();
        assert!(devices.is_empty());
    }

    #[test]
    fn device_skips_entries_without_organization_id() {
        let page = devices_page(r#"[{"Identifier": "abc", "Name": "OhneOrg"}]"#, 1);
        let devices = map_devices_pages(&[page]).unwrap();
        assert!(devices.is_empty());
    }

    #[test]
    fn device_falls_back_to_external_id_when_no_name_field_present() {
        let page = devices_page(r#"[{"Identifier": "abc", "OrganizationId": 1}]"#, 1);
        let devices = map_devices_pages(&[page]).unwrap();
        assert_eq!(devices[0].name, "abc");
    }

    #[test]
    fn devices_pages_combine_across_multiple_fetched_pages() {
        let page1 = devices_page(
            r#"[{"Identifier": "a1", "Name": "SRV-01", "OrganizationId": 1}]"#,
            2,
        );
        let page2 = devices_page(
            r#"[{"Identifier": "a2", "Name": "SRV-02", "OrganizationId": 1}]"#,
            2,
        );

        let devices = map_devices_pages(&[page1, page2]).unwrap();

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "a1");
        assert_eq!(devices[1].external_id, "a2");
    }

    #[test]
    fn basic_auth_header_base64_encodes_token_id_colon_token_secret() {
        let creds = PulsewayCredentials {
            token_id: "abc".to_string(),
            token_secret: "xyz".to_string(),
        };
        assert_eq!(basic_auth_header(&creds), "Basic YWJjOnh5eg==");
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
    fn maps_429_rate_limit_status_to_unreachable_error() {
        // No special retry/backoff logic for this integration (see module
        // docs) -- 429 falls into the same generic "other status ->
        // Unreachable" bucket as any other non-2xx response.
        let err = map_ureq_error(ureq::Error::StatusCode(429));
        assert!(matches!(err, PluginError::Unreachable(_)));
    }

    #[test]
    fn plugin_id_returns_configured_connection_id() {
        let plugin = PulsewayPlugin::new(
            "pulseway:acme-123".to_string(),
            "https://api.pulseway.com/v3".to_string(),
        );
        assert_eq!(plugin.id(), "pulseway:acme-123");
    }
}
