//! Real plugin implementation for Iru (Apple MDM), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Iru plugin". Fourth real
//! integration after NinjaOne (`plugin::ninja`), Level.io (`plugin::level`)
//! and Snipe-IT (`plugin::snipeit`).
//!
//! **Naming note for future readers**: Iru was formerly called "Kandji".
//! The product was rebranded to "Iru", but as of this writing its own public
//! API reference (<https://api-docs.iru.com>) still uses the old
//! `kandji.io`/`api.kandji.io` domain names and still says "Kandji" in
//! several places. That is not a mistake in this module -- `BASE_URL`-style
//! hosts below (`{sub_domain}.api.kandji.io`) are copied verbatim from Iru's
//! own current documentation, not a leftover from before the rebrand.
//!
//! - **Authentication**: static bearer token in the `Authorization` header
//!   (`Authorization: Bearer <token>`) -- a single secret, no OAuth2 grant,
//!   no token exchange. Passed through 1:1 as `PluginCredentials.secret`,
//!   exactly like Level.io/Snipe-IT (no JSON encoding of multiple values
//!   needed, unlike NinjaOne).
//! - **Self-hosted-style, subdomain-per-tenant base URL**: unlike Level.io's
//!   fixed `BASE_URL` constant, Iru is subdomain-per-tenant:
//!   `https://{sub_domain}.api.kandji.io` (US region) or
//!   `https://{sub_domain}.api.eu.kandji.io` (EU region). Treated the same
//!   way `plugin::ninja`/`plugin::snipeit` treat `base_url` -- a
//!   user-supplied field on the connection (`IruConnectionMeta.base_url`),
//!   the user enters their whole API URL including subdomain and region
//!   (e.g. `https://acme.api.kandji.io`); this module deliberately does not
//!   try to split subdomain/region into separate fields, exactly like
//!   `plugin::snipeit` takes Snipe-IT's whole base URL as one string.
//! - **No organization/tenant concept**: Iru's API has no
//!   organization/site/account-scoping endpoints -- a connection (one
//!   subdomain + one bearer token) maps directly to exactly one local
//!   customer (`IruConnectionMeta.customer_id`), the same simple 1:1 model
//!   as Level.io (`plugin::level`), NOT the granular per-tenant mapping
//!   NinjaOne/Snipe-IT need (`NinjaOrgMapping`/`SnipeitCompanyMapping`).
//! - **Device list**: `GET {base_url}/api/v1/devices?limit=300`, verified
//!   live against Iru's current API docs. Optionally supports an `offset`
//!   query parameter for pagination beyond one page. Unlike Snipe-IT's
//!   `{"total": ..., "rows": [...]}` envelope, the response is a **plain
//!   JSON array** -- verified live, confirmed via Iru's own current example
//!   response. `list_devices` walks all pages internally (up to `MAX_PAGES`
//!   pages of `PAGE_LIMIT` devices each, the same defensive-looping
//!   convention as `plugin::snipeit`/`plugin::level`, protection against a
//!   misbehaving remote end) and returns a single, already-merged list --
//!   the caller sees nothing of Iru's pagination. Since there is no `total`
//!   field to check against (unlike Snipe-IT), a page short of `PAGE_LIMIT`
//!   rows is treated as the last page, exactly the same heuristic
//!   `plugin::snipeit::fetch_all_hardware` already falls back to when
//!   Snipe-IT's own `total` field is missing/unusable.
//! - **Polymorphic device object -- fields treated defensively**: Iru's own
//!   docs describe `/api/v1/devices` as polymorphic: "If Windows or Android
//!   management is turned on, additional fields will be returned in the
//!   response. All visible fields based on platform enablement status will
//!   be present for all device types, but values will be blank for
//!   non-applicable devices." Every field here beyond `device_id` is
//!   therefore `Option<String>`, read defensively (`.as_str()`, never an
//!   assumed-present field), never a hard parse failure for a single
//!   missing/blank field.
//! - **No hostname/IP address field**: verified via Iru's own live example
//!   response for `/api/v1/devices` -- the device object has neither a
//!   hostname nor an IP address field, the same situation as Snipe-IT (see
//!   `plugin::snipeit` module docs). `IruDevice.hostname`/`ip_address` are
//!   therefore ALWAYS `None` -- not a guess, an honest, verified omission.
//!   `device_name` is Iru's identifying/display field (like Ninja/Level's
//!   `hostname`); for the "link to existing local system" match-key
//!   convention, this module follows Snipe-IT's own precedent
//!   (`SnipeitPluginSection.tsx::matchKeyForDevice`) since there is equally
//!   no hostname field here: the first available identifying field, in
//!   order `serial_number` then `asset_tag`, matched against the local
//!   system's free-text `hostname` field (frontend concern, see
//!   `IruPluginSection.tsx`).
//! - **Device details**: a single-device GET endpoint exists,
//!   `GET {base_url}/api/v1/devices/{device_id}` -- following the same URL
//!   family as Iru's documented device-action endpoints (e.g.
//!   `.../devices/{device_id}/action/shutdown`). Passes the response through
//!   unmodified as `serde_json::Value`, exactly like every other plugin's
//!   `get_system_details`.
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the exact same dependency as
//!   `plugin::ninja`/`plugin::level`/`plugin::snipeit` (no second HTTP
//!   client in this codebase). Connection/timeout/non-2xx errors map to
//!   `PluginError::Unreachable`, HTTP 401/403 to
//!   `PluginError::Authentication`, unexpected JSON to
//!   `PluginError::UnexpectedResponse`.
//! - **Credential encoding**: Iru needs only a single secret value (the
//!   bearer token), passed through 1:1 as `PluginCredentials.secret` --
//!   like Level.io/Snipe-IT, no JSON encoding of multiple values needed
//!   (unlike NinjaOne).

use serde::{Deserialize, Serialize};
use std::time::Duration;
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Fixed API path suffix appended to the user-supplied `base_url` (see
/// module documentation). Iru is subdomain-per-tenant -- unlike Level.io's
/// fixed `BASE_URL` constant -- so no hardcoded host here, just this shared
/// path suffix, analogous to `plugin::snipeit::API_PATH`.
const API_PATH: &str = "/api/v1";

/// Protection against a misbehaving remote end: more than
/// `MAX_PAGES * PAGE_LIMIT` devices per sync run are not fetched. Chosen the
/// same as `plugin::snipeit::MAX_PAGES` -- an MDM-managed Apple device fleet
/// can realistically number in the thousands.
const MAX_PAGES: usize = 50;
/// Verified live against Iru's own current API docs (`GET
/// /api/v1/devices?limit=300`).
const PAGE_LIMIT: u32 = 300;

/// Non-secret metadata of an Iru connection, as stored in `config.toml`
/// (`Config::iru_connections`). The bearer token belongs, per the credential
/// principle, exclusively in the OS keyring, never here. Unlike
/// `SnipeitConnectionMeta`/`NinjaConnectionMeta` but like
/// `LevelConnectionMeta`: Iru has no organization concept, so a connection
/// here directly carries its `customer_id` -- a connection corresponds to
/// exactly one local customer. Unlike `LevelConnectionMeta` but like
/// `SnipeitConnectionMeta`: Iru is subdomain-per-tenant, so a connection also
/// needs a user-supplied `base_url` (see module documentation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IruConnectionMeta {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
    pub base_url: String,
}

/// A single device from `GET /api/v1/devices` or
/// `GET /api/v1/devices/{device_id}`. `hostname`/`ip_address` are ALWAYS
/// `None` (see module documentation) -- present as fields anyway so this
/// type structurally matches `plugin::ninja::NinjaDevice`/
/// `plugin::level::LevelDevice`/`plugin::snipeit::SnipeitDevice` and a
/// future UI can handle every plugin's device list uniformly.
/// `serial_number`/`asset_tag` are Iru's own natural identification fields
/// (see the "link to existing system" match-key convention in the module
/// documentation); `model`/`platform`/`os_version` are additional
/// display-only fields Iru's device object carries. Every field beyond
/// `external_id`/`name` is `Option<String>` -- Iru's device object is
/// documented as polymorphic across platforms (Mac/Windows/Android), so a
/// field being blank/absent for one platform is expected, not an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IruDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub serial_number: Option<String>,
    pub asset_tag: Option<String>,
    pub model: Option<String>,
    pub platform: Option<String>,
    pub os_version: Option<String>,
}

/// A plugin object for exactly one configured Iru connection. `id` here is
/// already the fully qualified identifier (`"iru:<connection_id>"`), so that
/// `Plugin::id()` works unmodified as the `plugin_id`/keyring account (see
/// trait documentation in `plugin::mod`).
pub struct IruPlugin {
    id: String,
    base_url: String,
}

impl IruPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live fetch of ALL devices of this connection, with pagination walked
    /// internally (see module documentation). Richer than the trait method
    /// `list_systems`, which deliberately stays with the narrow,
    /// plugin-agnostic `ExternalSystem` type (no `serial_number`/
    /// `asset_tag`/`model`/`platform`/`os_version` field there).
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<IruDevice>, PluginError> {
        let agent = build_agent();
        let raw = fetch_all_devices(&agent, &self.base_url, &credentials.secret)?;
        Ok(map_devices_response(&raw))
    }
}

/// Checks a bearer token against Iru (a lightweight call: one page with
/// `limit=1`), without persisting anything. For
/// `commands::iru::test_iru_connection`, so users notice a typo in the base
/// URL/token before actually creating a connection (writing credentials to
/// the keyring) -- exactly the same pattern as
/// `plugin::level::test_credentials`/`plugin::snipeit::test_credentials`.
pub fn test_credentials(base_url: &str, token: &str) -> Result<(), PluginError> {
    let agent = build_agent();
    fetch_devices_page(&agent, base_url, token, 0, 1)?;
    Ok(())
}

impl Plugin for IruPlugin {
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
            &format!("/devices/{external_id}"),
            &credentials.secret,
        )
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Iru's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!(
            "IruPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}",
            self.id
        );
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

fn fetch_devices_page(
    agent: &Agent,
    base_url: &str,
    token: &str,
    offset: u32,
    limit: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{}{API_PATH}/devices", base_url.trim_end_matches('/'));
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

/// Fetches all pages of `GET /api/v1/devices` and returns the raw device
/// objects (not yet mapped to `IruDevice`) as a single, merged list. Unlike
/// `plugin::snipeit::fetch_all_hardware`, there is no `total` field to check
/// against (Iru's response is a plain array, see module documentation) --
/// so, exactly like Snipe-IT's own fallback for when `total` is
/// missing/unusable, a page that comes back with fewer than `PAGE_LIMIT`
/// rows is treated as the last page. Breaks off early once `MAX_PAGES` is
/// reached (protection against a misbehaving remote end).
fn fetch_all_devices(
    agent: &Agent,
    base_url: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut offset: u32 = 0;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_devices_page(agent, base_url, token, offset, PAGE_LIMIT)?;
        let rows = parse_devices_page(&page_json)?;
        let got = rows.len();
        all.extend(rows);
        offset += PAGE_LIMIT;
        if (got as u32) < PAGE_LIMIT {
            break;
        }
    }
    Ok(all)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Iru-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Iru-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extracts the device array from a single Iru device page response. Unlike
/// every other plugin in this codebase, Iru's `/api/v1/devices` response is
/// a **plain JSON array**, not an object with a `data`/`rows` envelope field
/// -- verified live against Iru's own current API docs. Pure function,
/// testable with hardcoded JSON, no real network access needed.
fn parse_devices_page(json: &serde_json::Value) -> Result<Vec<serde_json::Value>, PluginError> {
    json.as_array().cloned().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete JSON-Array in Iru-Geräte-Antwort".to_string())
    })
}

/// Maps a (already merged across all pages) list of raw Iru device objects
/// to `IruDevice` values. Pure function, testable with hardcoded JSON --
/// Iru's "List Devices" and "Get Device" endpoints return device objects in
/// the same shape (verified live).
fn map_devices_response(devices: &[serde_json::Value]) -> Vec<IruDevice> {
    devices.iter().filter_map(map_device).collect()
}

/// A single device object from Iru's `/devices` response. `device_id` is the
/// only field this module treats as required -- every other field is read
/// defensively (`Option<String>`), since Iru documents the device object as
/// polymorphic across platforms (see module documentation): a field being
/// blank/absent for a given device is expected, not an error. The display
/// name falls back from `device_name` (Iru's own name field, can be blank)
/// through `model` and `serial_number` down to the external ID -- the same
/// "first genuinely informative field wins" pattern as
/// `plugin::snipeit::map_hardware_asset`. A device without a usable
/// `device_id` is skipped instead of failing the whole call -- a single
/// broken device object shouldn't make the whole list unusable (analogous to
/// `plugin::level::map_level_device`/`plugin::snipeit::map_hardware_asset`).
fn map_device(value: &serde_json::Value) -> Option<IruDevice> {
    let external_id = value["device_id"].as_str()?.to_string();
    let serial_number = value["serial_number"].as_str().map(str::to_string);
    let asset_tag = value["asset_tag"].as_str().map(str::to_string);
    let model = value["model"].as_str().map(str::to_string);
    let platform = value["platform"].as_str().map(str::to_string);
    let os_version = value["os_version"].as_str().map(str::to_string);
    let raw_name = value["device_name"].as_str().filter(|s| !s.is_empty());
    let name = raw_name
        .or(model.as_deref())
        .or(serial_number.as_deref())
        .unwrap_or(external_id.as_str())
        .to_string();
    Some(IruDevice {
        external_id,
        name,
        hostname: None,
        ip_address: None,
        serial_number,
        asset_tag,
        model,
        platform,
        os_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices_page_as_a_plain_json_array() {
        let json = serde_json::json!([{"device_id": "dev-1"}, {"device_id": "dev-2"}]);
        let rows = parse_devices_page(&json).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn parse_devices_page_rejects_a_non_array_response() {
        let json = serde_json::json!({"data": []});
        let result = parse_devices_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn maps_iru_devices_json_array_into_iru_devices() {
        // Shape verified live against Iru's own current API docs.
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {
                    "device_id": "03f81208-2b6a-4a77-81f5-cf1633bcfb95",
                    "device_name": "accuhive's MacBook Air",
                    "model": "MacBook Air (M1, 2020)",
                    "serial_number": "FVHHFKF7Q6L4",
                    "platform": "Mac",
                    "os_version": "14.4.1"
                },
                {
                    "device_id": "dev-2",
                    "serial_number": "SN-ONLY"
                }
            ]"#,
        )
        .unwrap();

        let devices = map_devices_response(json.as_array().unwrap());

        assert_eq!(devices.len(), 2);
        assert_eq!(
            devices[0].external_id,
            "03f81208-2b6a-4a77-81f5-cf1633bcfb95"
        );
        assert_eq!(devices[0].name, "accuhive's MacBook Air");
        assert_eq!(devices[0].model.as_deref(), Some("MacBook Air (M1, 2020)"));
        assert_eq!(devices[0].serial_number.as_deref(), Some("FVHHFKF7Q6L4"));
        assert_eq!(devices[0].platform.as_deref(), Some("Mac"));
        assert_eq!(devices[0].os_version.as_deref(), Some("14.4.1"));
        assert_eq!(devices[0].hostname, None);
        assert_eq!(devices[0].ip_address, None);
        assert_eq!(devices[1].external_id, "dev-2");
        assert_eq!(devices[1].name, "SN-ONLY");
    }

    #[test]
    fn falls_back_to_model_when_device_name_is_blank() {
        let json =
            serde_json::json!([{"device_id": "dev-1", "device_name": "", "model": "iPhone 15"}]);
        let devices = map_devices_response(json.as_array().unwrap());
        assert_eq!(devices[0].name, "iPhone 15");
    }

    #[test]
    fn falls_back_to_external_id_when_nothing_else_present() {
        let json = serde_json::json!([{"device_id": "dev-9"}]);
        let devices = map_devices_response(json.as_array().unwrap());
        assert_eq!(devices[0].name, "dev-9");
        assert_eq!(devices[0].serial_number, None);
        assert_eq!(devices[0].asset_tag, None);
    }

    #[test]
    fn skips_devices_without_a_usable_device_id() {
        let json = serde_json::json!([{"device_name": "Ohne ID"}]);
        let devices = map_devices_response(json.as_array().unwrap());
        assert!(devices.is_empty());
    }

    #[test]
    fn keeps_asset_tag_when_present() {
        let json = serde_json::json!([{"device_id": "dev-1", "asset_tag": "AT-042"}]);
        let devices = map_devices_response(json.as_array().unwrap());
        assert_eq!(devices[0].asset_tag.as_deref(), Some("AT-042"));
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
        let plugin = IruPlugin::new(
            "iru:acme-123".to_string(),
            "https://acme.api.kandji.io".to_string(),
        );
        assert_eq!(plugin.id(), "iru:acme-123");
    }
}
