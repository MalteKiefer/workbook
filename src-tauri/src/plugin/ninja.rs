//! Real plugin implementation for NinjaOne (formerly NinjaRMM), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "NinjaOne plugin". Structure and
//! signatures follow `plugin::dummy::DummyPlugin`, but talk over real HTTPS
//! (crate `ureq`, synchronous, no async runtime) to NinjaOne's public REST
//! API.
//!
//! A `NinjaPlugin` belongs to exactly one "Ninja connection" created by the
//! user (`NinjaConnectionMeta`, a set of OAuth2 credentials for exactly one
//! Ninja tenant). IMPORTANT: a connection is NOT bound to exactly one local
//! customer -- a single Ninja tenant itself models multiple "Organizations",
//! e.g. because the user creating the connection is themselves an MSP and
//! keeps several of their own customers as separate organizations within it.
//! That's why this module, in addition to the devices (`list_systems`/
//! `list_devices`), also provides the organization list (`list_organizations`)
//! of a connection; which organization corresponds to which local customer
//! (if any) is a separate, granular mapping (`NinjaOrgMapping`,
//! `Config::ninja_org_mappings`), maintained via `commands::plugins`. `id`
//! here is not a fixed constant like in `DummyPlugin`, but assigned per
//! connection (see `commands::plugins`, which uses the fully qualified
//! `"ninja:<connection_id>"` identifier both as the keyring account and as
//! `external_refs.plugin_id`).
//!
//! Authentication runs over the OAuth2 client-credentials grant. The access
//! token is deliberately not cached across multiple calls -- this app calls
//! plugin methods rarely/manually, not in a hot loop, so "fetch fresh per
//! method call" is simple and correct enough for v1.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Non-secret metadata of a Ninja connection, as stored in `config.toml`
/// (`Config::ninja_connections`). Client ID/secret belong, per the
/// credential principle, exclusively in the OS keyring, never here.
/// Deliberately WITHOUT `customer_id` -- a connection is a Ninja tenant, not
/// a local customer; which Ninja "Organization" within this tenant
/// corresponds to which local customer is tracked granularly in
/// `NinjaOrgMapping`/`Config::ninja_org_mappings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NinjaConnectionMeta {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

/// Mapping of a single Ninja "Organization" (within a connection) to a local
/// customer. Lives in `Config::ninja_org_mappings`, not in
/// `NinjaConnectionMeta` -- a connection can see multiple organizations,
/// each of which can be mapped independently (or left unmapped).
/// `organization_name` is stored in addition to `organization_id` so that,
/// e.g., a future UI list can display a readable name without another live
/// call against Ninja.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NinjaOrgMapping {
    pub connection_id: String,
    pub organization_id: String,
    pub organization_name: String,
    pub customer_id: i64,
}

/// An organization reported by `GET /v2/organizations`. Separate from
/// `ExternalSystem` (those are devices) -- its own, small shape type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NinjaOrganization {
    pub id: String,
    pub name: String,
}

/// A single device from `GET /v2/devices`, enriched with organization
/// membership (`organizationId`) and IP address (`ipAddresses`) -- both are
/// needed by `commands::plugins::sync_ninja_connection` for grouping by
/// organization and for display, which the generic, plugin-agnostic
/// `ExternalSystem` type from `plugin::mod` deliberately does not provide
/// (it stays the narrow, trait-generic minimum that `DummyPlugin` must also
/// be able to satisfy). Hence its own, Ninja-specific type instead of
/// extending `ExternalSystem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NinjaDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub organization_id: String,
}

/// The two secret values a Ninja connection needs to authenticate.
/// `PluginCredentials.secret` is, per the trait contract, a single opaque
/// string that the plugin interprets itself -- here encoded as JSON
/// (`serde_json::to_string`/`from_str`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NinjaCredentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// A plugin object for exactly one configured Ninja connection. `id` here is
/// already the fully qualified identifier (`"ninja:<connection_id>"`), so
/// that `Plugin::id()` works unmodified as the `plugin_id`/keyring account
/// (see trait documentation in `plugin::mod`).
pub struct NinjaPlugin {
    id: String,
    base_url: String,
}

impl NinjaPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live fetch of this connection's organization list
    /// (`GET /v2/organizations`). Separate from the `Plugin` trait method
    /// `list_systems`, because organizations are not devices and the generic
    /// trait has no room for that.
    pub fn list_organizations(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<NinjaOrganization>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        let json = fetch_json(&agent, &self.base_url, "/v2/organizations", &token)?;
        map_organizations_response(&json)
    }

    /// Live fetch of all this connection's devices, enriched with
    /// organization membership and IP address (see `NinjaDevice`). Richer
    /// than the trait method `list_systems`, which deliberately stays with
    /// the narrow, plugin-agnostic `ExternalSystem` type.
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<NinjaDevice>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        let json = fetch_json(&agent, &self.base_url, "/v2/devices", &token)?;
        map_ninja_devices_response(&json)
    }
}

/// Checks a client ID/secret pair against NinjaOne (OAuth2 client-credentials
/// grant), without persisting anything -- the access token is discarded
/// after a successful fetch. For `commands::plugins::test_ninja_connection`,
/// so users notice typos in the base URL/credentials before actually
/// creating a connection (writing credentials to the keyring).
pub fn test_credentials(
    base_url: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<(), PluginError> {
    let creds = NinjaCredentials {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
    };
    let agent = build_agent();
    fetch_access_token(&agent, base_url, &creds)?;
    Ok(())
}

impl Plugin for NinjaPlugin {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn list_systems(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<ExternalSystem>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        let json = fetch_json(&agent, &self.base_url, "/v2/devices", &token)?;
        map_devices_response(&json)
    }

    fn get_system_details(
        &self,
        credentials: &PluginCredentials,
        external_id: &str,
    ) -> Result<serde_json::Value, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        fetch_json(
            &agent,
            &self.base_url,
            &format!("/v2/device/{external_id}"),
            &token,
        )
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // NinjaOne's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("NinjaPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn parse_credentials(credentials: &PluginCredentials) -> Result<NinjaCredentials, PluginError> {
    serde_json::from_str(&credentials.secret)
        .map_err(|e| PluginError::Authentication(format!("Ninja-Zugangsdaten ungültig: {e}")))
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
    creds: &NinjaCredentials,
) -> Result<String, PluginError> {
    let url = format!("{}/ws/oauth/token", base_url.trim_end_matches('/'));
    let mut response = agent
        .post(&url)
        .send_form([
            ("grant_type", "client_credentials"),
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("scope", "monitoring"),
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

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Ninja-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Ninja-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Maps the JSON list returned by `GET /v2/devices` to `ExternalSystem`
/// values. Deliberately extracted as its own, pure function -- can be
/// tested with a fixed JSON string, without any real network access.
fn map_devices_response(json: &serde_json::Value) -> Result<Vec<ExternalSystem>, PluginError> {
    let array = json.as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete JSON-Liste von Geräten".to_string())
    })?;
    Ok(array.iter().filter_map(map_device).collect())
}

/// A single device object from NinjaOne's `/v2/devices` response. `id` is
/// missing or has an unexpected shape -> the device is skipped instead of
/// failing the whole call (a single broken device object shouldn't make the
/// whole list unusable).
fn map_device(value: &serde_json::Value) -> Option<ExternalSystem> {
    let external_id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let system_name = value["systemName"].as_str();
    let display_name = value["displayName"].as_str();
    let hostname = value["hostname"]
        .as_str()
        .or(system_name)
        .map(str::to_string);
    let name = display_name
        .or(system_name)
        .unwrap_or(external_id.as_str())
        .to_string();
    Some(ExternalSystem {
        external_id,
        name,
        hostname,
    })
}

/// Maps the JSON list returned by `GET /v2/organizations` to
/// `NinjaOrganization` values. Pure function, testable on its own with
/// hardcoded JSON, analogous to `map_devices_response`.
fn map_organizations_response(
    json: &serde_json::Value,
) -> Result<Vec<NinjaOrganization>, PluginError> {
    let array = json.as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete JSON-Liste von Organisationen".to_string())
    })?;
    Ok(array.iter().filter_map(map_organization).collect())
}

/// A single organization object from NinjaOne's `/v2/organizations` response
/// (`{"id": <number>, "name": "..."}`, per NinjaOne's public API
/// specification). If `id` is missing or has an unexpected shape -> the
/// organization is skipped instead of failing the whole call, analogous to
/// `map_device`.
fn map_organization(value: &serde_json::Value) -> Option<NinjaOrganization> {
    let id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let name = value["name"].as_str().unwrap_or(id.as_str()).to_string();
    Some(NinjaOrganization { id, name })
}

/// Maps the JSON list returned by `GET /v2/devices` to `NinjaDevice` values
/// (including organization membership and IP address) -- the counterpart to
/// `map_devices_response`, which only populates the narrow, trait-generic
/// `ExternalSystem` type. Pure function, testable on its own, no real
/// network access needed.
fn map_ninja_devices_response(json: &serde_json::Value) -> Result<Vec<NinjaDevice>, PluginError> {
    let array = json.as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete JSON-Liste von Geräten".to_string())
    })?;
    Ok(array.iter().filter_map(map_ninja_device).collect())
}

/// A single device object from NinjaOne's `/v2/devices` response, enriched
/// with `organizationId` (per NinjaOne's public API specification, an
/// integer field that assigns each device to exactly one organization) and
/// `ipAddresses` (an array of IP address strings; the primary/first address
/// is used, with `publicIP` as a fallback if `ipAddresses` is empty or
/// absent). A device without a usable `id` OR without `organizationId` is
/// skipped -- without organization membership it can't be meaningfully
/// grouped, and a single broken device object shouldn't make the whole list
/// unusable.
fn map_ninja_device(value: &serde_json::Value) -> Option<NinjaDevice> {
    let external_id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let organization_id = match &value["organizationId"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let system_name = value["systemName"].as_str();
    let display_name = value["displayName"].as_str();
    let hostname = value["hostname"]
        .as_str()
        .or(system_name)
        .map(str::to_string);
    let name = display_name
        .or(system_name)
        .unwrap_or(external_id.as_str())
        .to_string();
    let ip_address = value["ipAddresses"]
        .as_array()
        .and_then(|addresses| addresses.first())
        .and_then(|first| first.as_str())
        .map(str::to_string)
        .or_else(|| value["publicIP"].as_str().map(str::to_string));
    Some(NinjaDevice {
        external_id,
        name,
        hostname,
        ip_address,
        organization_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_devices_json_array_into_external_systems() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {"id": 101, "systemName": "SRV-01", "hostname": "srv-01.customer.local", "displayName": "Server 01"},
                {"id": 202, "systemName": "FW-EDGE"}
            ]"#,
        )
        .unwrap();

        let systems = map_devices_response(&json).unwrap();

        assert_eq!(systems.len(), 2);
        assert_eq!(systems[0].external_id, "101");
        assert_eq!(systems[0].name, "Server 01");
        assert_eq!(
            systems[0].hostname.as_deref(),
            Some("srv-01.customer.local")
        );
        assert_eq!(systems[1].external_id, "202");
        assert_eq!(systems[1].name, "FW-EDGE");
        assert_eq!(systems[1].hostname.as_deref(), Some("FW-EDGE"));
    }

    #[test]
    fn rejects_non_array_json() {
        let json = serde_json::json!({"not": "an array"});
        let result = map_devices_response(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn skips_devices_without_a_usable_id() {
        let json = serde_json::json!([{"systemName": "OhneId"}]);
        let systems = map_devices_response(&json).unwrap();
        assert!(systems.is_empty());
    }

    #[test]
    fn falls_back_to_external_id_when_no_name_field_present() {
        let json = serde_json::json!([{"id": 7}]);
        let systems = map_devices_response(&json).unwrap();
        assert_eq!(systems[0].name, "7");
        assert_eq!(systems[0].hostname, None);
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
        let plugin = NinjaPlugin::new(
            "ninja:acme-123".to_string(),
            "https://app.ninjarmm.com".to_string(),
        );
        assert_eq!(plugin.id(), "ninja:acme-123");
    }

    #[test]
    fn maps_organizations_json_array_into_ninja_organizations() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {"id": 1, "name": "ACME Hauptsitz"},
                {"id": 2, "name": "ACME Zweigstelle"}
            ]"#,
        )
        .unwrap();

        let organizations = map_organizations_response(&json).unwrap();

        assert_eq!(organizations.len(), 2);
        assert_eq!(
            organizations[0],
            NinjaOrganization {
                id: "1".to_string(),
                name: "ACME Hauptsitz".to_string()
            }
        );
        assert_eq!(
            organizations[1],
            NinjaOrganization {
                id: "2".to_string(),
                name: "ACME Zweigstelle".to_string()
            }
        );
    }

    #[test]
    fn organizations_rejects_non_array_json() {
        let json = serde_json::json!({"not": "an array"});
        let result = map_organizations_response(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn organization_falls_back_to_id_when_no_name_field_present() {
        let json = serde_json::json!([{"id": 9}]);
        let organizations = map_organizations_response(&json).unwrap();
        assert_eq!(organizations[0].name, "9");
    }

    #[test]
    fn organization_skips_entries_without_a_usable_id() {
        let json = serde_json::json!([{"name": "OhneId"}]);
        let organizations = map_organizations_response(&json).unwrap();
        assert!(organizations.is_empty());
    }

    #[test]
    fn maps_ninja_devices_json_array_with_organization_and_ip() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {
                    "id": 101,
                    "organizationId": 1,
                    "systemName": "SRV-01",
                    "hostname": "srv-01.customer.local",
                    "displayName": "Server 01",
                    "ipAddresses": ["10.0.0.5", "10.0.0.6"]
                },
                {
                    "id": 202,
                    "organizationId": 2,
                    "systemName": "FW-EDGE",
                    "publicIP": "203.0.113.9"
                }
            ]"#,
        )
        .unwrap();

        let devices = map_ninja_devices_response(&json).unwrap();

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "101");
        assert_eq!(devices[0].organization_id, "1");
        assert_eq!(devices[0].name, "Server 01");
        assert_eq!(devices[0].ip_address.as_deref(), Some("10.0.0.5"));
        assert_eq!(devices[1].external_id, "202");
        assert_eq!(devices[1].organization_id, "2");
        assert_eq!(devices[1].ip_address.as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn ninja_device_rejects_non_array_json() {
        let json = serde_json::json!({"not": "an array"});
        let result = map_ninja_devices_response(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn ninja_device_skips_devices_without_organization_id() {
        let json = serde_json::json!([{"id": 5, "systemName": "OhneOrg"}]);
        let devices = map_ninja_devices_response(&json).unwrap();
        assert!(devices.is_empty());
    }

    #[test]
    fn ninja_device_has_no_ip_address_when_neither_field_present() {
        let json = serde_json::json!([{"id": 5, "organizationId": 1, "systemName": "OhneIp"}]);
        let devices = map_ninja_devices_response(&json).unwrap();
        assert_eq!(devices[0].ip_address, None);
    }
}
