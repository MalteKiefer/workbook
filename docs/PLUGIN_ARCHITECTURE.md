# Plugin-Architektur

Status: Fünf echte Integrationen umgesetzt -- NinjaOne (`plugin::ninja`,
`commands::plugins`), Level.io (`plugin::level`, `commands::level`),
Snipe-IT (`plugin::snipeit`, `commands::snipeit`), Microsoft Intune
(`plugin::intune`, `commands::intune`) und Iru (`plugin::iru`,
`commands::iru`) --, alle mit Mehrfach-Verbindungs-Unterstützung.
`DummyPlugin` bleibt als Attrappen-Referenzimplementierung bestehen. Noch
kein UI-Aufruf im Sinne einer Command Palette -- die Kommandos sind aber
vollständig Ende-zu-Ende von einem Frontend aus nutzbar (`PluginsView.tsx`
als dünne Hülle um
`NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`/`IntunePluginSection.tsx`/`IruPluginSection.tsx`).

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

## Microsoft-Intune-Plugin (`plugin::intune`) -- vierte echte Integration

`plugin/intune.rs` implementiert `Plugin` für Geräteverwaltung über die
Microsoft-Graph-API. Vierte echte Integration nach NinjaOne, Level.io und
Snipe-IT -- strukturell am nächsten an Level.io (eine Verbindung bindet
direkt an genau einen lokalen Kunden, kein granulares
Organisations-Zuordnungs-Konzept nötig), authentifiziert aber wie NinjaOne
über einen OAuth2-Client-Credentials-Grant, nur gegen Microsofts eigene
Identitätsplattform statt NinjaOnes und mit einem dritten Geheimwert
(`tenant_id`) zusätzlich zu `client_id`/`client_secret`.

- **Authentifizierung**: OAuth2-Client-Credentials-Grant gegen Azure AD
  (`POST https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token`,
  `grant_type=client_credentials`, `client_id`, `client_secret`,
  `scope=https://graph.microsoft.com/.default`) -- verifiziert gegen
  Microsofts eigene Dokumentation der Identitätsplattform. Genau wie bei
  `plugin::ninja` wird der Zugriffstoken bewusst nicht zwischen Aufrufen
  zwischengespeichert, sondern pro Trait-Methodenaufruf neu geholt -- diese
  App ruft Plugin-Methoden selten und manuell auf, nie in einer heißen
  Schleife, daher bleibt das einfach und korrekt genug für v1.
- **Fester Host, kein `base_url`**: anders als NinjaOne/Snipe-IT (selbst
  gehostete bzw. regionsabhängige Instanzen) ist der Host der
  Microsoft-Graph-API immer `graph.microsoft.com` -- deshalb, wie bei
  Level.ios `BASE_URL`-Konstante, kein vom Nutzer angegebenes
  `base_url`-Feld an `IntuneConnectionMeta` nötig.
- **Geräte**: `GET {GRAPH_BASE_URL}/v1.0/deviceManagement/managedDevices`,
  paginiert über `@odata.nextLink` (eine vollständige Fortsetzungs-URL im
  Antwort-Umschlag, Microsoft Graphs Standard-Paginierungsmuster --
  verifiziert gegen Microsofts eigene Graph-API-Dokumentation für diesen
  Endpunkt). `IntunePlugin::list_devices` durchläuft alle Seiten intern (bis
  zu `MAX_PAGES` Seiten als Schutz gegen eine sich falsch verhaltende
  Gegenstelle) und liefert eine einzige, bereits zusammengefügte Liste --
  der Aufrufer sieht nichts von Graphs Paginierung, dasselbe Prinzip wie bei
  Level.io/Snipe-IT. Reine, für sich testbare Funktionen `parse_devices_page`
  (Seiten-Antwort -> Geräte-Array + Fortsetzungs-URL) und
  `map_intune_devices` (Geräte-Array -> `IntuneDevice`), beide mit
  hartkodierten JSON-Fixtures getestet (die Geräte-Fixture stammt direkt aus
  Microsofts eigenem offiziellem Beispiel für diesen Endpunkt), kein echter
  Netzwerkzugriff in Tests.
- **Gerätedetails**: `GET {GRAPH_BASE_URL}/v1.0/deviceManagement/managedDevices/{id}`,
  reicht die Antwort unverändert als `serde_json::Value` durch, genau wie
  bei Level.io/NinjaOne.
- **Nur ein Ausschnitt der Felder abgebildet**: ein reales
  `managedDevices`-Objekt hat rund 50 Felder; dieses Modul dekodiert nur die
  tatsächlich genutzten (`id`, `deviceName`, `operatingSystem`, `osVersion`,
  `serialNumber`, `manufacturer`, `model`, `complianceState`,
  `lastSyncDateTime`, `userPrincipalName`) -- dieselbe
  "nur abbilden, was auch genutzt wird"-Konvention, die NinjaOnes
  Geräteabbildung bereits verfolgt. `deviceName` dient als
  identifizierendes/Anzeige-Feld, analog dazu wie Ninja/Level `hostname`
  verwenden -- es füllt sowohl `IntuneDevice::name` als auch
  `IntuneDevice::hostname`, da Intunes Geräteobjekt kein von `deviceName`
  getrenntes physisches Hostname-Feld kennt.
- **Keine IP-Adresse**: verifiziert gegen Microsofts eigenes offizielles
  `managedDevices`-Schema -- an keiner Stelle dieses Objekts existiert ein
  IP-Adress-Feld (`wiFiMacAddress` ist eine MAC-Adresse, keine IP-Adresse,
  und auch sonst nicht als eine solche nutzbar). `IntuneDevice::ip_address`
  ist deshalb IMMER `None` -- keine Auslassung aus Bequemlichkeit, sondern
  dieselbe ehrlich verifizierte Abwesenheit, die Snipe-IT für
  `hostname`/`ip_address` bereits dokumentiert (siehe oben, Abschnitt
  "Snipe-IT-Plugin").
- **HTTP-Client**: dieselbe `ureq`-3.4.1-Abhängigkeit wie
  `plugin::ninja`/`plugin::level`/`plugin::snipeit`, kein zweiter
  HTTP-Client in dieser Codebasis.
- **Zugangsdaten-Kodierung**: Intune braucht drei Geheimwerte (`tenant_id`,
  `client_id`, `client_secret`) -- einen mehr als NinjaOnes zwei.
  `PluginCredentials.secret` ist laut Trait-Vertrag ein einziger opaker
  String, den das Plugin selbst interpretiert -- hier als JSON-Objekt
  kodiert (`serde_json::to_string`/`from_str`), genau wie `plugin::ninja`
  seine `NinjaCredentials` kodiert, nur mit einem dritten Feld.

### Intune-Verbindungen sind 1:1 an einen Kunden gebunden

- Nicht-geheime Metadaten (`id`, `customer_id`, `label`) liegen als
  `IntuneConnectionMeta` in `Config::intune_connections` (`config.toml`,
  `#[serde(default)]`-kompatibel mit älteren Konfigurationen ohne dieses
  Feld). Bewusst OHNE `base_url` -- siehe `GRAPH_BASE_URL` oben.
- Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"intune:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Level.io/NinjaOne.
- Weil jede Verbindung genau eine `customer_id` trägt, braucht
  `sync_intune_connection` keine Fallunterscheidung "zugeordnet/
  unzugeordnet" wie `sync_ninja_connection` -- jedes synchronisierte Gerät
  gehört automatisch zum Kunden der Verbindung, `linked_system_id` wird für
  jedes Gerät direkt gegen die `external_refs`-Zeilen dieses Kunden geprüft.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie Level.io, nur ohne Gruppierung (Intune kennt
kein Gruppen-Konzept): `sync_intune_connection` schreibt das Ergebnis jedes
Laufs zusätzlich als JSON nach
`data_dir/plugin-cache/intune-<connection_id>.json`
(`{"synced_at_utc": "...", "devices": [...]}`). `get_cached_intune_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff) und liefert `None`,
wenn für eine Verbindung noch nie synchronisiert wurde. `remove_intune_connection`
löscht diese Cache-Datei (bestes Bemühen).

### Tauri-Kommandos (`commands::intune`)

`test_intune_connection`, `list_intune_connections`, `add_intune_connection`,
`remove_intune_connection`, `sync_intune_connection`,
`get_cached_intune_sync`, `link_system_to_intune`,
`unlink_system_from_intune`, `get_intune_system_details` -- dünne Wrapper
nach demselben Muster wie `commands::level`, ohne
Organisations-Zuordnungskommandos (kein Intune-Äquivalent zu
`map_ninja_organization`/`unmap_ninja_organization` nötig, siehe oben).

`test_intune_connection` prüft ein Tenant-ID/Client-ID/Client-Secret-Tripel
per OAuth2-Grant, ohne irgendetwas zu persistieren. Das Übernehmen eines
extern gelieferten Werts in ein selbst gepflegtes Feld (`name`, `hostname`,
`ip_address`, `notes` in `systems`) bleibt dabei -- wie bei den anderen drei
Integrationen -- ausschließlich eine bewusste, manuelle Aktion über
`get_intune_system_details` plus eine spätere UI-Aktion; kein Kommando hier
schreibt automatisch in diese vier Felder.

### Frontend (`IntunePluginSection.tsx`)

Strukturell an `LevelPluginSection.tsx` angelehnt (Kunde-Zuordnungs-`<select>`
inklusive "+ Neuen Kunden anlegen…" im Verbindungs-Anlage-Dialog,
Vergleichs-/Übernahme-Panel für verknüpfte Geräte, `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation), aber ohne Level.ios Gruppen-Verschachtelung -- Intune
kennt kein Gruppen-Konzept, die Geräteliste ist deshalb eine echte flache,
filterbare, 10-pro-Seite-paginierte Liste ohne zusätzliche Verschachtelungs-
ebene. Der Verbindungs-Anlage-Dialog hat vier Felder statt Levels zwei
(Label, Tenant-ID, Client-ID, Client-Secret als `type="password"`) -- analog
zu NinjaOnes Client-ID/-Secret-Paar, nur mit der zusätzlichen Tenant-ID.
`DeviceSummaryLine` zeigt Betriebssystem/-version und Compliance-Status
zusätzlich zum Namen an; das Vergleichs-Panel ergänzt schreibgeschützte
Zusatzinformationen (Compliance, Betriebssystem, Hersteller, Modell,
Seriennummer, Benutzer), da ein lokales System dafür keine eigenen Felder
hat. Analog zu Snipe-ITs `asset_tag`/`serial`-Vorbelegung schreibt
"Neu anlegen" Seriennummer/Compliance-Status/Betriebssystem einmalig in das
neu angelegte Systems `notes`-Feld (`buildInitialNotes`) -- eine einmalige
Vorbelegung bei der Erstanlage, keine spätere automatische Überschreibung.
Die IP-Adress-Zeile im Vergleichs-Panel erscheint wie bei den anderen drei
Integrationen, liefert für Intune aber nie einen externen Wert (siehe oben,
"Keine IP-Adresse") -- "Übernehmen" bleibt für diese Zeile deshalb immer
deaktiviert. `PluginsView.tsx` bindet die Sektion als vierte Karte neben
`NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`
ein.

## Iru-Plugin (`plugin::iru`) -- fünfte echte Integration

`plugin/iru.rs` implementiert `Plugin` für Irus öffentliche REST-API über
HTTPS. Iru ist ein Apple-MDM-Produkt (Mobile Device Management).

**Umbenennungs-Hinweis für künftige Leser**: Iru hieß früher "Kandji". Das
Produkt wurde zu "Iru" umbenannt, aber Irus eigene öffentliche API-Referenz
(<https://api-docs.iru.com>) verwendet zum Zeitpunkt dieser Umsetzung
weiterhin die alten `kandji.io`/`api.kandji.io`-Domainnamen und an mehreren
Stellen weiterhin den Namen "Kandji". Das ist kein Fehler in `plugin::iru`
oder `commands::iru` -- Host-Muster wie `{sub_domain}.api.kandji.io` sind
wörtlich aus Irus eigener aktueller Dokumentation übernommen, kein Rest aus
der Zeit vor der Umbenennung.

Strukturell am nächsten an Level.io -- einfacheres 1:1-Modell wie Level,
aber mit einer selbst-gehosteten-artigen Basis-URL wie Snipe-IT:

- **Authentifizierung**: statischer Bearer-Token im `Authorization`-Header
  (`Authorization: Bearer <token>`) -- ein einziger Geheimwert, kein
  OAuth2-Grant, kein Token-Austausch. Wird 1:1 als `PluginCredentials.secret`
  durchgereicht, genau wie bei Level.io/Snipe-IT (keine JSON-Kodierung
  mehrerer Werte nötig, anders als bei NinjaOne).
- **Selbst-gehostete-artige, Mandant-pro-Subdomain-Basis-URL**: anders als
  Level.ios feste `BASE_URL`-Konstante ist Iru Mandant-pro-Subdomain:
  `https://{sub_domain}.api.kandji.io` (US-Region) oder
  `https://{sub_domain}.api.eu.kandji.io` (EU-Region). Wird wie bei
  NinjaOne/Snipe-IT als nutzerseitig eingegebenes `base_url`-Feld an der
  Verbindung behandelt (`IruConnectionMeta.base_url`) -- der Nutzer gibt
  seine vollständige API-URL inklusive Subdomain und Region ein (z. B.
  `https://acme.api.kandji.io`), keine Aufteilung in getrennte
  Subdomain-/Region-Felder, genau wie Snipe-IT seine ganze Basis-URL als
  einen einzigen String entgegennimmt.
- **Kein Organisations-/Mandanten-Konzept**: Irus API kennt keine
  Organisations-/Site-/Konto-Scoping-Endpunkte -- eine Verbindung (eine
  Subdomain + ein Bearer-Token) entspricht direkt genau einem lokalen
  Kunden (`IruConnectionMeta.customer_id`), dasselbe einfache 1:1-Modell wie
  bei Level.io (`plugin::level`), NICHT die granulare
  Pro-Mandant-Zuordnung, die NinjaOne/Snipe-IT brauchen
  (`NinjaOrgMapping`/`SnipeitCompanyMapping`).
- **Geräte**: `GET {base_url}/api/v1/devices?limit=300`, live gegen Irus
  aktuelle API-Dokumentation verifiziert. Unterstützt optional einen
  `offset`-Query-Parameter für Paginierung über eine Seite hinaus. Anders
  als Snipe-ITs `{"total": ..., "rows": [...]}`-Umschlag ist die Antwort ein
  **reines JSON-Array** -- live verifiziert, über Irus eigenes aktuelles
  Beispiel bestätigt. `list_devices` durchläuft alle Seiten intern (bis zu
  `MAX_PAGES` Seiten à `PAGE_LIMIT` Geräten, dieselbe defensive
  Schleifen-Konvention wie bei `plugin::snipeit`/`plugin::level`, Schutz
  gegen eine sich falsch verhaltende Gegenstelle) und liefert eine einzige,
  bereits zusammengefügte Liste -- der Aufrufer sieht nichts von Irus
  Paginierung. Da es (anders als bei Snipe-IT) kein `total`-Feld zum
  Gegenprüfen gibt, gilt eine Seite mit weniger als `PAGE_LIMIT` Zeilen als
  letzte Seite -- genau dieselbe Rückfalllogik, auf die
  `plugin::snipeit::fetch_all_hardware` bereits zurückfällt, wenn Snipe-ITs
  eigenes `total`-Feld fehlt/unbrauchbar ist.
- **Polymorphes Geräteobjekt -- Felder defensiv behandelt**: Irus eigene
  Dokumentation beschreibt `/api/v1/devices` als polymorph: "If Windows or
  Android management is turned on, additional fields will be returned in
  the response. All visible fields based on platform enablement status will
  be present for all device types, but values will be blank for
  non-applicable devices." Jedes Feld hier außer `device_id` ist deshalb
  `Option<String>`, defensiv gelesen (`.as_str()`, nie ein als-vorhanden
  angenommenes Feld), nie ein harter Parse-Fehler wegen eines einzelnen
  fehlenden/leeren Feldes.
- **Kein Hostname-/IP-Adress-Feld**: verifiziert über Irus eigenes
  Live-Beispiel für `/api/v1/devices` -- das Geräteobjekt hat weder ein
  Hostname- noch ein IP-Adress-Feld, dieselbe Situation wie bei Snipe-IT
  (siehe `plugin::snipeit`-Moduldoku). `IruDevice.hostname`/`ip_address`
  sind deshalb IMMER `None` -- keine Vermutung, sondern eine ehrliche,
  verifizierte Auslassung. `device_name` ist Irus identifizierendes
  Anzeigefeld (wie Ninjas/Levels `hostname`); für den
  "mit bestehendem System verknüpfen"-Abgleichsschlüssel folgt dieses Modul
  Snipe-ITs eigenem Vorbild (`SnipeitPluginSection.tsx::matchKeyForDevice`),
  da hier ebenfalls kein Hostname-Feld existiert: das erste vorhandene
  identifizierende Feld, in der Reihenfolge `serial_number` dann
  `asset_tag`, verglichen gegen das freie Textfeld `hostname` des lokalen
  Systems (Frontend-Aufgabe, siehe `IruPluginSection.tsx`).
- **Gerätedetails**: ein Einzelgerät-GET-Endpunkt existiert,
  `GET {base_url}/api/v1/devices/{device_id}` -- folgt derselben
  URL-Familie wie Irus dokumentierte Geräte-Aktions-Endpunkte (z. B.
  `.../devices/{device_id}/action/shutdown`). Reicht die Antwort
  unverändert als `serde_json::Value` durch, genau wie bei jedem anderen
  Plugin.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie
  `plugin::ninja`/`plugin::level`/`plugin::snipeit` (kein zweiter
  HTTP-Client in dieser Codebasis).
- **Zugangsdaten-Kodierung**: Iru braucht nur einen einzigen Geheimwert (den
  Bearer-Token), 1:1 als `PluginCredentials.secret` durchgereicht -- wie
  Level.io/Snipe-IT, keine JSON-Kodierung mehrerer Werte nötig (anders als
  NinjaOne).

### Iru-Verbindungen sind 1:1 an einen Kunden gebunden

- Nicht-geheime Metadaten (`id`, `customer_id`, `label`, `base_url`) liegen
  als `IruConnectionMeta` in `Config::iru_connections` (`config.toml`,
  `#[serde(default)]`-kompatibel mit älteren Konfigurationen ohne dieses
  Feld). Anders als `LevelConnectionMeta` (feste `BASE_URL`-Konstante),
  aber wie `SnipeitConnectionMeta`: eine Iru-Verbindung trägt zusätzlich
  eine nutzerseitig eingegebene `base_url` -- Iru ist
  Mandant-pro-Subdomain, kein fester Host.
- Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"iru:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Level.io/Snipe-IT
  (`commands::iru::generate_connection_id`/`plugin_id_for`).
- Weil jede Verbindung genau eine `customer_id` trägt, braucht
  `sync_iru_connection` keine Fallunterscheidung "zugeordnet/unzugeordnet"
  wie `sync_ninja_connection`/`sync_snipeit_connection` -- jedes
  synchronisierte Gerät gehört automatisch zum Kunden der Verbindung,
  `linked_system_id` wird für jedes Gerät direkt gegen die
  `external_refs`-Zeilen dieses Kunden geprüft.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie Level.io/Snipe-IT, nur ohne
Organisations-/Firmen-Gruppierung: `sync_iru_connection` schreibt das
Ergebnis jedes Laufs zusätzlich als JSON nach
`data_dir/plugin-cache/iru-<connection_id>.json`
(`{"synced_at_utc": "...", "devices": [...]}`). `get_cached_iru_sync` liest
ausschließlich diese Datei (kein Netzwerkzugriff) und liefert `None`, wenn
für eine Verbindung noch nie synchronisiert wurde. `remove_iru_connection`
löscht diese Cache-Datei (bestes Bemühen).

### Tauri-Kommandos (`commands::iru`)

`test_iru_connection`, `list_iru_connections`, `add_iru_connection`,
`remove_iru_connection`, `sync_iru_connection`, `get_cached_iru_sync`,
`link_system_to_iru`, `unlink_system_from_iru`, `get_iru_system_details` --
dünne Wrapper nach demselben Muster wie `commands::level`, aber mit
`base_url` sowohl im Anlage-Kommando als auch im Verbindungs-DTO (wie
`commands::snipeit`), und ohne Organisations-Zuordnungskommandos (kein
Iru-Äquivalent zu `map_ninja_organization`/`unmap_ninja_organization` bzw.
`map_snipeit_company`/`unmap_snipeit_company` nötig, siehe oben).

`test_iru_connection` prüft eine Basis-URL/Token-Kombination per
leichtgewichtigem Aufruf (eine Seite mit `limit=1`), ohne irgendetwas zu
persistieren. Das Übernehmen eines extern gelieferten Werts in ein selbst
gepflegtes Feld (`name`, `hostname`, `ip_address`, `notes` in `systems`)
bleibt dabei -- wie bei allen anderen Plugins -- ausschließlich eine
bewusste, manuelle Aktion über `get_iru_system_details` plus eine spätere
UI-Aktion; kein Kommando hier schreibt automatisch in diese vier Felder.

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Wie Snipe-IT (und anders als NinjaOne/Level.io, die `hostname` als
Abgleichsschlüssel verwenden) hat ein Iru-Gerät kein Hostname-Feld.
`IruPluginSection.tsx`s `matchKeyForDevice` nimmt deshalb das erste
vorhandene, wirklich identifizierende Iru-Feld in dieser Reihenfolge:
`serial_number` zuerst (Irus primärer Pro-Gerät-Identifikator), dann
`asset_tag` (ein freies, von einem Admin ggf. nicht gepflegtes Textfeld).
Verglichen wird dieser Wert weiterhin gegen das einzige freie Textfeld, das
ein lokales System dafür hat -- `System.hostname` --, exakt wie bei
Snipe-IT/NinjaOne/Level.io. Weil ein lokales System kein eigenes
`serial_number`/`asset_tag`-Feld hat, werden diese beiden Iru-Felder beim
"Neu anlegen" zusätzlich einmalig in das neu angelegte Systems
`notes`-Feld geschrieben (siehe `IruPluginSection.tsx::createAndLink`) --
eine einmalige Vorbelegung bei der Erstanlage, keine spätere automatische
Überschreibung.

### Frontend (`IruPluginSection.tsx`)

Strukturell am nächsten an `LevelPluginSection.tsx`: Kunde-Zuordnungs-
`<select>` inklusive "+ Neuen Kunden anlegen…" im Anlage-Formular statt
einer separaten Organisations-/Firmen-Zuordnungs-UI. Anders als
`LevelPluginSection.tsx` gibt es aber KEINE Gruppierung der Geräteliste --
Iru hat kein Level-artiges "Groups"-Konzept, daher ist die Geräteliste je
Verbindung eine einzige flache, filterbare, 10-pro-Seite-paginierte Liste
mit `j`/`k`/`Enter`/`l`/`u`-Tastaturnavigation (strukturell einfacher als
`LevelPluginSection.tsx`s Gruppen-Verschachtelung, näher an
`SnipeitPluginSection.tsx`s Geräteliste innerhalb einer einzelnen Firma,
nur ohne die Firmen-Ebene selbst). Der Verbindungs-Anlage-Dialog hat vier
Felder (Kunde, Label, Base-URL, API-Token als `type="password"`).
`DeviceSummaryLine` zeigt `model`/`serial_number`/`asset_tag` statt
`hostname`/`ip_address` (Letztere sind für Iru immer leer, siehe oben).
`PluginsView.tsx` bindet die Sektion als fünfte Karte neben
`NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`/`IntunePluginSection.tsx`
ein.
