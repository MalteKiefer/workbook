# Plugin-Architektur

Status: Vier echte Integrationen umgesetzt -- NinjaOne (`plugin::ninja`,
`commands::plugins`), Level.io (`plugin::level`, `commands::level`),
Snipe-IT (`plugin::snipeit`, `commands::snipeit`) und Apple Business Manager
(`plugin::abm`, `commands::abm`) --, alle mit
Mehrfach-Verbindungs-Unterstützung. `DummyPlugin` bleibt als
Attrappen-Referenzimplementierung bestehen. Noch kein UI-Aufruf im Sinne
einer Command Palette -- die Kommandos sind aber vollständig Ende-zu-Ende
von einem Frontend aus nutzbar (`PluginsView.tsx` als dünne Hülle um
`NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`/
`AbmPluginSection.tsx`).

## Isolationsprinzip

Ein Plugin implementiert `trait Plugin` (`src-tauri/src/plugin/mod.rs`):
`id`, `list_systems`, `get_system_details`, `link_system`. Ein Plugin fasst
**nie** SQLite direkt an — die einzige Berührung mit der Anwendung läuft über
dieses Trait-Interface. Der Aufrufer (die Anwendung) ist dafür zuständig, das
Ergebnis von `list_systems`/`get_system_details` in die Tabelle
`external_refs` zu schreiben, über
`db::external_refs::upsert(conn, system_id, plugin_id, external_id,
payload_json, tz)`. `link_system` selbst schreibt nichts in die Datenbank —
es meldet dem Plugin nur, dass eine Verknüpfung besteht; das Persistieren
bleibt beim Aufrufer.

Jede Trait-Methode ist eine synchrone, reine Funktion mit direktem
Rückgabewert — keine Signatur setzt einen interaktiven Dialog oder eine
Maus-Interaktion voraus. Eine künftige Command Palette kann jeden
Plugin-Aufruf direkt hinter einen Tastaturbefehl hängen, ohne die Trait-Form
ändern zu müssen.

## Credential-Prinzip

Zugangsdaten für ein Plugin (API-Keys, Benutzername/Passwort, …) landen
ausschließlich im OS-Schlüsselspeicher — Windows Credential Manager bzw.
Secret Service unter Linux, über die `keyring`-Crate (Version 4.2.0,
`v1`-Kompatibilitäts-API). Der Wrapper dafür ist
`src-tauri/src/plugin/secrets.rs`: `store_secret(plugin_id, secret)` und
`load_secret(plugin_id) -> Option<String>`, Service-Name `"wartungsdoku"`,
Konto = `plugin_id`. Zugangsdaten stehen **nie** in `config.toml` und **nie**
in der SQLite-Datenbank — auch nicht verschlüsselt. Ein Plugin erhält seine
Zugangsdaten zur Laufzeit als opakes `PluginCredentials { secret }` und
interpretiert dessen Inhalt selbst; dieses Crate öffnet ihn nicht.

Kein automatisierter Test ruft `store_secret`/`load_secret` gegen den
echten Schlüsselspeicher auf — das würde auf Entwicklungsrechnern einen
verwaisten Credential-Eintrag hinterlassen, und Sandbox-/CI-Umgebungen haben
oft gar keinen echten Schlüsselspeicher. Die Korrektheit des Wrappers stützt
sich auf sauberes Kompilieren/Typchecking gegen die reale `keyring`-API
(gegen Version 4.2.0 verifiziert: `Entry::new(service, username) ->
Result<Self>`, `set_password(&str) -> Result<()>`, `get_password() ->
Result<String>`, `Error::NoEntry`) sowie die Testsuite der Crate selbst.

Genau diese Trennung ist es auch, die es sicher macht, `config.toml`
(nicht-geheime Verbindungs-Metadaten wie `ninja_connections`/
`ninja_org_mappings`) unverändert in ein `backup::create_backup`-Zip
aufzunehmen (siehe `src-tauri/src/backup/mod.rs`): weil Zugangsdaten dort
nachweislich nie hineingelangen, kann die Datei als Ganzes gesichert und
wiederhergestellt werden, ohne je ein Geheimnis in einer Backup-Datei
abzulegen. Nach einer Wiederherstellung (insbesondere auf einem anderen
Rechner oder nach geleertem Schlüsselspeicher) sind Verbindungs-Metadaten
also sofort wieder da, die zugehörigen Zugangsdaten müssen aber erneut über
`add_ninja_connection`/das Level.io-Äquivalent eingegeben werden.

## Externe Daten überschreiben nie selbst gepflegte Inhalte

`external_refs`-Zeilen sind grundsätzlich ergänzend, nie autoritativ für ein
System. Konkret: Wenn eine künftige UI die Felder eines Systems zusammen mit
verknüpften `external_refs.payload_json`-Daten anzeigt, gewinnen immer die
selbst gepflegten Felder in der `systems`-Tabelle — `name`, `hostname`,
`ip_address`, `notes` (vom Nutzer eingegeben). Extern gelieferte Daten werden
als zusätzliche, klar als "extern" markierte, rein lesbare Information
danebengestellt, niemals automatisch in diese vier Felder zurückgeschrieben.
Eine Übernahme externer Werte in ein selbst gepflegtes Feld ist ausschließlich
eine bewusste, manuelle Aktion des Nutzers — kein automatischer Sync-Schritt
darf das je tun.

## Eigenes Plugin ergänzen (Anleitung)

`src-tauri/src/plugin/dummy.rs` (`DummyPlugin`) ist die Vorlage: eine
dokumentierte Referenzimplementierung, die `Plugin` vollständig und
Ende-zu-Ende lauffähig umsetzt, aber ausschließlich statische Beispieldaten
liefert (kein Netzwerkzugriff). Für eine echte Integration (z. B. ein
RMM-Tool):

1. Neues Modul unter `src-tauri/src/plugin/<name>.rs`, `struct <Name>Plugin`,
   `impl Plugin for <Name>Plugin` — Signaturen wie bei `DummyPlugin`
   abschreiben.
2. `list_systems`/`get_system_details` rufen den echten externen Dienst auf,
   authentifiziert über die per `plugin::secrets::load_secret(id)` geladenen
   Zugangsdaten; Netzwerk-/Auth-/Parsing-Fehler werden auf `PluginError`
   gemappt (`Authentication`, `Unreachable`, `UnexpectedResponse`).
3. Der Aufrufer (nicht das Plugin) schreibt Ergebnisse über
   `db::external_refs::upsert` in die Datenbank.
4. `pub mod <name>;` in `plugin/mod.rs` ergänzen.

Noch nicht Teil dieser Ausbaustufe: ein generisches Plugin-Registry/-Loader
für beliebige künftige Plugins und Command-Palette-Einträge. Tauri-Kommandos,
die ein konkretes Plugin (NinjaOne) aus dem Frontend ansteuerbar machen, gibt
es inzwischen -- siehe nächster Abschnitt.

## NinjaOne-Plugin (`plugin::ninja`) -- erste echte Integration

`plugin/ninja.rs` implementiert `Plugin` für NinjaOnes öffentliche REST-API
über HTTPS. Die Umsetzung folgt exakt der obigen Anleitung:

- **Authentifizierung**: OAuth2-Client-Credentials-Grant gegen
  `POST {base_url}/ws/oauth/token` (`grant_type=client_credentials`,
  `client_id`, `client_secret`, `scope=monitoring`). Der Zugriffstoken wird
  bewusst nicht zwischen Aufrufen zwischengespeichert, sondern pro
  Trait-Methodenaufruf neu geholt -- diese App ruft Plugin-Methoden selten und
  manuell auf, nie in einer heißen Schleife, daher ist das einfach und korrekt
  genug für v1.
- **Geräte**: `list_systems` ruft `GET {base_url}/v2/devices` auf und bildet
  jedes Geräteobjekt auf `ExternalSystem` ab (reine, für sich testbare
  Funktion `map_devices_response`, mit hartkodierten JSON-Fixtures getestet,
  kein echter Netzwerkzugriff in Tests). `get_system_details` ruft
  `GET {base_url}/v2/device/{id}` auf und reicht die Antwort unverändert als
  `serde_json::Value` durch.
- **HTTP-Client**: `ureq` 3.4.1, synchron (kein async-Runtime in dieser
  Codebasis), mit `default-features = false, features = ["rustls", "gzip",
  "json"]` in `Cargo.toml` -- `rustls` ist absichtlich explizit statt über die
  Crate-Standardauswahl aktiviert, um klarzustellen, dass bewusst der reine
  Rust-TLS-Stack (kein systemweit installiertes OpenSSL nötig) verwendet wird.
  Verbindungs-/Timeout-/Nicht-2xx-Fehler werden auf `PluginError::Unreachable`
  gemappt, HTTP 401/403 auf `PluginError::Authentication`, unerwartete
  JSON-Formen (inkl. fehlerhafter Zugangsdaten-Antwort) auf
  `PluginError::UnexpectedResponse`/`Authentication`.
- **Zugangsdaten-Kodierung**: NinjaOne braucht zwei Geheimwerte
  (`client_id`, `client_secret`); `PluginCredentials.secret` ist laut
  Trait-Vertrag aber ein einziger opaker String. `plugin::ninja` kodiert
  beide als JSON-Objekt in diesem einen String
  (`serde_json::to_string(&NinjaCredentials { .. })`) und parst ihn beim
  Aufruf wieder zurück -- ohne die Trait-Signatur zu ändern.

### Mehrere Ninja-Verbindungen, jede mit mehreren Organisationen

Ein Nutzer kann mehrere Ninja-Mandanten verwalten, daher ist eine
"Ninja-Verbindung" ein eigenständiger, konfigurierbarer Datensatz statt eines
fest verdrahteten Singletons. WICHTIG (Korrektur gegenüber einer früheren
Ausbaustufe): Eine Verbindung ist ein Satz OAuth2-Zugangsdaten für **genau
einen Ninja-Mandanten**, aber **nicht** an genau einen lokalen Kunden
gebunden. NinjaOne modelliert innerhalb eines Mandanten selbst mehrere
"Organizations" -- ein realistisches Szenario ist, dass der lokale Kunde, der
diese Ninja-Verbindung besitzt, selbst ein Managed-Service-Provider ist und
darin wiederum mehrere eigene Kunden als getrennte Organisationen führt. Eine
1:1-Bindung Verbindung->Kunde (wie in einer früheren Ausbaustufe über ein
`customer_id`-Feld an der Verbindung selbst) wäre für dieses Szenario falsch.

- Nicht-geheime Metadaten (`id`, `label`, `base_url`, bewusst OHNE
  `customer_id`) liegen als `NinjaConnectionMeta` in
  `Config::ninja_connections` (`config.toml`, `#[serde(default)]`-kompatibel
  mit älteren Konfigurationen ohne dieses Feld).
- Die Verbindungs-`id` wird beim Anlegen aus dem Label geschlagwortet
  (`slugify`) und um einen Millisekunden-Zeitstempel ergänzt, kein
  zusätzliches `uuid`-Crate nötig.
- Der vollqualifizierte Bezeichner `"ninja:<connection_id>"` dient
  gleichzeitig als Schlüsselspeicher-Konto (`plugin::secrets::store_secret`)
  und als `external_refs.plugin_id` -- ein System kann also unabhängig je
  Verbindung verknüpft werden, ohne die bestehende
  Eine-Zeile-je-(system_id,plugin_id)-Upsert-Semantik in
  `db::external_refs` anzufassen.
- Welche Organisation innerhalb einer Verbindung welchem lokalen Kunden
  entspricht (falls überhaupt), ist eine separate, granulare Zuordnung:
  `NinjaOrgMapping { connection_id, organization_id, organization_name,
  customer_id }` in `Config::ninja_org_mappings` (ebenfalls
  `#[serde(default)]`-kompatibel). Jede Organisation kann unabhängig
  zugeordnet oder unzugeordnet gelassen werden; eine unzugeordnete
  Organisation liefert bei jeder Synchronisierung ihre Geräte weiterhin (zur
  Ansicht), aber immer mit `linked_system_id: None` -- ohne `customer_id`
  gibt es keine sinnvolle Menge lokaler Systeme, gegen die man
  querverweisen könnte.
- `GET {base_url}/v2/organizations` (dieselbe OAuth2-Bearer-Authentifizierung
  wie Geräte) liefert die Organisationsliste einer Verbindung, gemappt über
  die reine, für sich testbare Funktion `map_organizations_response` in
  `plugin::ninja` (Organisationsobjekt laut NinjaOnes öffentlicher
  API-Spezifikation: `{"id": <Zahl>, "name": "..."}`).
- Jedes Geräteobjekt aus `GET {base_url}/v2/devices` trägt ein
  `organizationId`-Feld (Integer laut NinjaOnes Spezifikation), das es genau
  einer Organisation zuordnet. `plugin::ninja::NinjaDevice` (gemappt über die
  reine Funktion `map_ninja_devices_response`) erweitert die bestehende
  Geräteabbildung um `organization_id` sowie `ip_address` -- Letzteres aus
  dem `ipAddresses`-Array (primäre/erste Adresse), mit `publicIP` als
  Rückfallebene. Dieser reichhaltigere Typ ist bewusst getrennt vom
  schmalen, plugin-übergreifenden `ExternalSystem` aus `plugin::mod`
  (`Plugin::list_systems`) gehalten -- der bleibt das trait-generische
  Minimum, das auch `DummyPlugin` erfüllen können muss, und wird von der
  Ninja-Synchronisierung inzwischen nicht mehr verwendet.

### Zwischenspeicher für Offline-Ansicht (`data_dir/plugin-cache/`)

`sync_ninja_connection` schreibt das Ergebnis jedes Laufs zusätzlich als JSON
nach `data_dir/plugin-cache/ninja-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`, Zeitstempel über die bestehende
`time::now_with_tz`-Konvention). `get_cached_ninja_sync` liest ausschließlich
diese Datei (kein Netzwerkzugriff) und liefert `None`, wenn für eine
Verbindung noch nie synchronisiert wurde -- so kann eine UI beim Öffnen einer
Verbindung sofort die zuletzt bekannten Geräte anzeigen, während "Sync jetzt"
(`sync_ninja_connection`) die explizite Live-Aktualisierung bleibt.
`remove_ninja_connection` löscht diese Cache-Datei (bestes Bemühen) und alle
`ninja_org_mappings`-Zeilen der entfernten Verbindung gleich mit.

### Tauri-Kommandos (`commands::plugins`)

`test_ninja_connection`, `list_ninja_connections`, `add_ninja_connection`,
`remove_ninja_connection`, `list_ninja_organizations`,
`map_ninja_organization`, `unmap_ninja_organization`, `sync_ninja_connection`,
`get_cached_ninja_sync`, `link_system_to_ninja`, `unlink_system_from_ninja`,
`get_ninja_system_details` -- dünne Wrapper nach dem Muster von
`commands::export`.

`test_ninja_connection` prüft ein Zugangsdaten-Paar per OAuth2-Grant, ohne
irgendetwas zu persistieren (für ein "Verbindung testen" vor dem Anlegen).
`list_ninja_organizations` liefert die Live-Organisationsliste einer
Verbindung, angereichert um `mapped_customer_id` (aus
`Config::ninja_org_mappings`, `None` falls unzugeordnet).
`map_ninja_organization`/`unmap_ninja_organization` pflegen genau diese
Zuordnung (Upsert bzw. Löschen nach `(connection_id, organization_id)`);
`unmap_ninja_organization` rührt bestehende `external_refs`-Verknüpfungen
nicht an -- ein bereits verknüpftes Gerät bleibt verknüpft, auch wenn seine
Organisation nachträglich entzuordnet wird, das Trennen ist eine bewusste,
separate Aktion.

`sync_ninja_connection` liefert `Vec<NinjaOrgDeviceGroupDto>` -- Geräte nach
Organisation gruppiert, jede Gruppe mit ihrer `customer_id` (`None`, wenn
unzugeordnet). Für jede zugeordnete Organisation wird für jedes Gerät
zusätzlich `linked_system_id` befüllt, falls bereits ein lokales System
dieses Kunden für diese Verbindung verknüpft ist, und in diesem Fall gleich
`external_refs.payload_json`/`synced_at_*` aktualisiert. Für unzugeordnete
Organisationen bleibt `linked_system_id` immer `None`, kein
`external_refs`-Zugriff nötig. Das Übernehmen eines extern gelieferten Werts
in ein selbst gepflegtes Feld (`name`, `hostname`, `ip_address`, `notes` in
`systems`) bleibt dabei ausschließlich eine bewusste, manuelle Aktion über
`get_ninja_system_details` plus eine spätere UI-Aktion -- kein Kommando hier
schreibt automatisch in diese vier Felder.

## Level.io-Plugin (`plugin::level`) -- zweite echte Integration

`plugin/level.rs` implementiert `Plugin` für Level.ios öffentliche REST-API
über HTTPS (`https://api.level.io/v2`, feste Konstante -- Level hat anders
als NinjaOne keine Regionen-/Instanz-Varianten). Deutlich einfacher als die
NinjaOne-Integration:

- **Authentifizierung**: statischer API-Key im `Authorization`-Header, OHNE
  `Bearer`-Präfix und ohne Token-Austausch -- verifiziert gegen Level.ios
  eigene Referenzseite (<https://developers.level.io/reference/authentication>:
  "Provide your API key as the authorization value", Beispiel
  `-H "Authorization: APIKEY"`). Kein OAuth2-Grant wie bei NinjaOne.
- **Kein Organisations-/Mandanten-Konzept**: Level.ios API-Referenz kennt
  keine Organisations-, Konto- oder Site-Endpunkte (verifiziert über
  <https://developers.level.io/llms.txt>). Level selbst arbeitet auf
  Kontoebene. Deshalb entspricht eine Level-"Verbindung" (ein API-Key) hier
  direkt genau einem lokalen Kunden -- `LevelConnectionMeta.customer_id` --,
  OHNE die granulare Organisations-Zuordnungsebene, die NinjaOne braucht
  (`NinjaOrgMapping`). Das ist bewusst das einfache 1:1-Modell, das für
  NinjaOne in einer früheren Ausbaustufe verworfen wurde (siehe oben) -- für
  Level ist es korrekt, weil Level keine Unter-Mandanten kennt.
- **Geräte**: `GET {BASE_URL}/devices`, cursor-paginiert (`has_more` +
  `starting_after`, verifiziert über
  <https://developers.level.io/reference/listdevices>). `LevelPlugin::list_devices`
  durchläuft alle Seiten intern (bis zu 20 Seiten à 100 Geräten als Schutz
  gegen eine sich falsch verhaltende Gegenstelle) und liefert eine einzige,
  bereits zusammengefügte Liste -- der Aufrufer sieht nichts von Levels
  Pagination. Reine, für sich testbare Funktionen `parse_devices_page`
  (Seiten-Antwort -> Geräte-Array + Fortsetzungs-Flag) und
  `map_level_devices` (Geräte-Array -> `LevelDevice`), beide mit
  hartkodierten JSON-Fixtures getestet, kein echter Netzwerkzugriff in Tests.
- **Gerätedetails**: `GET {BASE_URL}/devices/{id}` (verifiziert über
  <https://developers.level.io/reference/showdevice>, "Show Device" --
  existiert als echter Einzelgeräte-Endpunkt, keine Notlösung über die
  Listen-API nötig), reicht die Antwort unverändert als `serde_json::Value`
  durch.
- **IP-Adresse**: `include_network_interfaces=true` liefert pro Gerät ein
  `network_interfaces`-Array (`[{..., "ip_addresses": ["..."]}]`) --
  verifiziert über Levels Antwortschema für `GET /v2/devices`. Anders als
  NinjaOne hat Level kein flaches IP-Feld auf oberster Ebene; die erste
  nicht-leere Adresse der ersten Netzwerkschnittstelle wird verwendet
  (`extract_ip_address`). Diese IP-Extraktion ist also eine echte, verifizierte
  Zuordnung -- keine Auslassung.
- **HTTP-Client**: dieselbe `ureq`-3.4.1-Abhängigkeit wie `plugin::ninja`,
  kein zweiter HTTP-Client in dieser Codebasis.
- **Zugangsdaten-Kodierung**: Level braucht nur einen einzigen Geheimwert
  (den API-Key), der 1:1 als `PluginCredentials.secret` durchgereicht wird --
  anders als bei NinjaOne keine JSON-Kodierung mehrerer Werte nötig.

### Level-Verbindungen sind 1:1 an einen Kunden gebunden

- Nicht-geheime Metadaten (`id`, `customer_id`, `label`) liegen als
  `LevelConnectionMeta` in `Config::level_connections` (`config.toml`,
  `#[serde(default)]`-kompatibel mit älteren Konfigurationen ohne dieses
  Feld).
- Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"level:<connection_id>"`-Bezeichner (Schlüsselspeicher-
  Konto UND `external_refs.plugin_id`) folgen exakt demselben Muster wie bei
  NinjaOne (`commands::level::generate_connection_id`/`plugin_id_for`,
  identisch zu den Ninja-Gegenstücken).
- Weil jede Verbindung genau eine `customer_id` trägt, braucht
  `sync_level_connection` keine Fallunterscheidung "zugeordnet/
  unzugeordnet" wie `sync_ninja_connection` -- jedes synchronisierte Gerät
  gehört automatisch zum Kunden der Verbindung, `linked_system_id` wird für
  jedes Gerät direkt gegen die `external_refs`-Zeilen dieses Kunden geprüft.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie NinjaOne, nur ohne Organisations-Gruppierung:
`sync_level_connection` schreibt das Ergebnis jedes Laufs zusätzlich als JSON
nach `data_dir/plugin-cache/level-<connection_id>.json`
(`{"synced_at_utc": "...", "devices": [...]}`). `get_cached_level_sync` liest
ausschließlich diese Datei (kein Netzwerkzugriff) und liefert `None`, wenn für
eine Verbindung noch nie synchronisiert wurde. `remove_level_connection`
löscht diese Cache-Datei (bestes Bemühen).

### Tauri-Kommandos (`commands::level`)

`test_level_connection`, `list_level_connections`, `add_level_connection`,
`remove_level_connection`, `sync_level_connection`, `get_cached_level_sync`,
`link_system_to_level`, `unlink_system_from_level`,
`get_level_system_details` -- dünne Wrapper nach demselben Muster wie
`commands::plugins`, aber ohne Organisations-Zuordnungskommandos (kein
Level-Äquivalent zu `map_ninja_organization`/`unmap_ninja_organization`
nötig, siehe oben).

`test_level_connection` prüft einen API-Key per leichtgewichtigem Aufruf
(eine Seite mit `limit=1`), ohne irgendetwas zu persistieren. Das Übernehmen
eines extern gelieferten Werts in ein selbst gepflegtes Feld (`name`,
`hostname`, `ip_address`, `notes` in `systems`) bleibt dabei -- wie bei
NinjaOne -- ausschließlich eine bewusste, manuelle Aktion über
`get_level_system_details` plus eine spätere UI-Aktion; kein Kommando hier
schreibt automatisch in diese vier Felder.

### Frontend (`PluginsView.tsx`)

`PluginsView.tsx` ist eine dünne Hülle, die nur noch die "Plugins"-Überschrift
rendert und zwei Plugin-Sektionen einbindet: `NinjaPluginSection.tsx`
(mechanisch unverändert aus der ursprünglichen `PluginsView.tsx`
herausgezogen) und `LevelPluginSection.tsx` (neu). Beide folgen denselben
UI-Konventionen (Karten-Layout, Busy/Status/Error-Zustand je Aktion, siehe
auch `BackupView.tsx`). `LevelPluginSection.tsx` ist strukturell einfacher als
die Ninja-Sektion: eine flache Geräteliste je Verbindung statt einer
Gruppierung nach Organisation, ein Kunde-Auswahlfeld im Anlage-Formular
(statt einer separaten Organisations-Zuordnungs-UI), und kein
Level-Dashboard-Link-Element (kein verifiziertes Geräte-URL-Muster in Levels
öffentlicher Doku gefunden).

## Snipe-IT-Plugin (`plugin::snipeit`) -- dritte echte Integration

`plugin/snipeit.rs` implementiert `Plugin` für Snipe-ITs öffentliche REST-API
über HTTPS. Snipe-IT ist ein selbst gehostetes, quelloffenes
IT-Asset-Management-System -- strukturell näher an NinjaOne (Mehrfach-
Mandantenfähigkeit über eigene "Companies" innerhalb einer Verbindung,
konfigurierbare `base_url`) als an Level.io, aber mit einer wichtigen,
verifizierten Abweichung von beiden: Snipe-IT ist Asset-/Inventar-
verwaltung, keine RMM-Überwachungssoftware.

- **Authentifizierung**: statischer Bearer-Token im `Authorization`-Header
  (`Authorization: Bearer <token>`) -- ein "Personal Access Token", den der
  Nutzer selbst in Snipe-ITs eigener Weboberfläche erzeugt (Profil -> API
  Tokens; verifiziert über Snipe-ITs eigenes `routes/api.php` auf GitHub:
  `POST/GET/DELETE /api/v1/account/personal-access-tokens`). Kein
  OAuth2-Grant wie bei NinjaOne, kein Token-Austausch -- genauso einfach wie
  Level.ios Authentifizierung, nur mit `Bearer `-Präfix (Level hat keins).
- **Selbst gehostet**: wie NinjaOne (und anders als Level.ios feste
  `BASE_URL`-Konstante) braucht eine Snipe-IT-Verbindung eine vom Nutzer
  angegebene Basis-URL (`SnipeitConnectionMeta.base_url`); `/api/v1` wird
  beim Aufbau jeder Anfrage-URL fest angehängt.
- **Firmen (Mehrmandantenfähigkeit)**: `GET {base_url}/api/v1/companies`
  (verifiziert über Snipe-ITs `routes/api.php`) liefert die Firmenliste einer
  Instanz. Eine einzelne Snipe-IT-Instanz kann Assets mehrerer Firmen
  verwalten (z. B. ein MSP, der Kundenbestände in einer gemeinsamen Instanz
  führt) -- deshalb exakt dasselbe granulare Zuordnungsprinzip wie bei
  NinjaOnes "Organizations": `SnipeitCompanyMapping { connection_id,
  company_id, company_name, customer_id }` in
  `Config::snipeit_company_mappings`, `SnipeitConnectionMeta` selbst bewusst
  OHNE `customer_id`.
- **Assets**: `GET {base_url}/api/v1/hardware`, Offset-paginiert (`limit`/
  `offset`-Query-Parameter -- NICHT Cursor-basiert wie NinjaOne/Level.io),
  verifiziert über Snipe-ITs eigene API-Referenzseite. Der Standard-`limit`-
  Wert ist mit 2 absurd niedrig, `plugin::snipeit` schickt deshalb immer
  explizit `PAGE_LIMIT` (100) mit. Der Antwort-Umschlag ist über Snipe-ITs
  eigenen Quellcode verifiziert (`DatatablesTransformer::transformDatatables`):
  `{"total": <Zahl>, "rows": [...], "current_page": ..., "per_page": ...,
  "total_pages": ..., "prev_page_url": ..., "next_page_url": ...}` --
  `total`/`rows`, keine Vermutung. `list_devices`/`list_companies` durchlaufen
  alle Seiten intern (bis zu `MAX_PAGES` Seiten à `PAGE_LIMIT`, Schutz gegen
  eine sich falsch verhaltende Gegenstelle) und liefern eine einzige,
  bereits zusammengefügte Liste, exakt dasselbe Prinzip wie bei
  NinjaOne/Level.io.
- **Kein Hostname/keine IP-Adresse**: verifiziert über Snipe-ITs eigenen
  Quellcode (`AssetsTransformer::transformAsset`) -- das Kern-Asset-Objekt
  hat nachweislich weder ein Hostname- noch ein IP-Adress-Feld.
  `SnipeitDevice.hostname`/`ip_address` sind deshalb IMMER `None` -- keine
  Auslassung aus Bequemlichkeit, sondern eine verifizierte, ehrliche
  Tatsache. Stattdessen sind Snipe-ITs eigene, natürliche
  Identifikationsfelder -- `asset_tag` (Snipe-ITs primärer Identifikator,
  `name` ist oft leer/`null`) und `serial` -- hier erstklassige Felder.
  Snipe-IT liefert zusätzlich ein `custom_fields`-Objekt pro Asset
  (verifiziert: nach admin-konfiguriertem Feldnamen benannte Schlüssel,
  keine feste Liste) -- da diese Feldnamen instanzspezifisch und frei
  konfigurierbar sind, verzichtet `plugin::snipeit` bewusst auf brüchige
  Ratelogik nach einem "hostname"-artigen benutzerdefinierten Feld; das
  saubere, `asset_tag`/`serial`-basierte Modell mit `hostname`/`ip_address`
  immer `None` ist für v1 die ehrliche, korrekte Lösung.
- **Web-Oberflächen-Link**: `{base_url}/hardware/{id}` zeigt die
  Asset-Detailseite in Snipe-ITs eigener Weboberfläche, verifiziert über
  Snipe-ITs `routes/web/hardware.php` (Laravel-Resource-Route
  `hardware/{asset}`). `commands::snipeit::ExternalSystemDto.snipeit_url`
  ist deshalb ein echtes, aus Snipe-ITs Quellcode abgeleitetes
  Gegenstück zu `ExternalSystemDto.ninja_url`, kein erfundenes URL-Schema.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie
  `plugin::ninja`/`plugin::level`.
- **Zugangsdaten-Kodierung**: Snipe-IT braucht nur einen einzigen Geheimwert
  (den Personal Access Token), 1:1 als `PluginCredentials.secret`
  durchgereicht -- wie Level.io, keine JSON-Kodierung mehrerer Werte nötig
  (anders als NinjaOne).

### Snipe-IT-Verbindungen, jede mit mehreren Firmen

Strukturell identisch zu NinjaOnes Verbindungs-/Organisations-Modell:

- Nicht-geheime Metadaten (`id`, `label`, `base_url`, bewusst OHNE
  `customer_id`) liegen als `SnipeitConnectionMeta` in
  `Config::snipeit_connections` (`#[serde(default)]`-kompatibel).
  Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"snipeit:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei NinjaOne/Level.io.
- Welche Firma innerhalb einer Verbindung welchem lokalen Kunden entspricht
  (falls überhaupt), steht granular in `Config::snipeit_company_mappings`.
  Eine nicht zugeordnete Firma liefert bei jeder Synchronisierung ihre
  Assets weiterhin (zur Ansicht), aber immer mit `linked_system_id: None`.
- Ein Asset, das laut Snipe-IT selbst KEINER Firma zugeordnet ist
  (`"company": null`/fehlend -- ein legitimer Fall, z. B. bei
  Alleinstellungs-Instanzen, die das Firmen-Konzept gar nicht nutzen),
  bekommt den synthetischen, nicht-numerischen Platzhalter
  `plugin::snipeit::UNASSIGNED_COMPANY_ID`. `commands::snipeit::
  group_devices_by_company` (analog zu `commands::plugins::
  group_devices_by_organization`) zeigt solche Assets als eigene Gruppe
  ("Ohne Firma (Snipe-IT)") statt sie stillschweigend zu verwerfen.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie NinjaOne/Level.io:
`sync_snipeit_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/snipeit-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`). `get_cached_snipeit_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff), rejoint dabei aber
`customer_id` je Gruppe live gegen die AKTUELLEN
`Config::snipeit_company_mappings` (nicht den beim letzten Sync
eingefrorenen Wert) -- exakt wie `commands::plugins::get_cached_ninja_sync`
--, und liefert `None`, wenn für eine Verbindung noch nie synchronisiert
wurde. `remove_snipeit_connection` löscht diese Cache-Datei (bestes
Bemühen) und alle `snipeit_company_mappings`-Zeilen der entfernten
Verbindung gleich mit.

### Tauri-Kommandos (`commands::snipeit`)

`test_snipeit_connection`, `list_snipeit_connections`,
`add_snipeit_connection`, `remove_snipeit_connection`,
`list_snipeit_companies`, `map_snipeit_company`, `unmap_snipeit_company`,
`sync_snipeit_connection`, `get_cached_snipeit_sync`,
`link_system_to_snipeit`, `unlink_system_from_snipeit`,
`get_snipeit_system_details` -- dünne Wrapper nach dem Muster von
`commands::plugins`. `list_snipeit_companies` liefert die Live-Firmenliste
einer Verbindung (analog zu `list_ninja_organizations`), wird aber vom
Frontend nicht aufgerufen -- `SnipeitPluginSection.tsx` ist wie
`NinjaPluginSection.tsx` konsequent Cache-first (`get_cached_snipeit_sync`
beim Öffnen, `sync_snipeit_connection` nur auf "Aktualisieren"); der Befehl
bleibt für Symmetrie und einen möglichen künftigen
Ersteinrichtungs-Anwendungsfall erhalten. Das Übernehmen eines extern
gelieferten Werts in ein selbst gepflegtes Feld (`name`, `hostname`,
`ip_address`, `notes` in `systems`) bleibt -- wie bei NinjaOne/Level.io --
ausschließlich eine bewusste, manuelle Aktion über
`get_snipeit_system_details` plus eine spätere UI-Aktion; kein Kommando
hier schreibt automatisch in diese vier Felder.

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Anders als NinjaOne/Level.io (die per Konvention `hostname` als
Abgleichsschlüssel für den "Mit bestehendem System verknüpfen"-Vorschlag
verwenden) hat ein Snipe-IT-Asset kein Hostname-Feld. `SnipeitPluginSection.
tsx`s `matchKeyForDevice` nimmt deshalb das erste vorhandene, wirklich
identifizierende Snipe-IT-Feld in dieser Reihenfolge: `asset_tag` zuerst
(Snipe-ITs primärer Identifikator), dann `hostname` (falls doch einmal
vorhanden), zuletzt `serial`. Verglichen wird dieser Wert weiterhin gegen
das einzige freie Textfeld, das ein lokales System dafür hat --
`System.hostname` --, exakt wie bei NinjaOne/Level.io. Weil ein lokales
System kein eigenes `asset_tag`/`serial`-Feld hat, werden diese beiden
Snipe-IT-Felder beim "Neu anlegen" zusätzlich einmalig in das neu
angelegte Systems `notes`-Feld geschrieben (siehe
`SnipeitPluginSection.tsx::createAndLink`) -- eine einmalige Vorbelegung bei
der Erstanlage, keine spätere automatische Überschreibung.

### Frontend (`SnipeitPluginSection.tsx`)

Strukturell die reifste, aktuellste Fassung des Musters -- mechanisch an
`NinjaPluginSection.tsx`s post-Paginierung-Stand angelehnt (Firmen
eingeklappt mit Geräte-Anzahl-Zusammenfassung, Kunde-Zuordnungs-`<select>`
inklusive "+ Neuen Kunden anlegen…", aufklappbare, filterbare,
10-pro-Seite-paginierte Geräteliste mit `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation, Vergleichs-/Übernahme-Panel für verknüpfte Geräte). Der
Verbindungs-Anlage-Dialog hat drei Felder statt Ninjas vier (Label,
Base-URL, Personal-Access-Token als `type="password"`) -- kein
Client-ID/-Secret-Paar nötig. `DeviceSummaryLine` zeigt `asset_tag`/`serial`
statt `hostname`/`ip_address` (Letztere sind für Snipe-IT praktisch immer
leer, siehe oben), unterdrückt aber den redundanten
`asset_tag`-Zusatz, wenn der Anzeigename ohnehin schon der Asset-Tag ist
(Backend-Fallback-Fall). `PluginsView.tsx` bindet die Sektion als dritte
Karte neben `NinjaPluginSection.tsx`/`LevelPluginSection.tsx` ein.

## Apple-Business-Manager-Plugin (`plugin::abm`) -- vierte echte Integration

`plugin/abm.rs` implementiert `Plugin` für Apple Business Managers (ABM)
öffentliche REST-API über HTTPS. Vierte echte Integration nach NinjaOne,
Level.io und Snipe-IT -- und mit deutlichem Abstand die aufwendigste
Authentifizierung aller vier Plugins: ein ES256-signiertes JWT als
Client-Assertion, gegen einen OAuth2-Access-Token eingetauscht. Beide
Schritte sind gegen Apples eigene Referenzimplementierung verifiziert, keine
Vermutung.

### Authentifizierung: JWT-Client-Assertion + OAuth2-Token-Austausch

**Schritt 1 -- Client-Assertion bauen.** Ein vom Nutzer in ABMs eigener
Oberfläche (Einstellungen -> API) erzeugter "API-Client" liefert drei Werte:
eine Client-ID (Format `BUSINESSAPI.<uuid>`), eine Key-ID sowie einen
privaten EC-P256-Schlüssel im unverschlüsselten PKCS#8-PEM-Format
(`-----BEGIN PRIVATE KEY-----`). Aus diesen drei Werten wird ein
ES256-signiertes JWT gebaut:

- Header: `{"alg": "ES256", "kid": "<key_id>", "typ": "JWT"}`.
- Nutzlast: `{"iss": "<client_id>", "sub": "<client_id>", "aud":
  "https://account.apple.com/auth/oauth2/v2/token", "iat": <jetzt>, "exp":
  <jetzt + 900>, "jti": "<zufällig>"}` -- 15 Minuten Gültigkeit.

WICHTIG, verifiziert und bewusst so: die `aud`-Klaim trägt `/v2/token`, der
tatsächlich in Schritt 2 angesprochene Token-Endpunkt (`TOKEN_URL` in
`plugin::abm`) hat dieses `/v2` in seinem Pfad aber NICHT. Dieser scheinbare
Widerspruch stammt unmittelbar aus Apples eigener Referenzimplementierung
und ist kein zu behebender Fehler -- `plugin::abm` bildet ihn deshalb exakt
so nach, mit einem Kommentar an genau dieser Stelle, damit niemand ihn
versehentlich "korrigiert".

Signiert wird über die `jsonwebtoken`-Crate (Version 9):
`EncodingKey::from_ec_pem(private_key_pem.as_bytes())` liest den PEM-Schlüssel,
`encode(&header, &claims, &key)` mit `Header { alg: Algorithm::ES256, kid:
Some(key_id), .. }` erzeugt das fertige JWT. Für die `jti`-Klaim ist keine
eigene `uuid`-Abhängigkeit nötig -- `rand` ist ohnehin schon Abhängigkeit
dieser Codebasis, `plugin::abm::generate_jti` erzeugt daraus 16 Zufallsbytes
und formatiert sie mit gesetzten RFC-4122-Versions-/Varianten-Bits als
kanonisch aussehenden UUID-v4-String (rein kosmetisch -- Apple validiert die
interne Struktur von `jti` nicht, wichtig ist nur ein frischer, effektiv
eindeutiger Wert je Assertion).

**Schritt 2 -- Token-Austausch.**
`POST https://account.apple.com/auth/oauth2/token`,
`Content-Type: application/x-www-form-urlencoded`, mit Formularfeldern
`grant_type=client_credentials`, `client_id=<client_id>`,
`client_assertion_type=urn:ietf:params:oauth:client-assertion-type:jwt-bearer`,
`client_assertion=<in Schritt 1 gebautes JWT>`, `scope=business.api`. Die
Antwort ist ein Standard-OAuth2-JSON-Token-Response
(`access_token`/`token_type`/`expires_in`) -- nur `access_token` wird
verwendet. Genau wie bei NinjaOnes OAuth2-Grant wird der Access-Token
bewusst nicht zwischen Aufrufen zwischengespeichert, sondern
(`fetch_access_token`) pro Trait-Methodenaufruf frisch geholt -- dieselbe
Begründung wie bei `plugin::ninja`: diese App ruft Plugin-Methoden selten
und manuell auf, nie in einer heißen Schleife.

**Schritt 3 -- Geräte abrufen.** `GET
https://api-business.apple.com/v1/orgDevices` (feste Basis-URL, `API_BASE`
-- anders als NinjaOne/Snipe-IT ist ABM Apples eigener gehosteter Dienst,
keine Instanz-/Regionen-Variante, also kein nutzerseitig konfigurierbares
`base_url`-Feld), `Authorization: Bearer <access_token>`. Die Antwort folgt
dem JSON:API-Format: `{"data": [{"id": ..., "type": "orgDevices",
"attributes": {"serialNumber": ..., "deviceModel": ..., ...}}], "links":
{"next": "..."}}`.

### Zugangsdaten-Kodierung: drei Geheimwerte statt einem

ABM braucht drei Geheimwerte (`client_id`, `key_id`, `private_key_pem`).
`PluginCredentials.secret` ist laut Trait-Vertrag aber ein einziger opaker
String. `plugin::abm` kodiert alle drei als JSON-Objekt in diesem einen
String (`AbmCredentials { client_id, key_id, private_key_pem }`,
`serde_json::to_string`/`from_str`) -- exakt dasselbe Prinzip wie
`plugin::ninja::NinjaCredentials` für seine zwei Geheimwerte, nur um einen
dritten erweitert. Der private Schlüssel landet dabei wie jedes andere
Plugin-Geheimnis ausschließlich im OS-Schlüsselspeicher (siehe
"Credential-Prinzip" oben) -- als reiner PEM-Text, den der Nutzer in ein
`<textarea>` einfügt (`AbmPluginSection.tsx`), nicht als Dateiverweis. Das
folgt demselben Credential-Prinzip wie jedes andere Plugin dieser Codebasis:
Zugangsdaten sind immer ein opaker String, nie ein Dateipfad.

### JSON:API-Pagination über `links.next`

Anders als NinjaOne/Level.io (Cursor-Query-Parameter) und Snipe-IT
(`limit`/`offset`) liefert ABM Pagination im JSON:API-üblichen Format: jede
Seite trägt optional `links.next` als vollständige, direkt abrufbare
Folge-URL. `plugin::abm::fetch_all_devices` folgt dieser Kette intern (bis
zu `MAX_PAGES` = 50 Seiten à `PAGE_LIMIT` = 100 Geräten als Schutz gegen
eine sich falsch verhaltende Gegenstelle) und liefert eine einzige, bereits
zusammengefügte Liste -- der Aufrufer sieht nichts von ABMs Pagination,
exakt dasselbe Prinzip wie bei NinjaOne/Level.io/Snipe-IT. Fehlt
`links.next` auf einer Seite, wird das defensiv als letzte (oder einzige)
Seite behandelt, nicht als Fehler -- `parse_devices_page` ist eine reine,
für sich mit hartkodierten JSON-Fixtures testbare Funktion.

### Kein Hostname/keine IP-Adresse

Wie Snipe-IT (Asset-/Inventarverwaltung, keine RMM-Überwachungssoftware) ist
auch ABM strukturell kein Live-Telemetrie-Dienst, sondern ein
Einkaufs-/Registrierungsverzeichnis: ein `orgDevices`-Objekt hat laut Apples
eigener Attributliste (`serialNumber`, `deviceModel`, `productFamily`,
`productType`, `deviceCapacity`, `color`, `status`) weder ein
Hostname- noch ein IP-Adress-Feld. `AbmDevice.hostname`/`ip_address` sind
deshalb IMMER `None` -- keine Auslassung aus Bequemlichkeit, sondern eine
verifizierte, ehrliche Tatsache, genau wie bei `plugin::snipeit`. Stattdessen
ist `serial_number` (die auf dem physischen Gerät aufgedruckte Seriennummer)
ABMs primäres, natürliches Identifikationsfeld -- das Gegenstück zu
Snipe-ITs `asset_tag` --, `device_model` der natürliche Anzeigename-Fallback
(`map_abm_device`: Anzeigename fällt von `deviceModel` über `serialNumber`
auf die externe ID zurück, ABM-Geräte haben kein frei vergebbares
Spitzname-Feld wie Levels `nickname`).

### ABM-Verbindungen sind 1:1 an einen Kunden gebunden

Genau wie Level.io (und anders als NinjaOne/Snipe-IT) ist ABM inhärent
einzel-organisationsgebunden: ein Satz Zugangsdaten spricht für genau eine
Apple-Business-Manager-Organisation, es gibt kein Unter-Mandanten-/
Site-Konzept. Eine ABM-"Verbindung" entspricht deshalb hier direkt genau
einem lokalen Kunden -- `AbmConnectionMeta.customer_id`, OHNE die granulare
Organisations-Zuordnungsebene, die NinjaOne/Snipe-IT brauchen.

- Nicht-geheime Metadaten (`id`, `customer_id`, `label`, bewusst OHNE
  `base_url` -- siehe `API_BASE`) liegen als `AbmConnectionMeta` in
  `Config::abm_connections` (`config.toml`, `#[serde(default)]`-kompatibel
  mit älteren Konfigurationen ohne dieses Feld).
- Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"abm:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Level.io/Snipe-IT.
- Weil jede Verbindung genau eine `customer_id` trägt, braucht
  `sync_abm_connection` keine Fallunterscheidung "zugeordnet/unzugeordnet"
  wie `sync_ninja_connection` -- jedes synchronisierte Gerät gehört
  automatisch zum Kunden der Verbindung, `linked_system_id` wird für jedes
  Gerät direkt gegen die `external_refs`-Zeilen dieses Kunden geprüft.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie Level.io: `sync_abm_connection` schreibt das
Ergebnis jedes Laufs zusätzlich als JSON nach
`data_dir/plugin-cache/abm-<connection_id>.json` (`{"synced_at_utc": "...",
"devices": [...]}`). `get_cached_abm_sync` liest ausschließlich diese Datei
(kein Netzwerkzugriff) und liefert `None`, wenn für eine Verbindung noch nie
synchronisiert wurde. `remove_abm_connection` löscht diese Cache-Datei
(bestes Bemühen).

### Tauri-Kommandos (`commands::abm`)

`test_abm_connection`, `list_abm_connections`, `add_abm_connection`,
`remove_abm_connection`, `sync_abm_connection`, `get_cached_abm_sync`,
`link_system_to_abm`, `unlink_system_from_abm`, `get_abm_system_details` --
dünne Wrapper nach demselben Muster wie `commands::level`, ohne
Organisations-Zuordnungskommandos (kein ABM-Äquivalent zu
`map_ninja_organization`/`unmap_ninja_organization` nötig, siehe oben).

`test_abm_connection` prüft ein Client-ID/Key-ID/Private-Key-Tripel über
Apples OAuth2-Token-Endpunkt (Client-Credentials-Grant via signierter
JWT-Assertion), ohne irgendetwas zu persistieren -- so bemerkt der Nutzer
einen fehlerhaft eingefügten Schlüssel oder eine vertippte ID, bevor
Zugangsdaten tatsächlich in den Schlüsselspeicher geschrieben werden. Das
Übernehmen eines extern gelieferten Werts in ein selbst gepflegtes Feld
(`name`, `hostname`, `ip_address`, `notes` in `systems`) bleibt dabei -- wie
bei allen anderen drei Plugins -- ausschließlich eine bewusste, manuelle
Aktion über `get_abm_system_details` plus eine spätere UI-Aktion; kein
Kommando hier schreibt automatisch in diese vier Felder.

### Frontend (`AbmPluginSection.tsx`)

Strukturell am engsten an `LevelPluginSection.tsx` angelehnt: Kunde-Auswahl
im Anlage-Formular (inklusive "+ Neuen Kunden anlegen…"), Cache-first
(`get_cached_abm_sync` beim Öffnen, `sync_abm_connection` nur auf
"Aktualisieren"), 10-pro-Seite-paginierte, filterbare, `j`/`k`/`Enter`/`l`/
`u`-tastaturnavigierbare Geräteliste, Vergleichs-/Übernahme-Panel für
verknüpfte Geräte. Anders als bei Level gibt es aber KEINE Gruppierungsebene
-- ABM kennt kein Level-artiges "Groups"-Konzept, daher ist die Geräteliste
je Verbindung eine einzige flache, navigierbare Liste ohne
Gruppen-Kopfzeilen (eine strukturell vereinfachte Fassung von Levels Muster,
mit der Gruppen-Zwischenschicht ersatzlos entfernt).

Der Verbindungs-Anlage-Dialog hat fünf Felder statt Levels drei (Kunde,
Label, Client-ID, Key-ID, sowie ein mehrzeiliges `<textarea>` für den
privaten Schlüssel -- bewusst kein einzeiliges `<input>`, PEM-Schlüssel sind
mehrzeilig). Wie bei Snipe-IT (kein Hostname-Feld) nimmt der
Verknüpfungs-Vorschlag "Mit bestehendem System verknüpfen"
(`matchKeyForDevice`) ABMs eigenes natürliches Identifikationsfeld,
`serial_number`, als Abgleichsschlüssel gegen `System.hostname` -- das
einzige freie Textfeld, das ein lokales System dafür hat. Weil ein lokales
System kein eigenes `serial_number`/`device_model`-Feld hat, werden diese
beiden ABM-Felder beim "Neu anlegen" zusätzlich einmalig in das neu
angelegte Systems `notes`-Feld geschrieben (`AbmPluginSection.tsx::
createAndLink`, mechanisch identisch zu `SnipeitPluginSection.tsx`s
gleichnamiger Funktion) -- eine einmalige Vorbelegung bei der Erstanlage,
keine spätere automatische Überschreibung. `PluginsView.tsx` bindet die
Sektion als vierte Karte neben `NinjaPluginSection.tsx`/
`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx` ein.
