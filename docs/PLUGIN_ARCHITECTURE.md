# Plugin-Architektur

Status: Neun echte Integrationen umgesetzt, nämlich NinjaOne (`plugin::ninja`,
`commands::plugins`), Level.io (`plugin::level`, `commands::level`),
Snipe-IT (`plugin::snipeit`, `commands::snipeit`), Microsoft Intune
(`plugin::intune`, `commands::intune`), Iru (`plugin::iru`,
`commands::iru`), Jamf Pro (`plugin::jamf`, `commands::jamf`), Apple
Business Manager (`plugin::abm`, `commands::abm`), Tactical RMM
(`plugin::tacticalrmm`, `commands::tacticalrmm`) und Acronis Cyber Protect
Cloud (`plugin::acronis`, `commands::acronis`), alle mit
Mehrfach-Verbindungs-Unterstützung. Acronis ist dabei anders als alle acht
übrigen: kein RMM/MDM-Gerätebestand, sondern Sicherungsstatus pro Gerät
(siehe eigener Abschnitt unten). `DummyPlugin` bleibt als
Attrappen-Referenzimplementierung bestehen. Noch kein UI-Aufruf im Sinne
einer Command Palette -- die Kommandos sind aber vollständig Ende-zu-Ende
von einem Frontend aus nutzbar (`PluginsView.tsx` als dünne Hülle um
`NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`/`IntunePluginSection.tsx`/`IruPluginSection.tsx`/`JamfPluginSection.tsx`/`AbmPluginSection.tsx`/`TacticalRmmPluginSection.tsx`/`AcronisPluginSection.tsx`).

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

## Jamf-Pro-Plugin (`plugin::jamf`) -- sechste echte Integration

`plugin/jamf.rs` implementiert `Plugin` für Jamf Pros öffentliche REST-API
über HTTPS. Jamf Pro ist Apples eigenes Geräteverwaltungswerkzeug (MDM) für
macOS/iOS/iPadOS -- strukturell näher an NinjaOne (Mehrfach-Standort-
Delegation über eigene "Sites" innerhalb einer Verbindung, konfigurierbare
`base_url`) als an Level.io. Die Umsetzung folgt exakt der obigen Anleitung:

- **Authentifizierung**: OAuth2-Client-Credentials-Grant, strukturell
  identisch zu NinjaOne -- `POST {base_url}/api/oauth/token`,
  `Content-Type: application/x-www-form-urlencoded`, Body
  `grant_type=client_credentials&client_id=<id>&client_secret=<secret>`
  (verifiziert über Jamfs eigene Entwicklerdoku,
  <https://developer.jamf.com>). Anders als bei NinjaOne gibt es kein
  `scope`-Feld. Der Zugriffstoken wird -- wie bei NinjaOne -- bewusst nicht
  zwischen Aufrufen zwischengespeichert, sondern pro Trait-Methodenaufruf neu
  geholt.
- **Selbst gehostet/cloud-gehostet**: wie NinjaOne (und anders als Level.ios
  feste `BASE_URL`-Konstante) braucht eine Jamf-Verbindung eine vom Nutzer
  angegebene Basis-URL (`JamfConnectionMeta.base_url`, z. B.
  `https://yourserver.jamfcloud.com`).
- **Sites (Mehrfach-Standort-Delegation)**: `GET {base_url}/api/v1/sites`
  (verifiziert über Jamfs eigene Entwicklerdoku,
  <https://developer.jamf.com/jamf-pro/reference/get_v1-sites>) liefert eine
  einfache JSON-Liste von `{"id": "...", "name": "...", "divisionId": ...}`-
  Objekten, NICHT paginiert -- strukturell identisch zu NinjaOnes
  `GET /v2/organizations`. Ein einzelner Jamf-Pro-Server kann Inventar über
  mehrere "Sites" delegieren (z. B. ein MSP oder eine Organisation mit
  mehreren Standorten) -- deshalb exakt dasselbe granulare
  Zuordnungsprinzip wie bei NinjaOnes "Organizations":
  `JamfSiteMapping { connection_id, site_id, site_name, customer_id }` in
  `Config::jamf_site_mappings`, `JamfConnectionMeta` selbst bewusst OHNE
  `customer_id`.
- **Computer**: `GET {base_url}/api/v1/computers-inventory?section=GENERAL&section=HARDWARE`,
  Seite/Seitengröße-paginiert (`page`/`page-size`-Query-Parameter, NICHT
  Cursor-basiert wie NinjaOne/Level.io, NICHT Offset-basiert wie Snipe-IT --
  ein drittes, eigenes Paginierungsschema, verifiziert über Jamfs eigene
  Entwicklerdoku,
  <https://developer.jamf.com/jamf-pro/reference/get_v1-computers-inventory>).
  Der Antwort-Umschlag ist über dieselbe Quelle verifiziert: `{"totalCount":
  <Zahl>, "results": [...]}`. `list_computers` durchläuft alle Seiten intern
  (bis zu `MAX_PAGES` Seiten à `PAGE_SIZE` (100) Computer, Schutz gegen eine
  sich falsch verhaltende Gegenstelle, exakt dasselbe Prinzip wie
  `plugin::snipeit::fetch_all_hardware`) und liefert eine einzige, bereits
  zusammengefügte Liste.
- **Site-Zugehörigkeit/Identifikationsfelder**: `general.site.id`/
  `general.site.name` jedes Computers ordnet ihn genau einer Site zu
  (verifiziert über Jamfs eigenes Antwortschema für
  `GET /api/v1/computers-inventory`) -- das Feld, das dieses Modul gegen
  `JamfSiteMapping` abgleicht, analog zu NinjaOnes `organizationId`.
  `general.name` ist Jamfs eigener Anzeigename eines Computers UND dient
  zugleich als Hostname-äquivalentes Identifikationsfeld (diese API hat kein
  von `general.name` getrenntes "Hostname"-Feld für einen macOS-Computer) --
  verwendet sowohl als `JamfDevice.name` als auch als `JamfDevice.hostname`,
  passend zu NinjaOnes/Level.ios Hostname-basierter
  "Mit bestehendem System verknüpfen"-Abgleichskonvention. Die IP-Adresse
  kommt aus `general.lastIpAddress`, mit `general.lastReportedIpV4` als
  Rückfallebene (beide über Jamfs eigenes Antwortschema verifiziert).
- **Computer-Detail**: `GET {base_url}/api/v1/computers-inventory/{id}` mit
  denselben `section=GENERAL&section=HARDWARE`-Query-Parametern wie der
  Listenaufruf (verifizierter Endpunkt, bewusst dem Schwester-Endpunkt
  `computers-inventory-detail/{id}` vorgezogen -- beide sind in Jamfs
  aktueller OpenAPI-Spezifikation zum Zeitpunkt der Verifizierung als
  veraltet markiert, ohne dokumentierten Ersatz; Jamf hat eine lange
  Historie solcher "veralteter" Endpunkte, die über Jahre erhalten bleiben,
  z. B. die gesamte Classic API. Der gewählte Endpunkt liefert dieselbe
  `general`/`hardware`-Sektionsform wie der Listenaufruf oben statt jeder
  Sektion, die Jamf kennt, was das externe-Feld-Vergleichspanel in
  `JamfPluginSection.tsx` vorhersagbar hält).
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie
  `plugin::ninja`/`plugin::level`/`plugin::snipeit`. Verbindungs-/Timeout-/
  Nicht-2xx-Fehler werden auf `PluginError::Unreachable` gemappt, HTTP
  401/403 auf `PluginError::Authentication`, unerwartete JSON-Formen
  (inkl. fehlgeschlagenem Token-Austausch) auf
  `PluginError::UnexpectedResponse`/`Authentication` -- dieselben
  Konventionen wie bei den drei anderen Plugins.
- **Zugangsdaten-Kodierung**: wie NinjaOne braucht Jamf zwei Geheimwerte
  (`client_id`, `client_secret`); `PluginCredentials.secret` ist laut
  Trait-Vertrag aber ein einziger opaker String -- hier als JSON kodiert,
  exakt wie `plugin::ninja::NinjaCredentials`.

### Mehrere Jamf-Verbindungen, jede mit mehreren Sites

Strukturell identisch zu NinjaOnes Verbindungs-/Organisations-Modell:

- Nicht-geheime Metadaten (`id`, `label`, `base_url`, bewusst OHNE
  `customer_id`) liegen als `JamfConnectionMeta` in
  `Config::jamf_connections` (`#[serde(default)]`-kompatibel).
  Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"jamf:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei NinjaOne/Level.io/Snipe-IT.
- Welche Site innerhalb einer Verbindung welchem lokalen Kunden entspricht
  (falls überhaupt), steht granular in `Config::jamf_site_mappings`. Eine
  nicht zugeordnete Site liefert bei jeder Synchronisierung ihre Computer
  weiterhin (zur Ansicht), aber immer mit `linked_system_id: None`.
- Ein Computer, dessen `site_id` zu keiner der über `list_sites` gemeldeten
  Sites passt (sollte laut Jamfs Datenmodell normalerweise nicht vorkommen,
  ist aber z. B. bei einer zwischenzeitlich gelöschten Site denkbar), wird
  nicht stillschweigend verworfen, sondern erscheint als eigene Gruppe unter
  der rohen Site-ID -- exakt dieselbe Konvention wie
  `commands::plugins::group_devices_by_organization`.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie NinjaOne/Level.io/Snipe-IT:
`sync_jamf_connection` schreibt das Ergebnis jedes Laufs zusätzlich als JSON
nach `data_dir/plugin-cache/jamf-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`). `get_cached_jamf_sync` liest
ausschließlich diese Datei (kein Netzwerkzugriff), rejoint dabei aber
`customer_id` je Gruppe live gegen die AKTUELLEN
`Config::jamf_site_mappings` (nicht den beim letzten Sync eingefrorenen
Wert) -- exakt wie `commands::plugins::get_cached_ninja_sync` --, und
liefert `None`, wenn für eine Verbindung noch nie synchronisiert wurde.
`remove_jamf_connection` löscht diese Cache-Datei (bestes Bemühen) und alle
`jamf_site_mappings`-Zeilen der entfernten Verbindung gleich mit.

### Tauri-Kommandos (`commands::jamf`)

`test_jamf_connection`, `list_jamf_connections`, `add_jamf_connection`,
`remove_jamf_connection`, `list_jamf_sites`, `map_jamf_site`,
`unmap_jamf_site`, `sync_jamf_connection`, `get_cached_jamf_sync`,
`link_system_to_jamf`, `unlink_system_from_jamf`, `get_jamf_system_details`
-- dünne Wrapper nach dem Muster von `commands::plugins`. `list_jamf_sites`
liefert die Live-Site-Liste einer Verbindung (analog zu
`list_ninja_organizations`), wird aber vom Frontend nicht aufgerufen --
`JamfPluginSection.tsx` ist wie die anderen drei Sektionen konsequent
Cache-first (`get_cached_jamf_sync` beim Öffnen, `sync_jamf_connection` nur
auf "Aktualisieren"); der Befehl bleibt für Symmetrie und einen möglichen
künftigen Ersteinrichtungs-Anwendungsfall erhalten. Das Übernehmen eines
extern gelieferten Werts in ein selbst gepflegtes Feld (`name`, `hostname`,
`ip_address`, `notes` in `systems`) bleibt -- wie bei den anderen drei
Plugins -- ausschließlich eine bewusste, manuelle Aktion über
`get_jamf_system_details` plus eine spätere UI-Aktion; kein Kommando hier
schreibt automatisch in diese vier Felder.

`commands::external_directory::list_unlinked_external_systems_for_customer`
bezieht Jamf-Geräte über eine eigene `collect_jamf`-Funktion mit ein, exakt
nach demselben Muster wie `collect_ninja`/`collect_snipeit` (Rejoin gegen
die aktuellen `jamf_site_mappings`, nicht gegen den eingefrorenen
Cache-Wert).

### Frontend (`JamfPluginSection.tsx`)

Strukturell an `SnipeitPluginSection.tsx`s Muster angelehnt (Sites
eingeklappt mit Geräte-Anzahl-Zusammenfassung, Kunde-Zuordnungs-`<select>`
inklusive "+ Neuen Kunden anlegen…", aufklappbare, filterbare,
10-pro-Seite-paginierte Geräteliste mit `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation, Vergleichs-/Übernahme-Panel für verknüpfte Geräte). Der
Verbindungs-Anlage-Dialog hat vier Felder wie bei Ninja (Label, Base-URL,
Client-ID, Client-Secret als `type="password"`). Eine verifizierte
Besonderheit gegenüber allen drei anderen Plugins: `ExternalSystemDto.hostname`
ist auf dem Backend IMMER identisch zu `ExternalSystemDto.name` (Jamf hat
kein von `general.name` getrenntes Hostname-Feld, siehe oben) --
`DeviceSummaryLine` zeigt deshalb bewusst KEIN zweites Hostname-Segment
(das wäre eine reine Wiederholung des Namens), sondern `serial_number`/
`asset_tag`, analog zu Snipe-ITs `asset_tag`/`serial`-Darstellung. Aus
demselben Grund hat ein lokales System kein eigenes Feld für
`serial_number`/`asset_tag` -- beim "Neu anlegen" werden diese einmalig in
das neu angelegte Systems `notes`-Feld geschrieben (wie bei Snipe-IT). Das
externe-Feld-Vergleichspanel (`findExternalValue`) ist eine echte,
verifizierte Erweiterung gegenüber Ninja/Snipe-IT: Jamfs eigene
`get_jamf_system_details`-Antwort verschachtelt Felder unter `general`/
`hardware` statt sie flach auf oberster Ebene zu liefern, deshalb scannt
`findExternalValue` hier zusätzlich zur obersten Ebene auch innerhalb dieser
beiden bekannten Container -- keine Vermutung, sondern aus Jamfs eigenem
Antwortschema abgeleitet. `PluginsView.tsx` bindet die Sektion als sechste
Karte neben `NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/
`SnipeitPluginSection.tsx`/`IntunePluginSection.tsx`/`IruPluginSection.tsx`
ein.

## Apple-Business-Manager-Plugin (`plugin::abm`) -- siebte echte Integration

`plugin/abm.rs` implementiert `Plugin` für Apple Business Managers (ABM)
öffentliche REST-API über HTTPS. Siebte echte Integration nach NinjaOne,
Level.io, Snipe-IT, Microsoft Intune, Iru und Jamf Pro -- und mit deutlichem
Abstand die aufwendigste Authentifizierung aller sieben Plugins: ein
ES256-signiertes JWT als Client-Assertion, gegen einen OAuth2-Access-Token
eingetauscht. Beide Schritte sind gegen Apples eigene
Referenzimplementierung verifiziert, keine Vermutung.

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
Sektion als siebte Karte neben `NinjaPluginSection.tsx`/
`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`/`IntunePluginSection.tsx`/`IruPluginSection.tsx`/`JamfPluginSection.tsx`
ein.

## Tactical-RMM-Plugin (`plugin::tacticalrmm`) -- achte echte Integration

`plugin/tacticalrmm.rs` implementiert `Plugin` für Tactical RMMs REST-API
über HTTPS. Tactical RMM (<https://github.com/amidaware/tacticalrmm>) ist
ein selbst gehostetes, quelloffenes RMM-Tool (Remote Monitoring &
Management) -- strukturell am nächsten an NinjaOne: Mehrfach-
Mandantenfähigkeit über eine echte "Client" -> "Site" -> "Agent"-Hierarchie
innerhalb einer Verbindung (konzeptionell dieselbe Form wie NinjaOnes
"Organization" -> "Device"), dazu eine konfigurierbare `base_url` wie
NinjaOne/Snipe-IT. Authentifizierung ist dagegen so einfach wie bei
Level.io/Snipe-IT -- ein einzelnes statisches Geheimnis, kein OAuth2-Grant
wie bei NinjaOne.

Jede Angabe unten ist direkt gegen Tactical RMMs eigenen Backend-Quellcode
auf GitHub verifiziert (`amidaware/tacticalrmm`, `master`-Branch,
`api/tacticalrmm/`) -- nicht aus Dokumentations-Prosa geraten:

- **Authentifizierung**: statischer API-Key im eigenen `X-API-KEY`-Header
  (NICHT `Authorization: Bearer`), verifiziert über
  `tacticalrmm/auth.py::APIAuthentication`/`get_authorization_header`
  (`request.META.get("HTTP_X_API_KEY", ...)`). Der Nutzer erzeugt ihn selbst
  in Tactical RMMs eigener Weboberfläche (Settings -> Global Settings -> API
  Keys). Kein Token-Austausch, kein OAuth2-Grant -- wie Level.io/Snipe-IT,
  nur mit anderem Header-Namen und ohne `Bearer `-Präfix.
- **Selbst gehostet**: wie NinjaOne/Snipe-IT (und anders als Level.ios feste
  `BASE_URL`-Konstante) braucht eine Tactical-RMM-Verbindung eine vom Nutzer
  angegebene Basis-URL. Tactical RMMs REST-API hat keinen festen
  Pfad-Präfix -- verifiziert über `tacticalrmm/urls.py`:
  `path("clients/", include("clients.urls"))`/
  `path("agents/", include("agents.urls"))` liegen direkt unter der
  API-Wurzel, anders als Snipe-ITs festes `/api/v1`-Suffix.
  `TacticalRmmConnectionMeta.base_url` wird deshalb ohne zusätzlichen
  Pfad-Suffix direkt verwendet.
- **Clients (Mehrmandantenfähigkeit)**: `GET {base_url}/clients/`,
  verifiziert über `clients/views.py::GetAddClients.get`
  (`return Response(ClientSerializer(clients, many=True).data)`) -- ein
  nackter, UNPAGINIERTER JSON-Array (kein `{"total", "rows"}`-Umschlag wie
  Snipe-IT, kein Cursor wie Level.io). Jedes Client-Objekt hat die Form
  `{"id": <Zahl>, "name": "...", "sites": [...], ...}`
  (`clients/serializers.py::ClientSerializer`). `sites` ist ein
  verschachteltes Array -- bewusst nicht ausgewertet, siehe unten: die
  Zuordnungs-Granularität bleibt auf Client-Ebene, analog dazu, wie NinjaOne
  auf Organisations-Ebene zuordnet und nicht feiner.
- **Agenten**: `GET {base_url}/agents/?detail=true` (der STANDARD, wenn der
  `detail`-Query-Parameter ganz weggelassen wird -- hier trotzdem explizit
  mitgeschickt, für Klarheit/Zukunftssicherheit). WICHTIG, eine echte
  Korrektur gegenüber einer anfänglichen, ungeprüften Annahme:
  `detail=false` ist NICHT die reichhaltigere Liste -- verifiziert über
  `agents/views.py::GetAgents.get`: der Zweig für "`detail` fehlt ODER
  `detail=true`" verwendet `AgentTableSerializer` (reichhaltig: Hostname,
  Client-Name, Site-Name, Status, Plattform, IP, ...); der
  `detail=false`-Zweig verwendet den deutlich schlankeren
  `AgentHostnameSerializer` (`id`, `hostname`, `agent_id`, `client`, `site`
  -- nur Namen, kein Status/Plattform/IP). `plugin::tacticalrmm` fragt daher
  bewusst explizit `detail=true` an -- der genaue Gegenwert dessen, was eine
  erste, ungeprüfte Lektüre der Ausgangs-Vorgabe nahegelegt hätte. Ebenfalls
  ein nackter, unpaginierter JSON-Array wie `/clients/`.
- **Kein numerisches Client-/Site-Feld an einem Agenten, in KEINER der
  beiden Agenten-Serializer-Varianten**: verifiziert gegen
  `agents/serializers.py` -- `AgentTableSerializer.Meta.fields` hat
  `"client_name"` (`ReadOnlyField(source="site.client.name")`) und
  `"site_name"` (`ReadOnlyField(source="site.name")`) als reine
  Anzeige-Strings, sonst nichts, was auf den übergeordneten Client/die Site
  verweist; `AgentHostnameSerializer` hat dieselbe Geschichte mit
  `"client"`/`"site"` (ebenfalls Namens-Strings). Das ist eine echte,
  verifizierte Abweichung von NinjaOne (`organizationId`, ein echter
  numerischer Fremdschlüssel am Gerät): ein Agent kann seinem Client nur
  über den NAMEN zugeordnet werden (`agent.client_name == client.name`),
  nicht über eine ID. `commands::tacticalrmm::group_agents_by_client` ist
  bewusst um diese verifizierte Realität herum geschrieben, nicht um eine
  angenommene ID-basierte Form. `TacticalRmmClientMapping.client_id` selbst
  IST weiterhin die echte, numerische Client-ID (aus `/clients/`, wo eine ID
  tatsächlich existiert) -- nur die Pro-Agent-Zuordnung braucht den
  Namens-basierten Behelf.
- **Agenten-Kennung**: `agent_id`, ein eindeutiger String (verifiziert über
  `agents/models.py`: `agent_id = models.CharField(max_length=200,
  unique=True)`), verwendet als URL-Pfad-Segment für einen einzelnen Agenten
  (`GET {base_url}/agents/{agent_id}/`, verifiziert über `agents/urls.py`).
  Der Django-interne numerische `id`-Primärschlüssel ist aus
  `AgentTableSerializer` bewusst ausgeschlossen -- `agent_id` ist die
  einzige Kennung, die dieses Plugin je sieht oder verwendet;
  `ExternalSystem::external_id` trägt hier immer `agent_id`, nie eine
  numerische ID.
- **IP-Adresse**: `public_ip` ist ein einzelnes String-Feld (verifiziert
  über `agents/models.py`); `local_ips` ist eine berechnete, durch Kommas
  getrennte STRING-Eigenschaft (verifiziert: `", ".join(...)`), KEIN Array
  wie NinjaOnes `ipAddresses` -- und kann auf der Agenten-Seite selbst auch
  den wörtlichen Fehlertext `"error getting local ips"` enthalten.
  `extract_ip_address` nimmt das erste, durch Komma getrennte Token von
  `local_ips`, wenn es nach einem echten Wert aussieht (nicht leer, enthält
  nicht "error"), sonst `public_ip` als Rückfallebene -- dasselbe
  "primär + Rückfallebene"-Prinzip wie bei `plugin::ninja`/`plugin::level`,
  nur an Tactical RMMs tatsächlich verifizierte String- (statt Array-)Form
  angepasst.
- **Status/Plattform**: `status` ist einer der verifizierten String-
  Konstanten `"online"`/`"offline"`/`"overdue"` (`tacticalrmm/constants.py`);
  `plat` ist eines von `"windows"`/`"linux"`/`"darwin"` (`AgentPlat`-
  Text-Choices in derselben Datei). Beide werden hier als freie Strings
  durchgereicht -- kein Rust-Enum --, damit ein künftiger Tactical-RMM-Wert
  das Parsen nicht bricht.
- **Kein verifizierter Weboberflächen-Link**: anders als NinjaOne/Snipe-IT
  wird Tactical RMMs Web-Dashboard (eine eigene Vue-SPA,
  `amidaware/tacticalrmm-web`) üblicherweise auf einer EIGENEN Subdomain/
  einem eigenen Host betrieben, unabhängig von der API-`base_url` -- eine
  gut dokumentierte Tactical-RMM-Deployment-Konvention (typischerweise ein
  `api.`-Host für das Backend und ein separater `rmm.`-/Wurzel-Host für das
  Dashboard). Anders als NinjaOnes `ninja_url`/Snipe-ITs `snipeit_url`
  (beide nachweisbar aus `base_url` ableitbar, weil diese Tools API und
  Weboberfläche vom SELBEN Host bedienen) gibt es hier keinen ehrlichen Weg,
  einen Dashboard-Link allein aus `base_url` zu bauen -- diese Integration
  hat deshalb bewusst KEIN `tacticalrmm_url`-Feld/DTO-Attribut. Eine
  ehrliche Auslassung, analog zu Snipe-ITs verifiziertem `hostname`/
  `ip_address` immer `None` (siehe `plugin::snipeit`-Moduldokumentation),
  keine als Feature verkleidete Vermutung.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie
  `plugin::ninja`/`plugin::level`/`plugin::snipeit`.
- **Zugangsdaten-Kodierung**: Tactical RMM braucht nur einen einzigen
  Geheimwert (den API-Key), 1:1 als `PluginCredentials.secret`
  durchgereicht -- wie Level.io/Snipe-IT, keine JSON-Kodierung mehrerer
  Werte nötig (anders als NinjaOne).

### Tactical-RMM-Verbindungen, jede mit mehreren Clients

Strukturell identisch zu NinjaOnes/Snipe-ITs Verbindungs-/Organisations-
bzw. -Firmen-Modell:

- Nicht-geheime Metadaten (`id`, `label`, `base_url`, bewusst OHNE
  `customer_id`) liegen als `TacticalRmmConnectionMeta` in
  `Config::tacticalrmm_connections` (`#[serde(default)]`-kompatibel).
  Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"tacticalrmm:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei NinjaOne/Level.io/Snipe-IT.
- Welcher Client innerhalb einer Verbindung welchem lokalen Kunden
  entspricht (falls überhaupt), steht granular in
  `Config::tacticalrmm_client_mappings`
  (`TacticalRmmClientMapping { connection_id, client_id, client_name,
  customer_id }`). Ein nicht zugeordneter Client liefert bei jeder
  Synchronisierung seine Agenten weiterhin (zur Ansicht), aber immer mit
  `linked_system_id: None`.
- `commands::tacticalrmm::group_agents_by_client` gruppiert Agenten nach
  Client -- strukturell analog zu `commands::plugins::
  group_devices_by_organization`/`commands::snipeit::
  group_devices_by_company`, mit EINER bewussten, verifizierten Abweichung:
  die Zuordnung Agent -> Client läuft über den NAMEN
  (`agent.client_name == client.name`), nicht über eine ID, weil Tactical
  RMMs Agenten-Listen-API keine numerische Client-ID trägt (siehe oben).
  `TacticalRmmClientMapping`-Nachschlagen verwendet trotzdem weiterhin die
  echte `client_id`, weil die auf der Client-Liste selbst verfügbar ist.
  Agenten, deren `client_name` zu keinem bekannten Client passt (sollte
  normalerweise nicht vorkommen, ist aber nicht ausgeschlossen -- z. B. ein
  zwischen Client- und Agenten-Abruf im selben Sync-Lauf umbenannter
  Client), werden nicht stillschweigend verworfen, sondern als eigene
  Restgruppe angehängt, mit dem rohen Namen als synthetischer ID UND
  Anzeigename -- exakt wie bei NinjaOnes/Snipe-ITs Restgruppen-Behandlung.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie NinjaOne/Level.io/Snipe-IT:
`sync_tacticalrmm_connection` schreibt das Ergebnis jedes Laufs zusätzlich
als JSON nach `data_dir/plugin-cache/tacticalrmm-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`). `get_cached_tacticalrmm_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff), rejoint dabei aber
`customer_id` je Gruppe live gegen die AKTUELLEN
`Config::tacticalrmm_client_mappings` (nicht den beim letzten Sync
eingefrorenen Wert) -- exakt wie `commands::plugins::get_cached_ninja_sync`
--, und liefert `None`, wenn für eine Verbindung noch nie synchronisiert
wurde. `remove_tacticalrmm_connection` löscht diese Cache-Datei (bestes
Bemühen) und alle `tacticalrmm_client_mappings`-Zeilen der entfernten
Verbindung gleich mit.

### Tauri-Kommandos (`commands::tacticalrmm`)

`test_tacticalrmm_connection`, `list_tacticalrmm_connections`,
`add_tacticalrmm_connection`, `remove_tacticalrmm_connection`,
`list_tacticalrmm_clients`, `map_tacticalrmm_client`,
`unmap_tacticalrmm_client`, `sync_tacticalrmm_connection`,
`get_cached_tacticalrmm_sync`, `link_system_to_tacticalrmm`,
`unlink_system_from_tacticalrmm`, `get_tacticalrmm_system_details` -- dünne
Wrapper nach dem Muster von `commands::plugins`. `list_tacticalrmm_clients`
liefert die Live-Client-Liste einer Verbindung (analog zu
`list_ninja_organizations`/`list_snipeit_companies`), wird aber vom
Frontend nicht aufgerufen -- `TacticalRmmPluginSection.tsx` ist wie
`NinjaPluginSection.tsx`/`SnipeitPluginSection.tsx` konsequent Cache-first
(`get_cached_tacticalrmm_sync` beim Öffnen, `sync_tacticalrmm_connection`
nur auf "Aktualisieren"); der Befehl bleibt für Symmetrie und einen
möglichen künftigen Ersteinrichtungs-Anwendungsfall erhalten. Das Übernehmen
eines extern gelieferten Werts in ein selbst gepflegtes Feld (`name`,
`hostname`, `ip_address`, `notes` in `systems`) bleibt -- wie bei
NinjaOne/Level.io/Snipe-IT -- ausschließlich eine bewusste, manuelle Aktion
über `get_tacticalrmm_system_details` plus eine spätere UI-Aktion; kein
Kommando hier schreibt automatisch in diese vier Felder.

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Anders als Snipe-IT (das mangels Hostname-Feld auf `asset_tag`/`serial`
ausweichen muss, siehe oben) hat ein Tactical-RMM-Agent ein echtes
`hostname`-Feld -- Tactical RMM ist RMM-Überwachungssoftware, keine
Asset-/Inventarverwaltung. `TacticalRmmPluginSection.tsx`s
`matchKeyForDevice` verwendet deshalb, wie bei NinjaOne/Level.io, direkt
`device.hostname` als Abgleichsschlüssel für den "Mit bestehendem System
verknüpfen"-Vorschlag, verglichen gegen das einzige freie Textfeld, das ein
lokales System dafür hat -- `System.hostname`.

### Frontend (`TacticalRmmPluginSection.tsx`)

Mechanisch an `SnipeitPluginSection.tsx`s Stand angelehnt (Clients
eingeklappt mit Geräte-Anzahl-Zusammenfassung, Kunde-Zuordnungs-`<select>`
inklusive "+ Neuen Kunden anlegen…", aufklappbare, filterbare,
10-pro-Seite-paginierte Geräteliste mit `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation, Vergleichs-/Übernahme-Panel für verknüpfte Geräte). Der
Verbindungs-Anlage-Dialog hat drei Felder wie bei Snipe-IT (Label, Base-URL,
API-Key als `type="password"`) -- kein Client-ID/-Secret-Paar nötig, wie bei
NinjaOne. `DeviceSummaryLine` zeigt zusätzlich einen Online/Offline/
Überfällig-Statuspunkt, die Plattform (Windows/Linux/macOS) und den
Site-Namen -- Anzeige-Kontext, der bei Ninja/Level/Snipe-IT keine
Entsprechung hat. Anders als `NinjaPluginSection.tsx`/
`SnipeitPluginSection.tsx` gibt es bewusst KEINEN "In X öffnen"-Link auf
einer Geräte-Zeile (siehe oben, kein verifizierter Weboberflächen-Link).
`PluginsView.tsx` bindet die Sektion als achte Karte neben
`NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`/`IntunePluginSection.tsx`/`IruPluginSection.tsx`/`JamfPluginSection.tsx`/`AbmPluginSection.tsx`
ein.

## Acronis-Plugin (`plugin::acronis`), neunte echte Integration, anders als alle acht übrigen

`plugin/acronis.rs` implementiert `Plugin` für Acronis Cyber Protect Clouds
REST-API (`developer.acronis.com`). Acronis ist als einziges Plugin hier
**kein** RMM-/MDM-Tool: jede der acht vorherigen Integrationen
(NinjaOne, Level.io, Snipe-IT, Intune, Iru, Jamf Pro, Apple Business
Manager, Tactical RMM) liefert einen Geräte-/Asset-Bestand. Acronis ist
eine Backup-/Cyber-Protection-Plattform; die einzige Aufgabe dieses
Plugins ist es, **Sicherungsstatus pro Kunde/Gerät** sichtbar zu machen
(läuft die Sicherung gerade sauber, mit Warnung, oder fehlgeschlagen),
niemals Geräteverwaltung. Konkret: `AcronisResource.backup_status` (ein
Alert-Manager-`severity`-Wert, siehe unten) ist die gesamte Nutzlast, die
dieses Plugin über die reine Identität hinaus liefert; "Agent
installieren"/"Skript ausführen"/Bestandsverwaltung gibt es hier gar
nicht.

Strukturell fügt es sich trotzdem in genau dasselbe `Plugin`-Trait und
dasselbe Verbindungs-/Zuordnungs-/Sync-/Verknüpfungs-Muster wie jedes
andere Plugin hier ein: wie Tactical RMM/NinjaOne/Snipe-IT/Jamf kann eine
Verbindung (ein Acronis-API-Client) mehrere Mandanten ("Customer"
innerhalb von Acronis' eigener Mandanten-Hierarchie) sehen, also gilt
dasselbe Verbindungs-plus-Zuordnungstabellen-Muster
(`AcronisConnectionMeta`/`AcronisTenantMapping`, Zuordnungs-Granularität
auf Ebene "Mandant vom Typ customer", siehe unten). Authentifizierung ist
OAuth2-Client-Credentials wie bei NinjaOne/Intune, plus ein zusätzlicher,
für dieses Plugin einzigartiger Discovery-Schritt (siehe unten).

Jede Angabe unten stammt unverändert aus der verifizierten Recherche, mit
der dieses Plugin gebaut wurde (Acronis' eigene Dokumentation auf
developer.acronis.com plus drei live abgerufene OpenAPI-Spezifikationen),
nicht geraten:

- **Authentifizierung**: OAuth2-Client-Credentials-Grant, `POST
  {datacenter_url}/api/2/idp/token`, Header `Authorization: Basic
  base64(client_id:client_secret)`, Formular-Body
  `grant_type=client_credentials`. Antwort `{"access_token", "token_type":
  "bearer", "expires_on", ...}`, als `Authorization: Bearer
  <access_token>` bei jedem weiteren Aufruf verwendet. Wie bei jedem
  anderen Plugin hier wird der Token pro echter Operation frisch geholt,
  nie über Aufrufe hinweg zwischengespeichert/erneuert (diese Anwendung
  ruft Plugin-Methoden selten/manuell auf, nicht in einer heißen Schleife,
  dieselbe Begründung wie bei `plugin::ninja`/`plugin::intune`).
- **`datacenter_url` ist vollständig nutzerseitig angegeben**: Beim
  Anlegen eines API-Clients in der Acronis-Verwaltungskonsole (einmalig,
  außerhalb dieser Anwendung, vom Nutzer selbst erledigt, bevor dieses
  Plugin konfiguriert wird) erhält der Nutzer DREI Werte auf einmal:
  `client_id`, `client_secret` UND eine `datacenter_url` (z. B.
  `https://eu2-cloud.acronis.com`). Es existiert keine feste/aufzählbare
  Liste von Rechenzentren in der Dokumentation, also muss der Nutzer sie,
  wie Tactical RMMs `base_url`, selbst eintragen. Das macht dies zu einem
  echten DREI-Werte-Zugangsdatensatz, anders als jedes bisherige Plugin
  hier, aber `datacenter_url` ist selbst kein Geheimnis (sie wird in
  Acronis' eigener Oberfläche offen angezeigt, genau wie eine Basis-URL),
  also liegt sie nach dem Credential-Prinzip (siehe oben,
  "Credential-Prinzip") in `AcronisConnectionMeta.datacenter_url`
  (`config.toml`), NICHT im Schlüsselspeicher-Geheimnis, exakt dasselbe
  Prinzip "nicht-geheime Metadaten gehören in die Konfiguration", das
  schon für `base_url` bei Tactical RMM/NinjaOne/Snipe-IT/Iru/Jamf gilt.
  `AcronisCredentials` bleibt dadurch ein sauberer Zwei-Werte-JSON
  (`client_id`, `client_secret`), genau wie
  `plugin::ninja::NinjaCredentials`.
- **Basis-URL**: `{datacenter_url}/api/2` für jeden Endpunkt unten.
- **Zusätzlicher Discovery-Schritt, einzigartig für dieses Plugin**: nach
  Erhalt eines Bearer-Tokens liefert `GET
  {datacenter_url}/api/2/clients/{client_id}` (Bearer-Auth) `{"tenant_id":
  "<uuid>", "type": "api_client", ...}`, den eigenen Wurzel-Mandanten
  des API-Clients, den Anker, den `/tenants` braucht, um den gesamten
  zugänglichen Mandanten-Baum in einem Aufruf abzulaufen (siehe unten).
  Wird einmal pro echter Operation ausgeführt (`test_credentials`,
  `list_tenants`, einmal je zugeordnetem Mandanten innerhalb eines
  `sync_acronis_connection`-Laufs), wie der Token selbst, nicht über
  Aufrufe hinweg zwischengespeichert.
- **Mandanten (die Zuordnungs-Entität)**: `GET {base}/tenants`,
  Query-Parameter `subtree_root_id=<der entdeckte Wurzel-Mandant>`,
  `lod=full`, Cursor `after`. Antwort-Umschlag (KEIN nackter Array, anders
  als Tactical RMMs `/clients/`): `{"timestamp", "paging": {"cursors":
  {"after": "..."}}, "items": [{"id", "parent_id", "name", "kind"}]}`.
  `kind` ist eines von `root | partner | folder | customer | unit`; NUR
  Mandanten mit `kind == "customer"` sind gültige Zuordnungsziele
  (ansonsten organisatorische Container, keine echten Kundenkonten),
  spiegelt, wie Tactical RMMs Client-Ebene (nicht Site) die
  Zuordnungs-Granularität ist, hier nur über ein `kind`-Feld statt eine
  Hierarchie-Ebene gesteuert. `map_tenant` (einzelnes Element,
  ungefiltert) und `filter_customer_tenants` (der `kind ==
  "customer"`-Filter) sind bewusst getrennte, unabhängig testbare, reine
  Funktionen; siehe deren eigene Doc-Kommentare.
- **Sicherungsstatus pro Gerät, eine echte Scope-Entscheidung, keine
  Vermutung**: die Dokumentation bietet kein einziges, offensichtliches
  "ist die Sicherung ok"-Feld, und zwei getrennte APIs sind beteiligt:
  1. **Ressourcen-Identität**: `GET
     {base}/resource_management/v4/resources`, Query-Parameter
     `tenant_id=<ein zugeordneter Mandant>`, Cursor `before`/`after`.
     Antwort `{"items": [{"id", "name", "agent_id", "external_id",
     "type"}], "paging": {"cursors": {...}}}`. Dieses Plugin verwendet
     `id` als `AcronisResource::external_id` und `name` als Anzeigename,
     NICHT das eigene, verwirrend benannte `external_id`-Feld der Ressource
     (ein anderes, Acronis-internes Konzept, nicht der Identitäts-Schlüssel
     dieser Anwendung) und NICHT `agent_id` (Acronis' eigenes
     Agenten-Konzept, hier irrelevant).
  2. **Sicherungs-Gesundheit**: `GET {base}/alert_manager/v1/resource_status`,
     Query-Parameter `tenant=<derselbe zugeordnete Mandant>` (man beachte
     den anders benannten Query-Parameter, `tenant`, nicht `tenant_id`,
     eine echte, leicht zu übersehende Inkonsistenz in Acronis' eigener
     API, hier exakt wie verifiziert übernommen). Antwort: `{"items":
     [{"id": "<resourceId>", "severity":
     "ok|information|warning|error|critical", "alert": {...}}]}`. DAS
     ist die "ist die Sicherung gerade in Ordnung"-Antwort für dieses
     Plugin. `join_resources_with_severity` verknüpft `id` (Acronis'
     Ressourcen-ID, derselbe Wert wie oben unter #1) mit
     `AcronisResource::external_id` und kopiert `severity` unverändert
     nach `AcronisResource::backup_status` (freier String, wie Tactical
     RMMs `status`/`platform`, kein Rust-Enum, damit ein künftiger neuer
     Severity-Wert das Parsen nicht bricht). Eine Ressource OHNE
     passenden Alert-Manager-Eintrag (nie gesichert / nicht geschützt)
     bekommt `backup_status: None`, ein legitimer, erwarteter Fall, kein
     Fehler.
  3. **Bewusst NICHT versucht**: das Filtern von
     `resource_management/v4/resource_statuses`s `policies[]`-Array nach
     einem bestimmten Backup-Policy-Typ-CTI-String. Dieser String wurde
     nie gegen echte Dokumentation/Spezifikationen bestätigt, und ihn
     falsch zu erraten würde dem Nutzer stillschweigend bedeutungslose
     Daten zeigen. Für eine reichhaltigere Pro-Ressource-Nutzlast ruft
     dieses Plugin stattdessen `GET
     {base}/resource_management/v4/resource_statuses?tenant_id=<id>`
     UNGEFILTERT auf und reicht das rohe JSON unverändert durch (siehe
     `get_resource_statuses` unten), derselbe "rohes, freies JSON, der
     Aufrufer/die UI interpretiert es"-Vertrag, den jedes andere Plugin
     hier für `Plugin::get_system_details` verwendet.
- **Ein echter Widerspruch, um den dieses Plugin herum entworfen werden
  musste**: Acronis hat KEINEN verifizierten Einzelressourcen-Detail-
  Endpunkt. `resource_statuses` ist immer MANDANTEN-bezogen, nie
  ressourcenbezogen, aber das `Plugin`-Trait-Methode
  `get_system_details(&self, credentials, external_id)` hat keinen Platz
  für einen Mandanten-Parameter. Statt einen unbestätigten
  Einzelressourcen-Endpunkt zu erraten, liefert `Plugin::get_system_details`
  für `AcronisPlugin` bewusst `PluginError::UnexpectedResponse` mit genau
  dieser Erklärung, eine ehrliche Lücke, analog zu Tactical RMMs
  fehlendem Weboberflächen-Link oder Snipe-ITs immer-`None`-`hostname`/
  `ip_address` (siehe deren eigene Moduldokumentation), keine als Feature
  verkleidete Vermutung. Die echte, reichhaltigere Nutzlast steht
  stattdessen über die eigene Methode `get_resource_statuses(&self,
  credentials, tenant_id)` zur Verfügung, die SEHR WOHL eine Mandanten-ID
  entgegennimmt: `commands::acronis` ruft diese direkt auf (nie die
  Trait-Methode) für `sync_acronis_connection`/`link_system_to_acronis`/
  `get_acronis_system_details`, die alle bereits Mandanten-Kontext aus der
  Zuordnung tragen, mit der sie gerade arbeiten. Das ist die eine echte,
  bewusste Abweichung von jedem anderen Plugin hier (wo die Trait-Methode
  DIE echte Implementierung ist), hier explizit benannt, siehe auch
  `plugin::acronis`s eigene Moduldokumentation. `Plugin::list_systems` hat
  dasselbe zugrunde liegende Problem (kein verifizierter
  "alle Mandanten auf einmal"-Endpunkt, und kein Mandanten-Parameter an
  der Trait-Methode) und ist deshalb ebenfalls bewusst schmal: es liefert
  eine leere Liste. `commands::acronis` ruft auch sie nie auf, sondern
  verwendet stattdessen die mandanten-bezogene eigene Methode
  `list_resources_with_status`, einmal je zugeordnetem Mandanten.
- **Paginierung**: Cursor-basiert, `paging.cursors.after` in der Antwort,
  als Query-Parameter `after` für die nächste Seite zurückgegeben; ein
  fehlendes/leeres `after` bedeutet keine weiteren Seiten. Dieselbe Form
  für `/tenants`, `/resource_management/v4/resources` und
  `/alert_manager/v1/resource_status` (pro Antwort einzeln über
  `parse_paged_items` ausgelesen, nicht als eine fest angenommene globale
  Konstante, falls eine künftige Acronis-Version den Umschlag eines
  Endpunkts ändert, ohne die anderen anzupassen).
- **Rate-Limits**: nirgends erreichbar dokumentiert. Hier vermerkt, keine
  besondere Behandlung (kein Backoff/Retry), dieselbe ehrliche Lücke wie
  bei jedem anderen Plugin hier.
- **Kein Weboberflächen-Tiefenlink**: nicht dokumentiert, deshalb
  ausgelassen, dieselbe ehrliche Auslassung wie bei `plugin::tacticalrmm`.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie jedes
  andere Plugin hier.
- **Zugangsdaten-Kodierung**: zwei Geheimwerte (`client_id`,
  `client_secret`); `datacenter_url` bewusst ausgeschlossen (siehe
  oben), JSON-kodiert genau wie `plugin::ninja::NinjaCredentials`.

### Acronis-Verbindungen, jede mit mehreren Mandanten

Strukturell wie Tactical RMMs Verbindungs-/Client-Modell:

- Nicht-geheime Metadaten (`id`, `label`, `datacenter_url`, bewusst OHNE
  `customer_id`) liegen als `AcronisConnectionMeta` in
  `Config::acronis_connections` (`#[serde(default)]`-kompatibel).
- Welcher Mandant innerhalb einer Verbindung welchem lokalen Kunden
  entspricht (falls überhaupt), steht granular in
  `Config::acronis_tenant_mappings` (`AcronisTenantMapping {
  connection_id, tenant_id, tenant_name, customer_id }`).

### Zwei bewusste Abweichungen von `commands::tacticalrmm`s Form

Beide sind durch Acronis' eigene API-Form erzwungen (siehe
`plugin::acronis`s Moduldokumentation für die vollständige Begründung),
hier explizit benannt, nicht stillschweigend aus dem Ausgangs-Briefing
übernommen:

1. `sync_acronis_connection` läuft NUR über bereits ZUGEORDNETE Mandanten
   (`Config::acronis_tenant_mappings`), nicht über jeden Mandanten, den die
   Verbindung sehen kann. Tactical RMMs/NinjaOnes/Snipe-ITs einzelner
   "alle Clients/Agenten auflisten, dann gruppieren"-Aufruf ist günstig und
   verbindungsweit; Acronis' Ressourcen-Endpunkt VERLANGT
   Mandanten-Bezug (`tenant_id`-Query-Parameter, siehe oben); es gibt
   keine verifizierte "alle Mandanten auf einmal"-Variante, die
   stattdessen aufgerufen werden könnte. Ein nicht zugeordneter Mandant
   zeigt deshalb überhaupt keine Ressourcen, bis er zugeordnet wird
   (anders als Tactical RMM, das die Agenten eines nicht zugeordneten
   Clients trotzdem zeigt, nur ohne `customer_id`). `list_acronis_tenants`
   bleibt verfügbar (spiegelt `list_tacticalrmm_clients`), damit die
   Zuordnungs-UI weiterhin eine Kandidatenliste zum Zuordnen HAT.
2. `link_system_to_acronis`/`get_acronis_system_details` brauchen einen
   `tenant_id`-Parameter, den `link_system_to_tacticalrmm`/
   `get_tacticalrmm_system_details` nicht brauchen; siehe oben, "Ein
   echter Widerspruch".

Eine dritte, direkte Folge davon zeigt sich im Frontend
(`AcronisPluginSection.tsx`): Anders als jede andere Plugin-Sektion hier
ist die Mandanten-Entdeckung NICHT Cache-first: das Öffnen der
Verbindungs-Übersicht löst zusätzlich zum üblichen
`get_cached_acronis_sync` immer einen LIVEN `list_acronis_tenants`-Aufruf
aus, rein um die Kandidatenliste für die Kunde-Zuordnung zu befüllen. Die
Zuordnungs-Quelle der Wahrheit ist deshalb die live geladene
`AcronisTenantDto.mapped_customer_id`, NICHT `group.customer_id` aus dem
Zwischenspeicher (anders als `TacticalRmmPluginSection.tsx`, das
Zuordnungs-Status ausschließlich aus seinen zwischengespeicherten
Geräte-Gruppen liest).

### Zwischenspeicher für Offline-Ansicht

Dieselbe Konvention wie Tactical RMM/NinjaOne/Level.io/Snipe-IT:
`sync_acronis_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/acronis-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`), hier allerdings NUR
Gruppen für Mandanten, die zum Sync-Zeitpunkt bereits zugeordnet waren
(siehe oben). `get_cached_acronis_sync` liest ausschließlich diese Datei
(kein Netzwerkzugriff), rejoint dabei aber `customer_id` je Gruppe live
gegen die AKTUELLEN `Config::acronis_tenant_mappings` (nicht den beim
letzten Sync eingefrorenen Wert). `remove_acronis_connection` löscht
diese Cache-Datei (bestes Bemühen) und alle
`acronis_tenant_mappings`-Zeilen der entfernten Verbindung gleich mit.

### Tauri-Kommandos (`commands::acronis`)

`test_acronis_connection`, `list_acronis_connections`,
`add_acronis_connection`, `remove_acronis_connection`,
`list_acronis_tenants`, `map_acronis_tenant`, `unmap_acronis_tenant`,
`sync_acronis_connection`, `get_cached_acronis_sync`,
`link_system_to_acronis`, `unlink_system_from_acronis`,
`get_acronis_system_details`: dünne Wrapper nach dem Muster von
`commands::tacticalrmm`, mit den zwei oben genannten, notwendigen
Abweichungen. `sync_acronis_connection` holt für jeden zugeordneten
Mandanten die tenant-weite, ungefilterte `resource_statuses`-Nutzlast
höchstens EINMAL je Sync-Lauf (verzögert, nur falls dieser Mandant
tatsächlich eine verknüpfte Ressource hat, deren zwischengespeicherte
Nutzlast aufgefrischt werden muss), nicht einmal je Ressource, weil es
keinen Pro-Ressource-Endpunkt gibt und deshalb jede verknüpfte Ressource
eines Mandanten dieselbe rohe Nutzlast teilt. Das Übernehmen eines extern
gelieferten Werts in ein selbst gepflegtes Feld bleibt, wie bei jedem
anderen Plugin hier, ausschließlich eine bewusste, manuelle Aktion;
kein Kommando hier schreibt automatisch in `name`/`hostname`/
`ip_address`/`notes`.

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Anders als jede RMM-/MDM-Sektion hier hat eine Acronis-Ressource KEIN
Hostname-Feld (siehe oben: die einzige Pro-Ressource-Nutzlast dieses
Plugins über die Identität hinaus ist `backup_status`).
`AcronisPluginSection.tsx`s `matchKeyForDevice` vergleicht deshalb
`device.name` gegen den Namen eines lokalen Systems (`System.name`), nicht
gegen `hostname`.

### Frontend (`AcronisPluginSection.tsx`)

Mechanisch an `TacticalRmmPluginSection.tsx`s Stand angelehnt (Mandanten
eingeklappt mit Ressourcen-Anzahl-Zusammenfassung, Kunde-Zuordnungs-`
<select>` inklusive "+ Neuen Kunden anlegen…", aufklappbare, filterbare,
10-pro-Seite-paginierte Ressourcenliste mit `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation). Der Verbindungs-Anlage-Dialog hat VIER Felder (Label,
Datacenter-URL, Client-ID, Client-Secret als `type="password"`), der
echte Drei-Werte-Zugangsdatensatz (siehe oben). `DeviceSummaryLine` zeigt
statt Online/Offline/Plattform/Site (die es hier nicht gibt) einen
Sicherungsstatus-Punkt: `"ok"`/`"information"` -> "In Ordnung" (grün),
`"warning"` -> "Warnung" (Amber; kein `--warning`-CSS-Custom-Property
existiert in `theme.css`, deshalb ein literaler Farbwert, analog zu
`TacticalRmmPluginSection.tsx`s literalen Statusfarben), `"error"`/
`"critical"` -> "Fehler" (rot), fehlend/`None` -> "Kein Status" (grau).
Anders als `TacticalRmmPluginSection.tsx` gibt es KEINE
Vergleichen/Übernehmen-Tabelle für verknüpfte Ressourcen: Acronis hat
keinen verifizierten Einzelressourcen-Endpunkt, nur die tenant-weite,
ungefilterte `resource_statuses`-Antwort ohne bestätigte Feldnamen jenseits
von `id`/`name`/`severity` (siehe oben); das Details-Panel zeigt deshalb
nur die per `id === device.external_id` gefundene eigene Roh-JSON-Ressource
dieser Antwort, rein lesbar, statt Feldnamen zu erraten, die nie
verifiziert wurden. `PluginsView.tsx` bindet die Sektion alphabetisch
zwischen `AbmPluginSection.tsx` und `IntunePluginSection.tsx` ein.
