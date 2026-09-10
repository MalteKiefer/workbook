//! Real plugin implementation for Vultr (cloud VPS/server hosting), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Vultr-Plugin". Structurally
//! closest to `plugin::level`/`plugin::intune`: a connection binds directly
//! to exactly one local customer, no organization/tenant mapping layer
//! needed -- but simpler still in *purpose*: this plugin surfaces server
//! inventory (a customer's cloud instances), not RMM/MDM device management
//! telemetry, so there is no group/site/organization concept to resolve at
//! all (unlike Level's groups), and no backup-status concept either (unlike
//! `plugin::acronis`) -- the simplest kind of plugin in this codebase.
//!
//! Facts below verified against Vultr's real, official Go SDK source
//! (`govultr`, <https://github.com/vultr/govultr>) and
//! <https://www.vultr.com/api/> (the interactive API reference at
//! docs.vultr.com).
//!
//! - **Authentication**: static API key, header `Authorization: Bearer
//!   <api-key>` -- WITH a `Bearer` prefix (unlike Level.io's bare-key
//!   header). Generated in the customer portal under Account > API (requires
//!   "Enable API" to be turned on first) -- an out-of-band, one-time setup
//!   step the user performs themselves, exactly like every other API-key
//!   plugin here.
//! - **Fixed host, no `base_url`**: Vultr's API host is always
//!   `api.vultr.com` -- like `plugin::level::BASE_URL`/
//!   `plugin::intune::GRAPH_BASE_URL`, a constant instead of a configurable
//!   `base_url` field on `VultrConnectionMeta`.
//! - **Tenancy**: confirmed 1:1 for what this plugin needs -- a Vultr API key
//!   sees exactly one account's instances ("List all instances on your
//!   account"). Vultr does have a genuine "Sub-Accounts" feature, but each
//!   sub-account is a fully independent Vultr user with its OWN separate API
//!   key -- so an MSP with multiple Vultr sub-accounts needs one connection
//!   (one API key) per sub-account here, exactly the same shape as
//!   Level.io's/Intune's "one connection = one customer" pattern, NOT a
//!   parent-account-sees-all-children mapping table like NinjaOne's
//!   `NinjaOrgMapping`. `VultrConnectionMeta` therefore carries `customer_id`
//!   directly, like `LevelConnectionMeta`/`IntuneConnectionMeta`.
//! - **Instances (the inventory endpoint)**: `GET {BASE_URL}/instances` ->
//!   `{"instances": [...], "meta": {...}}`. Per-instance fields this module
//!   maps: `id` (string, a UUID -- used directly as `external_id`), `label`
//!   (string, user-set display name) and `hostname` (a separate field --
//!   `label` wins if non-empty, else `hostname`, else the external ID,
//!   matching the "prefer human label, fall back to raw identifier"
//!   convention already used by several plugins here, see `map_instance`),
//!   `main_ip` (string, IPv4 -- the primary `ip_address`), `v6_main_ip`
//!   (string, IPv6 -- an EMPTY STRING, not `null`, when IPv6 is disabled for
//!   that instance; treated the same as absent, never stored as `Some("")`,
//!   see `non_empty_str`), `plan` (string, the instance size/flavor -- used
//!   as a `platform`-equivalent display string, like Datto RMM's
//!   `deviceClass`), `region` (string, e.g. `"ewr"` -- informational display
//!   only).
//! - **`status` vs. `power_status`**: a real instance object has BOTH a
//!   `status` field (overall lifecycle state, e.g. `"active"`/`"pending"`)
//!   AND a separate `power_status` field (the running/stopped power state) --
//!   two distinct fields, verified in the real SDK struct. This module
//!   surfaces `power_status` as `VultrInstance::status` (deliberately
//!   choosing that name over Vultr's own `status` field): it's the more
//!   immediately actionable "is it on" signal an IT admin cares about at a
//!   glance, whereas the lifecycle `status` field matters mostly during
//!   provisioning/deletion -- a much smaller slice of this plugin's actual
//!   use. See `map_instance`.
//! - **Pagination**: cursor-based. Query params `per_page` (default 100, max
//!   500 -- this module always requests the max, `PAGE_LIMIT = 500`, to
//!   minimize round-trips) and `cursor`. Response envelope: `{"instances":
//!   [...], "meta": {"total": <int>, "links": {"next": "<cursor or empty
//!   string>", "prev": "..."}}}`. More pages exist while `meta.links.next`
//!   is a non-empty string -- that string becomes the next request's
//!   `cursor` query param (see `parse_instances_page`). `fetch_all_instances`
//!   walks all pages internally and returns a single, already-merged list --
//!   the caller sees nothing of Vultr's pagination.
//!   `fetch_all_instances_via` is deliberately generic over the page-fetch
//!   function (rather than hardcoding a real HTTP call), so the
//!   pagination-combining logic itself -- follow the cursor exactly as
//!   given, stop when it's empty/absent -- is testable as a pure function
//!   over pre-fetched, hardcoded JSON pages, without a real network call --
//!   exactly the same pattern as `plugin::dattormm::fetch_all_pages`.
//!   Capped at `MAX_PAGES`, protection against a misbehaving remote end.
//! - **Single-instance detail**: `GET {BASE_URL}/instances/{id}` ->
//!   `{"instance": {...}}`, passed through unmodified as `serde_json::Value`,
//!   exactly like every other plugin here.
//! - **Rate limits**: documented -- 30 requests/second per source IP, `429`
//!   on exceeding, no `Retry-After` header documented. No special retry/
//!   backoff logic needed here -- this app's usage pattern (manual/
//!   occasional syncs) is nowhere near 30 req/s, matching every other plugin
//!   in this codebase.
//! - **No dashboard deep-link**: NOT verifiable against the current v2 API --
//!   the only concrete Vultr instance-URL pattern found in public references
//!   uses a deprecated v1 numeric ID, not the v2 UUID `id` this plugin
//!   actually has. Deliberately left out entirely, the same honest-omission
//!   principle as `plugin::tacticalrmm`'s missing dashboard link.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the exact same dependency as
//!   every other plugin here.
//! - **Credential encoding**: Vultr needs only a single secret value (the API
//!   key), which is 1:1 `PluginCredentials.secret` -- no JSON encoding
//!   needed, exactly like `plugin::level`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Vultr's API host is fixed -- no region/instance variants, no self-hosting
/// -- hence a constant instead of a configurable `base_url` field on
/// `VultrConnectionMeta`, analogous to `plugin::level::BASE_URL`.
pub const BASE_URL: &str = "https://api.vultr.com/v2";

/// Vultr's documented maximum `per_page` -- always requested, to minimize
/// round-trips (see module docs, "Pagination").
const PAGE_LIMIT: u32 = 500;

/// Protection against a misbehaving remote end (a `meta.links.next` that
/// never stops appearing): more than `MAX_PAGES` pages of `PAGE_LIMIT`
/// instances each are not fetched, the same defensive convention as
/// `plugin::level::MAX_PAGES`/`plugin::dattormm::MAX_PAGES`.
const MAX_PAGES: usize = 20;

/// Non-secret metadata of a Vultr connection, as stored in `config.toml`
/// (`Config::vultr_connections`). The API key belongs, per the credential
/// principle, exclusively in the OS keyring, never here. Like
/// `LevelConnectionMeta`/`IntuneConnectionMeta`: a Vultr API key sees exactly
/// one account's instances, no sub-account-sees-children mapping table (see
/// module docs, "Tenancy") -- a connection here directly carries its
/// `customer_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VultrConnectionMeta {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

/// A single instance from `GET /v2/instances` or `GET /v2/instances/{id}`.
/// Analogous to `plugin::level::LevelDevice`, but for server inventory
/// instead of RMM device telemetry -- no group concept (Vultr has none
/// visible through a single API key), no separate `hostname` field of its
/// own (the display-name fallback already folds it into `name`, see
/// `map_instance`). `linked_system_id` here is always `None` -- this struct
/// only reflects what Vultr's API itself returns; the actual linked-system
/// lookup happens one layer up in `commands::vultr`
/// (`to_external_system_dto`, mirroring `commands::level`'s own division of
/// labour), which is where the field is actually populated for the frontend
/// DTO.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VultrInstance {
    pub external_id: String,
    pub name: String,
    pub ip_address: Option<String>,
    /// IPv6 address (`v6_main_ip`) -- Vultr returns an EMPTY STRING, not
    /// `null`, when IPv6 is disabled for an instance; `map_instance` treats
    /// that the same as absent (see `non_empty_str`), never storing
    /// `Some("")`.
    pub ipv6_address: Option<String>,
    /// Vultr's `power_status` field (NOT the separate `status` field -- see
    /// module docs, "`status` vs. `power_status`").
    pub status: Option<String>,
    /// Vultr's `plan` field (the instance size/flavor), used as a
    /// `platform`-equivalent display string, like Datto RMM's `deviceClass`.
    pub platform: Option<String>,
    pub region: Option<String>,
    pub linked_system_id: Option<i64>,
}

/// A plugin object for exactly one configured Vultr connection. `id` here is
/// already the fully qualified identifier (`"vultr:<connection_id>"`), so
/// that `Plugin::id()` works unmodified as the `plugin_id`/keyring account
/// (see trait documentation in `plugin::mod`). Like `LevelPlugin`, this type
/// needs no `base_url` field (see `BASE_URL` above) and no cached token --
/// the API key comes fresh from `PluginCredentials` on every call.
pub struct VultrPlugin {
    id: String,
}

impl VultrPlugin {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    /// Live fetch of ALL instances of this connection, with pagination
    /// walked internally (see module documentation). Richer than the trait
    /// method `list_systems`, which deliberately stays with the narrow,
    /// plugin-agnostic `ExternalSystem` type (no `ip_address`/`status`/etc.
    /// fields there).
    pub fn list_instances(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<VultrInstance>, PluginError> {
        let agent = build_agent();
        let raw_instances = fetch_all_instances(&agent, &credentials.secret)?;
        Ok(map_instances(&raw_instances))
    }
}

/// Checks an API key against Vultr (a lightweight call: one page with
/// `per_page=1`), without persisting anything. For
/// `commands::vultr::test_vultr_connection`, so users notice a typo in the
/// API key before actually creating a connection (writing credentials to the
/// keyring).
pub fn test_credentials(api_key: &str) -> Result<(), PluginError> {
    let agent = build_agent();
    fetch_instances_page(&agent, api_key, None, 1)?;
    Ok(())
}

impl Plugin for VultrPlugin {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn list_systems(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<ExternalSystem>, PluginError> {
        let instances = self.list_instances(credentials)?;
        Ok(instances
            .into_iter()
            .map(|i| ExternalSystem {
                external_id: i.external_id,
                name: i.name,
                // Vultr's instance object has no separate hostname field --
                // the display-name fallback chain already folds it into
                // `name` (see module docs/`map_instance`).
                hostname: None,
            })
            .collect())
    }

    fn get_system_details(
        &self,
        credentials: &PluginCredentials,
        external_id: &str,
    ) -> Result<serde_json::Value, PluginError> {
        let agent = build_agent();
        fetch_instance_json(&agent, &credentials.secret, external_id)
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Vultr's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("VultrPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

fn fetch_instances_page(
    agent: &Agent,
    api_key: &str,
    cursor: Option<&str>,
    per_page: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{BASE_URL}/instances");
    let mut request = agent
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .query("per_page", per_page.to_string());
    if let Some(c) = cursor {
        request = request.query("cursor", c);
    }
    let mut response = request.call().map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

fn fetch_instance_json(
    agent: &Agent,
    api_key: &str,
    external_id: &str,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{BASE_URL}/instances/{external_id}");
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// Walks every page of `GET /v2/instances` via `fetch_page`, merging the
/// `instances` arrays into a single list, following the `meta.links.next`
/// cursor exactly as documented (an empty string, or a missing/absent
/// field, both mean "last page" -- see `parse_instances_page`). Deliberately
/// generic over `fetch_page` (rather than hardcoding a real HTTP call) so
/// the pagination-combining logic itself is testable as a pure function
/// over pre-fetched, hardcoded JSON pages, without a real network call --
/// exactly the same pattern as `plugin::dattormm::fetch_all_pages`. Capped
/// at `MAX_PAGES` (see module docs).
fn fetch_all_instances_via<F>(mut fetch_page: F) -> Result<Vec<serde_json::Value>, PluginError>
where
    F: FnMut(Option<&str>) -> Result<serde_json::Value, PluginError>,
{
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_page(cursor.as_deref())?;
        let (data, next_cursor) = parse_instances_page(&page_json)?;
        all.extend(data);
        match next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(all)
}

fn fetch_all_instances(agent: &Agent, api_key: &str) -> Result<Vec<serde_json::Value>, PluginError> {
    fetch_all_instances_via(|cursor| fetch_instances_page(agent, api_key, cursor, PAGE_LIMIT))
}

/// Extracts the instance list and next-page cursor from a single Vultr page
/// response (`GET /v2/instances`: `{"instances": [...], "meta": {"total":
/// <int>, "links": {"next": "<cursor or empty string>", "prev": "..."}}}`,
/// verified against docs.vultr.com/govultr). Pure function, testable with
/// hardcoded JSON, no real network access needed. `meta.links.next` being an
/// empty string OR missing entirely both mean "no more pages" -- Vultr's own
/// documented convention is the empty string, but this also tolerates a
/// missing field defensively (same "graceful, not an error" principle as
/// `plugin::level::parse_devices_page`'s `has_more` default).
fn parse_instances_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, Option<String>), PluginError> {
    let data = json["instances"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete 'instances'-Liste in Vultr-Antwort".to_string())
    })?;
    let next_cursor = json["meta"]["links"]["next"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok((data.clone(), next_cursor))
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Vultr-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Vultr-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Reads a string field, treating both a missing/`null` field AND an empty
/// string as absent -- Vultr's own documented convention for optional string
/// fields like `v6_main_ip` (see module docs). Used for every optional
/// string field `map_instance` reads, not just the IPv6 case, for
/// consistency (a `""` label or hostname is exactly as useless as a missing
/// one).
fn non_empty_str(value: &serde_json::Value, key: &str) -> Option<String> {
    value[key].as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Maps a (already merged across all pages) list of raw Vultr instance
/// objects to `VultrInstance` values. Pure function, testable with hardcoded
/// JSON -- Vultr's "List Instances" and "Get Instance" endpoints return
/// instance objects in exactly the same shape.
fn map_instances(instances: &[serde_json::Value]) -> Vec<VultrInstance> {
    instances.iter().filter_map(map_instance).collect()
}

/// A single instance object from Vultr's response. `id` missing or not a
/// string -> the instance is skipped instead of failing the whole call (a
/// single broken instance object shouldn't make the whole list unusable,
/// analogous to `plugin::level::map_level_device`). Display-name fallback
/// chain: `label` (if non-empty) -> `hostname` (if non-empty) -> the
/// external ID, matching the "prefer human label, fall back to raw
/// identifier" convention already used by several plugins here (see module
/// docs). `status` surfaces `power_status`, not Vultr's own `status` field
/// (see module docs). `linked_system_id` is always `None` here -- see the
/// `VultrInstance` doc comment.
fn map_instance(value: &serde_json::Value) -> Option<VultrInstance> {
    let external_id = value["id"].as_str()?.to_string();
    let label = non_empty_str(value, "label");
    let hostname = non_empty_str(value, "hostname");
    let name = label.or(hostname).unwrap_or_else(|| external_id.clone());
    Some(VultrInstance {
        external_id,
        name,
        ip_address: non_empty_str(value, "main_ip"),
        ipv6_address: non_empty_str(value, "v6_main_ip"),
        status: non_empty_str(value, "power_status"),
        platform: non_empty_str(value, "plan"),
        region: non_empty_str(value, "region"),
        linked_system_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_instance_json() -> serde_json::Value {
        serde_json::json!({
            "id": "cb676a46-66fd-4dfb-b839-443f2e6c0b60",
            "os": "Debian 12 x64",
            "ram": 4096,
            "disk": 80,
            "main_ip": "203.0.113.10",
            "vcpu_count": 2,
            "region": "ewr",
            "plan": "vc2-2c-4gb",
            "date_created": "2020-10-10T01:56:20+00:00",
            "status": "active",
            "power_status": "running",
            "server_status": "ok",
            "allowed_bandwidth": 4000,
            "netmask_v4": "255.255.254.0",
            "gateway_v4": "203.0.113.1",
            "v6_network": "2001:db8:1002:1000::",
            "v6_main_ip": "2001:db8:1002:1000:5400:2ff:feb5:3b3c",
            "v6_network_size": 64,
            "label": "my-server-01",
            "internal_ip": "",
            "kvm": "https://my.vultr.com/subs/vps/novnc/api.php?data=...",
            "hostname": "my-server-01.example.com",
            "os_id": 391,
            "app_id": 0,
            "image_id": "",
            "firewall_group_id": "",
            "features": ["ipv6", "backups"],
            "tags": []
        })
    }

    #[test]
    fn parses_instances_page_with_data_and_next_cursor() {
        let json = serde_json::json!({
            "instances": [{"id": "inst-1"}, {"id": "inst-2"}],
            "meta": {"total": 2, "links": {"next": "cursor-abc", "prev": ""}}
        });
        let (data, next_cursor) = parse_instances_page(&json).unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(next_cursor.as_deref(), Some("cursor-abc"));
    }

    #[test]
    fn parses_instances_page_treats_empty_next_as_last_page() {
        let json = serde_json::json!({
            "instances": [],
            "meta": {"total": 0, "links": {"next": "", "prev": ""}}
        });
        let (data, next_cursor) = parse_instances_page(&json).unwrap();
        assert!(data.is_empty());
        assert_eq!(next_cursor, None);
    }

    #[test]
    fn parses_instances_page_treats_missing_next_as_last_page() {
        let json = serde_json::json!({"instances": []});
        let (data, next_cursor) = parse_instances_page(&json).unwrap();
        assert!(data.is_empty());
        assert_eq!(next_cursor, None);
    }

    #[test]
    fn parse_instances_page_rejects_missing_instances_field() {
        let json = serde_json::json!({"meta": {"links": {"next": ""}}});
        let result = parse_instances_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn maps_full_sample_instance_from_realistic_response() {
        let instances = map_instances(std::slice::from_ref(&sample_instance_json()));
        assert_eq!(instances.len(), 1);
        let instance = &instances[0];
        assert_eq!(instance.external_id, "cb676a46-66fd-4dfb-b839-443f2e6c0b60");
        assert_eq!(instance.name, "my-server-01");
        assert_eq!(instance.ip_address.as_deref(), Some("203.0.113.10"));
        assert_eq!(
            instance.ipv6_address.as_deref(),
            Some("2001:db8:1002:1000:5400:2ff:feb5:3b3c")
        );
        // power_status ("running"), NOT status ("active") -- see module docs.
        assert_eq!(instance.status.as_deref(), Some("running"));
        assert_eq!(instance.platform.as_deref(), Some("vc2-2c-4gb"));
        assert_eq!(instance.region.as_deref(), Some("ewr"));
        assert_eq!(instance.linked_system_id, None);
    }

    #[test]
    fn skips_instances_without_a_usable_id() {
        let json = serde_json::json!([{"label": "ohne-id"}]);
        let instances = map_instances(json.as_array().unwrap());
        assert!(instances.is_empty());
    }

    #[test]
    fn falls_back_to_hostname_when_label_missing() {
        let json = serde_json::json!([{"id": "inst-1", "hostname": "srv-01.example.com"}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].name, "srv-01.example.com");
    }

    #[test]
    fn falls_back_to_external_id_when_label_and_hostname_absent() {
        let json = serde_json::json!([{"id": "inst-7"}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].name, "inst-7");
    }

    #[test]
    fn empty_label_string_falls_back_to_hostname() {
        let json = serde_json::json!([{"id": "inst-1", "label": "", "hostname": "srv-01.example.com"}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].name, "srv-01.example.com");
    }

    #[test]
    fn label_wins_over_hostname_when_both_present() {
        let json = serde_json::json!([{"id": "inst-1", "label": "Mail Server", "hostname": "srv-01.example.com"}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].name, "Mail Server");
    }

    #[test]
    fn empty_main_ip_is_treated_as_absent() {
        let json = serde_json::json!([{"id": "inst-1", "main_ip": ""}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].ip_address, None);
    }

    #[test]
    fn v6_main_ip_empty_string_is_treated_as_absent() {
        let json = serde_json::json!([{"id": "inst-1", "v6_main_ip": ""}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].ipv6_address, None);
    }

    #[test]
    fn v6_main_ip_missing_entirely_is_none() {
        let json = serde_json::json!([{"id": "inst-1"}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].ipv6_address, None);
    }

    #[test]
    fn v6_main_ip_present_is_captured() {
        let json = serde_json::json!([{"id": "inst-1", "v6_main_ip": "2001:db8::1"}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].ipv6_address.as_deref(), Some("2001:db8::1"));
    }

    #[test]
    fn status_field_surfaces_power_status_not_lifecycle_status() {
        let json = serde_json::json!([{"id": "inst-1", "status": "active", "power_status": "stopped"}]);
        let instances = map_instances(json.as_array().unwrap());
        assert_eq!(instances[0].status.as_deref(), Some("stopped"));
    }

    #[test]
    fn missing_optional_fields_all_default_to_none_gracefully() {
        let json = serde_json::json!([{"id": "inst-1"}]);
        let instances = map_instances(json.as_array().unwrap());
        let instance = &instances[0];
        assert_eq!(instance.ip_address, None);
        assert_eq!(instance.ipv6_address, None);
        assert_eq!(instance.status, None);
        assert_eq!(instance.platform, None);
        assert_eq!(instance.region, None);
        assert_eq!(instance.name, "inst-1");
    }

    #[test]
    fn fetch_all_instances_via_combines_multiple_pages_via_cursor() {
        let mut seen_cursors: Vec<Option<String>> = Vec::new();
        let result = fetch_all_instances_via(|cursor| {
            seen_cursors.push(cursor.map(str::to_string));
            match cursor {
                None => Ok(serde_json::json!({
                    "instances": [{"id": "inst-1"}],
                    "meta": {"total": 2, "links": {"next": "cursor-2", "prev": ""}}
                })),
                Some("cursor-2") => Ok(serde_json::json!({
                    "instances": [{"id": "inst-2"}],
                    "meta": {"total": 2, "links": {"next": "", "prev": "cursor-1"}}
                })),
                other => panic!("unexpected cursor: {other:?}"),
            }
        })
        .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(seen_cursors, vec![None, Some("cursor-2".to_string())]);
    }

    #[test]
    fn fetch_all_instances_via_stops_when_next_is_empty_string() {
        let result = fetch_all_instances_via(|_cursor| {
            Ok(serde_json::json!({
                "instances": [{"id": "inst-1"}],
                "meta": {"total": 1, "links": {"next": "", "prev": ""}}
            }))
        })
        .unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn fetch_all_instances_via_stops_after_max_pages_guard() {
        let mut call_count = 0usize;
        let result = fetch_all_instances_via(|_cursor| {
            call_count += 1;
            Ok(serde_json::json!({
                "instances": [{"id": format!("inst-{call_count}")}],
                "meta": {"total": 999, "links": {"next": "always-more", "prev": ""}}
            }))
        })
        .unwrap();

        assert_eq!(call_count, MAX_PAGES);
        assert_eq!(result.len(), MAX_PAGES);
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
        // Vultr's documented rate limit (30 req/s/IP) -- 429 is not an
        // authentication problem, so it maps like any other non-2xx status
        // (see module docs, "Rate limits").
        let err = map_ureq_error(ureq::Error::StatusCode(429));
        assert!(matches!(err, PluginError::Unreachable(_)));
    }

    #[test]
    fn plugin_id_returns_configured_connection_id() {
        let plugin = VultrPlugin::new("vultr:acme-123".to_string());
        assert_eq!(plugin.id(), "vultr:acme-123");
    }
}
