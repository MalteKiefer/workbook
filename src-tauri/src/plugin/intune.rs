//! Real plugin implementation for Microsoft Intune (device management via
//! the Microsoft Graph API), see `docs/PLUGIN_ARCHITECTURE.md` section
//! "Microsoft-Intune-Plugin". Fourth real integration after NinjaOne
//! (`plugin::ninja`), Level.io (`plugin::level`), and Snipe-IT
//! (`plugin::snipeit`). Structurally closest to `plugin::level`: a
//! connection binds directly to exactly one local customer -- one Azure
//! AD/Entra ID tenant is one organization here, no further sub-tenant
//! concept needed (unlike NinjaOne/Snipe-IT's "Organizations"/"Companies"
//! within a single connection) -- but authenticated like `plugin::ninja`:
//! an OAuth2 client-credentials grant, just against Microsoft's identity
//! platform instead of NinjaOne's own, and needing a third credential value
//! (`tenant_id`) in addition to `client_id`/`client_secret`.
//!
//! - **Authentication**: OAuth2 client-credentials grant against Azure AD
//!   (`POST https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token`,
//!   `grant_type=client_credentials`, `client_id`, `client_secret`,
//!   `scope=https://graph.microsoft.com/.default`) -- verified against
//!   Microsoft's own identity platform documentation. Exactly like
//!   `plugin::ninja`, the access token is deliberately not cached across
//!   calls, but fetched fresh per trait-method call -- this app calls plugin
//!   methods rarely/manually, not in a hot loop, so that stays simple and
//!   correct enough for v1.
//! - **Fixed host, no `base_url`**: unlike NinjaOne/Snipe-IT (self-hosted or
//!   region-specific instances), Microsoft Graph's host is always
//!   `graph.microsoft.com` -- so, like Level.io's `BASE_URL` constant, no
//!   user-supplied `base_url` field is needed on `IntuneConnectionMeta`.
//! - **Device list**: `GET {GRAPH_BASE_URL}/v1.0/deviceManagement/managedDevices`,
//!   paginated via `@odata.nextLink` (a full continuation URL in the
//!   response envelope, standard Microsoft Graph pagination -- verified
//!   against Microsoft's own Graph API documentation for this endpoint).
//!   `list_devices` walks all pages internally (up to `MAX_PAGES` pages, as
//!   protection against a misbehaving remote end) and returns a single,
//!   already-merged list -- the caller sees nothing of Graph's pagination,
//!   the same principle as `plugin::level`/`plugin::snipeit`.
//! - **Device details**: `GET {GRAPH_BASE_URL}/v1.0/deviceManagement/managedDevices/{id}`,
//!   passes the response through unmodified as `serde_json::Value`, exactly
//!   like `plugin::level`/`plugin::ninja`.
//! - **Only a subset of fields mapped**: a real `managedDevices` object has
//!   roughly 50 fields; this module only deserializes the ones it actually
//!   uses (`id`, `deviceName`, `operatingSystem`, `osVersion`,
//!   `serialNumber`, `manufacturer`, `model`, `complianceState`,
//!   `lastSyncDateTime`, `userPrincipalName`) -- the same "only map what's
//!   used" convention `plugin::ninja`'s device mapping already follows.
//!   `deviceName` is used as the identifying/display field, analogous to how
//!   Ninja/Level use `hostname` -- it doubles as both `IntuneDevice::name`
//!   and `IntuneDevice::hostname`, since Intune's managed-device object has
//!   no separate physical-hostname field distinct from `deviceName`.
//! - **No IP address**: verified against Microsoft's own official
//!   `managedDevices` schema -- there is no IP address field anywhere on
//!   this object (`wiFiMacAddress` is a MAC address, not an IP address, and
//!   is not otherwise usable as one). `IntuneDevice::ip_address` is
//!   therefore ALWAYS `None` -- not an oversight, the same honestly
//!   verified absence `plugin::snipeit` already documents for
//!   `hostname`/`ip_address` there (see
//!   `docs/PLUGIN_ARCHITECTURE.md`, "Snipe-IT-Plugin" section).
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the exact same dependency as
//!   `plugin::ninja`/`plugin::level`/`plugin::snipeit` (no second HTTP
//!   client in this codebase).
//! - **Credential encoding**: Intune needs three secret values (`tenant_id`,
//!   `client_id`, `client_secret`) -- one more than NinjaOne's two.
//!   `PluginCredentials.secret` is, per the trait contract, a single opaque
//!   string that the plugin interprets itself -- here encoded as JSON
//!   (`serde_json::to_string`/`from_str`) exactly like `plugin::ninja`
//!   encodes its `NinjaCredentials`, just with a third field.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Microsoft Graph's host is fixed -- no region/instance variants like
/// NinjaOne, no self-hosting like Snipe-IT -- hence a constant instead of a
/// configurable `base_url` field on `IntuneConnectionMeta`, analogous to
/// `plugin::level::BASE_URL`.
pub const GRAPH_BASE_URL: &str = "https://graph.microsoft.com";

/// Protection against a misbehaving remote end (an `@odata.nextLink` that
/// never stops appearing): more than `MAX_PAGES` pages are not fetched, the
/// same defensive convention as `plugin::level::MAX_PAGES`/
/// `plugin::snipeit::MAX_PAGES`.
const MAX_PAGES: usize = 50;

/// Non-secret metadata of an Intune connection, as stored in `config.toml`
/// (`Config::intune_connections`). The OAuth2 credentials belong, per the
/// credential principle, exclusively in the OS keyring, never here. Like
/// `LevelConnectionMeta` (and unlike `NinjaConnectionMeta`/
/// `SnipeitConnectionMeta`): one Azure AD/Entra ID tenant is one
/// organization, so a connection here directly carries its `customer_id` --
/// no further granular organization-mapping layer is needed. Deliberately
/// WITHOUT `base_url` -- see `GRAPH_BASE_URL` above.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntuneConnectionMeta {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

/// The three secret values an Intune connection needs to authenticate.
/// `PluginCredentials.secret` is, per the trait contract, a single opaque
/// string that the plugin interprets itself -- here encoded as JSON
/// (`serde_json::to_string`/`from_str`), analogous to
/// `plugin::ninja::NinjaCredentials`, just with a third field (`tenant_id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntuneCredentials {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// A single managed device from `GET /v1.0/deviceManagement/managedDevices`
/// or its `/{id}` single-device counterpart, narrowed down to the fields
/// this module actually uses (see module documentation). Analogous to
/// `plugin::level::LevelDevice`, but without a group concept -- Intune's
/// managed-device object has none, and (unlike Ninja/Snipe-IT) there is no
/// further sub-tenant concept to group by either, since a connection is
/// already bound to exactly one customer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntuneDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    /// ALWAYS `None` -- Microsoft Graph's `managedDevices` response has no
    /// IP address field at all (verified against Microsoft's own official
    /// schema). Not an oversight -- see module documentation.
    pub ip_address: Option<String>,
    pub operating_system: Option<String>,
    pub os_version: Option<String>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub compliance_state: Option<String>,
    pub last_sync_date_time: Option<String>,
    pub user_principal_name: Option<String>,
}

/// A plugin object for exactly one configured Intune connection. `id` here
/// is already the fully qualified identifier (`"intune:<connection_id>"`),
/// so that `Plugin::id()` works unmodified as the `plugin_id`/keyring
/// account (see trait documentation in `plugin::mod`). Unlike `NinjaPlugin`/
/// `SnipeitPlugin`, this type needs no `base_url` field (see
/// `GRAPH_BASE_URL` above) and, like `LevelPlugin`, no cached token -- the
/// OAuth2 credentials come fresh from `PluginCredentials` on every call.
pub struct IntunePlugin {
    id: String,
}

impl IntunePlugin {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    /// Live fetch of ALL managed devices of this connection, with
    /// pagination walked internally (see module documentation). Richer than
    /// the trait method `list_systems`, which deliberately stays with the
    /// narrow, plugin-agnostic `ExternalSystem` type (no `operating_system`/
    /// `compliance_state`/etc. fields there).
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<IntuneDevice>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &creds)?;
        let raw_devices = fetch_all_devices(&agent, &token)?;
        Ok(map_intune_devices(&raw_devices))
    }
}

/// Checks a tenant ID/client ID/client secret triple against Azure AD
/// (OAuth2 client-credentials grant), without persisting anything -- the
/// access token is discarded after a successful fetch. For
/// `commands::intune::test_intune_connection`, so users notice typos before
/// actually creating a connection (writing credentials to the keyring).
pub fn test_credentials(
    tenant_id: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<(), PluginError> {
    let creds = IntuneCredentials {
        tenant_id: tenant_id.to_string(),
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
    };
    let agent = build_agent();
    fetch_access_token(&agent, &creds)?;
    Ok(())
}

impl Plugin for IntunePlugin {
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
        let token = fetch_access_token(&agent, &creds)?;
        fetch_json(
            &agent,
            &format!("{GRAPH_BASE_URL}/v1.0/deviceManagement/managedDevices/{external_id}"),
            &token,
        )
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Intune's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("IntunePlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn parse_credentials(credentials: &PluginCredentials) -> Result<IntuneCredentials, PluginError> {
    serde_json::from_str(&credentials.secret)
        .map_err(|e| PluginError::Authentication(format!("Intune-Zugangsdaten ungültig: {e}")))
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

fn fetch_access_token(agent: &Agent, creds: &IntuneCredentials) -> Result<String, PluginError> {
    let url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        creds.tenant_id
    );
    let mut response = agent
        .post(&url)
        .send_form([
            ("grant_type", "client_credentials"),
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("scope", "https://graph.microsoft.com/.default"),
        ])
        .map_err(map_ureq_error)?;
    let token: TokenResponse = response.body_mut().read_json().map_err(map_ureq_error)?;
    Ok(token.access_token)
}

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

/// Fetches all pages of `GET /v1.0/deviceManagement/managedDevices` and
/// returns the raw device objects (not yet mapped to `IntuneDevice`) as a
/// single, merged list. Follows `@odata.nextLink` -- a full continuation
/// URL, unlike Level's/Snipe-IT's cursor-/offset-query-parameter pagination
/// -- until it's absent, or `MAX_PAGES` is reached (protection against a
/// misbehaving remote end).
fn fetch_all_devices(agent: &Agent, token: &str) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut url = format!("{GRAPH_BASE_URL}/v1.0/deviceManagement/managedDevices");
    for _ in 0..MAX_PAGES {
        let page_json = fetch_json(agent, &url, token)?;
        let (data, next_link) = parse_devices_page(&page_json)?;
        all.extend(data);
        match next_link {
            Some(next) => url = next,
            None => break,
        }
    }
    Ok(all)
}

/// Extracts the device list and continuation URL from a single Graph page
/// response (`{"value": [...], "@odata.nextLink": "..."}`, verified against
/// Microsoft's own official `managedDevices` documentation). Pure function,
/// testable with hardcoded JSON, no real network access needed, analogous to
/// `plugin::level::parse_devices_page`.
fn parse_devices_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, Option<String>), PluginError> {
    let data = json["value"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete 'value'-Liste in Intune-Antwort".to_string())
    })?;
    let next_link = json["@odata.nextLink"].as_str().map(str::to_string);
    Ok((data.clone(), next_link))
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Intune-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Intune-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Maps a (already merged across all pages) list of raw Graph managed-device
/// objects to `IntuneDevice` values. Pure function, testable with hardcoded
/// JSON -- Graph's "List managedDevices" and "Get managedDevice" endpoints
/// return device objects in exactly the same shape.
fn map_intune_devices(devices: &[serde_json::Value]) -> Vec<IntuneDevice> {
    devices.iter().filter_map(map_intune_device).collect()
}

/// A single managed-device object from Graph's `managedDevices` response.
/// `id` missing or not a string -> the device is skipped instead of failing
/// the whole call (a single broken device object shouldn't make the whole
/// list unusable, analogous to `plugin::ninja::map_device`). `deviceName`
/// doubles as both `name` and `hostname` (see module documentation) -- a
/// device without a usable `deviceName` falls back to the external ID for
/// `name`, and `hostname` stays `None`. `ip_address` is always `None` (see
/// module documentation and `IntuneDevice::ip_address`'s own doc comment).
fn map_intune_device(value: &serde_json::Value) -> Option<IntuneDevice> {
    let external_id = value["id"].as_str()?.to_string();
    let device_name = value["deviceName"].as_str().map(str::to_string);
    let name = device_name.clone().unwrap_or_else(|| external_id.clone());
    Some(IntuneDevice {
        external_id,
        name,
        hostname: device_name,
        ip_address: None,
        operating_system: value["operatingSystem"].as_str().map(str::to_string),
        os_version: value["osVersion"].as_str().map(str::to_string),
        serial_number: value["serialNumber"].as_str().map(str::to_string),
        manufacturer: value["manufacturer"].as_str().map(str::to_string),
        model: value["model"].as_str().map(str::to_string),
        compliance_state: value["complianceState"].as_str().map(str::to_string),
        last_sync_date_time: value["lastSyncDateTime"].as_str().map(str::to_string),
        user_principal_name: value["userPrincipalName"].as_str().map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixture modeled directly on Microsoft's own official documentation
    // example response for `GET /v1.0/deviceManagement/managedDevices` (see
    // module documentation for the source), narrowed to the fields this
    // module actually maps.
    fn sample_device_json() -> serde_json::Value {
        serde_json::json!({
            "id": "705c034c-034c-705c-4c03-5c704c035c70",
            "userId": "some-user-id",
            "deviceName": "Device Name value",
            "managedDeviceOwnerType": "company",
            "managementState": "retirePending",
            "enrolledDateTime": "2016-12-31T23:59:43.797191-08:00",
            "lastSyncDateTime": "2017-01-01T00:02:49.3205976-08:00",
            "operatingSystem": "Operating System value",
            "complianceState": "compliant",
            "osVersion": "Os Version value",
            "azureADRegistered": true,
            "emailAddress": "Email Address value",
            "azureADDeviceId": "Azure ADDevice Id value",
            "isSupervised": true,
            "isEncrypted": true,
            "userPrincipalName": "User Principal Name value",
            "model": "Model value",
            "manufacturer": "Manufacturer value",
            "serialNumber": "Serial Number value",
            "wiFiMacAddress": "Wi Fi Mac Address value",
            "userDisplayName": "User Display Name value"
        })
    }

    #[test]
    fn parses_devices_page_with_value_and_next_link() {
        let json = serde_json::json!({
            "value": [{"id": "dev-1"}, {"id": "dev-2"}],
            "@odata.nextLink": "https://graph.microsoft.com/v1.0/deviceManagement/managedDevices?$skiptoken=abc"
        });
        let (data, next_link) = parse_devices_page(&json).unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(
            next_link.as_deref(),
            Some("https://graph.microsoft.com/v1.0/deviceManagement/managedDevices?$skiptoken=abc")
        );
    }

    #[test]
    fn parses_devices_page_without_next_link_as_last_page() {
        let json = serde_json::json!({"value": []});
        let (data, next_link) = parse_devices_page(&json).unwrap();
        assert!(data.is_empty());
        assert_eq!(next_link, None);
    }

    #[test]
    fn parse_devices_page_rejects_missing_value_field() {
        let json = serde_json::json!({"@odata.nextLink": "..."});
        let result = parse_devices_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn maps_full_sample_device_from_official_docs() {
        let devices = map_intune_devices(std::slice::from_ref(&sample_device_json()));
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.external_id, "705c034c-034c-705c-4c03-5c704c035c70");
        assert_eq!(device.name, "Device Name value");
        assert_eq!(device.hostname.as_deref(), Some("Device Name value"));
        assert_eq!(device.ip_address, None);
        assert_eq!(
            device.operating_system.as_deref(),
            Some("Operating System value")
        );
        assert_eq!(device.os_version.as_deref(), Some("Os Version value"));
        assert_eq!(device.serial_number.as_deref(), Some("Serial Number value"));
        assert_eq!(device.manufacturer.as_deref(), Some("Manufacturer value"));
        assert_eq!(device.model.as_deref(), Some("Model value"));
        assert_eq!(device.compliance_state.as_deref(), Some("compliant"));
        assert_eq!(
            device.last_sync_date_time.as_deref(),
            Some("2017-01-01T00:02:49.3205976-08:00")
        );
        assert_eq!(
            device.user_principal_name.as_deref(),
            Some("User Principal Name value")
        );
    }

    #[test]
    fn skips_devices_without_a_usable_id() {
        let json = serde_json::json!([{"deviceName": "ohne-id"}]);
        let devices = map_intune_devices(json.as_array().unwrap());
        assert!(devices.is_empty());
    }

    #[test]
    fn falls_back_to_external_id_when_no_device_name_present() {
        let json = serde_json::json!([{"id": "dev-7"}]);
        let devices = map_intune_devices(json.as_array().unwrap());
        assert_eq!(devices[0].name, "dev-7");
        assert_eq!(devices[0].hostname, None);
    }

    #[test]
    fn ip_address_is_always_none_even_with_wifi_mac_address_present() {
        let devices = map_intune_devices(std::slice::from_ref(&sample_device_json()));
        assert_eq!(devices[0].ip_address, None);
    }

    #[test]
    fn parses_credentials_from_json_secret() {
        let creds = PluginCredentials {
            secret: r#"{"tenant_id":"tid","client_id":"cid","client_secret":"sec"}"#.into(),
        };
        let parsed = parse_credentials(&creds).unwrap();
        assert_eq!(parsed.tenant_id, "tid");
        assert_eq!(parsed.client_id, "cid");
        assert_eq!(parsed.client_secret, "sec");
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
        let plugin = IntunePlugin::new("intune:acme-123".to_string());
        assert_eq!(plugin.id(), "intune:acme-123");
    }
}
