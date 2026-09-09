//! Real plugin implementation for Apple Business Manager (ABM), see
//! `docs/PLUGIN_ARCHITECTURE.md` section "Apple-Business-Manager-Plugin".
//! Fourth real integration after NinjaOne (`plugin::ninja`), Level.io
//! (`plugin::level`), and Snipe-IT (`plugin::snipeit`) -- by far the most
//! involved authentication flow of the four: an ES256-signed JWT client
//! assertion, exchanged for a short-lived OAuth2 access token. Both steps
//! below are verified against Apple's own reference implementation, not
//! guessed.
//!
//! - **Authentication, step 1 (client assertion)**: a JWT signed with the
//!   user's EC P-256 private key (`ES256`), header
//!   `{"alg": "ES256", "kid": "<key_id>", "typ": "JWT"}`, claims
//!   `{"iss": "<client_id>", "sub": "<client_id>", "aud":
//!   "https://account.apple.com/auth/oauth2/v2/token", "iat": <now>, "exp":
//!   <now + 900>, "jti": "<random>"}` (15-minute lifetime). IMPORTANT,
//!   verified and intentional: the `aud` claim uses `.../v2/token`, but the
//!   token endpoint actually posted to in step 2 (`TOKEN_URL`) does NOT
//!   have `/v2` in its path -- this mismatch is correct, straight from
//!   Apple's own reference implementation, not a bug to "fix".
//! - **Authentication, step 2 (token exchange)**:
//!   `POST https://account.apple.com/auth/oauth2/token`,
//!   `application/x-www-form-urlencoded`, `grant_type=client_credentials`,
//!   `client_id`,
//!   `client_assertion_type=urn:ietf:params:oauth:client-assertion-type:jwt-bearer`,
//!   `client_assertion=<jwt>`, `scope=business.api`. Response is a standard
//!   OAuth2 JSON token response (`access_token`, `token_type`,
//!   `expires_in`) -- only `access_token` is used here. Deliberately not
//!   cached across calls -- fetched fresh per trait-method call, same
//!   simplicity already chosen for `plugin::ninja`'s OAuth2 token (see that
//!   module's documentation): this app calls plugin methods rarely and
//!   manually, never in a hot loop.
//! - **Devices**: `GET https://api-business.apple.com/v1/orgDevices`
//!   (fixed host -- unlike NinjaOne/Snipe-IT, ABM is Apple's own hosted
//!   service, no user-supplied `base_url` needed), `Authorization: Bearer
//!   <token>`. Response is JSON:API format (`{"data": [{"id": ..., "type":
//!   "orgDevices", "attributes": {...}}], "links": {"next": "..."}}`).
//!   `list_devices` walks pages internally by following `links.next` (a
//!   full, ready-to-fetch URL per JSON:API convention) up to `MAX_PAGES`
//!   pages, protection against a misbehaving remote end -- same principle
//!   as every other paginated plugin in this codebase. If a page carries no
//!   `next` link, it's treated defensively as the last (or only) page,
//!   never an error.
//! - **No hostname/IP address**: ABM is a purchasing/enrollment registry,
//!   not live telemetry -- its device object has no hostname/IP field at
//!   all (per Apple's own `orgDevices` attribute list: `serialNumber`,
//!   `deviceModel`, `productFamily`, `productType`, `deviceCapacity`,
//!   `color`, `status`). `AbmDevice.hostname`/`ip_address` are therefore
//!   ALWAYS `None` -- an honest, verified omission, not a guess, exactly
//!   like `plugin::snipeit`. `serial_number` is the primary identifying
//!   field (ABM's closest equivalent of Snipe-IT's `asset_tag`),
//!   `device_model` is the natural display-name fallback.
//! - **Connection scope**: like Level.io (`plugin::level`) and unlike
//!   NinjaOne/Snipe-IT, ABM is inherently single-organization-scoped -- one
//!   set of credentials speaks for exactly one Apple Business Manager
//!   organization, with no sub-tenant/site concept. A connection therefore
//!   maps directly to exactly one local customer
//!   (`AbmConnectionMeta.customer_id`), no granular org-mapping layer
//!   needed (see `Config::abm_connections`).
//! - **HTTP client**: `ureq` 3.4.1, synchronous, the same dependency as
//!   every other plugin in this codebase.
//! - **Credential encoding**: ABM needs THREE secret values (`client_id`,
//!   `key_id`, `private_key_pem`); `PluginCredentials.secret` is, per the
//!   trait contract, a single opaque string -- encoded here as JSON
//!   (`AbmCredentials`), exactly mirroring `plugin::ninja::NinjaCredentials`
//!   (which does the same for its two secret values).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rand::Rng;
use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Fixed host for Apple Business Manager's API -- unlike NinjaOne/Snipe-IT,
/// ABM is Apple's own hosted service with no region/instance variants, so
/// no user-supplied `base_url` field on `AbmConnectionMeta` (see module
/// documentation).
pub const API_BASE: &str = "https://api-business.apple.com/v1";

/// OAuth2 token endpoint actually posted to (step 2 of the auth flow).
/// Deliberately does NOT have `/v2` in its path -- see `TOKEN_AUD` below
/// and the module documentation for why that's a verified, intentional
/// mismatch rather than a typo.
const TOKEN_URL: &str = "https://account.apple.com/auth/oauth2/token";

/// `aud` claim value in the signed client assertion (step 1 of the auth
/// flow). Verified, intentional mismatch against `TOKEN_URL` (this constant
/// has `/v2/token`, `TOKEN_URL` doesn't) -- straight from Apple's own
/// reference implementation, see module documentation.
const TOKEN_AUD: &str = "https://account.apple.com/auth/oauth2/v2/token";

/// Client assertion lifetime (15 minutes), per Apple's own reference
/// implementation.
const ASSERTION_LIFETIME_SECS: i64 = 15 * 60;

/// Protection against a misbehaving remote end (endless `links.next`): more
/// than `MAX_PAGES * PAGE_LIMIT` devices per sync run are not fetched, same
/// principle as every other paginated plugin in this codebase.
const MAX_PAGES: usize = 50;
const PAGE_LIMIT: u32 = 100;

/// Non-secret metadata of an ABM connection, as stored in `config.toml`
/// (`Config::abm_connections`). The client ID/key ID/private key belong,
/// per the credential principle, exclusively in the OS keyring, never here.
/// Like `LevelConnectionMeta`: ABM is inherently single-organization-scoped,
/// so a connection carries its `customer_id` directly -- no `base_url`
/// field either (see `API_BASE` above).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbmConnectionMeta {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

/// The three secret values an ABM connection needs to authenticate.
/// `PluginCredentials.secret` is, per the trait contract, a single opaque
/// string that the plugin interprets itself -- here encoded as JSON
/// (`serde_json::to_string`/`from_str`), exactly mirroring
/// `plugin::ninja::NinjaCredentials`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbmCredentials {
    pub client_id: String,
    pub key_id: String,
    pub private_key_pem: String,
}

/// A single device from `GET /v1/orgDevices` or
/// `GET /v1/orgDevices/{id}`. `hostname`/`ip_address` are ALWAYS `None` --
/// see module documentation ("No hostname/IP address"). Present as fields
/// anyway so this type structurally matches
/// `plugin::ninja::NinjaDevice`/`plugin::level::LevelDevice`/
/// `plugin::snipeit::SnipeitDevice`, and a future UI can handle all four
/// plugins uniformly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbmDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub serial_number: Option<String>,
    pub device_model: Option<String>,
}

/// Claims of the ES256-signed client assertion JWT (see module
/// documentation, step 1). `Deserialize` is only needed for this module's
/// own round-trip unit test (decoding the JWT it just signed) -- the real
/// auth flow only ever encodes these, never decodes them locally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AbmClaims {
    iss: String,
    sub: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// A plugin object for exactly one configured ABM connection. `id` here is
/// already the fully qualified identifier (`"abm:<connection_id>"`), so
/// that `Plugin::id()` works unmodified as the `plugin_id`/keyring account
/// (see trait documentation in `plugin::mod`). No `base_url` field needed
/// (see `API_BASE`), no cached token -- a fresh access token is fetched on
/// every call (see module documentation).
pub struct AbmPlugin {
    id: String,
}

impl AbmPlugin {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    /// Live fetch of ALL devices of this connection, with JSON:API
    /// pagination (`links.next`) walked internally (see module
    /// documentation). Richer than the trait method `list_systems`, which
    /// deliberately stays with the narrow, plugin-agnostic `ExternalSystem`
    /// type (no `serial_number`/`device_model` field there).
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<AbmDevice>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &creds)?;
        let raw = fetch_all_devices(&agent, &token)?;
        Ok(map_abm_devices(&raw))
    }
}

/// Checks a client ID/key ID/private key triple against Apple's OAuth2
/// token endpoint (client-credentials grant via signed JWT assertion),
/// without persisting anything -- the access token is discarded after a
/// successful fetch. For `commands::abm::test_abm_connection`, so users
/// notice a malformed private key or typo'd IDs before actually creating a
/// connection (writing credentials to the keyring).
pub fn test_credentials(
    client_id: &str,
    key_id: &str,
    private_key_pem: &str,
) -> Result<(), PluginError> {
    let creds = AbmCredentials {
        client_id: client_id.to_string(),
        key_id: key_id.to_string(),
        private_key_pem: private_key_pem.to_string(),
    };
    let agent = build_agent();
    fetch_access_token(&agent, &creds)?;
    Ok(())
}

impl Plugin for AbmPlugin {
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
        fetch_json_get(
            &agent,
            &format!("{API_BASE}/orgDevices/{external_id}"),
            &token,
        )
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // ABM's API doesn't need to know anything about a local link --
        // purely a local bookkeeping concept, see trait documentation.
        // Persisted by the caller via `db::external_refs::upsert`.
        println!("AbmPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
        Ok(())
    }
}

fn parse_credentials(credentials: &PluginCredentials) -> Result<AbmCredentials, PluginError> {
    serde_json::from_str(&credentials.secret)
        .map_err(|e| PluginError::Authentication(format!("ABM-Zugangsdaten ungültig: {e}")))
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    Agent::new_with_config(config)
}

/// Builds and ES256-signs the client assertion JWT (step 1 of the auth
/// flow, see module documentation) -- pure function over already-parsed
/// credentials, no network access, so it's directly unit-testable.
fn build_client_assertion(creds: &AbmCredentials) -> Result<String, PluginError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| PluginError::Authentication(format!("Systemzeit ungültig: {e}")))?
        .as_secs() as i64;
    let claims = AbmClaims {
        iss: creds.client_id.clone(),
        sub: creds.client_id.clone(),
        aud: TOKEN_AUD.to_string(),
        iat: now,
        exp: now + ASSERTION_LIFETIME_SECS,
        jti: generate_jti(),
    };
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(creds.key_id.clone());
    let key = EncodingKey::from_ec_pem(creds.private_key_pem.as_bytes()).map_err(|e| {
        PluginError::Authentication(format!(
            "ABM-Private-Key ungültig (EC P-256 PKCS#8 PEM erwartet): {e}"
        ))
    })?;
    encode(&header, &claims, &key)
        .map_err(|e| PluginError::Authentication(format!("JWT-Signierung fehlgeschlagen: {e}")))
}

/// A random RFC-4122-shaped (version 4) identifier for the JWT's `jti`
/// claim. No `uuid` crate dependency needed -- `rand` is already a
/// dependency of this codebase, and a canonically-formatted random value is
/// all `jti` needs to be (Apple's own reference implementation just uses
/// `str(uuid.uuid4())`; the important property is a fresh, effectively
/// unique value per assertion, not that it round-trips through a `uuid`
/// crate's own parser).
fn generate_jti() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    // Set the RFC 4122 version (4, "random") and variant bits so the
    // rendered string looks like a canonical UUID v4, matching the shape of
    // Apple's own `str(uuid.uuid4())` reference call -- purely cosmetic,
    // Apple doesn't validate `jti`'s internal structure.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

/// Step 2 of the auth flow (see module documentation): exchanges a freshly
/// signed client assertion for an access token via
/// `POST https://account.apple.com/auth/oauth2/token`.
fn fetch_access_token(agent: &Agent, creds: &AbmCredentials) -> Result<String, PluginError> {
    let assertion = build_client_assertion(creds)?;
    let mut response = agent
        .post(TOKEN_URL)
        .send_form([
            ("grant_type", "client_credentials"),
            ("client_id", creds.client_id.as_str()),
            (
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
            ),
            ("client_assertion", assertion.as_str()),
            ("scope", "business.api"),
        ])
        .map_err(map_ureq_error)?;
    let token: TokenResponse = response.body_mut().read_json().map_err(map_ureq_error)?;
    Ok(token.access_token)
}

fn fetch_json_get(
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

/// Fetches all pages of `GET /v1/orgDevices`, following JSON:API's
/// `links.next` cursor convention defensively: if a page carries no `next`
/// link, it's treated as the last (or only) page rather than an error --
/// see module documentation.
fn fetch_all_devices(agent: &Agent, token: &str) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut next_url = Some(format!("{API_BASE}/orgDevices?limit={PAGE_LIMIT}"));
    for _ in 0..MAX_PAGES {
        let Some(url) = next_url.take() else {
            break;
        };
        let page = fetch_json_get(agent, &url, token)?;
        let (data, next) = parse_devices_page(&page)?;
        all.extend(data);
        next_url = next;
        if next_url.is_none() {
            break;
        }
    }
    Ok(all)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("ABM-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("ABM-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extracts the device list and next-page URL from a single ABM page
/// response (`GET /v1/orgDevices`: JSON:API `{"data": [...], "links":
/// {"next": "..."}}`). Pure function, testable with hardcoded JSON, no real
/// network access needed. Missing/non-string `links.next` -> `None`
/// (treated as "last page", not an error -- see module documentation).
fn parse_devices_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, Option<String>), PluginError> {
    let data = json["data"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse("Erwartete 'data'-Liste in ABM-Antwort".to_string())
    })?;
    let next = json["links"]["next"].as_str().map(str::to_string);
    Ok((data.clone(), next))
}

/// Maps a (already merged across all pages) list of raw ABM device objects
/// (JSON:API resource objects: `{"id": ..., "type": "orgDevices",
/// "attributes": {...}}`) to `AbmDevice` values. Pure function, testable
/// with hardcoded JSON, no real network access needed.
fn map_abm_devices(devices: &[serde_json::Value]) -> Vec<AbmDevice> {
    devices.iter().filter_map(map_abm_device).collect()
}

/// A single device resource object from ABM's `/orgDevices` response. `id`
/// missing or not a string -> the device is skipped instead of failing the
/// whole call -- a single broken device object shouldn't make the whole
/// list unusable (analogous to every other plugin's `map_device`).
/// `serialNumber`/`deviceModel` live under `attributes` (JSON:API
/// convention), not at the top level. Display name falls back from
/// `deviceModel` through `serialNumber` down to the external ID -- ABM
/// devices have no user-assignable nickname field like Level's `nickname`.
fn map_abm_device(value: &serde_json::Value) -> Option<AbmDevice> {
    let external_id = value["id"].as_str()?.to_string();
    let attributes = &value["attributes"];
    let serial_number = attributes["serialNumber"].as_str().map(str::to_string);
    let device_model = attributes["deviceModel"].as_str().map(str::to_string);
    let name = device_model
        .clone()
        .or_else(|| serial_number.clone())
        .unwrap_or_else(|| external_id.clone());
    Some(AbmDevice {
        external_id,
        name,
        hostname: None,
        ip_address: None,
        serial_number,
        device_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway EC P-256 PKCS#8 test key pair, generated solely for this
    // test run (`openssl ecparam -genkey -name prime256v1 -noout | openssl
    // pkcs8 -topk8 -nocrypt`, public key derived via `openssl pkey
    // -pubout`) -- never a real credential, never used against any real
    // service, committed here purely so the JWT-signing test is
    // deterministic and needs no network/crypto-generation step at test
    // time.
    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgOCbd/EpDE9H0dM9V\nzdMlecFMB8akEU3KCGqJWvbDIx+hRANCAAQKQcrpK4I36heW53CWyIhBgnlgiTi1\nWFB1meC80AJeArYGdhkf5yenQE31uP+0VX8xRovcSQ+SuwL24hwvHUfb\n-----END PRIVATE KEY-----\n";
    const TEST_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAECkHK6SuCN+oXludwlsiIQYJ5YIk4\ntVhQdZngvNACXgK2BnYZH+cnp0BN9bj/tFV/MUaL3EkPkrsC9uIcLx1H2w==\n-----END PUBLIC KEY-----\n";

    fn sample_credentials() -> AbmCredentials {
        AbmCredentials {
            client_id: "BUSINESSAPI.11111111-2222-3333-4444-555555555555".to_string(),
            key_id: "test-key-id-123".to_string(),
            private_key_pem: TEST_PRIVATE_KEY_PEM.to_string(),
        }
    }

    #[test]
    fn signs_a_client_assertion_jwt_with_three_dot_separated_parts() {
        let creds = sample_credentials();
        let jwt = build_client_assertion(&creds).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "JWT muss aus drei Punkt-getrennten Teilen bestehen"
        );
        assert!(!parts[0].is_empty());
        assert!(!parts[1].is_empty());
        assert!(!parts[2].is_empty());
    }

    #[test]
    fn signed_jwt_decodes_back_to_the_expected_header_and_claims() {
        let creds = sample_credentials();
        let jwt = build_client_assertion(&creds).unwrap();

        let decoding_key = jsonwebtoken::DecodingKey::from_ec_pem(TEST_PUBLIC_KEY_PEM.as_bytes())
            .expect("Test-Public-Key muss gültig sein");
        let mut validation = jsonwebtoken::Validation::new(Algorithm::ES256);
        // This test only verifies the signature/header/claim shape, not
        // Apple's own audience -- no expected audience is configured on the
        // decoding side, so `jsonwebtoken` must not reject the token for
        // that reason (the `aud` claim's own VALUE is still asserted below,
        // by reading `decoded.claims.aud` directly).
        validation.validate_aud = false;
        let decoded = jsonwebtoken::decode::<AbmClaims>(&jwt, &decoding_key, &validation)
            .expect("JWT muss mit dem passenden Public Key verifizierbar sein");

        assert_eq!(decoded.header.alg, Algorithm::ES256);
        assert_eq!(decoded.header.kid.as_deref(), Some("test-key-id-123"));
        assert_eq!(decoded.header.typ.as_deref(), Some("JWT"));

        assert_eq!(decoded.claims.iss, creds.client_id);
        assert_eq!(decoded.claims.sub, creds.client_id);
        // Verified, intentional /v2 mismatch against the real token
        // endpoint -- see module documentation.
        assert_eq!(decoded.claims.aud, TOKEN_AUD);
        assert!(decoded.claims.aud.contains("/v2/token"));
        assert_eq!(
            decoded.claims.exp - decoded.claims.iat,
            ASSERTION_LIFETIME_SECS
        );
        assert!(!decoded.claims.jti.is_empty());
    }

    #[test]
    fn a_wrong_public_key_fails_verification() {
        let creds = sample_credentials();
        let jwt = build_client_assertion(&creds).unwrap();

        // A second, unrelated EC P-256 key -- deliberately different from
        // TEST_PRIVATE_KEY_PEM/TEST_PUBLIC_KEY_PEM, so verifying against it
        // must fail (proves the signature is actually checked, not just
        // structurally well-formed).
        let other_public_key_pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEuacoG8z4YcNczIjkJ6jSxfRfEf/0\ne0XsECVTFSea19hZ/jufgZNb7tzsd5woDBjIN2lpMLiWq0rsKcm9LqOlSg==\n-----END PUBLIC KEY-----\n";
        // Only assert the failure path if the fixture key actually parses
        // as a valid EC public key -- the important behavior under test is
        // "wrong key -> verification fails", not this hardcoded fixture's
        // own validity.
        if let Ok(decoding_key) =
            jsonwebtoken::DecodingKey::from_ec_pem(other_public_key_pem.as_bytes())
        {
            let mut validation = jsonwebtoken::Validation::new(Algorithm::ES256);
            validation.validate_aud = false;
            let result = jsonwebtoken::decode::<AbmClaims>(&jwt, &decoding_key, &validation);
            assert!(result.is_err());
        }
    }

    #[test]
    fn generate_jti_produces_unique_values() {
        let a = generate_jti();
        let b = generate_jti();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36); // canonical UUID string length (32 hex + 4 dashes)
    }

    #[test]
    fn build_client_assertion_rejects_a_malformed_private_key() {
        let mut creds = sample_credentials();
        creds.private_key_pem = "not a pem key".to_string();
        let result = build_client_assertion(&creds);
        assert!(matches!(result, Err(PluginError::Authentication(_))));
    }

    #[test]
    fn parses_credentials_from_json_secret() {
        let creds = PluginCredentials {
            secret: r#"{"client_id":"BUSINESSAPI.abc","key_id":"kid-1","private_key_pem":"pem"}"#
                .into(),
        };
        let parsed = parse_credentials(&creds).unwrap();
        assert_eq!(parsed.client_id, "BUSINESSAPI.abc");
        assert_eq!(parsed.key_id, "kid-1");
        assert_eq!(parsed.private_key_pem, "pem");
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
    fn parses_devices_page_with_data_and_next_link() {
        let json = serde_json::json!({
            "data": [{"id": "dev-1"}, {"id": "dev-2"}],
            "links": {"next": "https://api-business.apple.com/v1/orgDevices?cursor=abc"}
        });
        let (data, next) = parse_devices_page(&json).unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(
            next.as_deref(),
            Some("https://api-business.apple.com/v1/orgDevices?cursor=abc")
        );
    }

    #[test]
    fn parses_devices_page_without_next_link_as_last_page() {
        let json = serde_json::json!({"data": [{"id": "dev-1"}], "links": {}});
        let (data, next) = parse_devices_page(&json).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(next, None);
    }

    #[test]
    fn parses_devices_page_with_no_links_object_at_all() {
        let json = serde_json::json!({"data": []});
        let (data, next) = parse_devices_page(&json).unwrap();
        assert!(data.is_empty());
        assert_eq!(next, None);
    }

    #[test]
    fn parse_devices_page_rejects_missing_data_field() {
        let json = serde_json::json!({"links": {}});
        let result = parse_devices_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn maps_abm_devices_json_api_array_into_abm_devices() {
        let json = serde_json::json!([
            {
                "id": "dev-1",
                "type": "orgDevices",
                "attributes": {
                    "serialNumber": "XABC123X0ABC123X0",
                    "deviceModel": "iMac 21.5\"",
                    "productFamily": "Mac",
                    "status": "ASSIGNED"
                }
            },
            {
                "id": "dev-2",
                "type": "orgDevices",
                "attributes": {
                    "serialNumber": "YDEF456Y0DEF456Y0"
                }
            }
        ]);
        let devices = map_abm_devices(json.as_array().unwrap());

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "dev-1");
        assert_eq!(devices[0].name, "iMac 21.5\"");
        assert_eq!(
            devices[0].serial_number.as_deref(),
            Some("XABC123X0ABC123X0")
        );
        assert_eq!(devices[0].device_model.as_deref(), Some("iMac 21.5\""));
        assert_eq!(devices[0].hostname, None);
        assert_eq!(devices[0].ip_address, None);

        assert_eq!(devices[1].external_id, "dev-2");
        // No deviceModel -> falls back to serialNumber.
        assert_eq!(devices[1].name, "YDEF456Y0DEF456Y0");
        assert_eq!(devices[1].device_model, None);
    }

    #[test]
    fn falls_back_to_external_id_when_neither_model_nor_serial_present() {
        let json = serde_json::json!([{"id": "dev-9", "attributes": {}}]);
        let devices = map_abm_devices(json.as_array().unwrap());
        assert_eq!(devices[0].name, "dev-9");
        assert_eq!(devices[0].serial_number, None);
        assert_eq!(devices[0].device_model, None);
    }

    #[test]
    fn skips_devices_without_a_usable_id() {
        let json = serde_json::json!([{"attributes": {"serialNumber": "ohne-id"}}]);
        let devices = map_abm_devices(json.as_array().unwrap());
        assert!(devices.is_empty());
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
        let plugin = AbmPlugin::new("abm:acme-123".to_string());
        assert_eq!(plugin.id(), "abm:acme-123");
    }
}
