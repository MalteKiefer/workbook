//! Echte Plugin-Implementierung für Snipe-IT (Open-Source-IT-Asset-Management),
//! siehe `docs/PLUGIN_ARCHITECTURE.md` Abschnitt "Snipe-IT-Plugin". Dritte echte
//! Integration nach NinjaOne (`plugin::ninja`) und Level.io (`plugin::level`).
//! Struktur und Signaturen orientieren sich an `plugin::dummy::DummyPlugin`,
//! sprechen aber über echtes HTTPS (Crate `ureq`, synchron, kein
//! async-Runtime) mit Snipe-ITs öffentlicher REST-API.
//!
//! - **Authentifizierung**: statischer Bearer-Token im `Authorization`-Header
//!   (`Authorization: Bearer <token>`) -- ein "Personal Access Token", den der
//!   Nutzer selbst in Snipe-ITs eigener Weboberfläche erzeugt (Profil -> API
//!   Tokens). Verifiziert über Snipe-ITs eigenes `routes/api.php` auf GitHub
//!   (`POST/GET/DELETE /api/v1/account/personal-access-tokens`). Kein
//!   OAuth2-Grant wie bei NinjaOne, kein Token-Austausch -- genau wie bei
//!   Level.io, nur mit `Bearer `-Präfix (Level hat keins).
//! - **Selbst gehostet**: anders als Level.io (feste `BASE_URL`-Konstante) und
//!   wie NinjaOne braucht eine Snipe-IT-Verbindung eine vom Nutzer angegebene
//!   Basis-URL (`SnipeitConnectionMeta.base_url`, z. B.
//!   `https://assets.example.com`) -- `API_PATH` (`/api/v1`) wird beim Aufbau
//!   jeder Anfrage-URL fest angehängt.
//! - **Firmen (Mehrmandantenfähigkeit)**: `GET {base_url}/api/v1/companies`
//!   verifiziert über Snipe-ITs `routes/api.php` (Index-/Show-/Store-/
//!   Update-/Destroy-/Selectlist-Endpunkte, hier nur die Liste gebraucht). Eine
//!   einzelne Snipe-IT-Instanz kann Assets mehrerer Firmen verwalten (z. B.
//!   ein MSP, der Kundenbestände in einer gemeinsamen Instanz führt) --
//!   deshalb exakt dasselbe granulare Zuordnungsprinzip wie bei NinjaOnes
//!   "Organizations" (`NinjaOrgMapping`): `SnipeitCompanyMapping` ordnet jede
//!   Firma unabhängig einem lokalen Kunden zu, `SnipeitConnectionMeta` selbst
//!   trägt bewusst KEINE `customer_id`.
//! - **Geräte/Assets**: `GET {base_url}/api/v1/hardware`, Offset-paginiert
//!   (NICHT Cursor-basiert wie NinjaOne/Level.io) -- `limit`/`offset`-
//!   Query-Parameter, verifiziert über Snipe-ITs eigene API-Referenzseite
//!   (<https://snipe-it.readme.io/reference/hardware-list>). Der
//!   Standard-`limit`-Wert ist mit 2 absurd niedrig, deshalb wird hier immer
//!   explizit `PAGE_LIMIT` (100) mitgeschickt. Response-Umschlag verifiziert
//!   über Snipe-ITs eigenen Quellcode
//!   (`app/Http/Transformers/DatatablesTransformer.php::transformDatatables`):
//!   `{"total": <Zahl>, "rows": [...], "current_page": ..., "per_page": ...,
//!   "total_pages": ..., "prev_page_url": ..., "next_page_url": ...}` --
//!   also `total`/`rows`, keine Vermutung. `list_devices` durchläuft alle
//!   Seiten intern (bis zu `MAX_PAGES` Seiten à `PAGE_LIMIT` Assets, Schutz
//!   gegen eine sich falsch verhaltende Gegenstelle) und liefert eine
//!   einzige, bereits zusammengefügte Liste -- der Aufrufer sieht nichts von
//!   Snipe-ITs Pagination, exakt dasselbe Prinzip wie bei NinjaOne/Level.io.
//! - **Kein Hostname/keine IP-Adresse**: Snipe-IT ist Asset-/Inventar-
//!   verwaltung, keine RMM-Überwachungssoftware -- anders als bei NinjaOne/
//!   Level.io hat das Kern-Asset-Objekt KEIN garantiertes Hostname-/
//!   IP-Adress-Feld. Verifiziert über Snipe-ITs eigenen Quellcode
//!   (`app/Http/Transformers/AssetsTransformer.php::transformAsset`): die
//!   dortige Feldliste enthält `id`, `name`, `asset_tag`, `serial`, `model`,
//!   `company`, `status_label`/`status` u. v. a., aber nachweislich weder
//!   `hostname` noch eine IP-Adresse. `SnipeitDevice.hostname`/`ip_address`
//!   sind deshalb IMMER `None` -- kein Rateversuch, sondern eine ehrliche,
//!   verifizierte Auslassung. Stattdessen sind Snipe-ITs eigene, natürliche
//!   Identifikationsfelder (`asset_tag`, `serial`) hier erstklassige Felder.
//!   Snipe-IT liefert zusätzlich pro Asset ein `custom_fields`-Objekt
//!   (verifiziert: `$fields_array[$field->name] = {field, value,
//!   field_format, element}`, also nach Feldname benannten Schlüsseln, keine
//!   feste Liste) -- da Feldnamen frei vom jeweiligen Snipe-IT-Administrator
//!   konfiguriert werden, gibt es hier bewusst KEINE brüchige Ratelogik nach
//!   einem "hostname"-artigen benutzerdefinierten Feld; ein sauberes,
//!   `asset_tag`/`serial`-basiertes Modell mit `hostname`/`ip_address` immer
//!   `None` ist für v1 ehrlich und korrekt.
//! - **Web-Oberflächen-Link**: `{base_url}/hardware/{id}` zeigt die
//!   Detailseite eines Assets in Snipe-ITs eigener Weboberfläche (NICHT der
//!   API), verifiziert über Snipe-ITs `routes/web/hardware.php`
//!   (`Route::resource('hardware', AssetsController::class, ...)`, die
//!   Standard-Laravel-"show"-Route ist `hardware/{asset}`). Analog zu
//!   NinjaOnes `ninja_url` gibt es deshalb ein `snipeit_url`-Gegenstück in
//!   `commands::snipeit::ExternalSystemDto` -- kein erfundenes URL-Schema,
//!   sondern aus Snipe-ITs eigenem Quellcode abgeleitet.
//! - **HTTP-Client**: `ureq` 3.4.1, synchron, exakt dieselbe Abhängigkeit wie
//!   `plugin::ninja`/`plugin::level` (kein zweiter HTTP-Client in dieser
//!   Codebasis).
//! - **Zugangsdaten-Kodierung**: Snipe-IT braucht nur einen einzigen
//!   Geheimwert (den Personal Access Token), der 1:1 als
//!   `PluginCredentials.secret` durchgereicht wird -- wie bei Level.io keine
//!   JSON-Kodierung mehrerer Werte nötig (anders als NinjaOne).

use serde::{Deserialize, Serialize};
use std::time::Duration;
use ureq::Agent;

use super::{ExternalSystem, Plugin, PluginCredentials, PluginError};

/// Fester API-Pfad-Suffix, der an die vom Nutzer angegebene `base_url`
/// angehängt wird (siehe Moduldokumentation). Snipe-IT ist selbst gehostet --
/// anders als Level.io (feste `BASE_URL`-Konstante) --, deshalb kein
/// fest verdrahteter Host, nur dieser gemeinsame Pfad-Suffix.
const API_PATH: &str = "/api/v1";

/// Schutz gegen eine sich falsch verhaltende Gegenstelle (z. B. `total`, das
/// nie sinkt): mehr als `MAX_PAGES * PAGE_LIMIT` Zeilen je Aufruf werden
/// nicht abgerufen. Höher als Level.ios `MAX_PAGES` (20) gewählt, weil eine
/// Snipe-IT-Instanz im MSP-Einsatz realistisch mehrere tausend Assets
/// verwalten kann.
const MAX_PAGES: usize = 50;
/// Snipe-ITs eigener Standard-`limit`-Wert ist laut eigener API-Referenz nur
/// 2 -- hier wird deshalb bei jeder Seite immer explizit dieser höhere Wert
/// mitgeschickt.
const PAGE_LIMIT: u32 = 100;

/// Synthetischer Platzhalter für `SnipeitDevice.company_id`, wenn ein Asset
/// laut Snipe-IT selbst KEINER Firma zugeordnet ist (`"company": null` oder
/// das Feld fehlt ganz -- ein legitimer, in Snipe-IT üblicher Fall, kein
/// Fehler: viele Alleinstellungs-Instanzen nutzen das Firmen-Konzept gar
/// nicht). Bewusst nicht-numerisch, damit er nie mit einer echten,
/// numerischen Snipe-IT-Firmen-ID kollidieren kann. `commands::snipeit`
/// gruppiert Assets mit dieser ID als eigene, klar beschriftete Gruppe statt
/// sie stillschweigend zu verwerfen -- analog zu `plugin::ninja`s Umgang mit
/// einer Geräte-`organizationId`, die auf keine bekannte Organisation passt.
pub const UNASSIGNED_COMPANY_ID: &str = "unassigned";

/// Nicht-geheime Metadaten einer Snipe-IT-Verbindung, wie sie in
/// `config.toml` stehen (`Config::snipeit_connections`). Der Personal Access
/// Token gehört laut Credential-Prinzip ausschließlich in den
/// OS-Schlüsselspeicher, niemals hierher. Bewusst OHNE `customer_id` -- eine
/// Verbindung ist eine Snipe-IT-Instanz, kein lokaler Kunde; welche
/// Snipe-IT-"Company" innerhalb dieser Instanz welchem lokalen Kunden
/// entspricht, steht granular in `SnipeitCompanyMapping`/
/// `Config::snipeit_company_mappings` -- exakt dasselbe Prinzip wie bei
/// `plugin::ninja::NinjaConnectionMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnipeitConnectionMeta {
    pub id: String,
    pub label: String,
    pub base_url: String,
}

/// Zuordnung einer einzelnen Snipe-IT-"Company" (innerhalb einer Verbindung)
/// zu einem lokalen Kunden. Lebt in `Config::snipeit_company_mappings`, nicht
/// in `SnipeitConnectionMeta` -- eine Verbindung kann mehrere Firmen sehen,
/// von denen jede unabhängig zugeordnet (oder unzugeordnet gelassen) werden
/// kann. `company_name` wird zusätzlich zur `company_id` gespeichert, damit
/// eine UI-Liste ohne erneuten Live-Aufruf gegen Snipe-IT einen lesbaren
/// Namen anzeigen kann -- analog zu `plugin::ninja::NinjaOrgMapping`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnipeitCompanyMapping {
    pub connection_id: String,
    pub company_id: String,
    pub company_name: String,
    pub customer_id: i64,
}

/// Eine von `GET /api/v1/companies` gemeldete Firma. Getrennt von
/// `ExternalSystem` (das sind Assets) -- eigener, kleiner Formtyp, analog zu
/// `plugin::ninja::NinjaOrganization`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnipeitCompany {
    pub id: String,
    pub name: String,
}

/// Ein einzelnes Asset aus `GET /api/v1/hardware` bzw.
/// `GET /api/v1/hardware/{id}`, angereichert um Firmenzugehörigkeit
/// (`company_id`) und Snipe-ITs eigene, natürliche Identifikationsfelder
/// (`asset_tag`, `serial`) -- das braucht `commands::snipeit::sync_snipeit_connection`
/// zum Gruppieren nach Firma und zur Anzeige, was der generische,
/// plugin-übergreifende `ExternalSystem`-Typ aus `plugin::mod` bewusst nicht
/// vorsieht. `hostname`/`ip_address` sind IMMER `None` (siehe
/// Moduldokumentation) -- als Felder trotzdem vorhanden, damit dieser Typ
/// strukturell zu `plugin::ninja::NinjaDevice`/`plugin::level::LevelDevice`
/// passt und eine künftige UI einheitlich damit umgehen kann.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnipeitDevice {
    pub external_id: String,
    pub name: String,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub asset_tag: Option<String>,
    pub serial: Option<String>,
    pub company_id: String,
}

/// Ein Plugin-Objekt für genau eine konfigurierte Snipe-IT-Verbindung. `id`
/// ist hier bereits der vollqualifizierte Bezeichner
/// (`"snipeit:<connection_id>"`), damit `Plugin::id()` unverändert als
/// `plugin_id`/Schlüsselspeicher-Konto taugt (siehe Trait-Dokumentation in
/// `plugin::mod`).
pub struct SnipeitPlugin {
    id: String,
    base_url: String,
}

impl SnipeitPlugin {
    pub fn new(id: String, base_url: String) -> Self {
        Self { id, base_url }
    }

    /// Live-Abruf der Firmenliste dieser Verbindung (`GET /api/v1/companies`,
    /// intern über alle Seiten hinweg zusammengefügt, siehe
    /// Moduldokumentation). Getrennt von der `Plugin`-Trait-Methode
    /// `list_systems`, weil Firmen keine Assets sind und der generische
    /// Trait dafür keinen Platz vorsieht -- analog zu
    /// `plugin::ninja::NinjaPlugin::list_organizations`.
    pub fn list_companies(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<SnipeitCompany>, PluginError> {
        let agent = build_agent();
        let raw = fetch_all_companies(&agent, &self.base_url, &credentials.secret)?;
        Ok(map_companies_response(&raw))
    }

    /// Live-Abruf aller Assets dieser Verbindung, mit intern durchlaufener
    /// Pagination (siehe Moduldokumentation). Reichhaltiger als die
    /// Trait-Methode `list_systems`, die absichtlich beim schmalen,
    /// plugin-übergreifenden `ExternalSystem`-Typ bleibt (kein
    /// `asset_tag`/`serial`/`company_id`-Feld dort).
    pub fn list_devices(
        &self,
        credentials: &PluginCredentials,
    ) -> Result<Vec<SnipeitDevice>, PluginError> {
        let agent = build_agent();
        let raw = fetch_all_hardware(&agent, &self.base_url, &credentials.secret)?;
        Ok(map_hardware_response(&raw))
    }
}

/// Prüft einen Personal Access Token gegen Snipe-IT (ein leichtgewichtiger
/// Aufruf: eine Seite mit `limit=1`), ohne irgendetwas zu persistieren. Für
/// `commands::snipeit::test_snipeit_connection`, damit Nutzer einen Tippfehler
/// in Basis-URL/Token bemerken, bevor sie eine Verbindung tatsächlich anlegen
/// (Zugangsdaten in den Schlüsselspeicher schreiben) -- exakt dasselbe Muster
/// wie `plugin::level::test_credentials`.
pub fn test_credentials(base_url: &str, token: &str) -> Result<(), PluginError> {
    let agent = build_agent();
    fetch_hardware_page(&agent, base_url, token, 0, 1)?;
    Ok(())
}

impl Plugin for SnipeitPlugin {
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
            &format!("/hardware/{external_id}"),
            &credentials.secret,
        )
    }

    fn link_system(&self, local_system_id: i64, external_id: &str) -> Result<(), PluginError> {
        // Snipe-ITs API muss von einer lokalen Verknüpfung nichts wissen --
        // rein lokales Bucheführungskonzept, siehe Trait-Dokumentation.
        // Persistiert wird das vom Aufrufer über `db::external_refs::upsert`.
        println!("SnipeitPlugin({}): verknüpfe lokales System {local_system_id} mit externer ID {external_id}", self.id);
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

fn fetch_hardware_page(
    agent: &Agent,
    base_url: &str,
    token: &str,
    offset: u32,
    limit: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{}{API_PATH}/hardware", base_url.trim_end_matches('/'));
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

fn fetch_companies_page(
    agent: &Agent,
    base_url: &str,
    token: &str,
    offset: u32,
    limit: u32,
) -> Result<serde_json::Value, PluginError> {
    let url = format!("{}{API_PATH}/companies", base_url.trim_end_matches('/'));
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

/// Ruft alle Seiten von `GET /api/v1/hardware` ab und reicht die rohen
/// Asset-Objekte (noch nicht auf `SnipeitDevice` gemappt) als eine einzige,
/// zusammengefügte Liste zurück. Bricht früher ab, wenn eine Seite weniger
/// als `PAGE_LIMIT` Zeilen liefert (letzte Seite) ODER die bereits
/// abgerufene Zeilenzahl `total` erreicht, oder wenn `MAX_PAGES` erreicht
/// ist (Schutz gegen eine sich falsch verhaltende Gegenstelle).
fn fetch_all_hardware(
    agent: &Agent,
    base_url: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut offset: u32 = 0;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_hardware_page(agent, base_url, token, offset, PAGE_LIMIT)?;
        let (rows, total) = parse_hardware_page(&page_json)?;
        let got = rows.len();
        all.extend(rows);
        offset += PAGE_LIMIT;
        if got < PAGE_LIMIT as usize || (all.len() as u64) >= total {
            break;
        }
    }
    Ok(all)
}

/// Ruft alle Seiten von `GET /api/v1/companies` ab, exakt nach demselben
/// Muster wie `fetch_all_hardware`.
fn fetch_all_companies(
    agent: &Agent,
    base_url: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, PluginError> {
    let mut all = Vec::new();
    let mut offset: u32 = 0;
    for _ in 0..MAX_PAGES {
        let page_json = fetch_companies_page(agent, base_url, token, offset, PAGE_LIMIT)?;
        let (rows, total) = parse_companies_page(&page_json)?;
        let got = rows.len();
        all.extend(rows);
        offset += PAGE_LIMIT;
        if got < PAGE_LIMIT as usize || (all.len() as u64) >= total {
            break;
        }
    }
    Ok(all)
}

fn map_ureq_error(e: ureq::Error) -> PluginError {
    match e {
        ureq::Error::StatusCode(code) if code == 401 || code == 403 => {
            PluginError::Authentication(format!("Snipe-IT-API antwortete mit Status {code}"))
        }
        ureq::Error::StatusCode(code) => {
            PluginError::Unreachable(format!("Snipe-IT-API antwortete mit Status {code}"))
        }
        ureq::Error::Json(err) => {
            PluginError::UnexpectedResponse(format!("Ungültige JSON-Antwort: {err}"))
        }
        other => PluginError::Unreachable(other.to_string()),
    }
}

/// Extrahiert Zeilen und Gesamtzahl aus einer einzelnen Snipe-IT-
/// Geräte-Seiten-Antwort (`GET /api/v1/hardware`: `{"total": <Zahl>, "rows":
/// [...], ...}`, verifiziert über Snipe-ITs eigenen Quellcode
/// (`DatatablesTransformer::transformDatatables`), siehe Moduldokumentation).
/// Reine Funktion, mit hartkodiertem JSON testbar, kein echter
/// Netzwerkzugriff nötig. `total` fehlt/hat unerwartete Form -> fällt auf die
/// tatsächliche Zeilenzahl dieser Seite zurück (dann bricht
/// `fetch_all_hardware` nach dieser einen Seite ab, statt in eine Endlos-
/// schleife zu laufen).
fn parse_hardware_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, u64), PluginError> {
    let rows = json["rows"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse(
            "Erwartete 'rows'-Liste in Snipe-IT-Geräte-Antwort".to_string(),
        )
    })?;
    let total = json["total"].as_u64().unwrap_or(rows.len() as u64);
    Ok((rows.clone(), total))
}

/// Dasselbe wie `parse_hardware_page`, nur für `GET /api/v1/companies` --
/// identischer Antwort-Umschlag (`DatatablesTransformer` wird von Snipe-IT
/// für beide Endpunkte verwendet), aber bewusst als eigene Funktion mit
/// eigener Fehlermeldung gehalten, analog zu
/// `plugin::level::parse_devices_page`/`parse_groups_page`.
fn parse_companies_page(
    json: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, u64), PluginError> {
    let rows = json["rows"].as_array().ok_or_else(|| {
        PluginError::UnexpectedResponse(
            "Erwartete 'rows'-Liste in Snipe-IT-Firmen-Antwort".to_string(),
        )
    })?;
    let total = json["total"].as_u64().unwrap_or(rows.len() as u64);
    Ok((rows.clone(), total))
}

/// Bildet eine (bereits über alle Seiten hinweg zusammengefügte) Liste roher
/// Snipe-IT-Asset-Objekte auf `SnipeitDevice`-Werte ab. Reine Funktion, mit
/// hartkodiertem JSON testbar -- Snipe-ITs "List Hardware"- und
/// "Show Hardware"-Endpunkte liefern Asset-Objekte in derselben Form.
fn map_hardware_response(rows: &[serde_json::Value]) -> Vec<SnipeitDevice> {
    rows.iter().filter_map(map_hardware_asset).collect()
}

/// Ein einzelnes Asset-Objekt aus Snipe-ITs `/hardware`-Antwort. Snipe-IT hat
/// -- anders als NinjaOne/Level.io -- oft kein sinnvoll gepflegtes `name`-
/// Feld (Assets werden dort primär über `asset_tag` identifiziert, `name`
/// ist ein optionaler Spitzname und häufig `null`/leer); der Anzeigename
/// fällt deshalb von `name` über `asset_tag` und `serial` bis zur externen
/// ID zurück. `company` ist ein verschachteltes Objekt (`{"id": ..., "name":
/// ...}`) oder `null`/fehlend, wenn das Asset keiner Firma zugeordnet ist --
/// ein legitimer Fall (siehe `UNASSIGNED_COMPANY_ID`-Dokumentation), kein
/// Fehler. Ein Asset ohne verwertbare `id` wird übersprungen statt den
/// gesamten Aufruf scheitern zu lassen -- ein einzelnes kaputtes Asset-Objekt
/// soll nicht die ganze Liste unbrauchbar machen (analog zu
/// `plugin::ninja::map_device`/`plugin::level::map_level_device`).
fn map_hardware_asset(value: &serde_json::Value) -> Option<SnipeitDevice> {
    let external_id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let asset_tag = value["asset_tag"].as_str().map(str::to_string);
    let serial = value["serial"].as_str().map(str::to_string);
    let raw_name = value["name"].as_str().filter(|s| !s.is_empty());
    let name = raw_name
        .or(asset_tag.as_deref())
        .or(serial.as_deref())
        .unwrap_or(external_id.as_str())
        .to_string();
    let company_id = match &value["company"]["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => UNASSIGNED_COMPANY_ID.to_string(),
    };
    Some(SnipeitDevice {
        external_id,
        name,
        hostname: None,
        ip_address: None,
        asset_tag,
        serial,
        company_id,
    })
}

/// Bildet die von `GET /api/v1/companies` gelieferte Zeilenliste auf
/// `SnipeitCompany`-Werte ab. Reine, für sich mit hartkodiertem JSON testbare
/// Funktion, analog zu `map_hardware_response`.
fn map_companies_response(rows: &[serde_json::Value]) -> Vec<SnipeitCompany> {
    rows.iter().filter_map(map_company).collect()
}

/// Ein einzelnes Firmenobjekt aus Snipe-ITs `/companies`-Antwort
/// (`{"id": <Zahl>, "name": "...", ...}`, verifiziert über Snipe-ITs eigenen
/// Quellcode, `CompaniesTransformer::transformCompany`). Fehlt `id` oder hat
/// eine unerwartete Form -> die Firma wird übersprungen statt den gesamten
/// Aufruf scheitern zu lassen, analog zu `map_hardware_asset`.
fn map_company(value: &serde_json::Value) -> Option<SnipeitCompany> {
    let id = match &value["id"] {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    let name = value["name"].as_str().unwrap_or(id.as_str()).to_string();
    Some(SnipeitCompany { id, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hardware_page_with_rows_and_total() {
        let json = serde_json::json!({
            "total": 2,
            "rows": [{"id": 1}, {"id": 2}]
        });
        let (rows, total) = parse_hardware_page(&json).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(total, 2);
    }

    #[test]
    fn parse_hardware_page_rejects_missing_rows_field() {
        let json = serde_json::json!({"total": 0});
        let result = parse_hardware_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn parse_hardware_page_total_defaults_to_rows_length_when_missing() {
        let json = serde_json::json!({"rows": [{"id": 1}]});
        let (rows, total) = parse_hardware_page(&json).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn parses_companies_page_with_rows_and_total() {
        let json = serde_json::json!({
            "total": 1,
            "rows": [{"id": 5, "name": "ACME GmbH"}]
        });
        let (rows, total) = parse_companies_page(&json).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn parse_companies_page_rejects_missing_rows_field() {
        let json = serde_json::json!({"total": 0});
        let result = parse_companies_page(&json);
        assert!(matches!(result, Err(PluginError::UnexpectedResponse(_))));
    }

    #[test]
    fn maps_hardware_json_array_into_snipeit_devices() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {
                    "id": 101,
                    "name": "Bürorechner Anna",
                    "asset_tag": "AT-0001",
                    "serial": "SN-XYZ",
                    "company": {"id": 7, "name": "ACME GmbH"}
                },
                {
                    "id": 202,
                    "asset_tag": "AT-0002"
                }
            ]"#,
        )
        .unwrap();

        let devices = map_hardware_response(json.as_array().unwrap());

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].external_id, "101");
        assert_eq!(devices[0].name, "Bürorechner Anna");
        assert_eq!(devices[0].asset_tag.as_deref(), Some("AT-0001"));
        assert_eq!(devices[0].serial.as_deref(), Some("SN-XYZ"));
        assert_eq!(devices[0].company_id, "7");
        assert_eq!(devices[0].hostname, None);
        assert_eq!(devices[0].ip_address, None);
        assert_eq!(devices[1].external_id, "202");
        assert_eq!(devices[1].name, "AT-0002");
    }

    #[test]
    fn falls_back_to_serial_when_no_name_or_asset_tag_present() {
        let json = serde_json::json!([{"id": 9, "serial": "SN-ONLY"}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].name, "SN-ONLY");
    }

    #[test]
    fn falls_back_to_external_id_when_nothing_else_present() {
        let json = serde_json::json!([{"id": 9}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].name, "9");
    }

    #[test]
    fn empty_name_string_falls_back_to_asset_tag() {
        let json = serde_json::json!([{"id": 9, "name": "", "asset_tag": "AT-9"}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].name, "AT-9");
    }

    #[test]
    fn skips_devices_without_a_usable_id() {
        let json = serde_json::json!([{"name": "OhneId"}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert!(devices.is_empty());
    }

    #[test]
    fn device_without_company_gets_unassigned_sentinel() {
        let json = serde_json::json!([{"id": 1, "company": null}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].company_id, UNASSIGNED_COMPANY_ID);
    }

    #[test]
    fn device_with_missing_company_field_gets_unassigned_sentinel() {
        let json = serde_json::json!([{"id": 1}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].company_id, UNASSIGNED_COMPANY_ID);
    }

    #[test]
    fn device_with_company_object_gets_its_id() {
        let json = serde_json::json!([{"id": 1, "company": {"id": 42, "name": "ACME"}}]);
        let devices = map_hardware_response(json.as_array().unwrap());
        assert_eq!(devices[0].company_id, "42");
    }

    #[test]
    fn maps_companies_json_array_into_snipeit_companies() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[
                {"id": 1, "name": "ACME Hauptsitz"},
                {"id": 2, "name": "ACME Zweigstelle"}
            ]"#,
        )
        .unwrap();

        let companies = map_companies_response(json.as_array().unwrap());

        assert_eq!(companies.len(), 2);
        assert_eq!(
            companies[0],
            SnipeitCompany {
                id: "1".to_string(),
                name: "ACME Hauptsitz".to_string()
            }
        );
        assert_eq!(
            companies[1],
            SnipeitCompany {
                id: "2".to_string(),
                name: "ACME Zweigstelle".to_string()
            }
        );
    }

    #[test]
    fn company_falls_back_to_id_when_no_name_field_present() {
        let json = serde_json::json!([{"id": 9}]);
        let companies = map_companies_response(json.as_array().unwrap());
        assert_eq!(companies[0].name, "9");
    }

    #[test]
    fn company_skips_entries_without_a_usable_id() {
        let json = serde_json::json!([{"name": "OhneId"}]);
        let companies = map_companies_response(json.as_array().unwrap());
        assert!(companies.is_empty());
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
        let plugin = SnipeitPlugin::new(
            "snipeit:acme-123".to_string(),
            "https://assets.example.com".to_string(),
        );
        assert_eq!(plugin.id(), "snipeit:acme-123");
    }
}
