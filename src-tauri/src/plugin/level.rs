//! Real plugin implementation for Level.io (RMM), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Level.io plugin". Second real
//! integration after NinjaOne (`plugin::ninja`), but noticeably simpler:
//!
//! - **Authentication**: static API key in the `Authorization` header,
//!   WITHOUT a "Bearer " prefix and without token exchange -- verified
//!   against Level.io's own authentication reference page
//!   (<https://developers.level.io/reference/authentication>: "Provide your
//!   API key as the authorization value", example `-H "Authorization:
//!   APIKEY"`). No OAuth2 grant needed like with NinjaOne.
//! - **No organization/tenant concept**: Level.io's API reference has no
//!   organization, account, or site endpoints (verified via
//!   <https://developers.level.io/llms.txt> -- exclusively device, group,
//!   alert, update/automation, and tag/custom-field endpoints, nothing
//!   about organizations/accounts). Level itself describes its API as
//!   operating at the account level. That's why a Level "connection" (one
//!   API key) here maps directly to exactly one local customer
//!   (`LevelConnectionMeta.customer_id`) -- NO granular organization-mapping
//!   layer like with Ninja (`NinjaOrgMapping`) is needed.
//! - **Device list**: `GET {BASE_URL}/devices`, cursor-paginated (`has_more`
//!   and `starting_after`, verified via
//!   <https://developers.level.io/reference/listdevices>). This layer walks
//!   all pages internally (up to `MAX_PAGES` pages of `PAGE_LIMIT` devices
//!   each, as protection against a misbehaving remote end) and returns a
//!   single, already-merged list -- the caller sees nothing of Level's
//!   pagination.
//! - **Device details**: `GET {BASE_URL}/devices/{id}` (verified via
//!   <https://developers.level.io/reference/showdevice>, "Show Device"),
//!   passes the response through unmodified as `serde_json::Value`,
//!   analogous to `plugin::ninja::NinjaPlugin::get_system_details`.
//! - **IP address**: `include_network_interfaces=true` returns a
//!   `network_interfaces` array per device (`[{..., "ip_addresses":
//!   ["..."]}]`) -- verified via Level's response schema for
//!   `GET /v2/devices`. Unlike Ninja, Level has no flat `ipAddresses` field
//!   at the top level; the first non-empty address of the first network
//!   interface is used.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the exact same dependency as
//!   `plugin::ninja` (no second HTTP client in this codebase).
//! - **Groups**: Level has no organizations, but does have a hierarchical
//!   group concept within an account -- every device carries a nullable
//!   `group_id` field (verified via
//!   <https://developers.level.io/reference/listdevices>: `"group_id": "..."`,
//!   `null` means "ungrouped"). Names for these come from
//!   `GET {BASE_URL}/groups` (verified via
//!   <https://developers.level.io/reference/listgroups>), page envelope
//!   exactly like `/devices` (`{"data": [...], "has_more": bool}`,
//!   `starting_after` cursor). `list_devices` calls both endpoints
//!   (`fetch_all_groups`/`fetch_all_devices`) and resolves `group_id` into a
//!   plaintext name server-side right here (`LevelDevice.group_name`, via
//!   `build_group_lookup`), analogous to how Ninja organizations are
//!   returned named (but not grouped) server-side. A group ID without a
//!   matching entry in the lookup table (e.g. a group deleted in the
//!   meantime) yields `group_name: None` -- not an error case, the frontend
//!   falls back to `Gruppe {group_id}` for that. The actual
//!   grouping/sorting/pagination of the display deliberately stays a
//!   frontend concern (see `LevelPluginSection.tsx`): this module still
//!   returns a flat `Vec<LevelDevice>`, no nested group structure.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Unlike NinjaOne, Level.io's API has no region/instance variants, hence a
/// fixed constant instead of a configurable `base_url` field on
/// `LevelConnectionMeta`.
pub const BASE_URL: &str = "https://api.level.io/v2";

/// Protection against a misbehaving remote end (endless `has_more: true`):
/// more than `MAX_PAGES * PAGE_LIMIT` devices per sync run are not fetched.
const MAX_PAGES: usize = 20;
const PAGE_LIMIT: u32 = 100;

/// Non-secret metadata of a Level connection, as stored in `config.toml`
/// (`Config::level_connections`). The API key belongs, per the credential
/// principle, exclusively in the OS keyring, never here. Unlike
/// `NinjaConnectionMeta`: Level has no organization concept, so a connection
/// here directly carries its `customer_id` -- a connection corresponds to
/// exactly one local customer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelConnectionMeta {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

/// A single device from `GET /v2/devices` or `GET /v2/devices/{id}`,
/// enriched with the (nested) IP address. Analogous to
/// `plugin::ninja::NinjaDevice`, but without `organization_id` -- Level has
/// no organizations, but (unlike Ninja) does have a hierarchical group
/// concept within an account: `group_id` is Level's raw, nullable field
/// (`None` means "ungrouped", a legitimate case, not an error),
/// `group_name` is the plaintext name resolved for it server-side via
/// `build_group_lookup`/`GET /v2/groups` (`None` if `group_id` is set but no
/// matching group was found -- e.g. a group deleted in the meantime).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub group_id: Option<String>,
    pub group_name: Option<String>,
}

/// A plugin object for exactly one configured Level connection. `id` here is
/// already the fully qualified identifier (`"level:<connection_id>"`), so
/// that `Plugin::id()` works unmodified as the `plugin_id`/keyring account
/// (see trait documentation in `plugin::mod`). Unlike `NinjaPlugin`, this
/// type needs no `base_url` field (see `BASE_URL` above) and no cached
/// token -- the API key comes fresh from `PluginCredentials` on every call.
pub struct LevelPlugin {
    id: String,
}

impl LevelPlugin {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    /// Live fetch of ALL devices of this connection, with pagination walked
    /// internally (see module documentation). Richer than the trait method
    /// `list_systems`, which deliberately stays with the narrow,
    /// plugin-agnostic `ExternalSystem` type (no `ip_address` field there).
    /// Also fetches the group list (`GET /v2/groups`) and resolves
    /// `group_id` per device into a plaintext name server-side (see module
    /// documentation, "Groups" section).
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<LevelDevice>, PluginError> {
        let agent = build_agent();
        let raw_groups = fetch_all_groups(&agent, &credentials.secret)?;
        let group_lookup = build_group_lookup(&raw_groups);
        let raw_devices = fetch_all_devices(&agent, &credentials.secret)?;
        Ok(map_level_devices(&raw_devices, &group_lookup))
    }
}

/// Checks an API key against Level (a lightweight call: one page with
/// `limit=1`), without persisting anything. For
/// `commands::level::test_level_connection`, so users notice a typo in the
/// API key before actually creating a connection (writing credentials to
/// the keyring).
pub fn test_credentials(api_key: &str) -> Result<(), PluginError> {
    let agent = build_agent();
    fetch_devices_page(&agent, api_key, None, 1)?;
    Ok(())
}

impl Plugin for LevelPlugin {
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
        fetch_device_json(&agent, &credentials.secret, external_id)
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Level's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("LevelPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

/// Fetches all pages of `GET /v2/devices` and returns the raw device objects
/// (not yet mapped to `LevelDevice`) as a single, merged list. Breaks off
/// early if `has_more` is missing/`false`, or if `MAX_PAGES` is reached, or
/// if a page provides no usable `id` field for the next cursor (in which
/// case further pagination isn't meaningfully possible).
fn fetch_all_devices(agent: &Agent, api_key: &str) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_devices_page(agent, api_key, cursor.as_deref(), PAGE_LIMIT)?;
        let (data, has_more) = parse_devices_page(&page_json)?;
        let next_cursor = data
            .last()
            .and_then(|d| d["id"].as_str())
            .map(str::to_string);
        all.extend(data);
        if !has_more {
            break;
        }
        match next_cursor {
            Some(id) => cursor = Some(id),
            None => break,
        }
    }
    Ok(all)
}

fn fetch_devices_page(
    agent: &Agent,
    api_key: &str,
    starting_after: Option<&str>,
    limit: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{BASE_URL}/devices");
    let mut request = agent
        .get(&url)
        .header("Authorization", api_key)
        .query("limit", limit.to_string())
        .query("include_network_interfaces", "true");
    if let Some(after) = starting_after {
        request = request.query("starting_after", after);
    }
    let mut response = request.call().map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

/// Fetches all pages of `GET /v2/groups`, following exactly the same
/// pattern as `fetch_all_devices` (same page envelope, same
/// `starting_after` cursor over the last-seen `id`, same `MAX_PAGES` brake
/// against a misbehaving remote end).
fn fetch_all_groups(agent: &Agent, api_key: &str) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_groups_page(agent, api_key, cursor.as_deref(), PAGE_LIMIT)?;
        let (data, has_more) = parse_groups_page(&page_json)?;
        let next_cursor = data
            .last()
            .and_then(|g| g["id"].as_str())
            .map(str::to_string);
        all.extend(data);
        if !has_more {
            break;
        }
        match next_cursor {
            Some(id) => cursor = Some(id),
            None => break,
        }
    }
    Ok(all)
}

fn fetch_groups_page(
    agent: &Agent,
    api_key: &str,
    starting_after: Option<&str>,
    limit: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{BASE_URL}/groups");
    let mut request = agent
        .get(&url)
        .header("Authorization", api_key)
        .query("limit", limit.to_string());
    if let Some(after) = starting_after {
        request = request.query("starting_after", after);
    }
    let mut response = request.call().map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

fn fetch_device_json(
    agent: &Agent,
    api_key: &str,
    external_id: &str,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{BASE_URL}/devices/{external_id}");
    let mut response = agent
        .get(&url)
        .header("Authorization", api_key)
        .query("include_network_interfaces", "true")
        .call()
        .map_err(map_ureq_error)?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(map_ureq_error)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Level-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Level-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extracts the device list and continuation flag from a single Level page
/// response (`GET /v2/devices`: `{"data": [...], "has_more": bool}`,
/// verified via Level's own reference page). Pure function, testable with
/// hardcoded JSON, no real network access needed.
fn parse_devices_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, bool), PluginError> {
    let data = json["data"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete 'data'-Liste in Level-Antwort".to_string())
    })?;
    let has_more = json["has_more"].as_bool().unwrap_or(false);
    Ok((data.clone(), has_more))
}

/// Extracts the group list and continuation flag from a single Level page
/// response (`GET /v2/groups`: `{"data": [...], "has_more": bool}`,
/// verified via Level's own reference page -- identical page envelope as
/// `/v2/devices`). Pure function, testable with hardcoded JSON, analogous
/// to `parse_devices_page`.
fn parse_groups_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, bool), PluginError> {
    let data = json["data"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse(
            "Erwartete 'data'-Liste in Level-Gruppen-Antwort".to_string(),
        )
    })?;
    let has_more = json["has_more"].as_bool().unwrap_or(false);
    Ok((data.clone(), has_more))
}

/// Builds a `group_id -> plaintext name` lookup table from the raw group
/// objects of `GET /v2/groups` (verified: `{"id": "...", "name": "...",
/// "parent_id": ...}`). A group without a usable `id` OR `name` is skipped
/// -- without both fields it doesn't work as a lookup entry, analogous to
/// this module's "skip broken object" pattern (see `map_level_device`).
fn build_group_lookup(groups: &[serde_json::Value]) -> HashMap<String, String> {
    groups
        .iter()
        .filter_map(|g| {
            let id = g["id"].as_str()?.to_string();
            let name = g["name"].as_str()?.to_string();
            Some((id, name))
        })
        .collect()
}

/// Maps a (already merged across all pages) list of raw Level device objects
/// to `LevelDevice` values. Pure function, testable with hardcoded JSON --
/// Level's "List Devices" and "Show Device" endpoints return device objects
/// in exactly the same shape. `group_lookup` (see `build_group_lookup`)
/// resolves the raw `group_id` field per device into a plaintext name.
fn map_level_devices(
    devices: &[serde_json::Value],
    group_lookup: &HashMap<String, String>,
) -> Vec<LevelDevice> {
    devices
        .iter()
        .filter_map(|d| map_level_device(d, group_lookup))
        .collect()
}

/// A single device object from Level's response. Unlike NinjaOne, Level has
/// no separate `displayName` vs. `systemName` distinction, but instead
/// `nickname` (user-defined, can be `null`) and `hostname`. `nickname` wins
/// for the display name, with `hostname` and finally the external ID as
/// fallbacks. A device without a usable `id` is skipped instead of failing
/// the whole call -- a single broken device object shouldn't make the whole
/// list unusable (analogous to `plugin::ninja::map_device`). `group_id` is
/// nullable (`null`/missing means "ungrouped", not an error case);
/// `group_lookup` resolves it into `group_name` if set and known.
fn map_level_device(
    value: &serde_json::Value,
    group_lookup: &HashMap<String, String>,
) -> Option<LevelDevice> {
    let external_id = value["id"].as_str()?.to_string();
    let hostname = value["hostname"].as_str().map(str::to_string);
    let nickname = value["nickname"].as_str().map(str::to_string);
    let name = nickname
        .or_else(|| hostname.clone())
        .unwrap_or_else(|| external_id.clone());
    let ip_address = extract_ip_address(value);
    let group_id = value["group_id"].as_str().map(str::to_string);
    let group_name = group_id
        .as_ref()
        .and_then(|id| group_lookup.get(id).cloned());
    Some(LevelDevice {
        external_id,
        name,
        hostname,
        ip_address,
        group_id,
        group_name,
    })
}

/// Looks up the first non-empty IP address in a device's
/// `network_interfaces` array (only present when
/// `include_network_interfaces=true`) -- `[{..., "ip_addresses": ["..."]},
/// ...]`, verified via Level's response schema. Unlike NinjaOne, Level
/// returns no flat IP field at the top level.
fn extract_ip_address(value: &serde_json::Value) -> Option<String> {
    let interfaces = value["network_interfaces"].as_array()?;
    for interface in interfaces {
        if let Some(addresses) = interface["ip_addresses"].as_array() {
            if let Some(first) = addresses.iter().find_map(|a| a.as_str()) {
                return Some(first.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices_page_with_data_and_has_more() {
        let json = serde_json::json!({
            "data": [{"id": "dev-1"}, {"id": "dev-2"}],
            "has_more": true
        });
        let (data, has_more) = parse_devices_page(&json).unwrap();
        assert_eq!(data.len(), 2);
        assert!(has_more);
    }

    #[test]
    fn parses_devices_page_defaults_has_more_to_false_when_missing() {
        let json = serde_json::json!({"data": []});
        let (data, has_more) = parse_devices_page(&json).unwrap();
        assert!(data.is_empty());
        assert!(!has_more);
    }

    #[test]
    fn parse_devices_page_rejects_missing_data_field() {
        let json = serde_json::json!({"has_more": false});
        let result = parse_devices_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn maps_level_devices_json_array_into_level_devices() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {"id": "dev-1", "hostname": "srv-01.customer.local", "nickname": "Server 01"},
                {"id": "dev-2", "hostname": "fw-edge"}
            ]"#,
        )
        .unwrap();
        let devices = map_level_devices(json.as_array().unwrap(), &HashMap::new());

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "dev-1");
        assert_eq!(devices[0].name, "Server 01");
        assert_eq!(
            devices[0].hostname.as_deref(),
            Some("srv-01.customer.local")
        );
        assert_eq!(devices[0].group_id, None);
        assert_eq!(devices[0].group_name, None);
        assert_eq!(devices[1].external_id, "dev-2");
        assert_eq!(devices[1].name, "fw-edge");
    }

    #[test]
    fn skips_devices_without_a_usable_id() {
        let json = serde_json::json!([{"hostname": "ohne-id"}]);
        let devices = map_level_devices(json.as_array().unwrap(), &HashMap::new());
        assert!(devices.is_empty());
    }

    #[test]
    fn falls_back_to_external_id_when_no_hostname_or_nickname_present() {
        let json = serde_json::json!([{"id": "dev-7"}]);
        let devices = map_level_devices(json.as_array().unwrap(), &HashMap::new());
        assert_eq!(devices[0].name, "dev-7");
        assert_eq!(devices[0].hostname, None);
    }

    #[test]
    fn extracts_first_ip_address_from_network_interfaces() {
        let json = serde_json::json!({
            "id": "dev-1",
            "network_interfaces": [
                {"ip_addresses": ["10.0.0.5", "10.0.0.6"]},
                {"ip_addresses": ["10.0.0.9"]}
            ]
        });
        let devices = map_level_devices(std::slice::from_ref(&json), &HashMap::new());
        assert_eq!(devices[0].ip_address.as_deref(), Some("10.0.0.5"));
    }

    #[test]
    fn skips_interfaces_with_empty_ip_addresses_and_uses_the_next_one() {
        let json = serde_json::json!({
            "id": "dev-1",
            "network_interfaces": [
                {"ip_addresses": []},
                {"ip_addresses": ["10.0.0.9"]}
            ]
        });
        let devices = map_level_devices(std::slice::from_ref(&json), &HashMap::new());
        assert_eq!(devices[0].ip_address.as_deref(), Some("10.0.0.9"));
    }

    #[test]
    fn has_no_ip_address_when_network_interfaces_absent() {
        let json = serde_json::json!({"id": "dev-1"});
        let devices = map_level_devices(std::slice::from_ref(&json), &HashMap::new());
        assert_eq!(devices[0].ip_address, None);
    }

    #[test]
    fn parses_groups_page_with_data_and_has_more() {
        let json = serde_json::json!({
            "data": [{"id": "grp-1", "name": "Werkstatt"}, {"id": "grp-2", "name": "Büro"}],
            "has_more": true
        });
        let (data, has_more) = parse_groups_page(&json).unwrap();
        assert_eq!(data.len(), 2);
        assert!(has_more);
    }

    #[test]
    fn parses_groups_page_defaults_has_more_to_false_when_missing() {
        let json = serde_json::json!({"data": []});
        let (data, has_more) = parse_groups_page(&json).unwrap();
        assert!(data.is_empty());
        assert!(!has_more);
    }

    #[test]
    fn parse_groups_page_rejects_missing_data_field() {
        let json = serde_json::json!({"has_more": false});
        let result = parse_groups_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn builds_group_lookup_from_groups_json_data() {
        let groups = serde_json::json!([
            {"id": "grp-1", "name": "Werkstatt", "parent_id": null},
            {"id": "grp-2", "name": "Büro", "parent_id": "grp-1"}
        ]);
        let lookup = build_group_lookup(groups.as_array().unwrap());
        assert_eq!(lookup.len(), 2);
        assert_eq!(lookup.get("grp-1").map(String::as_str), Some("Werkstatt"));
        assert_eq!(lookup.get("grp-2").map(String::as_str), Some("Büro"));
    }

    #[test]
    fn group_lookup_skips_entries_without_a_usable_id_or_name() {
        let groups = serde_json::json!([{"name": "Ohne Id"}, {"id": "grp-3"}]);
        let lookup = build_group_lookup(groups.as_array().unwrap());
        assert!(lookup.is_empty());
    }

    #[test]
    fn resolves_group_name_from_lookup_when_group_id_present() {
        let mut lookup = HashMap::new();
        lookup.insert("grp-1".to_string(), "Werkstatt".to_string());
        let json = serde_json::json!({"id": "dev-1", "group_id": "grp-1"});
        let devices = map_level_devices(std::slice::from_ref(&json), &lookup);
        assert_eq!(devices[0].group_id.as_deref(), Some("grp-1"));
        assert_eq!(devices[0].group_name.as_deref(), Some("Werkstatt"));
    }

    #[test]
    fn group_name_is_none_when_group_id_present_but_not_in_lookup() {
        let json = serde_json::json!({"id": "dev-1", "group_id": "grp-deleted"});
        let devices = map_level_devices(std::slice::from_ref(&json), &HashMap::new());
        assert_eq!(devices[0].group_id.as_deref(), Some("grp-deleted"));
        assert_eq!(devices[0].group_name, None);
    }

    #[test]
    fn device_group_id_is_none_when_field_is_null_or_absent() {
        let mut lookup = HashMap::new();
        lookup.insert("grp-1".to_string(), "Werkstatt".to_string());

        let json_null = serde_json::json!({"id": "dev-1", "group_id": null});
        let devices_null = map_level_devices(std::slice::from_ref(&json_null), &lookup);
        assert_eq!(devices_null[0].group_id, None);
        assert_eq!(devices_null[0].group_name, None);

        let json_absent = serde_json::json!({"id": "dev-2"});
        let devices_absent = map_level_devices(std::slice::from_ref(&json_absent), &lookup);
        assert_eq!(devices_absent[0].group_id, None);
        assert_eq!(devices_absent[0].group_name, None);
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
        let plugin = LevelPlugin::new("level:acme-123".to_string());
        assert_eq!(plugin.id(), "level:acme-123");
    }
}
