//! Echte Plugin-Implementierung für NinjaOne (vormals NinjaRMM), siehe
//! `docs/PLUGIN_ARCHITECTURE.md` Abschnitt "NinjaOne-Plugin". Struktur und
//! Signaturen orientieren sich an `plugin::dummy::DummyPlugin`, sprechen aber
//! über echtes HTTPS (Crate `ureq`, synchron, kein async-Runtime) mit
//! NinjaOnes öffentlicher REST-API.
//!
//! Ein `NinjaPlugin` gehört zu genau einer vom Nutzer angelegten "Ninja-
//! Verbindung" (`NinjaConnectionMeta`, ein Satz OAuth2-Zugangsdaten für genau
//! einen Ninja-Mandanten). WICHTIG: Eine Verbindung ist NICHT an genau einen
//! lokalen Kunden gebunden -- ein einzelner Ninja-Mandant modelliert selbst
//! mehrere "Organizations", z. B. weil der Nutzer, der die Verbindung anlegt,
//! seinerseits ein MSP ist und darin mehrere eigene Kunden als getrennte
//! Organisationen führt. Deshalb liefert dieses Modul zusätzlich zu den
//! Geräten (`list_systems`/`list_devices`) auch die Organisationsliste
//! (`list_organizations`) einer Verbindung; welche Organisation welchem
//! lokalen Kunden entspricht (falls überhaupt), ist eine separate, granulare
//! Zuordnung (`NinjaOrgMapping`, `Config::ninja_org_mappings`), gepflegt über
//! `commands::plugins`. `id` ist hier keine feste Konstante wie bei
//! `DummyPlugin`, sondern pro Verbindung vergeben (siehe
//! `commands::plugins`, das den vollqualifizierten `"ninja:<connection_id>"`-
//! Bezeichner sowohl als Schlüsselspeicher-Konto als auch als
//! `external_refs.plugin_id` verwendet).
//!
//! Authentifizierung läuft über den OAuth2-Client-Credentials-Grant. Der
//! Zugriffstoken wird bewusst nicht über mehrere Aufrufe hinweg
//! zwischengespeichert -- diese App ruft Plugin-Methoden selten/manuell auf,
//! nicht in einer heißen Schleife, daher ist "pro Methodenaufruf neu holen"
//! einfach und korrekt genug für v1.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Nicht-geheime Metadaten einer Ninja-Verbindung, wie sie in `config.toml`
/// stehen (`Config::ninja_connections`). Client-ID/-Secret gehören laut
/// Credential-Prinzip ausschließlich in den OS-Schlüsselspeicher, niemals
/// hierher. Bewusst OHNE `customer_id` -- eine Verbindung ist ein Ninja-
/// Mandant, kein lokaler Kunde; welche Ninja-"Organization" innerhalb dieses
/// Mandanten welchem lokalen Kunden entspricht, steht granular in
/// `NinjaOrgMapping`/`Config::ninja_org_mappings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NinjaConnectionMeta {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

/// Zuordnung einer einzelnen Ninja-"Organization" (innerhalb einer
/// Verbindung) zu einem lokalen Kunden. Lebt in `Config::ninja_org_mappings`,
/// nicht in `NinjaConnectionMeta` -- eine Verbindung kann mehrere
/// Organisationen sehen, von denen jede unabhängig zugeordnet (oder
/// unzugeordnet gelassen) werden kann. `organization_name` wird zusätzlich
/// zur `organization_id` gespeichert, damit z. B. eine künftige UI-Liste
/// ohne erneuten Live-Aufruf gegen Ninja einen lesbaren Namen anzeigen kann.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NinjaOrgMapping {
    pub connection_id: String,
    pub organization_id: String,
    pub organization_name: String,
    pub customer_id: i64,
}

/// Eine von `GET /v2/organizations` gemeldete Organisation. Getrennt von
/// `ExternalSystem` (das sind Geräte) -- eigener, kleiner Formtyp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NinjaOrganization {
    pub id: String,
    pub name: String,
}

/// Ein einzelnes Gerät aus `GET /v2/devices`, angereichert um
/// Organisationszugehörigkeit (`organizationId`) und IP-Adresse
/// (`ipAddresses`) -- beides braucht `commands::plugins::sync_ninja_connection`
/// zum Gruppieren nach Organisation und zur Anzeige, was der generische,
/// plugin-übergreifende `ExternalSystem`-Typ aus `plugin::mod` bewusst nicht
/// vorsieht (der bleibt das schmale, trait-generische Minimum, das auch
/// `DummyPlugin` erfüllen können muss). Deshalb ein eigener, Ninja-
/// spezifischer Typ statt einer Erweiterung von `ExternalSystem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NinjaDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub organization_id: String,
}

/// Die beiden Geheimwerte, die eine Ninja-Verbindung zum Authentifizieren
/// braucht. `PluginCredentials.secret` ist laut Trait-Vertrag ein einziger
/// opaker String, den das Plugin selbst interpretiert -- hier also als JSON
/// kodiert (`serde_json::to_string`/`from_str`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NinjaCredentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Ein Plugin-Objekt für genau eine konfigurierte Ninja-Verbindung. `id` ist
/// hier bereits der vollqualifizierte Bezeichner (`"ninja:<connection_id>"`),
/// damit `Plugin::id()` unverändert als `plugin_id`/Schlüsselspeicher-Konto
/// taugt (siehe Trait-Dokumentation in `plugin::mod`).
pub struct NinjaPlugin {
    id: String,
    base_url: String,
}

impl NinjaPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live-Abruf der Organisationsliste dieser Verbindung
    /// (`GET /v2/organizations`). Getrennt von der `Plugin`-Trait-Methode
    /// `list_systems`, weil Organisationen keine Geräte sind und der
    /// generische Trait dafür keinen Platz vorsieht.
    pub fn list_organizations(&self, credentials: &PluginCredentials) -> Result<Vec<NinjaOrganization>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        let json = fetch_json(&agent, &self.base_url, "/v2/organizations", &token)?;
        map_organizations_response(&json)
    }

    /// Live-Abruf aller Geräte dieser Verbindung, angereichert um
    /// Organisationszugehörigkeit und IP-Adresse (siehe `NinjaDevice`).
    /// Reichhaltiger als die Trait-Methode `list_systems`, die absichtlich
    /// beim schmalen, plugin-übergreifenden `ExternalSystem`-Typ bleibt.
    pub fn list_devices(&self, credentials: &PluginCredentials) -> Result<Vec<NinjaDevice>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        let json = fetch_json(&agent, &self.base_url, "/v2/devices", &token)?;
        map_ninja_devices_response(&json)
    }
}

/// Prüft ein Client-ID/-Secret-Paar gegen NinjaOne (OAuth2-Client-
/// Credentials-Grant), ohne irgendetwas zu persistieren -- der Zugriffstoken
/// wird nach erfolgreichem Abruf verworfen. Für
/// `commands::plugins::test_ninja_connection`, damit Nutzer Tippfehler in
/// Basis-URL/Zugangsdaten bemerken, bevor sie eine Verbindung tatsächlich
/// anlegen (Zugangsdaten in den Schlüsselspeicher schreiben).
pub fn test_credentials(base_url: &str, client_id: &str, client_secret: &str) -> Result<(), PluginError> {
    let creds = NinjaCredentials { client_id: client_id.to_string(), client_secret: client_secret.to_string() };
    let agent = build_agent();
    fetch_access_token(&agent, base_url, &creds)?;
    Ok(())
}

impl Plugin for NinjaPlugin {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn list_systems(&self, credentials: &PluginCredentials) -> Result<Vec<ExternalSystem>, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        let json = fetch_json(&agent, &self.base_url, "/v2/devices", &token)?;
        map_devices_response(&json)
    }

    fn get_system_details(&self, credentials: &PluginCredentials, external_id: &str) -> Result<serde_json::Value, PluginError> {
        let creds = parse_credentials(credentials)?;
        let agent = build_agent();
        let token = fetch_access_token(&agent, &self.base_url, &creds)?;
        fetch_json(&agent, &self.base_url, &format!("/v2/device/{external_id}"), &token)
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // NinjaOnes API muss von einer lokalen Verknüpfung nichts wissen --
        // rein lokales Bucheführungskonzept, siehe Trait-Dokumentation.
        // Persistiert wird das vom Aufrufer über `db::external_refs::upsert`.
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

fn fetch_access_token(agent: &Agent, base_url: &str, creds: &NinjaCredentials) -> Result<String, PluginError> {
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

fn fetch_json(agent: &Agent, base_url: &str, path: &str, bearer_token: &str) -> Result<serde_json::Value, PluginError> {
    let url = format!("{}{path}", base_url.trim_end_matches('/'));
    let mut response = agent
        .get(&url)
        .header("Authorization", format!("Bearer {bearer_token}"))
        .call()
        .map_err(map_ureq_error)?;
    response.body_mut().read_json::<serde_json::Value>().map_err(map_ureq_error)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Ninja-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => PluginError::Unreachable(format!("Ninja-API antwortete mit Status {code}")),
        ureq::Error::Json(err) => PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}")),
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Bildet die von `GET /v2/devices` gelieferte JSON-Liste auf
/// `ExternalSystem`-Werte ab. Bewusst als eigene, reine Funktion
/// herausgezogen -- lässt sich mit einem festen JSON-String testen, ganz
/// ohne echten Netzwerkzugriff.
fn map_devices_response(json: &serde_json::Value) -> Result<Vec<ExternalSystem>, PluginError> {
    let array = json
        .as_array()
        .ok_or_else(|| PluginError::UnexpectedResponse("Erwartete JSON-Liste von Geräten".to_string()))?;
    Ok(array.iter().filter_map(map_device).collect())
}

/// Ein einzelnes Geräteobjekt aus NinjaOnes `/v2/devices`-Antwort. `id`
/// fehlt oder hat eine unerwartete Form -> Gerät wird übersprungen statt den
/// gesamten Aufruf scheitern zu lassen (ein einzelnes kaputtes Geräteobjekt
/// soll nicht die ganze Liste unbrauchbar machen).
fn map_device(value: &serde_json::Value) -> Option<ExternalSystem> {
    let external_id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let system_name = value["systemName"].as_str();
    let display_name = value["displayName"].as_str();
    let hostname = value["hostname"].as_str().or(system_name).map(str::to_string);
    let name = display_name.or(system_name).unwrap_or(external_id.as_str()).to_string();
    Some(ExternalSystem { external_id, name, hostname })
}

/// Bildet die von `GET /v2/organizations` gelieferte JSON-Liste auf
/// `NinjaOrganization`-Werte ab. Reine, für sich mit hartkodiertem JSON
/// testbare Funktion, analog zu `map_devices_response`.
fn map_organizations_response(json: &serde_json::Value) -> Result<Vec<NinjaOrganization>, PluginError> {
    let array = json
        .as_array()
        .ok_or_else(|| PluginError::UnexpectedResponse("Erwartete JSON-Liste von Organisationen".to_string()))?;
    Ok(array.iter().filter_map(map_organization).collect())
}

/// Ein einzelnes Organisationsobjekt aus NinjaOnes `/v2/organizations`-
/// Antwort (`{"id": <Zahl>, "name": "..."}`, laut NinjaOnes öffentlicher
/// API-Spezifikation). Fehlt oder hat `id` eine unerwartete Form -> die
/// Organisation wird übersprungen statt den gesamten Aufruf scheitern zu
/// lassen, analog zu `map_device`.
fn map_organization(value: &serde_json::Value) -> Option<NinjaOrganization> {
    let id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let name = value["name"].as_str().unwrap_or(id.as_str()).to_string();
    Some(NinjaOrganization { id, name })
}

/// Bildet die von `GET /v2/devices` gelieferte JSON-Liste auf `NinjaDevice`-
/// Werte ab (samt Organisationszugehörigkeit und IP-Adresse) -- das
/// Gegenstück zu `map_devices_response`, das nur den schmalen, trait-
/// generischen `ExternalSystem`-Typ befüllt. Reine, für sich testbare
/// Funktion, kein echter Netzwerkzugriff nötig.
fn map_ninja_devices_response(json: &serde_json::Value) -> Result<Vec<NinjaDevice>, PluginError> {
    let array = json
        .as_array()
        .ok_or_else(|| PluginError::UnexpectedResponse("Erwartete JSON-Liste von Geräten".to_string()))?;
    Ok(array.iter().filter_map(map_ninja_device).collect())
}

/// Ein einzelnes Geräteobjekt aus NinjaOnes `/v2/devices`-Antwort, angereichert
/// um `organizationId` (laut NinjaOnes öffentlicher API-Spezifikation ein
/// Integer-Feld, das jedes Gerät genau einer Organisation zuordnet) und
/// `ipAddresses` (ein Array von IP-Adress-Strings; primäre/erste Adresse wird
/// verwendet, mit `publicIP` als Rückfallebene, falls `ipAddresses` leer oder
/// nicht vorhanden ist). Ein Gerät ohne verwertbare `id` ODER ohne
/// `organizationId` wird übersprungen -- ohne Organisationszugehörigkeit
/// lässt es sich nicht sinnvoll gruppieren, und ein einzelnes kaputtes
/// Geräteobjekt soll nicht die ganze Liste unbrauchbar machen.
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
    let hostname = value["hostname"].as_str().or(system_name).map(str::to_string);
    let name = display_name.or(system_name).unwrap_or(external_id.as_str()).to_string();
    let ip_address = value["ipAddresses"]
        .as_array()
        .and_then(|addresses| addresses.first())
        .and_then(|first| first.as_str())
        .map(str::to_string)
        .or_else(|| value["publicIP"].as_str().map(str::to_string));
    Some(NinjaDevice { external_id, name, hostname, ip_address, organization_id })
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
        assert_eq!(systems[0].hostname.as_deref(), Some("srv-01.customer.local"));
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
        let creds = PluginCredentials { secret: r#"{"client_id":"abc","client_secret":"xyz"}"#.into() };
        let parsed = parse_credentials(&creds).unwrap();
        assert_eq!(parsed.client_id, "abc");
        assert_eq!(parsed.client_secret, "xyz");
    }

    #[test]
    fn rejects_malformed_credentials_json() {
        let creds = PluginCredentials { secret: "not json".into() };
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
        let plugin = NinjaPlugin::new("ninja:acme-123".to_string(), "https://app.ninjarmm.com".to_string());
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
        assert_eq!(organizations[0], NinjaOrganization { id: "1".to_string(), name: "ACME Hauptsitz".to_string() });
        assert_eq!(organizations[1], NinjaOrganization { id: "2".to_string(), name: "ACME Zweigstelle".to_string() });
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
