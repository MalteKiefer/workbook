//! Echte Plugin-Implementierung für Level.io (RMM), siehe
//! `docs/PLUGIN_ARCHITECTURE.md` Abschnitt "Level.io-Plugin". Zweite echte
//! Integration nach NinjaOne (`plugin::ninja`), aber deutlich einfacher:
//!
//! - **Authentifizierung**: statischer API-Key im `Authorization`-Header,
//!   OHNE "Bearer "-Präfix und ohne Token-Austausch -- verifiziert gegen
//!   Level.ios eigene Authentifizierungs-Referenzseite
//!   (<https://developers.level.io/reference/authentication>: "Provide your
//!   API key as the authorization value", Beispiel `-H "Authorization:
//!   APIKEY"`). Kein OAuth2-Grant wie bei NinjaOne nötig.
//! - **Kein Organisations-/Mandanten-Konzept**: Level.ios API-Referenz kennt
//!   keine Organisations-, Konto- oder Site-Endpunkte (verifiziert über
//!   <https://developers.level.io/llms.txt> -- ausschließlich Geräte-,
//!   Gruppen-, Alert-, Update-/Automation- und Tag-/Custom-Field-Endpunkte,
//!   nichts zu Organisationen/Konten). Level selbst beschreibt seine API als
//!   auf Kontoebene arbeitend. Deshalb entspricht eine Level-"Verbindung"
//!   (ein API-Key) hier direkt genau einem lokalen Kunden
//!   (`LevelConnectionMeta.customer_id`) -- KEINE granulare
//!   Organisations-Zuordnungsebene wie bei Ninja (`NinjaOrgMapping`) nötig.
//! - **Geräteliste**: `GET {BASE_URL}/devices`, cursor-paginiert (`has_more`
//!   + `starting_after`, verifiziert über
//!   <https://developers.level.io/reference/listdevices>). Diese Ebene
//!   durchläuft alle Seiten intern (bis zu `MAX_PAGES` Seiten à
//!   `PAGE_LIMIT` Geräten, als Schutz gegen eine sich falsch verhaltende
//!   Gegenstelle) und liefert eine einzige, bereits zusammengefügte Liste --
//!   der Aufrufer sieht nichts von Levels Pagination.
//! - **Gerätedetails**: `GET {BASE_URL}/devices/{id}` (verifiziert über
//!   <https://developers.level.io/reference/showdevice>, "Show Device"),
//!   reicht die Antwort unverändert als `serde_json::Value` durch, analog zu
//!   `plugin::ninja::NinjaPlugin::get_system_details`.
//! - **IP-Adresse**: `include_network_interfaces=true` liefert pro Gerät ein
//!   `network_interfaces`-Array (`[{..., "ip_addresses": ["..."]}]`) --
//!   verifiziert über Levels Antwortschema für `GET /v2/devices`. Level hat
//!   (anders als Ninja) kein flaches `ipAddresses`-Feld auf oberster Ebene;
//!   die erste nicht-leere Adresse der ersten Netzwerkschnittstelle wird
//!   verwendet.
//! - **HTTP-Client**: `ureq` 3.4.1, synchron, exakt dieselbe Abhängigkeit wie
//!   `plugin::ninja` (kein zweiter HTTP-Client in dieser Codebasis).
//! - **Gruppen**: Level hat zwar keine Organisationen, aber ein hierarchisches
//!   Gruppenkonzept innerhalb eines Kontos -- jedes Gerät trägt ein
//!   nullable `group_id`-Feld (verifiziert über
//!   <https://developers.level.io/reference/listdevices>: `"group_id": "..."`,
//!   `null` bedeutet "ungrouped"). Namen dafür liefert `GET {BASE_URL}/groups`
//!   (verifiziert über <https://developers.level.io/reference/listgroups>),
//!   Seiten-Umschlag exakt wie bei `/devices` (`{"data": [...], "has_more":
//!   bool}`, `starting_after`-Cursor). NOCH NICHT UMGESETZT: `list_devices`
//!   löst `group_id` aktuell nicht in einen Klartextnamen auf -- geplante
//!   Erweiterung ist, beide Endpunkte abzurufen und `group_id` bereits hier
//!   serverseitig in einen Namen aufzulösen (`LevelDevice.group_name`),
//!   analog dazu, wie Ninja-Organisationen serverseitig benannt (aber nicht
//!   gruppiert) zurückgegeben werden. Die eigentliche Gruppierung/
//!   Sortierung/Paginierung der Anzeige bleibt auch danach bewusst
//!   Frontend-Angelegenheit (siehe `LevelPluginSection.tsx`).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Level.ios API hat -- anders als NinjaOne -- keine Regionen-/Instanz-
/// Varianten, daher eine feste Konstante statt eines konfigurierbaren
/// `base_url`-Felds an `LevelConnectionMeta`.
pub const BASE_URL: &str = "https://api.level.io/v2";

/// Schutz gegen eine sich falsch verhaltende Gegenstelle (endloses
/// `has_more: true`): mehr als `MAX_PAGES * PAGE_LIMIT` Geräte je Sync-Lauf
/// werden nicht abgerufen.
const MAX_PAGES: usize = 20;
const PAGE_LIMIT: u32 = 100;

/// Nicht-geheime Metadaten einer Level-Verbindung, wie sie in `config.toml`
/// stehen (`Config::level_connections`). Der API-Key gehört laut
/// Credential-Prinzip ausschließlich in den OS-Schlüsselspeicher, niemals
/// hierher. Anders als `NinjaConnectionMeta`: Level hat kein
/// Organisationskonzept, deshalb trägt eine Verbindung hier direkt ihre
/// `customer_id` -- eine Verbindung entspricht genau einem lokalen Kunden.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelConnectionMeta {
    pub id: String,
    pub customer_id: i64,
    pub label: String,
}

/// Ein einzelnes Gerät aus `GET /v2/devices` bzw. `GET /v2/devices/{id}`,
/// angereichert um die (verschachtelt gelieferte) IP-Adresse. Analog zu
/// `plugin::ninja::NinjaDevice`, aber ohne `organization_id` -- Level kennt
/// keine Organisationen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
}

/// Ein Plugin-Objekt für genau eine konfigurierte Level-Verbindung. `id` ist
/// hier bereits der vollqualifizierte Bezeichner (`"level:<connection_id>"`),
/// damit `Plugin::id()` unverändert als `plugin_id`/Schlüsselspeicher-Konto
/// taugt (siehe Trait-Dokumentation in `plugin::mod`). Anders als
/// `NinjaPlugin` braucht dieser Typ kein `base_url`-Feld (siehe `BASE_URL`
/// oben) und keinen zwischengespeicherten Token -- der API-Key kommt bei
/// jedem Aufruf frisch aus `PluginCredentials`.
pub struct LevelPlugin {
    id: String,
}

impl LevelPlugin {
    pub fn new(id: String) -> Self {
        Self { id }
    }

    /// Live-Abruf ALLER Geräte dieser Verbindung, mit intern durchlaufener
    /// Pagination (siehe Moduldokumentation). Reichhaltiger als die
    /// Trait-Methode `list_systems`, die absichtlich beim schmalen,
    /// plugin-übergreifenden `ExternalSystem`-Typ bleibt (kein
    /// `ip_address`-Feld dort).
    pub fn list_devices(&self, credentials: &PluginCredentials) -> Result<Vec<LevelDevice>, PluginError> {
        let agent = build_agent();
        let raw_devices = fetch_all_devices(&agent, &credentials.secret)?;
        Ok(map_level_devices(&raw_devices))
    }
}

/// Prüft einen API-Key gegen Level (ein leichtgewichtiger Aufruf: eine Seite
/// mit `limit=1`), ohne irgendetwas zu persistieren. Für
/// `commands::level::test_level_connection`, damit Nutzer einen Tippfehler
/// im API-Key bemerken, bevor sie eine Verbindung tatsächlich anlegen
/// (Zugangsdaten in den Schlüsselspeicher schreiben).
pub fn test_credentials(api_key: &str) -> Result<(), PluginError> {
    let agent = build_agent();
    fetch_devices_page(&agent, api_key, None, 1)?;
    Ok(())
}

impl Plugin for LevelPlugin {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn list_systems(&self, credentials: &PluginCredentials) -> Result<Vec<ExternalSystem>, PluginError> {
        let devices = self.list_devices(credentials)?;
        Ok(devices
            .into_iter()
            .map(|d| ExternalSystem { external_id: d.external_id, name: d.name, hostname: d.hostname })
            .collect())
    }

    fn get_system_details(&self, credentials: &PluginCredentials, external_id: &str) -> Result<serde_json::Value, PluginError> {
        let agent = build_agent();
        fetch_device_json(&agent, &credentials.secret, external_id)
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Levels API muss von einer lokalen Verknüpfung nichts wissen -- rein
        // lokales Bucheführungskonzept, siehe Trait-Dokumentation. Persistiert
        // wird das vom Aufrufer über `db::external_refs::upsert`.
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

/// Ruft alle Seiten von `GET /v2/devices` ab und reicht die rohen
/// Geräteobjekte (noch nicht auf `LevelDevice` gemappt) als eine einzige,
/// zusammengefügte Liste zurück. Bricht früher ab, wenn `has_more` fehlt/
/// `false` ist, oder wenn `MAX_PAGES` erreicht ist, oder wenn eine Seite kein
/// verwertbares `id`-Feld für den nächsten Cursor liefert (dann lässt sich
/// nicht sinnvoll weiter paginieren).
fn fetch_all_devices(agent: &Agent, api_key: &str) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_devices_page(agent, api_key, cursor.as_deref(), PAGE_LIMIT)?;
        let (data, has_more) = parse_devices_page(&page_json)?;
        let next_cursor = data.last().and_then(|d| d["id"].as_str()).map(str::to_string);
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

fn fetch_devices_page(agent: &Agent, api_key: &str, starting_after: Option<&str>, limit: u32) -> Result<serde_json::Value, PluginError> {
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
    response.body_mut().read_json::<serde_json::Value>().map_err(map_ureq_error)
}

fn fetch_device_json(agent: &Agent, api_key: &str, external_id: &str) -> Result<serde_json::Value, PluginError> {
    let url = format!("{BASE_URL}/devices/{external_id}");
    let mut response = agent
        .get(&url)
        .header("Authorization", api_key)
        .query("include_network_interfaces", "true")
        .call()
        .map_err(map_ureq_error)?;
    response.body_mut().read_json::<serde_json::Value>().map_err(map_ureq_error)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Level-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => PluginError::Unreachable(format!("Level-API antwortete mit Status {code}")),
        ureq::Error::Json(err) => PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}")),
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extrahiert Geräteliste und Fortsetzungs-Flag aus einer einzelnen
/// Level-Seiten-Antwort (`GET /v2/devices`: `{"data": [...], "has_more":
/// bool}`, verifiziert über Levels eigene Referenzseite). Reine Funktion,
/// mit hartkodiertem JSON testbar, kein echter Netzwerkzugriff nötig.
fn parse_devices_page(json: &serde_json::Value) -> Result<(Vec<serde_json::Value>, bool), PluginError> {
    let data = json["data"]
        .as_array()
        .ok_or_else(|| PluginError::UnexpectedResponse("Erwartete 'data'-Liste in Level-Antwort".to_string()))?;
    let has_more = json["has_more"].as_bool().unwrap_or(false);
    Ok((data.clone(), has_more))
}

/// Bildet eine (bereits über alle Seiten hinweg zusammengefügte) Liste roher
/// Level-Geräteobjekte auf `LevelDevice`-Werte ab. Reine Funktion, mit
/// hartkodiertem JSON testbar -- Levels "List Devices"- und "Show Device"-
/// Endpunkte liefern Geräteobjekte in exakt derselben Form.
fn map_level_devices(devices: &[serde_json::Value]) -> Vec<LevelDevice> {
    devices.iter().filter_map(map_level_device).collect()
}

/// Ein einzelnes Geräteobjekt aus Levels Antwort. Level hat -- anders als
/// NinjaOne -- keine separate `displayName` vs. `systemName`-Unterscheidung,
/// sondern `nickname` (nutzerdefiniert, kann `null` sein) und `hostname`.
/// `nickname` gewinnt für den Anzeigenamen, mit `hostname` und zuletzt der
/// externen ID als Rückfallebene. Ein Gerät ohne verwertbare `id` wird
/// übersprungen statt den gesamten Aufruf scheitern zu lassen -- ein
/// einzelnes kaputtes Geräteobjekt soll nicht die ganze Liste unbrauchbar
/// machen (analog zu `plugin::ninja::map_device`).
fn map_level_device(value: &serde_json::Value) -> Option<LevelDevice> {
    let external_id = value["id"].as_str()?.to_string();
    let hostname = value["hostname"].as_str().map(str::to_string);
    let nickname = value["nickname"].as_str().map(str::to_string);
    let name = nickname.or_else(|| hostname.clone()).unwrap_or_else(|| external_id.clone());
    let ip_address = extract_ip_address(value);
    Some(LevelDevice { external_id, name, hostname, ip_address })
}

/// Sucht die erste nicht-leere IP-Adresse im (nur bei
/// `include_network_interfaces=true` vorhandenen) `network_interfaces`-Array
/// eines Geräts -- `[{..., "ip_addresses": ["..."]}, ...]`, verifiziert über
/// Levels Antwortschema. Level liefert -- anders als NinjaOne -- kein
/// flaches IP-Feld auf oberster Ebene.
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
        let devices = map_level_devices(json.as_array().unwrap());

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "dev-1");
        assert_eq!(devices[0].name, "Server 01");
        assert_eq!(devices[0].hostname.as_deref(), Some("srv-01.customer.local"));
        assert_eq!(devices[1].external_id, "dev-2");
        assert_eq!(devices[1].name, "fw-edge");
    }

    #[test]
    fn skips_devices_without_a_usable_id() {
        let json = serde_json::json!([{"hostname": "ohne-id"}]);
        let devices = map_level_devices(json.as_array().unwrap());
        assert!(devices.is_empty());
    }

    #[test]
    fn falls_back_to_external_id_when_no_hostname_or_nickname_present() {
        let json = serde_json::json!([{"id": "dev-7"}]);
        let devices = map_level_devices(json.as_array().unwrap());
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
        let devices = map_level_devices(std::slice::from_ref(&json));
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
        let devices = map_level_devices(std::slice::from_ref(&json));
        assert_eq!(devices[0].ip_address.as_deref(), Some("10.0.0.9"));
    }

    #[test]
    fn has_no_ip_address_when_network_interfaces_absent() {
        let json = serde_json::json!({"id": "dev-1"});
        let devices = map_level_devices(std::slice::from_ref(&json));
        assert_eq!(devices[0].ip_address, None);
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
