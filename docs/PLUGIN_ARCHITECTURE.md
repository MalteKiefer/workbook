# Plugin-Architektur

Status: Siebzehn echte Integrationen umgesetzt: NinjaOne (`plugin::ninja`,
`commands::plugins`), Level.io (`plugin::level`, `commands::level`),
Snipe-IT (`plugin::snipeit`, `commands::snipeit`), Microsoft Intune
(`plugin::intune`, `commands::intune`), Iru (`plugin::iru`,
`commands::iru`), Jamf Pro (`plugin::jamf`, `commands::jamf`), Apple
Business Manager (`plugin::abm`, `commands::abm`), Tactical RMM
(`plugin::tacticalrmm`, `commands::tacticalrmm`), Atera (`plugin::atera`,
`commands::atera`), Pulseway (`plugin::pulseway`, `commands::pulseway`),
Kaseya VSA (`plugin::kaseya`, `commands::kaseya`), Action1
(`plugin::action1`, `commands::action1`), Datto RMM
(`plugin::dattormm`, `commands::dattormm`), Acronis Cyber Protect Cloud
(`plugin::acronis`, `commands::acronis`), Hetzner Cloud
(`plugin::hetzner`, `commands::hetzner`), netcup (`plugin::netcup`,
`commands::netcup`) und Vultr (`plugin::vultr`, `commands::vultr`), alle
mit Mehrfach-Verbindungs-Unterstützung. Acronis ist dabei anders als die
übrigen: kein RMM/MDM-Gerätebestand, sondern Sicherungsstatus pro Gerät
(siehe eigener Abschnitt unten). Hetzner Cloud, netcup und Vultr sind
ebenfalls anders: kein RMM/MDM-Gerätebestand und kein Sicherungsstatus,
sondern reiner Server-Bestand (Cloud-VPS-Instanzen, siehe jeweils eigener
Abschnitt unten), die einfachste Plugin-Art in dieser Codebasis.
`DummyPlugin` bleibt als Attrappen-Referenzimplementierung bestehen. Noch
kein UI-Aufruf im Sinne einer Command Palette, die Kommandos sind aber
vollständig Ende-zu-Ende von einem Frontend aus nutzbar (`PluginsView.tsx`
als dünne Hülle um
`NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`/`IntunePluginSection.tsx`/`IruPluginSection.tsx`/`JamfPluginSection.tsx`/`AbmPluginSection.tsx`/`TacticalRmmPluginSection.tsx`/`AteraPluginSection.tsx`/`PulsewayPluginSection.tsx`/`KaseyaPluginSection.tsx`/`Action1PluginSection.tsx`/`DattoRmmPluginSection.tsx`/`AcronisPluginSection.tsx`/`HetznerPluginSection.tsx`/`NetcupPluginSection.tsx`/`VultrPluginSection.tsx`).

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

## Atera-Plugin (`plugin::atera`) -- neunte echte Integration

`plugin/atera.rs` implementiert `Plugin` für Ateras REST-API über HTTPS.
Atera (<https://www.atera.com/>) ist ein Cloud-gehostetes RMM/PSA-Tool für
Managed Service Provider -- kombiniert bewusst zwei Muster, die bereits an
anderer Stelle in dieser Codebase existieren, statt ein drittes einzuführen:

- Wie Level.io (`plugin::level`) und ANDERS als Tactical RMM/NinjaOne/
  Snipe-IT: Atera ist ein fester, einzelner Cloud-Host, nicht selbst
  gehostet -- kein vom Nutzer angegebenes `base_url`-Feld an
  `AteraConnectionMeta`, nur die Konstante `BASE_URL`
  (`https://app.atera.com/api/v3`).
- Wie Tactical RMM (`plugin::tacticalrmm`) und NinjaOne (`plugin::ninja`),
  und ANDERS als Level.io: Atera bildet eine echte Mehrmandantenfähigkeit
  innerhalb einer Verbindung ab ("Customer" -> "Agent") -- eine Verbindung
  (ein API-Key) kann mehrere Atera-Kunden sehen, jeder einzeln über
  `AteraCustomerMapping`/`Config::atera_customer_mappings` einem lokalen
  Kunden zugeordnet, exakt dieselbe Verbindungs-/Zuordnungstabellen-Form wie
  `TacticalRmmConnectionMeta`/`TacticalRmmClientMapping`.

Jede Angabe unten stammt aus dem Auftrags-Briefing für diese Integration,
selbst bereits gegen Ateras eigene offizielle API-Dokumentation verifiziert
(nicht hier neu hergeleitet oder geraten):

- **Authentifizierung**: statischer API-Key im eigenen `X-API-KEY`-Header
  (NICHT `Authorization: Bearer`) -- derselbe Header-Name/dieselbe
  Konvention wie Tactical RMM. Der Nutzer erzeugt ihn selbst in Ateras
  eigener Weboberfläche (Admin -> Data Management -> API). Ein einzelner
  Geheimwert, 1:1 als `PluginCredentials.secret` durchgereicht -- wie
  Tactical RMM/Level.io/Snipe-IT, keine JSON-Kodierung mehrerer Werte nötig
  (anders als NinjaOne).
- **Basis-URL**: fest, ein einzelner SaaS-Host,
  `https://app.atera.com/api/v3` (`BASE_URL`) -- wie
  `plugin::level::BASE_URL`, keine Verbindungs-spezifische Konfigurierbarkeit,
  keine Region-/Instanz-Varianten.
- **Kunden (Mehrmandantenfähigkeit)**: `GET /customers`, ein PAGINIERTER
  Umschlag (camelCase-Umschlag-Schlüssel, PascalCase-Feld-Namen innerhalb der
  Elemente -- tatsächlich unterschiedliche Schreibweisen in derselben
  Antwort, kein Tippfehler):
  `{"items": [{"CustomerID": <Zahl>, "CustomerName": "...", ...}],
  "page": <Zahl>, "itemsInPage": <Zahl>, "totalPages": <Zahl>,
  "totalItemCount": <Zahl>, "nextLink": "..."}`. Anders als Tactical RMMs
  nacktem, unpaginiertem `/clients/`-Array braucht das eine echte
  Paginierungs-Schleife -- `fetch_all_pages` läuft `page`/`totalPages` ab
  (nicht `nextLink`, einfacher und gleichwertig korrekt: `nextLink` ist
  lediglich `page + 1` mit demselben `itemsInPage` neu zusammengesetzt), bis
  `page >= totalPages`, durchgehend mit `itemsInPage=50` (dem dokumentierten
  Maximum).
- **Agenten (NICHT "Devices")**: `GET /agents`, derselbe paginierte
  Umschlag wie `/customers` -- Ateras "Devices" ist ein eigenes SNMP-/
  Netzwerk-Monitoring-Konzept, bewusst nicht hier verwendet. Element-Felder
  (`AgentQueryDTO`, PascalCase): `AgentID` (Zahl, als `external_id`
  verwendet, als String), `MachineName` (String, Hostname/Anzeigename),
  `IpAddresses` (ein ECHTES JSON-Array lokaler IPs, anders als Tactical
  RMMs Komma-getrennter String -- `extract_ip_address` ist dadurch
  einfacher), `ReportedFromIP` (String, öffentliche/externe IP als
  Rückfallebene), `Online` (bool, auf die in dieser Codebase übliche
  String-Konvention `"online"`/`"offline"` abgebildet, z. B.
  `plugin::tacticalrmm`), `OS` (String, Plattform-/Betriebssystem-
  Anzeigetext), `CustomerID` (Zahl) -- ein ECHTER numerischer Fremdschlüssel
  direkt am Agenten, anders als Tactical RMMs Namens-basierte
  Client-Zuordnung (siehe `plugin::tacticalrmm`-Moduldokumentation) --
  die Gruppierung nach Kunde nutzt hier eine echte ID-Zuordnung, exakt wie
  bei `plugin::ninja`s `organizationId`, keinen Namens-Abgleich.
  `CustomerName` (String, für die Anzeige).
- **Einzelner Agent im Detail**: `GET /agents/{agentId}`.
- **Kein verifiziertes Dashboard-Link-Format**: das Schema trägt ein
  `AppViewUrl`-Feld am Agenten-Objekt laut Ateras eigener API, dessen genaue
  URL-Form aber nicht unabhängig bestätigt wurde. `map_agent` reicht es
  UNVERÄNDERT durch, falls vorhanden (`AteraAgent.view_url`), konstruiert
  aber niemals selbst eine URL, wenn es fehlt -- dasselbe Prinzip der
  "ehrlichen Auslassung" wie Tactical RMMs fehlendes `tacticalrmm_url`
  (siehe `plugin::tacticalrmm`-Moduldokumentation), hier nur pro Agent
  aufgelöst statt das Feld ganz aus dem Typ auszulassen, weil Atera es
  manchmal tatsächlich liefert.
- **Rate-Limits**: von Atera nicht offiziell dokumentiert -- keine
  Sonderbehandlung eingebaut (kein Backoff/Retry), dieselbe "nicht
  dokumentiert, also nicht geraten"-Haltung wie Tactical RMMs Moduldokumentation
  zu Rate-Limits.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie jedes
  andere Plugin in dieser Codebase.

### Atera-Verbindungen, jede mit mehreren Atera-Kunden

Strukturell an Tactical RMMs/NinjaOnes Verbindungs-/Client- bzw.
-Organisations-Modell angelehnt, mit EINER bewussten Abweichung (keine
`base_url`):

- Nicht-geheime Metadaten (`id`, `label`, bewusst OHNE `base_url` -- siehe
  oben -- und bewusst OHNE `customer_id`) liegen als `AteraConnectionMeta` in
  `Config::atera_connections` (`#[serde(default)]`-kompatibel).
  Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"atera:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Tactical RMM/NinjaOne/Level.io/Snipe-IT.
- Welcher Atera-Kunde innerhalb einer Verbindung welchem lokalen Kunden
  entspricht (falls überhaupt), steht granular in
  `Config::atera_customer_mappings` (`AteraCustomerMapping { connection_id,
  customer_id, customer_name, local_customer_id }` -- bewusst
  `local_customer_id` statt schlicht `customer_id` für den lokalen
  Fremdschlüssel, um Ateras eigenes "Customer"-Konzept nicht mit der
  lokalen `Customer`-Entität zu verwechseln). Ein nicht zugeordneter
  Atera-Kunde liefert bei jeder Synchronisierung seine Agenten weiterhin
  (zur Ansicht), aber immer mit `linked_system_id: None`.
- `commands::atera::group_agents_by_customer` gruppiert Agenten nach
  Atera-Kunde -- strukturell analog zu `commands::plugins::
  group_devices_by_organization` (NinjaOne), NICHT zu Tactical RMMs
  namens-basiertem `group_agents_by_client`: die Zuordnung Agent ->
  Atera-Kunde läuft über eine ECHTE numerische ID
  (`agent.customer_id == customer.id`), weil Ateras Agenten-Listen-API einen
  echten `CustomerID`-Fremdschlüssel trägt (siehe oben).
  Agenten, deren `customer_id` zu keinem bekannten Atera-Kunden passt
  (sollte normalerweise nicht vorkommen, ist aber nicht ausgeschlossen --
  z. B. ein zwischen Kunden- und Agenten-Abruf im selben Sync-Lauf
  gelöschter Kunde), werden nicht stillschweigend verworfen, sondern als
  eigene Restgruppe angehängt, mit der rohen ID als synthetischer ID UND
  Anzeigename (kein Name bekannt) -- exakt wie bei NinjaOnes
  Restgruppen-Behandlung.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie Tactical RMM/NinjaOne/Level.io/Snipe-IT:
`sync_atera_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/atera-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`). `get_cached_atera_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff), rejoint dabei aber
`customer_id` je Gruppe live gegen die AKTUELLEN
`Config::atera_customer_mappings` (nicht den beim letzten Sync
eingefrorenen Wert) -- exakt wie
`commands::tacticalrmm::get_cached_tacticalrmm_sync` --, und liefert
`None`, wenn für eine Verbindung noch nie synchronisiert wurde.
`remove_atera_connection` löscht diese Cache-Datei (bestes Bemühen) und alle
`atera_customer_mappings`-Zeilen der entfernten Verbindung gleich mit.

### Tauri-Kommandos (`commands::atera`)

`test_atera_connection`, `list_atera_connections`, `add_atera_connection`,
`remove_atera_connection`, `list_atera_customers`, `map_atera_customer`,
`unmap_atera_customer`, `sync_atera_connection`, `get_cached_atera_sync`,
`link_system_to_atera`, `unlink_system_from_atera`,
`get_atera_system_details` -- dünne Wrapper nach dem Muster von
`commands::tacticalrmm`. `list_atera_customers` liefert die Live-
Kundenliste einer Verbindung (analog zu `list_tacticalrmm_clients`), wird
aber vom Frontend nicht aufgerufen -- `AteraPluginSection.tsx` ist wie
`TacticalRmmPluginSection.tsx` konsequent Cache-first
(`get_cached_atera_sync` beim Öffnen, `sync_atera_connection` nur auf
"Aktualisieren"); der Befehl bleibt für Symmetrie und einen möglichen
künftigen Ersteinrichtungs-Anwendungsfall erhalten. Das Übernehmen eines
extern gelieferten Werts in ein selbst gepflegtes Feld (`name`, `hostname`,
`ip_address`, `notes` in `systems`) bleibt -- wie bei allen anderen Plugins
-- ausschließlich eine bewusste, manuelle Aktion über
`get_atera_system_details` plus eine spätere UI-Aktion; kein Kommando hier
schreibt automatisch in diese vier Felder.

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Wie Tactical RMM (und anders als Snipe-IT, das mangels Hostname-Feld auf
`asset_tag`/`serial` ausweichen muss) hat ein Atera-Agent ein echtes
Hostname-Äquivalent (`MachineName`) -- Atera ist RMM-Software, keine
Asset-/Inventarverwaltung. `AteraPluginSection.tsx`s `matchKeyForDevice`
verwendet deshalb, wie bei Tactical RMM/NinjaOne/Level.io, direkt
`device.hostname` als Abgleichsschlüssel für den "Mit bestehendem System
verknüpfen"-Vorschlag, verglichen gegen das einzige freie Textfeld, das ein
lokales System dafür hat -- `System.hostname`.

### Frontend (`AteraPluginSection.tsx`)

Mechanisch an `TacticalRmmPluginSection.tsx`s Stand angelehnt (Atera-Kunden
eingeklappt mit Geräte-Anzahl-Zusammenfassung, Kunde-Zuordnungs-`<select>`
inklusive "+ Neuen Kunden anlegen…", aufklappbare, filterbare,
10-pro-Seite-paginierte Geräteliste mit `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation über `getKeymap()`/`matchesBinding` -- wie alle anderen
Plugin-Sektionen --, Vergleichs-/Übernahme-Panel für verknüpfte Geräte). Der
Verbindungs-Anlage-Dialog hat NUR ZWEI Felder (Label, API-Key als
`type="password"`) -- kein Base-URL-Feld, wie bei Level.io, und kein
Client-ID/-Secret-Paar, wie bei NinjaOne; das ist der einzige bedeutsame
UI-Unterschied zu Tactical RMMs dreifeldrigem Formular.
`DeviceSummaryLine` zeigt zusätzlich einen Online/Offline-Statuspunkt und
Ateras freien `OS`-Anzeigetext (kein Label-Nachschlagen wie bei Tactical
RMMs `windows`/`linux`/`darwin` -- Ateras `OS`-Feld ist freier Text, keine
kleine verifizierte Konstantenmenge). Anders als Tactical RMM (aber wie
NinjaOne/Snipe-IT) KANN eine Geräte-Zeile einen "In Atera öffnen"-Link
zeigen (`AteraLink`) -- aber nur, wenn Atera für diesen Agenten tatsächlich
ein `AppViewUrl` geliefert hat; ohne dieses Feld erscheint schlicht kein
Link, keine geratene URL. `PluginsView.tsx` bindet die Sektion alphabetisch
zwischen `AbmPluginSection.tsx` und `IntunePluginSection.tsx` ein (siehe
Kommentar dort).

## Pulseway-Plugin (`plugin::pulseway`) -- zehnte echte Integration

`plugin/pulseway.rs` implementiert `Plugin` für Pulseways REST-API über
HTTPS. Pulseway (<https://www.pulseway.com/>) ist in erster Linie ein
Cloud-gehostetes RMM-Tool, bietet aber auch eine selbst gehostete
"Enterprise Server"-Variante unter derselben API-Form an -- strukturell am
nächsten an Tactical RMM: eine konfigurierbare `base_url` (Standardwert der
Cloud-Host im Frontend-Formular, aber immer änderbar) UND eine echte
Mehrfach-Mandantenfähigkeit innerhalb einer Verbindung, hier sogar VIER
Ebenen tief ("Organization" -> "Site" -> "Group" -> "Device", eine Ebene
mehr als Tactical RMMs "Client" -> "Site" -> "Agent"). Authentifizierung
braucht dagegen, wie NinjaOne, ZWEI Geheimwerte (Token-ID + Token-Secret,
siehe unten) statt Tactical RMMs einzelnem API-Key.

Die Angaben unten stammen aus Pulseways eigener offizieller API-Referenz
unter <https://api.pulseway.com>, MIT EINER AUSNAHME -- dem genauen
Weboberflächen-Pfad zum Erzeugen eines Tokens, der aus einer
Drittanbieter-Integrationsanleitung stammt, nicht aus Pulseways eigener
Primärdokumentation, und deshalb unten ausdrücklich als "plausibel, nicht
vollständig verifiziert" markiert ist -- dieselbe Ehrlichkeits-Konvention,
die `plugin::tacticalrmm` für seine eigenen unbestätigten Angaben verwendet
(siehe dort, "Kein verifizierter Weboberflächen-Link"):

- **Authentifizierung**: HTTP Basic Auth --
  `Authorization: Basic <base64("{token_id}:{token_secret}")>`. Der Nutzer
  erzeugt ein Token-ID-/Token-Secret-Paar in Pulseways eigener
  Weboberfläche (PLAUSIBEL, NICHT VOLLSTÄNDIG VERIFIZIERT: "Configuration ->
  API Access -> Third Party Tokens -> Create Token", laut einer
  Drittanbieter-Integrationsanleitung, nicht Pulseways eigener
  Primärdokumentation). `basic_auth_header` baut den Header-Wert aus einem
  `PulsewayCredentials`-Wert mit der `base64`-Crate (bereits Abhängigkeit,
  0.23.1).
- **Basis-URL**: der feste Cloud-Host `https://api.pulseway.com/v3` ist der
  Standardwert (siehe Platzhalter im Frontend-Formular), aber Pulseway
  dokumentiert auch eine selbst gehostete "Enterprise Server"-Variante unter
  derselben API-Form (`https://<eigener-server>/api/v3`). Wie bei Tactical
  RMM/NinjaOne/Snipe-IT (und anders als Level.ios feste `BASE_URL`-
  Konstante) braucht eine Verbindung deshalb eine vom Nutzer angegebene,
  immer änderbare `PulsewayConnectionMeta.base_url`.
- **Mandanten-Hierarchie**: VIER Ebenen -- Organization -> Site -> Group ->
  Device. Die Zuordnungs-Granularität bleibt trotzdem auf
  Organisations-Ebene, exakt wie Tactical RMM auf Client-Ebene zuordnet und
  NinjaOne auf Organisations-Ebene -- keine zweite Zuordnungs-Ebene für
  Site/Group. Site und Group eines Geräts werden trotzdem als
  reine Anzeige-Information mitgeliefert (`PulsewayDevice.site_name`/
  `group_name`).
- **Organisationen**: `GET /organizations` liefert einen Umschlag --
  `{"Data": [...], "Meta": {"ResponseCode": 200, "TotalCount": <n>}}` --
  PascalCase-Umschlagsschlüssel UND PascalCase-Feldnamen (`Id`, `Name`,
  `Type`). WEDER Tactical RMMs nackter, unpaginierter Array NOCH Snipe-ITs
  `{"total", "rows"}`-Form -- ein eigener Umschlag, der einen expliziten
  `Data`-Unwrap braucht (siehe `map_organizations_pages`).
- **Geräte**: `GET /devices`, derselbe `Data`/`Meta`-Umschlag. Verifizierte
  Listenfelder: `Identifier` (ein String-GUID -- als `external_id`
  verwendet, analog zu Tactical RMMs `agent_id`), `Name` (der Hostname des
  Geräts -- Pulseways eigene API-Referenz dokumentiert dieses Feld ALS den
  Hostnamen, es gibt kein separates `hostname`-Feld), `OrganizationId` (ein
  echter NUMERISCHER Fremdschlüssel -- ein echter, verifizierter Unterschied
  zu Tactical RMM, dessen Agenten-Liste gar keine Client-ID trägt und einen
  NAMEN-basierten Join braucht, siehe oben; hier kann
  `commands::pulseway::group_devices_by_organization` nach ID joinen, exakt
  wie `commands::plugins::group_devices_by_organization` es für NinjaOne
  tut), `OrganizationName`, `SiteName`, `GroupName`, `IsAgentInstalled`
  (bool). **Verifizierte, bewusste Lücke**: die Listen-Schnittstelle hat
  KEINE IP-Adresse, KEINEN Online-/Offline-Status und KEIN Plattform-/
  OS-Feld -- diese drei existieren nur bei der Einzelgeräte-Detail-
  Schnittstelle (siehe unten). `PulsewayDevice` hat deshalb GAR KEINE
  `ip_address`-/`status`-/`platform`-Felder -- keine still auf `None`
  gesetzten Felder, die fälschlich nahelegen würden, ein Abrufversuch hätte
  stattgefunden, sondern Felder, die im Typ schlicht nicht existieren.
- **Einzelgeräte-Detail**: `GET /devices/{id}` (`id` = das `Identifier`-GUID
  aus der Liste) trägt die REICHHALTIGEN Felder, die der Listen-Schnittstelle
  fehlen -- `IsOnline` (bool), `ComputerType` (String, z. B. `"windows"`),
  `ExternalIpAddress` (String), `LocalIpAddresses` (ein Array von
  Netzwerkadapter-Objekten, je mit `IpV4`/`IpV6`). AUSSCHLIESSLICH von
  `Plugin::get_system_details` verwendet (ein Aufruf, Rohdaten unverändert
  durchgereicht, exakt wie `plugin::tacticalrmm::get_system_details`) --
  bewusst NIE pro Gerät während `list_systems`/eines Sync-Laufs aufgerufen,
  das wäre ein N+1-Aufrufmuster gegen ein dokumentiertes Ratenlimit von rund
  3600 Anfragen/Stunde (siehe unten). Der Listen-/Sync-Pfad verwendet
  ausschließlich die schlanken `/devices`-Felder; Status/IP/Plattform
  tauchen nur auf, wenn ein Nutzer die Details EINES Systems ansieht.
- **Paginierung**: OData-artige `$top`/`$skip`-Query-Parameter,
  `$count=true` für `Meta.TotalCount`. `Meta.NextQueryLink` (eine
  vollständige URL zur nächsten Seite) wird erst zurückgeliefert, sobald die
  Gesamtergebnisse 5000 überschreiten -- für typische MSP-Flottengrößen
  darunter würde in der Praxis schon eine einzelne Anfrage mit großem `$top`
  reichen, `fetch_all_pages` implementiert trotzdem eine ECHTE
  Paginierungs-Schleife (folgt `Meta.NextQueryLink`, solange vorhanden,
  sonst erhöht `$skip` um die Anzahl der Einträge der jeweils letzten Seite,
  solange die gesammelte Anzahl noch unter `Meta.TotalCount` liegt) -- die
  gleiche erschöpfende Abruf-Disziplin wie bei Tactical RMM, hier auf
  Pulseways tatsächlich paginierte Form angepasst.
- **Ratenlimits**: dokumentiert mit rund 3600 Anfragen/Stunde pro Token pro
  Endpunkt (variiert je Endpunkt); bei Überschreitung Status 429 mit
  `Retry-After`-Header. Für diese Integration ist keine besondere
  Retry-/Backoff-Logik implementiert -- entspricht jedem bestehenden Plugin
  in dieser Codebasis, keines davon implementiert Retry/Backoff; ein 429
  fällt einfach in `map_ureq_error`s generischen "anderer Status ->
  Unreachable"-Zweig wie jede andere Nicht-2xx-Antwort.
- **Kein Weboberflächen-Deep-Link**: ein echtes `ExternalUrl`-Feld existiert
  in Pulseways API, aber nur bei der separaten `/assets`-/`/assets/{id}`-
  Ressource, NICHT bei `/devices` -- die Verwendung würde einen zusätzlichen
  Aufruf pro Gerät erfordern, den dieses Plugin sonst nicht braucht (siehe
  N+1-/Ratenlimit-Hinweis oben). Eine ehrliche Auslassung, analog zu
  Tactical RMMs fehlendem `tacticalrmm_url` -- auch hier kein
  `pulseway_url`-Feld/DTO-Attribut.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie jedes
  andere echte Plugin in dieser Codebasis.
- **Zugangsdaten-Kodierung**: Pulseway braucht ZWEI Geheimwerte (Token-ID +
  Token-Secret), also trägt `PluginCredentials.secret` ein kleines
  JSON-Objekt (`{"token_id":"...","token_secret":"..."}`), mit
  `serde_json::from_str` geparst -- exakt `plugin::ninja::
  NinjaCredentials`s Muster (`client_id`/`client_secret`), NICHT Tactical
  RMMs Ein-String-Durchreichung.

### Pulseway-Verbindungen, jede mit mehreren Organisationen

Strukturell identisch zu Tactical RMMs/NinjaOnes Verbindungs-/Client- bzw.
-Organisations-Modell:

- Nicht-geheime Metadaten (`id`, `label`, `base_url`, bewusst OHNE
  `customer_id`) liegen als `PulsewayConnectionMeta` in
  `Config::pulseway_connections` (`#[serde(default)]`-kompatibel).
  Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"pulseway:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Tactical RMM/NinjaOne.
- Welche Organisation innerhalb einer Verbindung welchem lokalen Kunden
  entspricht (falls überhaupt), steht granular in
  `Config::pulseway_org_mappings` (`PulsewayOrgMapping { connection_id,
  organization_id, organization_name, customer_id }`). Eine nicht
  zugeordnete Organisation liefert bei jeder Synchronisierung ihre Geräte
  weiterhin (zur Ansicht), aber immer mit `linked_system_id: None`.
- `commands::pulseway::group_devices_by_organization` gruppiert Geräte nach
  Organisation -- strukturell analog zu `commands::plugins::
  group_devices_by_organization`, MIT EINER bewussten, verifizierten
  Übereinstimmung mit NinjaOne (und Abweichung von Tactical RMM): die
  Zuordnung Gerät -> Organisation läuft über die echte numerische ID
  (`device.organization_id == organization.id`), NICHT über den Namen, weil
  Pulseways Geräte-Listen-API sehr wohl eine numerische Organisations-ID
  trägt (siehe oben). Geräte, deren `organization_id` zu keiner bekannten
  Organisation passt (sollte normalerweise nicht vorkommen, ist aber nicht
  ausgeschlossen -- z. B. eine zwischen Organisations- und Geräte-Abruf im
  selben Sync-Lauf gelöschte Organisation), werden nicht stillschweigend
  verworfen, sondern als eigene Restgruppe angehängt -- mit dem Anzeigenamen
  aus dem Geräte-eigenen `organization_name`-Feld (Pulseway-Geräte tragen
  das redundant pro Gerät, anders als NinjaOne) statt der rohen ID, eine
  kleine Verbesserung gegenüber `commands::plugins::
  group_devices_by_organization`s Restgruppen-Behandlung.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie jedes andere Plugin:
`sync_pulseway_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/pulseway-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`). `get_cached_pulseway_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff), rejoint dabei aber
`customer_id` je Gruppe live gegen die AKTUELLEN
`Config::pulseway_org_mappings` (nicht den beim letzten Sync eingefrorenen
Wert) -- exakt wie `commands::tacticalrmm::get_cached_tacticalrmm_sync` --,
und liefert `None`, wenn für eine Verbindung noch nie synchronisiert wurde.
`remove_pulseway_connection` löscht diese Cache-Datei (bestes Bemühen) und
alle `pulseway_org_mappings`-Zeilen der entfernten Verbindung gleich mit.

### Tauri-Kommandos (`commands::pulseway`)

`test_pulseway_connection`, `list_pulseway_connections`,
`add_pulseway_connection`, `remove_pulseway_connection`,
`list_pulseway_organizations`, `map_pulseway_organization`,
`unmap_pulseway_organization`, `sync_pulseway_connection`,
`get_cached_pulseway_sync`, `link_system_to_pulseway`,
`unlink_system_from_pulseway`, `get_pulseway_system_details` -- dünne
Wrapper nach dem Muster von `commands::tacticalrmm`. `add_pulseway_connection`
nimmt ein Token-ID-/Token-Secret-Paar entgegen (nicht einen einzelnen
API-Key) und kodiert es vor dem Speichern über
`plugin::secrets::store_secret` als JSON
(`{"token_id":"...","token_secret":"..."}`), exakt wie
`commands::plugins::add_ninja_connection`s Client-ID-/-Secret-Paar.
`list_pulseway_organizations` liefert die Live-Organisationsliste einer
Verbindung (analog zu `list_tacticalrmm_clients`/
`list_ninja_organizations`), wird aber vom Frontend nicht aufgerufen --
`PulsewayPluginSection.tsx` ist wie jede andere Plugin-Sektion konsequent
Cache-first (`get_cached_pulseway_sync` beim Öffnen,
`sync_pulseway_connection` nur auf "Aktualisieren"); der Befehl bleibt für
Symmetrie und einen möglichen künftigen Ersteinrichtungs-Anwendungsfall
erhalten. Das Übernehmen eines extern gelieferten Werts in ein selbst
gepflegtes Feld (`name`, `hostname`, `ip_address`, `notes` in `systems`)
bleibt -- wie bei jedem anderen Plugin -- ausschließlich eine bewusste,
manuelle Aktion über `get_pulseway_system_details` plus eine spätere
UI-Aktion; kein Kommando hier schreibt automatisch in diese vier Felder.
`sync_pulseway_connection` ruft `get_system_details` (die
Einzelgeräte-Detail-Schnittstelle) nur für BEREITS verknüpfte Geräte auf --
niemals für alle Geräte einer Synchronisierung -- um das N+1-Aufrufmuster
gegen das Ratenlimit zu vermeiden (siehe oben).

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Wie Tactical RMM/NinjaOne hat ein Pulseway-Gerät ein Feld, das als Hostname
dient (`Name`, siehe oben) -- `PulsewayPluginSection.tsx`s
`matchKeyForDevice` verwendet deshalb direkt `device.hostname` (immer
identisch zu `device.name`, siehe DTO-Kommentar) als Abgleichsschlüssel für
den "Mit bestehendem System verknüpfen"-Vorschlag, verglichen gegen das
einzige freie Textfeld, das ein lokales System dafür hat --
`System.hostname`.

### Frontend (`PulsewayPluginSection.tsx`)

Mechanisch an `TacticalRmmPluginSection.tsx`s Stand angelehnt
(Organisationen eingeklappt mit Geräte-Anzahl-Zusammenfassung,
Kunde-Zuordnungs-`<select>` inklusive "+ Neuen Kunden anlegen…",
aufklappbare, filterbare, 10-pro-Seite-paginierte Geräteliste mit
`j`/`k`/`Enter`/`l`/`u`-Tastaturnavigation, Vergleichs-/Übernahme-Panel für
verknüpfte Geräte). Der Verbindungs-Anlage-Dialog hat VIER Felder (Label,
Base-URL, Token-ID, Token-Secret als `type="password"`) statt Tactical RMMs
drei -- ein Token-ID-/Token-Secret-Paar nötig, wie bei NinjaOnes Client-ID/
-Secret (`NinjaPluginSection.tsx`). `DeviceSummaryLine` zeigt statt eines
Online/Offline/Überfällig-Statuspunkts und einer Plattform-Badge (die es bei
Pulseway auf der Listen-Schnittstelle verifiziert nicht gibt, siehe oben)
Site- und Gruppen-Namen sowie einen einfachen "Agent installiert"/"Kein
Agent"-Punkt (`IsAgentInstalled`). Wie bei Tactical RMM gibt es bewusst
KEINEN "In Pulseway öffnen"-Link auf einer Geräte-Zeile (siehe oben, kein
verifizierter Weboberflächen-Link ohne zusätzlichen Aufruf).
`PluginsView.tsx` bindet die Sektion als zehnte Karte neben
`NinjaPluginSection.tsx`/`LevelPluginSection.tsx`/`SnipeitPluginSection.tsx`/`IntunePluginSection.tsx`/`IruPluginSection.tsx`/`JamfPluginSection.tsx`/`AbmPluginSection.tsx`/`TacticalRmmPluginSection.tsx`/`AteraPluginSection.tsx`
ein, alphabetisch zwischen `NinjaPluginSection.tsx` und
`SnipeitPluginSection.tsx`.
## Kaseya-VSA-Plugin (`plugin::kaseya`) -- elfte echte Integration

`plugin/kaseya.rs` implementiert `Plugin` für Kaseya VSAs REST-API über
HTTPS. Kaseya VSA (aktuell "VSA X", teils noch "VSA 10" genannt) ist ein
selbst gehostetes/Single-Tenant-RMM-Tool -- jede Kundin betreibt ihren
eigenen VSA-Server. Strukturell am nächsten an Tactical RMM: selbst
gehostet mit einer vom Nutzer angegebenen `base_url`, ein einzelnes
statisches Geheimnis zur Authentifizierung (kein OAuth2-Grant wie bei
NinjaOne), Zuordnungs-Granularität auf der obersten Mandanten-Ebene
("Organization") -- genau wie Tactical RMMs Client-Ebene.

WICHTIG, bevor irgendeine Angabe unten für bare Münze genommen wird: Kaseyas
öffentliche Dokumentation zu VSA X ist merklich dünner und teils
widersprüchlich im Vergleich zu jeder anderen Integration in dieser
Codebase. Jeder Punkt unten ist entweder als BESTÄTIGT markiert (konsistent
in Kaseyas eigener VSA-X-Admin-Dokumentation gefunden) oder als explizite
ENTSCHEIDUNG/OFFENES RISIKO, wo sich die Dokumentation selbst widerspricht
oder schweigt -- nichts hier wird stillschweigend sicherer dargestellt, als
es tatsächlich ist:

- **Authentifizierung (BESTÄTIGT)**: HTTP Basic Auth, Header
  `Authorization: Basic <base64("{token_id}:{token_secret}")>`. Verifiziert
  gegen Kaseyas eigene VSA-X-Admin-Dokumentation (Configuration -> API
  Access -> Third Party Tokens -> Create Token, dort auch optionales
  IP-Allowlisting/Ablaufdatum auf dem Token selbst -- beides muss dieses
  Plugin nicht kennen, wird vollständig Kaseya-seitig konfiguriert). Es
  werden zwei Geheimwerte benötigt (Token-ID + Token-Secret), daher hält
  `PluginCredentials.secret` -- genau wie bei `plugin::ninja::
  NinjaCredentials` -- ein kleines JSON-Objekt (`KaseyaCredentials`), keinen
  einzelnen opaken String wie Level.io/Snipe-IT/Tactical RMM. Hinweis für
  eine parallel entstehende Pulseway-Integration: Pulseways Authentifizierung
  ist mit derselben Form dokumentiert (`base64("{token_id}:{token_secret}")`
  Basic Auth) -- dieses Modul wurde NICHT in Abhängigkeit von diesem Plugin
  geschrieben, das Muster wiederholt sich hier nur zufällig.
- **Base-URL -- DOKUMENTIERTE WIDERSPRÜCHLICHKEIT, Entscheidung unten
  getroffen (NICHT stillschweigend aufgelöst)**: Kaseyas eigene aktuelle
  Dokumentation widerspricht sich über zwei Quellen hinweg, was die
  API-Wurzel tatsächlich ist -- eine Seite sagt `{server_domain}/api`, eine
  andere `{server_name}/api/v3/`. Das ließ sich aus der abgerufenen
  Dokumentation allein nicht auflösen (kein Weg, ohne eine echte
  VSA-X-Instanz zum Testen zu wissen, welche Angabe aktuell/korrekt ist).
  **Entscheidung**: genau wie `plugin::tacticalrmm`s eigener "kein fester
  Pfad-Präfix"-Umgang ist `KaseyaConnectionMeta.base_url` ein Freitext-Feld,
  das der Nutzer als VOLLSTÄNDIGE, bereits funktionierende API-Wurzel-URL
  seiner VSA-Instanz angibt -- dieses Modul hängt nie einen Pfad-Suffix an
  oder nimmt einen an (`/api`, `/api/v3` oder sonst etwas). Jede Anfrage
  unten wird als `{base_url}{path}` gebaut, nach Entfernen eines
  abschließenden Schrägstrichs, nichts weiter. Das Frontend-Anlage-Formular
  trägt einen kurzen deutschen Hinweis direkt neben dem Feld, der genau
  darauf hinweist (siehe `KaseyaPluginSection.tsx`). Wer die erste echte
  Verbindung gegen einen laufenden VSA-X-Server einrichtet, sollte das als
  die Nummer-eins-Sache zum erneuten Verifizieren behandeln.
- **Mandanten-Entitäten (Existenz BESTÄTIGT, Feldnamen jenseits von
  Id/Name UNBESTÄTIGT)**: "Organizations" (`GET {base_url}/organizations`)
  und "Groups" (`GET {base_url}/groups`) existieren beide als dokumentierte
  VSA-X-API-Entitäten. Dieses Plugin ordnet ausschließlich auf
  ORGANIZATION-Ebene zu (wie Tactical RMM auf Client-Ebene, nicht der
  feineren Site-Ebene) -- `KaseyaOrgMapping` verknüpft eine Organization mit
  einem lokalen Kunden; Groups werden von diesem Plugin nie eigenständig
  zugeordnet oder gelistet, sondern nur (als bloße, unaufgelöste `GroupId`)
  an einem Gerät zur reinen Anzeige referenziert. Organization-Feldnamen
  jenseits von `Id`/`Name` sind für VSA X speziell NICHT bestätigt --
  `map_organization` liest daher ausschließlich `value["Id"]`/
  `value["Name"]`, genauso tolerant/defensiv wie Tactical RMMs eigenes
  `map_client`: ein Eintrag ohne verwertbare `Id` wird übersprungen statt als
  fataler Fehler behandelt, ein fehlender `Name` fällt auf die `Id` selbst
  zurück.
- **Geräte (BESTÄTIGTE Felder; IP/Online-Status explizit NICHT bestätigt,
  siehe unten)**: `GET {base_url}/devices`, dokumentiert als filterbar nach
  `Identifier`, `Name`, `GroupId`, `SiteId`, `OrganizationId`
  (Query-Filter, die dieses Plugin aktuell nicht nutzt -- es holt immer die
  vollständige, paginierte Liste). Bestätigte Felder an einem Geräte-Objekt:
  `Identifier` (ein Geräte-GUID-String -- verwendet als
  `ExternalSystem::external_id`/`KaseyaDevice::external_id`), `Name`,
  `GroupId`, `OrganizationId` (ein echter numerischer/String-Fremdschlüssel,
  KEIN Namens-Join-Behelf wie Tactical RMMs `client_name` -- Geräte werden
  daher über die ID einer Organization zugeordnet, genau wie NinjaOnes
  `organizationId` funktioniert, nicht wie Tactical RMMs Agenten),
  `IsAgentInstalled` (bool), `IsMdmEnrolled` (bool). `OrganizationId` selbst
  ist auf `KaseyaDevice` in `Option` gehüllt -- weil dieses Plugin, wegen des
  Feldnamen-Zweifels unten, selbst DESSEN garantierte Anwesenheit auf jedem
  Geräte-Objekt nicht blind vertraut; ein Gerät ohne auflösbare
  Organisationszugehörigkeit wird trotzdem behalten (nie verworfen), sondern
  von `commands::kaseya` unter einer expliziten "Nicht zugeordnet"-Gruppe
  angezeigt, statt still zu verschwinden.
  **IP-Adresse und Online-/Offline-Status-Felder sind für VSA X explizit
  NICHT bestätigt** und fehlen deshalb bewusst komplett auf `KaseyaDevice`
  -- nicht geraten, nicht als immer-`None`-Feld verkleidet als echtes
  Attribut hinzugefügt. Das spiegelt Tactical RMMs eigenes "ehrliche
  Auslassung"-Prinzip (siehe dessen Moduldokumentation zum fehlenden
  `tacticalrmm_url`-Feld) und Snipe-ITs verifiziertes, immer-`None`
  `hostname`/`ip_address`. Reichhaltigere, unbestätigte Pro-Geräte-Daten
  bleiben vollständig `get_system_details`/`GET {base_url}/devices/{id}`
  überlassen, das als rohes, unverändertes JSON zurückgegeben wird -- genau
  wie `plugin::tacticalrmm::get_system_details`.
- **Ein echtes, ernstzunehmendes Risiko, explizit benannt statt
  stillschweigend vertraut**: die während der Recherche gefundenen
  VSA-X-Feldnamen (`Identifier`, `GroupId`, `OrganizationId`,
  `IsAgentInstalled`, `IsMdmEnrolled`) sind verdächtig identisch zu einem
  anderen Hersteller-Schema (Pulseway). Das kann tatsächlich korrekt sein
  (gemeinsames API-Tooling/eine White-Label-Backend-Beziehung zwischen
  beiden Produkten wäre im RMM-Markt nicht unüblich), oder es kann ein
  Rechercheartefakt sein (z. B. ein Dokumentations-Aggregator, der zwei
  Produkte vermischt, oder eine veraltete Kopie in der abgerufenen Quelle).
  Sowohl `map_organization` als auch `map_device` sind DESHALB bewusst
  DEFENSIV geschrieben: tolerant gegenüber fehlenden oder umbenannten
  Feldern, ein fehlerhafter Eintrag wird übersprungen statt den gesamten
  Aufruf scheitern zu lassen oder zu einem Panic zu führen -- derselbe
  tolerante Stil wie `plugin::tacticalrmm::map_client`/`map_agent` --, damit
  ein abweichender realer Feldname zu einer degradiert-aber-funktionierenden
  Synchronisierung führt (weniger/leerere Felder befüllt) statt zu einem
  harten Fehlschlag. Wer die erste echte Verbindung gegen einen laufenden
  VSA-X-Server einrichtet, sollte das erneute Verifizieren dieser genauen
  Feldnamen als ZWEITE Sache behandeln, direkt nach der Base-URL-
  Widersprüchlichkeit oben.
- **Paginierung (Mechanismus BESTÄTIGT, Umschlagform nur teilweise
  bestätigt)**: OData-artige Query-Parameter (`$top`/`$skip`/`$filter`/
  `$orderby`/`$count`) sind für VSA Xs Listen-Endpunkte dokumentiert, hier
  einheitlich auf `/organizations` und `/devices` angewendet.
  `NextQueryLink` ist dokumentiert, in einer Listen-Antwort zu erscheinen,
  sobald die Gesamtergebnisse 5000 überschreiten (dieselbe Konvention, die
  auch die Datto-RMM-/Pulseway-Plugins dieser Codebase verwenden würden,
  falls/wenn gebaut -- hier nicht vorausgesetzt). Der genaue JSON-Schlüssel,
  der das Item-Array selbst hält, und der genaue Schlüssel für eine
  Gesamtanzahl sind in der abgerufenen Dokumentation NIRGENDS benannt --
  nur `NextQueryLink` selbst ist ein benanntes, bestätigtes Antwortfeld.
  `parse_page` probiert deshalb defensiv eine kleine Menge plausibler
  Umschlagformen durch (ein nackter Top-Level-Array ganz ohne Umschlag,
  oder ein Objekt mit den Items unter einem von `"Result"`/`"value"`/
  `"Items"`/`"data"`, mit einer Gesamtanzahl unter einem von `"Count"`/
  `"TotalCount"`/`"@odata.count"`), statt EINE bestimmte Form als DIE
  richtige anzunehmen. `fetch_all_pages` implementiert eine echte
  Paginierungs-Schleife: solange `NextQueryLink` in einer Seite vorhanden
  ist, wird diese URL wörtlich als nächste Anfrage-URL verwendet; sobald es
  nicht mehr erscheint, läuft die Schleife weiter, indem `$skip` erhöht
  wird, solange eine erkannte Gesamtanzahl sagt, dass noch weitere Items
  ausstehen -- deckt damit beide dokumentierten Paginierungssignale exakt
  wie gefordert ab. Eine harte Iterationsgrenze (`MAX_PAGES`) schützt vor
  einer Endlosschleife, falls eine künftige/unerwartete Antwortform
  `parse_page` verwirrt -- eine Implementierungs-Sicherung, kein
  dokumentierter API-Fakt.
- **Einzelnes Geräte-Detail (BESTÄTIGT)**: `GET {base_url}/devices/{id}`,
  liefert ein einzelnes Geräte-Objekt -- von `get_system_details`
  unverändert als rohes JSON durchgereicht, genau wie
  `plugin::tacticalrmm::get_system_details`/das Pendant bei
  `plugin::ninja`.
- **Rate-Limits (BESTÄTIGT, hier nicht durchgesetzt)**: dokumentiert als
  etwa 3600 Anfragen/Stunde in der Standard-Stufe (einzelne Endpunkte haben
  offenbar eigene, niedrigere Limits, und wiederholt fehlgeschlagene
  Anfragen lösen dokumentiert eine separate, strengere Sperr-Stufe aus).
  Dafür ist keine Retry-/Backoff-Logik implementiert -- entspricht jedem
  anderen Plugin in dieser Codebase, keines davon implementiert
  Rate-Limit-Behandlung; diese App ruft Plugin-Methoden selten/manuell auf,
  nicht in einer heißen Schleife.
- **Web-Dashboard-Deep-Link -- bewusst weggelassen**: keine erreichbare
  VSA-X-Dokumentation beschreibt einen stabilen, aus der Base-URL
  ableitbaren Web-Dashboard-Link für ein einzelnes Gerät -- daher gibt es,
  genau wie bei Tactical RMMs fehlendem `tacticalrmm_url`, hier kein
  `kaseya_url`-Feld/DTO-Attribut. Eine ehrliche Auslassung, keine als
  Feature verkleidete Vermutung.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie jedes
  andere echte Plugin in dieser Codebase.
- **Zugangsdaten-Kodierung**: zwei Geheimwerte (Token-ID + Token-Secret),
  JSON-kodiert in `PluginCredentials.secret` und hier zurück geparst --
  siehe `KaseyaCredentials`/`parse_credentials`, strukturell identisch zu
  `plugin::ninja::NinjaCredentials`/`parse_credentials`.

### Kaseya-VSA-Verbindungen, jede mit mehreren Organisationen

Strukturell identisch zu Tactical RMMs/NinjaOnes Verbindungs-/
Client- bzw. -Organisations-Modell:

- Nicht-geheime Metadaten (`id`, `label`, `base_url`, bewusst OHNE
  `customer_id`) liegen als `KaseyaConnectionMeta` in
  `Config::kaseya_connections` (`#[serde(default)]`-kompatibel).
  Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"kaseya:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Tactical RMM/NinjaOne.
- Welche Organization innerhalb einer Verbindung welchem lokalen Kunden
  entspricht (falls überhaupt), steht granular in
  `Config::kaseya_org_mappings` (`KaseyaOrgMapping { connection_id,
  organization_id, organization_name, customer_id }`). Eine nicht
  zugeordnete Organization liefert bei jeder Synchronisierung ihre Geräte
  weiterhin (zur Ansicht), aber immer mit `linked_system_id: None`.
- `commands::kaseya::group_devices_by_organization` gruppiert Geräte nach
  Organization über die echte `OrganizationId` -- strukturell analog zu
  `commands::plugins::group_devices_by_organization` (NinjaOne), NICHT über
  einen Namens-Join wie bei Tactical RMM (siehe oben, Kaseyas
  `OrganizationId` ist ein echter Fremdschlüssel). Anders als bei NinjaOne
  ist `KaseyaDevice.organization_id` aber ein `Option` -- diese Integration
  vertraut nicht blind darauf, dass jedes Geräte-Objekt sie wirklich trägt
  (siehe Feldnamen-Zweifel oben). Es gibt deshalb ZWEI unterschiedliche
  Restfälle: (1) ein Gerät mit `Some(organization_id)`, die zu keiner
  gemeldeten Organization passt (sollte normalerweise nicht vorkommen, ist
  aber angesichts des Feldnamen-Zweifels plausibel) -- wird als eigene
  Restgruppe unter der rohen ID angehängt, genau wie NinjaOnes
  Restgruppen-Behandlung; (2) ein Gerät ganz OHNE `organization_id`
  (`None`) -- wird als eigene, feste "Nicht zugeordnet"-Gruppe angehängt,
  immer als LETZTE Gruppe, unterscheidbar von Fall (1). Eine Organization
  ohne Geräte erscheint weiterhin als leere Gruppe, damit eine künftige UI
  sie trotzdem zur Zuordnung anzeigen kann. Nichts hindert daran, auch die
  "Nicht zugeordnet"-Gruppe wie jede andere einem Kunden zuzuordnen --
  `KaseyaPluginSection.tsx` braucht dafür keinen Sonderfall, sie verhält
  sich wie jede normale Gruppe.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie Tactical RMM/NinjaOne/Level.io/Snipe-IT:
`sync_kaseya_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/kaseya-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`). `get_cached_kaseya_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff), rejoint dabei aber
`customer_id` je Gruppe live gegen die AKTUELLEN
`Config::kaseya_org_mappings` (nicht den beim letzten Sync eingefrorenen
Wert) -- exakt wie `commands::tacticalrmm::get_cached_tacticalrmm_sync` --,
und liefert `None`, wenn für eine Verbindung noch nie synchronisiert wurde.
`remove_kaseya_connection` löscht diese Cache-Datei (bestes Bemühen) und
alle `kaseya_org_mappings`-Zeilen der entfernten Verbindung gleich mit.

### Tauri-Kommandos (`commands::kaseya`)

`test_kaseya_connection`, `list_kaseya_connections`,
`add_kaseya_connection`, `remove_kaseya_connection`,
`list_kaseya_organizations`, `map_kaseya_organization`,
`unmap_kaseya_organization`, `sync_kaseya_connection`,
`get_cached_kaseya_sync`, `link_system_to_kaseya`,
`unlink_system_from_kaseya`, `get_kaseya_system_details` -- dünne Wrapper
nach dem Muster von `commands::tacticalrmm`. `add_kaseya_connection` nimmt
`base_url` + `token_id` + `token_secret` entgegen und kodiert das
Credential-Paar als JSON, bevor es via `plugin::secrets::store_secret`
gespeichert wird -- genau wie `commands::plugins::add_ninja_connection` es
mit seinem Zwei-Werte-Credential handhabt. `list_kaseya_organizations`
liefert die Live-Organisationsliste einer Verbindung (analog zu
`list_tacticalrmm_clients`/`list_ninja_organizations`), wird aber vom
Frontend nicht aufgerufen -- `KaseyaPluginSection.tsx` ist wie
`TacticalRmmPluginSection.tsx` konsequent Cache-first
(`get_cached_kaseya_sync` beim Öffnen, `sync_kaseya_connection` nur auf
"Aktualisieren"); der Befehl bleibt für Symmetrie und einen möglichen
künftigen Ersteinrichtungs-Anwendungsfall erhalten. Das Übernehmen eines
extern gelieferten Werts in ein selbst gepflegtes Feld (`name`, `hostname`,
`ip_address`, `notes` in `systems`) bleibt -- wie bei jedem anderen Plugin
-- ausschließlich eine bewusste, manuelle Aktion über
`get_kaseya_system_details` plus eine spätere UI-Aktion; kein Kommando hier
schreibt automatisch in diese vier Felder.

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Kaseyas bestätigtes Geräte-Schema trägt kein hostname-, asset-tag- oder
serial-artiges Feld -- nur `Name` (siehe oben). `KaseyaPluginSection.tsx`s
`matchKeyForDevice` verwendet deshalb, wie Snipe-IT es mit seiner eigenen
asset_tag/serial-Rückfallkette tut, den bestverfügbaren Kandidaten
(`device.name`) als Abgleichsschlüssel für den "Mit bestehendem System
verknüpfen"-Vorschlag, verglichen gegen das einzige freie Textfeld, das ein
lokales System dafür hat -- `System.hostname`. Das ist eine bewusste
Design-Entscheidung, kein bestätigter Kaseya-API-Fakt: `Name` gleicht in
der RMM-Praxis oft dem tatsächlichen Hostnamen des Agenten, garantiert ist
das für VSA X aber nicht.

### Frontend (`KaseyaPluginSection.tsx`)

Mechanisch an `TacticalRmmPluginSection.tsx`s Stand angelehnt
(Organisationen eingeklappt mit Geräte-Anzahl-Zusammenfassung,
Kunde-Zuordnungs-`<select>` inklusive "+ Neuen Kunden anlegen…",
aufklappbare, filterbare, 10-pro-Seite-paginierte Geräteliste mit
`j`/`k`/`Enter`/`l`/`u`-Tastaturnavigation, Vergleichs-/Übernahme-Panel für
verknüpfte Geräte). Der Verbindungs-Anlage-Dialog hat vier Felder (Label,
Base-URL inklusive des oben beschriebenen Ambiguitäts-Hinweistexts,
Token-ID, Token-Secret als `type="password"`) -- ein Zwei-Werte-Credential
wie bei NinjaOne, anders als Tactical RMMs einzelnem API-Key. Die "Nicht
zugeordnet"-Gruppe (siehe oben) braucht keinen eigenen Rendering-Zweig --
sie kommt vom Backend bereits als ganz normale
`KaseyaOrgDeviceGroupDto` und durchläuft exakt denselben Gruppen-
Rendering-Code wie jede echte Organization. `DeviceSummaryLine` zeigt
anstelle von IP/Status/Plattform (die Kaseya X nicht bestätigt, siehe oben)
die rohe Gruppen-ID (falls vorhanden) sowie zwei kleine Ja/Nein-Badges für
`is_agent_installed`/`is_mdm_enrolled`. Wie bei `TacticalRmmPluginSection.tsx`
gibt es bewusst KEINEN "In X öffnen"-Link auf einer Geräte-Zeile (siehe
oben, kein verifizierter Weboberflächen-Link). `PluginsView.tsx` bindet die
Sektion alphabetisch zwischen `JamfPluginSection.tsx` und
`LevelPluginSection.tsx` ein.

## Action1-Plugin (`plugin::action1`) -- zwölfte echte Integration

`plugin/action1.rs` implementiert `Plugin` für Action1s REST-API über HTTPS.
Action1 (<https://www.action1.com>) ist ein cloud gehostetes,
patch management fokussiertes RMM Tool (Remote Monitoring & Management). Bei
der Authentifizierung ist es strukturell am nächsten an Microsoft Intune:
ein OAuth2-Grant im Client-Credentials-Stil gegen einen Token-Endpunkt
(Client-ID und Client-Secret werden gegen einen Bearer-Token eingetauscht).
Bei der Mehrfach-Mandantenfähigkeit ist es dagegen am nächsten an
NinjaOne/Tactical RMM: eine Verbindung sieht mehrere "Organizations", jede
wird einzeln einem lokalen Kunden zugeordnet.

Ein Hinweis vorab: Action1s eigene Quellen widersprechen sich an einer
Stelle, und diese Integration löst diesen Widerspruch bewusst nicht heimlich
auf, sondern trifft eine dokumentierte, ausdrücklich als ungeprüft
markierte Entscheidung (siehe "Authentifizierung" unten).

Jede Angabe unten stammt aus Action1s eigener Prosa-Dokumentation und seiner
interaktiven OpenAPI 3.1/Swagger-Spezifikation:

- **Authentifizierung, dokumentierter, ungeklärter Widerspruch**:
  `POST {base_url}/oauth2/token`. Action1s Prosa-Dokumentation zeigt
  `Content-Type: application/x-www-form-urlencoded` mit dem Body
  `client_id=...&client_secret=...`. Die Swagger-Spezifikation zeigt
  stattdessen `Content-Type: application/json` mit dem Body
  `{"client_id": "...", "client_secret": "..."}`. Keine der beiden Quellen
  nennt irgendwo ein Feld `grant_type` (anders als bei NinjaOne/Intune, wo
  `grant_type=client_credentials` verifiziert ist). Diese Integration
  implementiert die JSON-Variante, denn die interaktive Swagger-Spezifikation
  bildet vermutlich eher den tatsächlich akzeptierten Vertrag des
  Live-Servers ab als statische Prosa. Diese Entscheidung ist ausdrücklich
  ungeprüft gegen einen echten Action1-Aufruf. Ein eigener Test
  (`token_request_body_is_json_with_client_id_and_client_secret_only` in
  `plugin::action1`) legt die genaue JSON-Form fest, damit eine spätere
  Korrektur, falls sich die Prosa-Variante doch als richtig erweist, ein
  Ein-Zeilen-Diff bleibt statt einer erneuten Recherche. Die Antwort (beide
  Quellen stimmen hier überein): `{"access_token": "<JWT>", "refresh_token":
  "...", "expires_in": 3600, "token_type": "bearer"}`, verwendet als
  `Authorization: Bearer <access_token>`. Client-ID und Client-Secret werden
  vom Nutzer in der Action1-Konsole erzeugt (Configuration, Users and API
  Credentials); die dort zugewiesene Rolle steuert den Zugriffsumfang, ein
  separater `scope`-Parameter existiert nicht.
- **Basis-URL**: vier feste, regionale Hosts, jeweils bereits mit
  `/api/3.0` im Pfad: `https://app.action1.com/api/3.0` (Nordamerika),
  `https://app.na-2.action1.com/api/3.0` (Nordamerika 2),
  `https://app.eu.action1.com/api/3.0` (Europa) und
  `https://app.au.action1.com/api/3.0` (Australien). Wie bei
  NinjaOne/Snipe-IT/Tactical RMM gibt der Nutzer dies selbst an
  (`Action1ConnectionMeta.base_url`). Das Frontend bietet die vier Optionen
  als Auswahlliste an, das Feld selbst bleibt ein einfacher String.
- **Organisationen (Mehrfach-Mandantenfähigkeit)**: `GET {base_url}/organizations`,
  verpackt in Action1s generischem `ResultPage`-Umschlag, der in der
  gesamten API verwendet wird: `{"id","type":"ResultPage","name","self",
  "items":[{"id","type":"Organization","name","description","self",
  "access"}],"total_items","limit","next_page","prev_page"}`. Die genaue
  Form der Organisations-ID ist über einen Platzhalterwert hinaus nicht
  bestätigt. Sie wird deshalb defensiv behandelt (String oder Zahl werden
  beide akzeptiert), nicht als garantiert numerisch angenommen.
- **Endpunkte (Geräte)**: `GET {base_url}/endpoints/managed/{orgId}`,
  derselbe `ResultPage`-Umschlag. Wichtig: dies ist ein Aufruf PRO
  ORGANISATION, anders als NinjaOnes `/v2/devices`, das alle Geräte des
  gesamten Tenants in einem einzigen Aufruf listet, unabhängig von der
  Organisation. Es gibt keinen dokumentierten Aufruf für "alle Endpunkte des
  Kontos". Das vollständige Geräteverzeichnis einer Verbindung zu holen
  erfordert deshalb genau einen `/endpoints/managed/{orgId}` Durchlauf pro
  Organisation (siehe unten, Rate-Limit). Verifizierte, verwendete Felder:
  `id` (als `external_id`), `device_name` (der echte Hostname, bevorzugt),
  `name` (ein separates, vom Nutzer editierbares Anzeige-Label, nur als
  Rückfallebene verwendet), `address` (IP), `platform` und `status` (Enum
  `Connected`/`Disconnected`/`Pending Uninstall`, das echte
  Konnektivitätsfeld, als freier String durchgereicht). Entscheidend:
  `organization_id`, ein echter UUID-String-Fremdschlüssel direkt am
  Endpunkt-Objekt. Anders als Tactical RMM, dessen Agentenliste nur einen
  `client_name`-String trägt und einen Namens-basierten Join erzwingt, lässt
  sich ein Action1-Endpunkt über eine echte ID zuordnen.
  `commands::action1::group_endpoints_by_organization` ist entsprechend als
  ID-Join geschrieben, nicht als Namens-Join.
- **`online_status` ist KEINE Konnektivität, nicht mit `status`
  verwechseln**: `online_status` ist ein separates Enum
  (`SUCCESS`/`WARNING`/`ERROR`), ein Zustands- bzw. Gesundheitsflag der
  Überwachung selbst, nicht "ist dieser Endpunkt online". Dieses Feld wird
  in der gesamten Integration bewusst nirgends gelesen. `status` ist das
  einzige Feld, das als Konnektivität angezeigt wird.
- **Pagination**: Query-Parameter `limit` (Seitengröße) und `from` (Offset
  des ersten Datensatzes) beim ersten Aufruf. `next_page`/`prev_page` im
  `ResultPage`-Umschlag sind bereits fertige, RELATIVE URLs, die den
  nächsten `from`/`limit`-Wert schon enthalten (Beispiel aus Action1s
  eigener Dokumentation: `/API/endpoints/managed?from=60&limit=10`). Eine
  Fortsetzung berechnet deshalb nie selbst `from`/`limit`, sondern folgt nur
  `next_page`. Das Verhalten auf dem letzten Blatt (null, fehlendes Feld
  oder leerer String) ist nicht ausdrücklich bestätigt. Ein leerer String
  wird deshalb defensiv genauso wie ein fehlendes Feld behandelt, "keine
  weitere Seite", um eine Endlosschleife zu vermeiden. `MAX_PAGES` begrenzt
  den Durchlauf zusätzlich, dieselbe defensive Konvention wie bei
  `plugin::intune`/`plugin::level`.
- **Einzelgerät-Detail**: `GET {base_url}/endpoints/managed/{orgId}/{endpointId}`,
  auch direkt über das Feld `self` jedes Listeneintrags gegeben, roher
  JSON-Durchgriff wie bei jedem anderen Plugin. Wichtig: dieser Aufruf
  braucht BEIDE IDs (Organisation und Endpunkt) im URL-Pfad, anders als
  Tactical RMMs `/agents/{agent_id}/`, wo eine einzelne ID genügt. Der
  generische `Plugin`-Trait (`get_system_details`/`link_system`) trägt nur
  eine einzelne `external_id`. Der empfohlene, tatsächlich verwendete Weg
  ist deshalb die eigene Methode
  `Action1Plugin::get_endpoint_details(credentials, organization_id,
  endpoint_id)`, die `commands::action1` direkt aufruft (dort ist
  `organization_id` aus der Organisations-/Geräte-Gruppierung ohnehin schon
  bekannt). Die Trait-Methode `get_system_details` existiert trotzdem, zur
  Schnittstellen-Konformität, implementiert über eine dokumentierte
  Kompakt-Kodierung `"<organisation_id>:<endpunkt_id>"` von `external_id`
  (siehe `split_compound_external_id`); `commands::action1` verlässt sich
  darauf nie.
- **Rate-Limit, spürbar enger als bei den anderen RMM-Plugins dieser
  Codebasis**: Action1 empfiehlt, unter 30 Anfragen pro Minute je
  Enterprise-Konto zu bleiben, über alle Endpunkte der API hinweg gezählt
  (zum Vergleich: Tactical RMM/NinjaOne haben in dieser Codebasis keine
  dokumentierte Grenze; Datto RMM liegt laut Recherche bei 600/60s,
  Pulseway bei rund 3600/Stunde). Eine Überschreitung liefert `429` mit
  `{"status":429,"details":{"retry_after":<Sekunden>}}`. Das ist hier
  konkret relevant, weil `sync_action1_connection` (siehe
  `commands::action1`) wegen der Endpunkte-Liste PRO Organisation (siehe
  oben) für eine reguläre Synchronisierung schon ungefähr "1 (Organisations-
  Aufruf) + 1 Endpunkte-Aufruf pro Organisation" braucht, bevor Pagination
  oder Detail-Aktualisierungen bereits verknüpfter Geräte überhaupt
  mitgezählt sind. Für ein Konto mit vielen Organisationen summiert sich das
  gegen ein Budget von 30 pro Minute schnell auf. Das bestehende Muster
  "vollständiger Abruf pro Synchronisierung, manuell ausgelöst" (siehe
  `plugin::tacticalrmm`/`plugin::ninja`) bleibt auch hier bestehen, es passt
  für die realistischen Flottengrößen eines einzelnen Administrators, den
  diese Anwendung anspricht. Diese Grenze wird trotzdem ausdrücklich
  benannt, damit eine künftige Änderung nicht ungesehen noch mehr Aufrufe
  pro Synchronisierung hinzufügt (etwa eine Live-Detail-Aktualisierung für
  jedes gelistete Gerät statt nur für bereits verknüpfte).
  `Action1Plugin::list_organizations_with_endpoints` holt deshalb schon
  jetzt genau EIN OAuth2-Token pro Synchronisierung, nicht eines pro
  Organisation.
- **Kein verifizierter Weboberflächen-Link**: nirgends erreichbar
  dokumentiert, deshalb bewusst weggelassen, dasselbe ehrliche
  Auslassungsprinzip wie bei Tactical RMMs fehlendem `tacticalrmm_url`.
- **HTTP-Client**: `ureq`, synchron, dieselbe Abhängigkeit wie jedes andere
  Plugin-Modul dieser Codebasis. Keine neue Cargo-Abhängigkeit nötig.
- **Zugangsdaten-Kodierung**: Action1 braucht zwei Geheimwerte (`client_id`,
  `client_secret`), als JSON in `PluginCredentials.secret` kodiert
  (`serde_json::to_string`/`from_str`), dasselbe Zwei-Werte-Muster wie bei
  `plugin::ninja::NinjaCredentials`.

### Action1-Verbindungen, jede mit mehreren Organisationen

Strukturell identisch zu NinjaOnes/Tactical RMMs Verbindungs- bzw.
Organisations-/Client-Modell:

- Nicht-geheime Metadaten (`id`, `label`, `base_url`, bewusst OHNE
  `customer_id`) liegen als `Action1ConnectionMeta` in
  `Config::action1_connections` (`#[serde(default)]`-kompatibel).
  Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"action1:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei jedem anderen Plugin hier.
- Welche Organisation innerhalb einer Verbindung welchem lokalen Kunden
  entspricht (falls überhaupt), steht granular in
  `Config::action1_org_mappings` (`Action1OrgMapping { connection_id,
  organization_id, organization_name, customer_id }`). Eine nicht
  zugeordnete Organisation liefert bei jeder Synchronisierung ihre
  Endpunkte weiterhin (zur Ansicht), aber immer mit
  `linked_system_id: None`.
- `commands::action1::group_endpoints_by_organization` gruppiert Endpunkte
  nach Organisation, strukturell analog zu
  `commands::plugins::group_devices_by_organization` (NinjaOne). Der
  entscheidende Unterschied zu `commands::tacticalrmm::group_agents_by_client`:
  hier läuft der Join echt über eine ID (`endpoint.organization_id`), nicht
  über einen Namen (siehe oben). Eine Organisation ohne Endpunkte erscheint
  trotzdem als leere Gruppe, damit sie in der Oberfläche zur Zuordnung
  angeboten werden kann. Endpunkte, deren `organization_id` zu keiner
  bekannten Organisation passt (sollte normalerweise nicht vorkommen, ist
  aber nicht ausgeschlossen, etwa eine zwischen Organisations- und
  Endpunkt-Abruf im selben Synchronisierungslauf gelöschte Organisation),
  werden nicht stillschweigend verworfen, sondern als eigene Restgruppe
  angehängt, mit der rohen ID als synthetischer ID UND, mangels besserem
  Namen, auch als Anzeigename.

### Zwischenspeicher für Offline-Ansicht

Dieselbe Konvention wie bei jedem anderen Plugin hier:
`sync_action1_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/action1-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`). `get_cached_action1_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff), rejoint dabei aber
`customer_id` je Gruppe live gegen die AKTUELLEN
`Config::action1_org_mappings` (nicht den beim letzten Sync eingefrorenen
Wert), genau wie `commands::tacticalrmm::get_cached_tacticalrmm_sync`, und
liefert `None`, wenn für eine Verbindung noch nie synchronisiert wurde.
`remove_action1_connection` löscht diese Cache-Datei (bestes Bemühen) und
alle `action1_org_mappings`-Zeilen der entfernten Verbindung gleich mit.

### Tauri-Kommandos (`commands::action1`)

`test_action1_connection`, `list_action1_connections`,
`add_action1_connection`, `remove_action1_connection`,
`list_action1_organizations`, `map_action1_organization`,
`unmap_action1_organization`, `sync_action1_connection`,
`get_cached_action1_sync`, `link_system_to_action1`,
`unlink_system_from_action1`, `get_action1_system_details`, dünne Wrapper
nach dem Muster von `commands::tacticalrmm`. Eine dokumentierte, notwendige
Abweichung vom sonst identischen Kommando-Zuschnitt: `link_system_to_action1`
und `get_action1_system_details` nehmen zusätzlich `organization_id`
entgegen, weil Action1s Einzelgerät-Detail-Aufruf beide IDs im URL-Pfad
braucht (siehe oben) und die Oberfläche `group.organization_id` an dieser
Stelle ohnehin bereits kennt; `unlink_system_from_action1` bleibt dagegen
identisch zu `unlink_system_from_tacticalrmm` (rein lokale Löschung, kein
API-Aufruf). `list_action1_organizations` liefert die Live-Organisationsliste
einer Verbindung (analog zu `list_ninja_organizations`/
`list_tacticalrmm_clients`), wird aber vom Frontend nicht aufgerufen,
`Action1PluginSection.tsx` ist konsequent Cache-first
(`get_cached_action1_sync` beim Öffnen, `sync_action1_connection` nur auf
"Aktualisieren"); der Befehl bleibt für Symmetrie und einen möglichen
künftigen Ersteinrichtungs-Anwendungsfall erhalten. Das Übernehmen eines
extern gelieferten Werts in ein selbst gepflegtes Feld (`name`, `hostname`,
`ip_address`, `notes` in `systems`) bleibt, wie bei jedem anderen Plugin
hier, ausschließlich eine bewusste, manuelle Aktion über
`get_action1_system_details` plus eine spätere UI-Aktion; kein Kommando
hier schreibt automatisch in diese vier Felder.

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Ein Action1-Endpunkt hat kein eigenes, separates `hostname`-Feld im
Frontend-DTO (siehe `plugin::action1`-Moduldokumentation): `name` bevorzugt
bereits den echten `device_name` gegenüber dem separaten, vom Nutzer
editierbaren `name`-Label. `Action1PluginSection.tsx`s `matchKeyForDevice`
verwendet deshalb `device.name` als Abgleichsschlüssel für den "Mit
bestehendem System verknüpfen"-Vorschlag, verglichen gegen das einzige
freie Textfeld, das ein lokales System dafür hat, `System.hostname`.

### Frontend (`Action1PluginSection.tsx`)

Mechanisch an `TacticalRmmPluginSection.tsx`s Stand angelehnt (Organisationen
eingeklappt mit Geräte-Anzahl-Zusammenfassung, Kunde-Zuordnungs-`<select>`
inklusive "+ Neuen Kunden anlegen…", aufklappbare, filterbare,
10-pro-Seite-paginierte Geräteliste mit `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation, Vergleichs-/Übernahme-Panel für verknüpfte Geräte). Der
Verbindungs-Anlage-Dialog hat vier Felder (Label, Region als
`<select>` über die vier festen Hosts, Client-ID, Client-Secret als
`type="password"`), ein Client-ID/-Secret-Paar wie bei NinjaOne, nur mit
Region statt freier Base-URL. `DeviceSummaryLine` zeigt zusätzlich einen
Verbunden/Getrennt/Deinstallation-ausstehend-Statuspunkt und die Plattform,
Anzeige-Kontext ohne eigenes Site-Konzept (Action1 kennt, anders als
Tactical RMM, keine Unterebene zwischen Organisation und Endpunkt). Wie bei
`TacticalRmmPluginSection.tsx` gibt es bewusst KEINEN "In Action1
öffnen"-Link auf einer Geräte-Zeile (siehe oben, kein verifizierter
Weboberflächen-Link). `PluginsView.tsx` bindet die Sektion alphabetisch vor
`AbmPluginSection.tsx` ein ("Action1" kommt vor "Apple"), als erste Karte
insgesamt.

## Datto-RMM-Plugin (`plugin::dattormm`) -- dreizehnte echte Integration

`plugin/dattormm.rs` implementiert `Plugin` für Datto RMMs REST-API über
HTTPS. Datto RMM (<https://www.datto.com/product/rmm/>, Teil von Kaseya) ist
ein cloud gehostetes, auf sechs Regionen ("Pods") verteiltes RMM-Tool
(Remote Monitoring & Management) -- strukturell am nächsten an Jamf Pro:
eine echte "Site"-Zuordnungsebene mit einem echten String-Fremdschlüssel
direkt am Gerät (`siteUid`, passend zu `Site.uid`), anders als Tactical RMM,
das Agenten mangels Client-ID am Agenten über den NAMEN einem Client
zuordnen muss (siehe `plugin::tacticalrmm`-Moduldokumentation). Datto RMMs
Hierarchie ist dabei flacher als Tactical RMMs Client -> Site -> Agent: nur
Site -> Device, keine übergeordnete Client-Ebene -- die Zuordnung erfolgt
deshalb direkt auf Site-Ebene, analog zu Jamf Pros eigener Site-Ebene.
Konfigurierbare `base_url` wie NinjaOne/Snipe-IT/Tactical RMM/Jamf Pro, weil
Datto RMM auf sechs unabhängige regionale API-Hosts verteilt ist, ohne
Möglichkeit, einen davon aus den anderen abzuleiten.

Jede Angabe unten stammt aus Datto RMMs eigener offizieller
Hilfe-Dokumentation und einer live abgerufenen OpenAPI-3.1-Spezifikation für
dessen öffentliche REST-API -- nicht aus Drittanbieter-Prosa geraten:

- **Authentifizierung**: ein OAuth2-ARTIGER Token-Austausch, aber KEIN
  generischer OAuth2-Client-Credentials-Grant wie bei
  `plugin::ninja`/`plugin::jamf`, und auch KEIN reiner
  Mandant-plus-Geheimnis-Austausch wie bei `plugin::intune` --
  `POST {base_url}/auth/oauth/token`, HTTP-Basic-Auth mit Dattos eigenem
  FESTEN, öffentlich dokumentierten OAuth-Client (`OAUTH_CLIENT_ID` =
  `"public-client"`, `OAUTH_CLIENT_SECRET` = `"public"` -- NICHT etwas, das
  ein Nutzer selbst erzeugt, dasselbe Konstanten-Paar für jeden
  Datto-RMM-Kunden) als Basic-Auth-Benutzername/-Passwort, Body
  `application/x-www-form-urlencoded` mit
  `grant_type=password&username=<API Key>&password=<API Secret Key>` -- das
  eigene API-Key/API-Secret-Key-Paar des Nutzers (erzeugt in Datto RMMs
  eigener Weboberfläche: Setup -> Users -> eigenen Nutzer wählen -> Generate
  API Keys) geht als `username`/`password` ein, bewusst NICHT als
  `client_id`/`client_secret` -- ein leicht zu machender Fehler, wenn man
  `plugin::intune`s OAuth2-Client-Credentials-Grant zu wörtlich als Vorlage
  nimmt (siehe `oauth_token_form`, das genau deshalb einen eigenen Unit-Test
  hat). Antwort 200 + JSON `{"access_token": "<JWT>", ...}` bei Erfolg, 400
  bei einem falschen Schlüsselpaar. Verwendet als
  `Authorization: Bearer <access_token>` bei jedem folgenden Aufruf. Tokens
  laufen nach 100 Stunden ab -- wie bei jedem anderen Plugin hier (siehe
  `plugin::intune`-Moduldokumentation) keine Refresh-Token-Behandlung: ein
  frischer Token wird pro Synchronisierung geholt.
- **Basis-URL -- mehrere Pods**: Datto RMM läuft über sechs regionale Pods
  (Pinotage, Merlot, Concord, Vidal, Zinfandel, Syrah), jeder mit eigenem
  API-Hostnamen (z. B. `https://merlot-api.centrastage.net`). Ein Nutzer
  findet die eigene, genaue Pod-API-URL auf der eigenen Datto-RMM-Nutzerseite
  (befüllt, nachdem API-Keys erzeugt wurden) -- es gibt keine Möglichkeit,
  sie automatisch abzuleiten, daher ist `DattoRmmConnectionMeta.base_url`
  ein einfacher, nutzerseitig angegebener `String`, wie bei
  `TacticalRmmConnectionMeta`/`JamfConnectionMeta`, NICHT eine feste
  Konstante wie bei `plugin::intune`s `GRAPH_BASE_URL`/`plugin::level`s
  `BASE_URL`. Die API-Version `v2` sitzt im Pfad (`{base_url}/api/v2/...`).
- **Mandantenfähigkeits-Einheit -- "Site"**:
  `GET {base_url}/api/v2/account/sites` (Query-Parameter `page`, `max`,
  `siteName`). Die Antwort ist ein umschlossenes Envelope
  `{"pageDetails": {...}, "sites": [...]}` (anders als Tactical RMMs
  nackter, unpaginierter Array). `uid` (ein String) ist der überall sonst
  verwendete echte Bezeichner -- hier als Zuordnungsschlüssel verwendet,
  NICHT die numerische `id`. Die Zuordnung erfolgt genau auf dieser Ebene --
  Datto RMMs flache Zuordnungsebene, einfacher als Tactical RMMs
  Client -> Site-Hierarchie, es gibt keine höhere Ebene zu beachten.
- **Geräte**: `GET {base_url}/api/v2/account/devices` (alle Geräte,
  kontoweit -- hier gegenüber dem Site-beschränkten
  `GET {base_url}/api/v2/site/{siteUid}/devices` für einen einzigen
  Synchronisierungslauf bevorzugt, mit clientseitiger Gruppierung über die
  direkt am Gerät vorhandenen Felder `siteUid`/`siteName`). Antwort:
  `{"pageDetails": {...}, "devices": [...]}`. `uid` (String) ist der echte
  Bezeichner, verwendet als `external_id`/`DattoRmmDevice::external_id`; die
  numerische `id` wird nicht verwendet, dieselbe Konvention wie bei `Site`.
  `intIpAddress`/`extIpAddress` sind beide reine Strings --
  `extract_ip_address` bevorzugt `intIpAddress`, weicht auf `extIpAddress`
  aus, dasselbe "primär + Rückfallebene"-Prinzip wie bei
  `plugin::tacticalrmm::extract_ip_address`/`plugin::ninja`, nur an Datto
  RMMs Feldnamen angepasst. `online` ist ein echter JSON-Boolean (kein
  String-Enum wie Tactical RMMs `status`) -- hier zu `"online"`/`"offline"`-
  Strings konvertiert, für Anzeige-Konsistenz mit jedem anderen Plugin
  dieser Codebasis (eine bewusste Wahl, keine feste API-Vorgabe -- Dattos
  eigenes Feld ist wirklich ein Boolean). `deviceClass`
  (`"device"`/`"printer"`/`"esxihost"`/`"rmmnetworkdevice"`/`"unknown"`)
  wird unverändert durchgereicht, ein freier String wie Tactical RMMs
  `plat` -- kein Rust-Enum, damit ein künftiger zusätzlicher Wert das Parsen
  nicht bricht. `siteUid` (String) ist ein ECHTER Fremdschlüssel direkt am
  Gerät, passend zu `Site.uid` 1:1 -- anders als bei Tactical RMM ist hier
  kein Namens-basierter Behelf nötig (siehe
  `commands::dattormm::group_devices_by_site`, das deshalb ID-basiert ist,
  analog zu `commands::plugins::group_devices_by_organization`/
  `commands::jamf::group_devices_by_site`, NICHT wie
  `commands::tacticalrmm::group_agents_by_client`s Namens-basierte
  Zuordnung). `portalUrl` ist ein echter, bestätigter
  Web-Dashboard-Tiefenlink, vorhanden sowohl bei `Site` als auch bei
  `Device` -- anders als Tactical RMMs verifiziertes FEHLEN eines solchen
  (siehe `plugin::tacticalrmm`-Moduldokumentation) liefert Datto RMM
  tatsächlich einen pro Site/Gerät, hier als `Option<String>` sowohl an
  `DattoRmmSite` als auch `DattoRmmDevice` bereitgestellt und in
  `DattoRmmPluginSection.tsx` dargestellt.
- **Zusätzliches Feld über die im Ausgangsauftrag wörtlich genannte
  `DattoRmmDevice`-Form hinaus**: `hostname: Option<String>` wird getrennt
  von `name` gehalten (das auf `external_id` zurückfällt, wenn `hostname`
  fehlt), analog zu `plugin::tacticalrmm::TacticalRmmAgent`s
  `hostname`/`name`-Trennung -- ohne dieses Feld würde
  `DattoRmmPluginSection.tsx`s Hostname-Abgleichs-Heuristik für "Mit
  bestehendem System verknüpfen" riskieren, den Hostnamen eines lokalen
  Systems gegen eine Datto-RMM-Geräte-UID zu vergleichen, sobald ein Gerät
  einmal keinen eigenen `hostname` hat.
- **Paginierung**: Query-Parameter `page` (0-indiziert) und `max` (gedeckelt
  bei 250/Seite -- `PAGE_SIZE` nutzt das Maximum). Jede Listen-Antwort trägt
  `pageDetails: {count, totalCount, prevPageUrl, nextPageUrl}`;
  `nextPageUrl` ist `null` auf der letzten Seite -- `fetch_all_pages`
  läuft, solange es nicht null ist, wobei GENAU diese URL für die nächste
  Anfrage verwendet wird (statt `page`/`max` selbst neu abzuleiten),
  dasselbe "der vom Server gelieferten Fortsetzungs-URL folgen"-Prinzip wie
  bei `plugin::intune`s `@odata.nextLink`-Durchlauf, nur unter einem anderen
  JSON-Schlüssel. Gedeckelt bei `MAX_PAGES` Seiten, Schutz gegen ein
  fehlerhaftes Gegenüber, dieselbe defensive Konvention wie bei
  `plugin::intune::MAX_PAGES`/`plugin::level::MAX_PAGES`.
- **Einzelgeräte-Detail**: `GET {base_url}/api/v2/device/{deviceUid}` (über
  den String `uid`) -- verwendet für `Plugin::get_system_details`, rohes
  JSON unverändert durchgereicht, dieselbe Konvention wie bei jedem anderen
  Plugin hier.
- **Rate-Limits**: 600 Lese-Anfragen/60s, 100 Schreib-Anfragen/60s
  (kontoweit, gleitendes Fenster); 429 nahe am Limit, 403 + eine temporäre
  IP-Sperre bei anhaltendem Verstoß. Keine Retry-/Backoff-Logik hier --
  entspricht jedem anderen Plugin dieser Codebasis.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie jedes
  andere Plugin hier.
- **Zugangsdaten-Kodierung**: Datto RMM braucht zwei Geheimwerte (API Key,
  API Secret Key) -- `PluginCredentials.secret` ist, laut Trait-Vertrag,
  ein einzelner opaker String, den das Plugin selbst interpretiert -- hier
  als JSON kodiert (`serde_json::to_string`/`from_str`), genau wie
  `plugin::ninja::NinjaCredentials`, nur mit anderen Feldnamen
  (`api_key`/`api_secret_key` statt `client_id`/`client_secret`, weil das
  tatsächlich das ist, was sie sind -- sie werden im OAuth-Anfragekörper als
  `username`/`password` verwendet, NICHT als `client_id`/`client_secret`,
  siehe den Authentifizierungs-Punkt oben).

### Datto-RMM-Verbindungen, jede mit mehreren Sites

Strukturell am nächsten an Jamf Pros Verbindungs-/Site-Modell, nicht an
Tactical RMMs zweistufigem Client -> Site-Modell:

- Nicht-geheime Metadaten (`id`, `label`, `base_url`, bewusst OHNE
  `customer_id`) liegen als `DattoRmmConnectionMeta` in
  `Config::dattormm_connections` (`#[serde(default)]`-kompatibel).
  Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"dattormm:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei jedem anderen Plugin hier.
- Welche Site innerhalb einer Verbindung welchem lokalen Kunden entspricht
  (falls überhaupt), steht granular in `Config::dattormm_site_mappings`
  (`DattoRmmSiteMapping { connection_id, site_uid, site_name,
  customer_id }`). Eine nicht zugeordnete Site liefert bei jeder
  Synchronisierung ihre Geräte weiterhin (zur Ansicht), aber immer mit
  `linked_system_id: None`.
- `commands::dattormm::group_devices_by_site` gruppiert Geräte nach Site --
  strukturell analog zu `commands::plugins::group_devices_by_organization`/
  `commands::jamf::group_devices_by_site`, über den ECHTEN `site_uid`-
  Fremdschlüssel (siehe oben), NICHT über einen Namens-Abgleich wie bei
  `commands::tacticalrmm::group_agents_by_client`. Eine Site ohne Geräte
  erscheint trotzdem als Gruppe (leere `devices`-Liste), damit eine
  künftige UI sie zur Zuordnung anzeigen kann. Geräte, deren `site_uid` zu
  keiner bekannten Site passt (sollte normalerweise nicht vorkommen, ist
  aber nicht ausgeschlossen -- z. B. eine zwischen Site- und Geräte-Abruf im
  selben Sync-Lauf gelöschte Site), werden nicht stillschweigend
  verworfen, sondern als eigene Restgruppe angehängt, mit der rohen
  `site_uid` als synthetischer ID UND (mangels echter Site-Daten)
  Anzeigename, sowie `portal_url: None` -- es gibt für diese Gruppe keine
  ehrlich verlinkbare Site.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie bei jedem anderen Plugin hier:
`sync_dattormm_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/dattormm-<connection_id>.json`
(`{"synced_at_utc": "...", "groups": [...]}`). `get_cached_dattormm_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff), rejoint dabei aber
`customer_id` je Gruppe live gegen die AKTUELLEN
`Config::dattormm_site_mappings` (nicht den beim letzten Sync eingefrorenen
Wert) -- exakt wie `commands::plugins::get_cached_ninja_sync` --, und
liefert `None`, wenn für eine Verbindung noch nie synchronisiert wurde.
`remove_dattormm_connection` löscht diese Cache-Datei (bestes Bemühen) und
alle `dattormm_site_mappings`-Zeilen der entfernten Verbindung gleich mit.

### Tauri-Kommandos (`commands::dattormm`)

`test_dattormm_connection`, `list_dattormm_connections`,
`add_dattormm_connection`, `remove_dattormm_connection`,
`list_dattormm_sites`, `map_dattormm_site`, `unmap_dattormm_site`,
`sync_dattormm_connection`, `get_cached_dattormm_sync`,
`link_system_to_dattormm`, `unlink_system_from_dattormm`,
`get_dattormm_system_details` -- dünne Wrapper nach dem Muster von
`commands::jamf`. `list_dattormm_sites` liefert die Live-Site-Liste einer
Verbindung (analog zu `list_jamf_sites`/`list_tacticalrmm_clients`), wird
aber vom Frontend nicht aufgerufen -- `DattoRmmPluginSection.tsx` ist wie
jede andere Plugin-Sektion hier konsequent Cache-first
(`get_cached_dattormm_sync` beim Öffnen, `sync_dattormm_connection` nur auf
"Aktualisieren"); der Befehl bleibt für Symmetrie und einen möglichen
künftigen Ersteinrichtungs-Anwendungsfall erhalten. Das Übernehmen eines
extern gelieferten Werts in ein selbst gepflegtes Feld (`name`, `hostname`,
`ip_address`, `notes` in `systems`) bleibt -- wie bei jedem anderen Plugin
hier -- ausschließlich eine bewusste, manuelle Aktion über
`get_dattormm_system_details` plus eine spätere UI-Aktion; kein Kommando
hier schreibt automatisch in diese vier Felder.

### Verknüpfungs-Vorschlag: welches Feld als Abgleichsschlüssel

Wie Tactical RMM (und anders als Snipe-IT, das mangels Hostname-Feld auf
`asset_tag`/`serial` ausweichen muss) hat ein Datto-RMM-Gerät ein echtes
`hostname`-Feld -- Datto RMM ist RMM-Überwachungssoftware, keine
Asset-/Inventarverwaltung. `DattoRmmPluginSection.tsx`s
`matchKeyForDevice` verwendet deshalb direkt `device.hostname` als
Abgleichsschlüssel für den "Mit bestehendem System verknüpfen"-Vorschlag,
verglichen gegen das einzige freie Textfeld, das ein lokales System dafür
hat -- `System.hostname`.

### Frontend (`DattoRmmPluginSection.tsx`)

Mechanisch an `TacticalRmmPluginSection.tsx`s Stand angelehnt (Sites
eingeklappt mit Geräte-Anzahl-Zusammenfassung, Kunde-Zuordnungs-`<select>`
inklusive "+ Neuen Kunden anlegen…", aufklappbare, filterbare,
10-pro-Seite-paginierte Geräteliste mit `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation, Vergleichs-/Übernahme-Panel für verknüpfte Geräte),
aber mit EINER Zuordnungsebene (Site) statt Tactical RMMs zwei (Client,
mit Site nur als Anzeige-Kontext) -- jedes "Client"-Konzept aus der
Tactical-RMM-Vorlage wird hier zu einem "Site"-Konzept. Der
Verbindungs-Anlage-Dialog hat VIER Felder (Label, Base-URL, API-Key UND
API-Secret-Key, beide als `type="password"`) -- ein Feld mehr als bei
Tactical RMM, weil Datto RMMs Token-Austausch zwei Geheimwerte statt einem
braucht (siehe oben). Der Base-URL-Hinweistext erklärt ausdrücklich, dass
es sich um die Pod-spezifische URL handelt, die auf der eigenen
Datto-RMM-Nutzerseite zu finden ist. `DeviceSummaryLine` zeigt zusätzlich
einen Online/Offline-Statuspunkt und die Plattform (`deviceClass`,
z. B. "Gerät"/"Drucker"/"ESXi-Host") -- KEIN "Überfällig"-Status, anders
als Tactical RMM (siehe oben, Datto RMM kennt nur zwei Statuswerte). Anders
als `TacticalRmmPluginSection.tsx` (das mangels verifiziertem
Weboberflächen-Link bewusst KEINEN "In X öffnen"-Link zeigt) gibt es hier
`DattoRmmLink`, gerendert sowohl auf jeder Geräte-Zeile als auch im
Site-Kopfbereich, weil Datto RMM tatsächlich einen `portal_url` pro
Gerät/Site liefert -- allerdings als optionaler Link (`url: string | null`),
der bei fehlendem `portal_url` nichts rendert, statt eines toten Links.

`PluginsView.tsx` bindet die Sektion neben jeder anderen Plugin-Sektion
hier ein, alphabetisch zwischen `AteraPluginSection.tsx` und
`IntunePluginSection.tsx` einsortiert (nach der Überschrift
"Datto-RMM-Verbindungen").

## Acronis-Plugin (`plugin::acronis`) -- vierzehnte echte Integration, anders als die übrigen dreizehn

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
- **Basis-URL, eine echte Falle, korrigiert nach einem echten 404 im
  Live-Betrieb**: Acronis teilt seine Plattform in getrennte APIs mit
  UNTERSCHIEDLICHEN Basis-Pfaden unter derselben `datacenter_url` auf,
  verifiziert gegen developer.acronis.com nachdem eine anfängliche
  (falsche) Annahme, alle Endpunkte teilten sich ein Präfix, echte
  404-Fehler gegen einen echten Mandanten verursacht hatte. Account
  Management v2 (`idp/token`, `clients/{client_id}`, `tenants`) liegt
  unter `{datacenter_url}/api/2` (`{base}` unten). Resource and Policy
  Management v4 und Alert Manager v1 liegen NICHT dort -- sie liegen
  direkt unter `{datacenter_url}/api` (ohne `2/`-Segment), eine eigene
  `{resource_base}`, nur von `fetch_all_resources`/`fetch_all_severities`/
  `get_resource_statuses` verwendet.
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
     {resource_base}/resource_management/v4/resources`, Query-Parameter
     `tenant_id=<kommagetrennter Teilbaum des zugeordneten Mandanten:
     seine eigene ID PLUS alle untergeordneten Mandanten/Units>` (siehe
     `fetch_tenant_subtree_csv`/`collect_subtree_tenant_ids` -- ein
     echter, live verifizierter Fix: dieser Filter ist EXAKTE
     Einzel-Übereinstimmung, NICHT rekursiv, gegen developer.acronis.com
     bestätigt, nachdem eine anfängliche Annahme, die ID des zugeordneten
     Mandanten allein genüge, bei einem echten Konto zu einer massiv zu
     niedrigen Ressourcen-Zahl führte, weil die meisten Geräte tatsächlich
     unter untergeordneten Units/Standorten registriert waren, nicht
     direkt unter dem Kunden-Mandanten selbst), Cursor `before`/`after`.
     Antwort `{"items": [{"id", "name", "agent_id", "external_id",
     "type"}], "paging": {"cursors": {...}}}`. Dieses Plugin verwendet
     `id` als `AcronisResource::external_id` und `name` als Anzeigename,
     NICHT das eigene, verwirrend benannte `external_id`-Feld der Ressource
     (ein anderes, Acronis-internes Konzept, nicht der Identitäts-Schlüssel
     dieser Anwendung) und NICHT `agent_id` (Acronis' eigenes
     Agenten-Konzept, hier irrelevant).
  2. **Sicherungs-Gesundheit**: `GET {resource_base}/alert_manager/v1/resource_status`,
     gefiltert nach den EXAKTEN Ressourcen-IDs aus Schritt 1
     (`id=<eine ID>` oder `id=or(id1,id2,...)` für mehrere, Acronis'
     eigene Filter-Ausdruckssyntax) -- ein korrigierter, echter Fehler in
     einer früheren Version dieses Moduls, das einen undokumentierten,
     stillschweigend ignorierten `tenant=<id>`-Parameter sandte;
     gegen Acronis' echte OpenAPI-Spezifikation verifiziert, dass dieser
     Endpunkt GAR KEINEN `tenant`/`tenant_id`-Parameter besitzt, nur `id`
     und `embed_alert`. Antwort: `{"items":
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
     {resource_base}/resource_management/v4/resource_statuses?tenant_id=<id>`
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
vor `Action1PluginSection.tsx` und weit vor `IntunePluginSection.tsx` ein.

## Hetzner-Cloud-Plugin (`plugin::hetzner`): fünfzehnte echte Integration

`plugin/hetzner.rs` implementiert `Plugin` für Hetzner Clouds öffentliche
REST-API (`https://api.hetzner.cloud/v1`, feste Konstante -- Hetzner Cloud
hat, wie Level.io, keine Regionen-/Instanz-Varianten). Anders als jede
RMM-/MDM-Integration hier ist der Zweck dieses Plugins **Server-Inventar**
(welche Hetzner-Cloud-Server ein Kunde hat), keine Geräteverwaltungs-Agenten
-- und anders als Acronis gibt es hier auch kein Sicherungsstatus-Konzept.
Strukturell die einfachste Integration in dieser Codebasis, architektonisch
am nächsten an `plugin::level`/`plugin::intune`: eine Verbindung ist 1:1 an
genau einen lokalen Kunden gebunden, keine Organisations-/Mandanten-
Zuordnungstabelle.

Verifiziert gegen Hetzners eigene offizielle Cloud-API-Dokumentation und den
`hcloud-go`-SDK-Quellcode:

- **Authentifizierung**: statischer API-Token, Header `Authorization:
  Bearer <token>`, KEIN OAuth2. Erzeugt in der Hetzner Cloud Console unter
  Security > API tokens (Nur-Lesen oder Lesen&Schreiben -- dieses Plugin
  macht ausschließlich GETs, beide Varianten funktionieren also, keine
  client-seitige Berechtigungsprüfung nötig). Der Token ist an genau EIN
  Hetzner-"Projekt" gebunden -- bestätigt: ein Token kann keine anderen
  Projekte sehen.
- **Kein Organisations-/Mandanten-Konzept**: ein API-Token = ein
  Hetzner-Projekt = genau ein lokaler Kunde, bestätigt 1:1. Deshalb
  entspricht eine Hetzner-"Verbindung" hier direkt genau einem lokalen
  Kunden -- `HetznerConnectionMeta.customer_id` --, OHNE eine
  Zuordnungstabelle wie `NinjaOrgMapping`/`TacticalRmmClientMapping`,
  dasselbe Prinzip wie bei Level.io/Intune.
- **Server (der Inventar-Endpunkt)**: `GET {BASE_URL}/servers` ->
  `{"servers": [...]}` -- eine umschließende Hülle, KEIN nacktes Array
  (anders als Level.ios `{"devices": [...]}`). Verifizierte Pro-Server-
  Felder: `id` (Ganzzahl, als `external_id` verwendet, in Textform),
  `name` (String, der Anzeigename -- dient zugleich als
  `HetznerServer::hostname`, dieselbe "kein separates Hostname-Feld"-
  Konvention wie bei Intunes `deviceName`), `status` (freier
  String-Durchreich, kein Rust-Enum, dieselbe Konvention wie bei jedem
  anderen Plugin hier), `server_type.name` (verschachtelt -- als
  `HetznerServer::platform` verwendet, z. B. "cx22"), `location.name`
  (verschachtelt -- als `HetznerServer::location` verwendet, z. B. "fsn1",
  rein informative Anzeige). `datacenter` wird bewusst NIE gelesen --
  Hetzners eigenes SDK markiert es als veraltet (Entfernung nach
  2026-10-01), abgelöst durch `location`.
- **IPv4 vs. IPv6, eine echte Unterscheidung, kein Versehen**:
  `public_net.ipv4.ip` ist eine einzelne Host-Adresse, direkt als
  `ip_address` verwendet. `public_net.ipv6.ip` wird für dieses Feld
  BEWUSST NIE verwendet -- verifiziert gegen Hetzners API-Schema, es ist
  ein **/64-CIDR-Subnetz** (z. B. `"2001:db8::/64"`), keine einzelne
  Host-Adresse. Dieses Subnetz als die reine IP eines Servers darzustellen
  wäre aktiv falsch, nicht nur eine Auslassung -- siehe
  `extract_ipv4_address`, das ausschließlich `public_net.ipv4.ip` liest.
  Dieselbe Regel gilt im Frontend (`findExternalIp` in
  `HetznerPluginSection.tsx`).
- **Einzelner Server, Detailansicht**: `GET {BASE_URL}/servers/{id}` ->
  `{"server": {...}}` -- unter einem "server"-Schlüssel verpackt, anders
  als Level.ios/Intunes Einzelgeräte-Endpunkte (die das Geräteobjekt direkt,
  unverpackt, liefern). Diese Hülle unverändert durchzureichen würde den
  flachen, Top-Level-Feld-Scan des Frontends
  (`findExternalValue` in `HetznerPluginSection.tsx`) brechen, deshalb
  packt `fetch_server_json` das innere Server-Objekt aus, bevor es
  zurückgegeben wird (`unwrap_server_envelope`, mit Rückfallwert auf das
  unveränderte Rohobjekt, falls "server" unerwartet fehlt).
- **Paginierung**: Query-Parameter `page` (1-basiert) und `per_page`.
  Antwort-Umschlag: `{"servers": [...], "meta": {"pagination": {"page",
  "per_page", "previous_page", "next_page", "last_page",
  "total_entries"}}}`. Weitere Seiten existieren, solange `next_page`
  nicht null/0 ist -- `collect_paginated` durchläuft alle Seiten intern
  (bis zu 50 Seiten à 50 Servern als Schutz gegen eine sich falsch
  verhaltende Gegenstelle, `PAGE_LIMIT` ist Hetzners eigener dokumentierter
  `per_page`-Höchstwert) und liefert eine einzige, bereits zusammengefügte
  Liste. Anders als bei den meisten anderen Plugins hier ist die
  Akkumulations-/Abbruchlogik selbst als eigene, netzwerkunabhängige
  Funktion (`collect_paginated`, mit injizierter Fetch-Funktion) reine,
  für sich mit hartkodierten JSON-Seiten testbare Logik -- diese Codebasis
  hat keine HTTP-Mocking-Abhängigkeit, das ist der einzige Weg, "die
  Paginierungsschleife sammelt tatsächlich über mehrere Seiten hinweg"
  ohne eine solche echt zu testen.
- **Rate-Limits**: Antwort-Header `RateLimit-Limit`/`RateLimit-Remaining`/
  `RateLimit-Reset` existieren; häufig genannt ~3600 Anfragen/Stunde je
  Projekt (häufig zitiert, nicht unabhängig aus primärer Fließtext-Doku
  verifiziert -- hier ehrlich vermerkt). Keine besondere
  Retry-/Backoff-Behandlung nötig, dieselbe Konvention wie bei jedem
  anderen Plugin hier.
- **Kein Weboberflächen-Tiefenlink**: ein plausibles Muster existiert
  (`https://console.hetzner.cloud/projects/{project_id}/servers/{server_id}`),
  benötigt aber eine `project_id`, die nichts in der `/servers`-Antwort
  liefert, und das Muster selbst ist nur eine beobachtete Konvention, keine
  dokumentierte Garantie -- deshalb komplett ausgelassen, dieselbe
  ehrliche Auslassung wie bei `plugin::tacticalrmm`s fehlendem
  Dashboard-Link.
- **HTTP-Client**: `ureq` 3.4.1, synchron, dieselbe Abhängigkeit wie jedes
  andere Plugin hier.
- **Zugangsdaten-Kodierung**: Hetzner braucht nur einen einzigen
  Geheimwert (den API-Token), der 1:1 als `PluginCredentials.secret`
  durchgereicht wird -- keine JSON-Kodierung nötig, genau wie bei
  Level.io.

### Hetzner-Verbindungen sind 1:1 an einen Kunden gebunden

- Nicht-geheime Metadaten (`id`, `customer_id`, `label`) liegen als
  `HetznerConnectionMeta` in `Config::hetzner_connections` (`config.toml`,
  `#[serde(default)]`-kompatibel mit älteren Konfigurationen ohne dieses
  Feld).
- Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"hetzner:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Level.io
  (`commands::hetzner::generate_connection_id`/`plugin_id_for`, identisch
  zu den Level-Gegenstücken).
- Weil jede Verbindung genau eine `customer_id` trägt, braucht
  `sync_hetzner_connection` keine Fallunterscheidung "zugeordnet/
  unzugeordnet" wie `sync_ninja_connection` -- jeder synchronisierte Server
  gehört automatisch zum Kunden der Verbindung, `linked_system_id` wird für
  jeden Server direkt gegen die `external_refs`-Zeilen dieses Kunden
  geprüft.
- `HetznerServer` (Plugin-Ebene, `plugin/hetzner.rs`) trägt bewusst KEIN
  `linked_system_id`-Feld -- ob ein Server bereits mit einem lokalen System
  verknüpft ist, ist ein Buchführungs-Anliegen des Aufrufers
  (`commands::hetzner`, über `db::external_refs`), nicht etwas, das die
  Plugin-Ebene selbst weiß. Genau dieselbe Trennung wie bei
  `LevelDevice`/`IntuneDevice`: `linked_system_id` existiert erst auf der
  `commands`-Ebene-DTO (`commands::hetzner::ExternalSystemDto`).

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie Level.io, nur mit zusätzlichen Hetzner-
eigenen Feldern (`status`, `platform`, `location`) statt Level-Gruppen:
`sync_hetzner_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/hetzner-<connection_id>.json`
(`{"synced_at_utc": "...", "devices": [...]}`). `get_cached_hetzner_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff) und liefert `None`,
wenn für eine Verbindung noch nie synchronisiert wurde.
`remove_hetzner_connection` löscht diese Cache-Datei (bestes Bemühen).

### Tauri-Kommandos (`commands::hetzner`)

`test_hetzner_connection`, `list_hetzner_connections`,
`add_hetzner_connection`, `remove_hetzner_connection`,
`sync_hetzner_connection`, `get_cached_hetzner_sync`,
`link_system_to_hetzner`, `unlink_system_from_hetzner`,
`get_hetzner_system_details` -- dünne Wrapper nach demselben Muster wie
`commands::level`, aber ohne jegliche Zuordnungskommandos (kein
Hetzner-Äquivalent zu `map_ninja_organization`/`unmap_ninja_organization`
nötig, siehe oben).

`test_hetzner_connection` prüft einen API-Token per leichtgewichtigem
Aufruf (eine Seite mit `per_page=1`), ohne irgendetwas zu persistieren. Das
Übernehmen eines extern gelieferten Werts in ein selbst gepflegtes Feld
(`name`, `hostname`, `ip_address`, `notes` in `systems`) bleibt dabei --
wie bei jedem anderen Plugin hier -- ausschließlich eine bewusste, manuelle
Aktion über `get_hetzner_system_details` plus eine spätere UI-Aktion; kein
Kommando hier schreibt automatisch in diese vier Felder.

### Eingebunden in den Journal-System-Picker (`commands::external_directory`)

Wie bei jedem anderen Plugin hier ist `collect_hetzner` (plus
`read_hetzner_cache_file`) in `list_unlinked_external_systems_for_customer_pure`
verdrahtet -- bewusst als Teil dieser Änderung selbst, nicht als
Nachtrag: ein echter, kürzlich behobener Fehler in dieser Codebasis
bestand genau darin, dass sechs Plugins ergänzt, aber nie in diesen
Aggregator verdrahtet wurden, wodurch ihre Geräte im System-Feld des
Journal-Eintrags (`EntryEditor.tsx`/`QuickCapture.tsx`) wochenlang
unsichtbar blieben. `collect_hetzner` folgt exakt demselben Muster wie
`collect_level`/`collect_intune`: keine Mandanten-Zuordnungstabelle, ein
Server erscheint genau dann, wenn seine Verbindung an den angefragten
Kunden gebunden ist UND er noch nicht lokal verknüpft ist.

### Frontend (`HetznerPluginSection.tsx`)

Strukturell an `IntunePluginSection.tsx` angelehnt: flache, filterbare,
10-pro-Seite-paginierte Serverliste je Verbindung (kein Gruppierungs-
Layer, Hetzner kennt weder Organisationen noch Level.ios Gruppen-Konzept),
`j`/`k`/`Enter`/`l`/`u`-Tastaturnavigation, ein Kunde-Auswahlfeld im
Anlage-Formular (statt einer separaten Zuordnungs-UI). Der
Verbindungs-Anlage-Dialog hat nur EIN Geheimwert-Feld (API-Token als
`type="password"`), wie bei Level.io, nicht Intunes Drei-Werte-Formular.
Die "Alle anlegen (N)"-Sammel-Schaltfläche neben der "Nicht
verknüpft"-Überschrift ist, wie bei
`AbmPluginSection.tsx`/`IntunePluginSection.tsx`/`IruPluginSection.tsx`,
direkt nach `connection.id` verschlüsselt (nicht Level.ios eigene, komplexere
UI-Gruppen-Verschlüsselung -- Hetzner hat kein Gruppen-Konzept, für das
diese Komplexität nötig wäre). `findExternalIp` im Frontend spiegelt
`plugin::hetzner::extract_ipv4_address` exakt: liest ausschließlich
`public_net.ipv4.ip`, niemals das IPv6-/64-CIDR-Feld. `PluginsView.tsx`
bindet die Sektion alphabetisch zwischen `DattoRmmPluginSection.tsx` und
`IntunePluginSection.tsx` ein ("Hetzner-Verbindungen" sortiert zwischen
"Datto-RMM-Verbindungen" und "Intune-Verbindungen").

## netcup-Plugin (`plugin::netcup`): sechzehnte echte Integration

`plugin/netcup.rs` implementiert `Plugin` für netcups "Server Control
Panel" (SCP) REST/JSON-API. netcup ist ein deutscher Hosting-Anbieter für
vServer-/Root-Server-VPS-Produkte; die einzige Aufgabe dieses Plugins ist
Server-Inventar, kein Geräteverwaltungs-Agent-Konzept, kein
Sicherungsstatus wie bei Acronis, die einfachste Art Plugin in dieser
Codebasis. Strukturell am nächsten an `plugin::level`: netcups API kennt
gar kein Organisations- oder Unterkonten-Konzept, also entspricht eine
netcup-"Verbindung" (ein API-Token) direkt genau einem lokalen Kunden,
ohne Zuordnungstabelle wie bei Tactical RMM.

Jede Angabe unten ist gegen netcups eigene, live abgerufene
OpenAPI-Spezifikation und unauthentifizierte Live-Probe-Anfragen gegen den
echten API-Host verifiziert, nicht aus Dokumentationsprosa geraten:

- **Authentifizierung**: statischer Bearer-Token, Header `Authorization:
  Bearer <token>`. Live bestätigt: eine unauthentifizierte Anfrage liefert
  `401` mit `{"message":"Authorization header missing."}`, ein
  fehlerhafter Header nennt das exakt erwartete Format, und ein
  wohlgeformter, aber falscher Token liefert `{"message":"Invalid
  token."}` (eindeutig JSON/Bearer, nicht die SOAP-Form, die manche
  netcup-Dokumentation an anderer Stelle noch beschreibt). Der Token wird
  vom Nutzer selbst in einer eingeloggten netcup-SCP-Sitzung erzeugt (ein
  "API"-Menüpunkt), ein einmaliger, außerhalb dieser Anwendung liegender
  Einrichtungsschritt, genau wie bei jedem anderen Plugin hier.
- **Basis-URL**: eine feste Konstante (`plugin::netcup::BASE_URL`), live
  bestätigt (`GET /scp-core/api/ping` liefert `200 OK` mit Body `"OK"`,
  ein öffentlicher, unauthentifizierter Health-Check). Wie bei
  `plugin::level::BASE_URL` gibt es kein nutzerseitiges `base_url`-Feld.
- **Mandantschaft**: bestätigt 1:1. Es existiert kein Reseller- oder
  Unterkonten-Wechsel in dieser API, kein "Nutzer auflisten"-Endpunkt,
  kein Konto-Scoping-Parameter auf `/servers`.
- **Serverliste (minimal, kein Status, keine IP)**: `GET
  {BASE_URL}/servers`, Query-Parameter `limit`/`offset`. Die Antwort ist
  ein **nackter JSON-Array** (verifiziert, NICHT in einen Umschlag wie
  Hetzners `{"servers": [...]}` verpackt), Elemente geformt wie `{"id":
  <int32>, "name": "...", "hostname": "... oder null", "nickname": "...
  oder null", "disabled": <bool>, "template": {...} oder null}`. `id`
  (als String) wird `external_id`; der Anzeigename bevorzugt `nickname`,
  dann `hostname`, dann als letzten Ausweg das rohe netcup-`name`-Feld
  (ein `vXXXXX`-artiger Bezeichner), dieselbe Konvention "menschliche
  Bezeichnung bevorzugen, auf den rohen Identifikator zurückfallen", die
  `plugin::tacticalrmm` bereits mit seinem hostname-dann-agent_id-Fallback
  etabliert hat. **Dieser Listen-Endpunkt liefert tatsächlich weder ein
  Status- noch ein IP-Adress-Feld.**
- **Paginierung**: `limit`/`offset` existieren, aber die Antwort trägt
  weder ein Gesamtanzahl-Feld noch eine dokumentierte
  Standard-/Maximal-Seitengröße. `fetch_all_servers` nutzt deshalb die
  übliche defensive Heuristik: `offset` in festen `PAGE_LIMIT`-Schritten
  erhöhen, solange die zuletzt geladene Seite genau `PAGE_LIMIT` Elemente
  zurückliefert; sobald eine Seite kürzer ist, ist Schluss. `MAX_PAGES`
  deckelt den gesamten Durchlauf zusätzlich, derselbe defensive Schutz wie
  `plugin::level::MAX_PAGES`.
- **Einzelserver-Detail (reich, echter Status plus IP)**: `GET
  {BASE_URL}/servers/{serverId}`. Die Antwort trägt `serverLiveInfo.state`
  (ein freiform libvirt-Domain-Status-String wie `RUNNING`/`SHUTOFF`, kein
  Rust-Enum), `ipv4Addresses` (ein Array aus `{id, ip, netmask, gateway,
  broadcast}`, der erste Eintrag liefert die Anzeige-IP), `ipv6Addresses`
  (Prefix-/Gateway-Paare statt Host-Adressen, deshalb bewusst nie für eine
  Anzeige-IP verwendet), sowie `site.city`/`architecture` als rein
  informative Anzeigefelder. `Plugin::get_system_details` reicht diese
  Antwort unverändert als `serde_json::Value` durch, genau der
  "Aufrufer/UI interpretiert es selbst"-Vertrag, den jedes andere Plugin
  hier für `get_system_details` nutzt, und ist der EINZIGE Ort, an dem
  dieses Modul den Detail-Endpunkt aufruft: `list_servers`/
  `list_systems`/`sync_netcup_connection` rufen ihn niemals einmal pro
  Server während eines Sync-Laufs auf, was ein N+1-Muster gegen ein
  undokumentiertes Rate-Limit wäre.
- **Rate-Limits**: nur für einen unabhängigen
  Failover-IP-Routing-Endpunkt dokumentiert (10 Anfragen pro 5 Minuten, 20
  Anfragen pro 60 Minuten). Für `/servers` oder den hier genutzten
  Detail-Endpunkt ist nichts dokumentiert, deshalb gibt es keine spezielle
  Retry-/Backoff-Logik.
- **Web-Dashboard-Link**: nicht verifizierbar. Eine vermutete SCP-Route
  lieferte einen echten `404` gegen den realen Host, deshalb bewusst
  weggelassen, dasselbe ehrliche Auslassungsprinzip wie bei Tactical RMMs
  fehlendem Dashboard-Link.

### Die Liste ist minimal, das Detail ist reich

Anders als bei jedem bisherigen RMM-/MDM-Plugin hier liefert netcups
Server-Listen-Endpunkt weder Status noch IP-Adresse; beide Felder
existieren nur in der Detail-Antwort eines einzelnen Servers.
`plugin::netcup::NetcupServer` (die Rückgabe von `list_servers`) trägt
deshalb bewusst nur `external_id` und `name`, ohne erfundene,
immer-`None`-Felder, dieselbe "verifizierte Lücke"-Ehrlichkeit, die
`plugin::pulseway::PulsewayDevice` für seine eigenen, dauerhaft
fehlenden Felder bereits etabliert hat. `commands::netcup::ExternalSystemDto`
(die Form, die tatsächlich im Cache und im Frontend landet) trägt zwar
`status`/`ip_address`-Felder wie jede andere Plugin-DTO hier, dokumentiert
aber klar in ihrem eigenen Doc-Kommentar, dass diese beiden Felder aus
einem Sync-Lauf IMMER `None` bleiben. `sync_netcup_connection` ruft
absichtlich NIE den Detail-Endpunkt pro Server auf (das wäre ein
N+1-Muster gegen ein undokumentiertes Rate-Limit); echte Status-/
IP-Werte erscheinen ausschließlich, wenn ein Server einzeln über
`get_netcup_system_details` geöffnet wird, exakt wie im Rust-Modul
dokumentiert. `NetcupPluginSection.tsx` spiegelt das im UI: die
Geräteliste zeigt keine Status-/IP-Spalte, ein Hinweistext im
Verbindungs-Kartenkopf erklärt, dass diese Information erst beim Öffnen
der Details erscheint.

### netcup-Verbindungen sind 1:1 an einen Kunden gebunden

- Nicht-geheime Metadaten (`id`, `customer_id`, `label`) liegen als
  `NetcupConnectionMeta` in `Config::netcup_connections` (`config.toml`,
  `#[serde(default)]`-kompatibel mit älteren Konfigurationen ohne dieses
  Feld).
- Verbindungs-`id`-Erzeugung (`slugify` plus Millisekunden-Zeitstempel)
  und der vollqualifizierte `"netcup:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Level.io
  (`commands::netcup::generate_connection_id`/`plugin_id_for`).
- Weil jede Verbindung genau eine `customer_id` trägt, braucht
  `sync_netcup_connection` keine Fallunterscheidung "zugeordnet/
  unzugeordnet" wie `sync_ninja_connection`; jeder synchronisierte Server
  gehört automatisch zum Kunden der Verbindung, `linked_system_id` wird
  für jeden Server direkt gegen die `external_refs`-Zeilen dieses Kunden
  geprüft.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie Level.io, nur ohne Gruppierung:
`sync_netcup_connection` schreibt das Ergebnis jedes Laufs zusätzlich als
JSON nach `data_dir/plugin-cache/netcup-<connection_id>.json`
(`{"synced_at_utc": "...", "devices": [...]}`). `get_cached_netcup_sync`
liest ausschließlich diese Datei (kein Netzwerkzugriff) und liefert
`None`, wenn für eine Verbindung noch nie synchronisiert wurde.
`remove_netcup_connection` löscht diese Cache-Datei (bestes Bemühen).

### Tauri-Kommandos (`commands::netcup`)

`test_netcup_connection`, `list_netcup_connections`,
`add_netcup_connection`, `remove_netcup_connection`,
`sync_netcup_connection`, `get_cached_netcup_sync`,
`link_system_to_netcup`, `unlink_system_from_netcup`,
`get_netcup_system_details`: dünne Wrapper nach demselben Muster wie
`commands::level`, ohne Organisations-Zuordnungskommandos (kein
netcup-Äquivalent zu `map_ninja_organization`/`unmap_ninja_organization`
nötig, siehe oben).

`test_netcup_connection` prüft einen API-Token per leichtgewichtigem,
echtem Aufruf (`GET /servers?limit=1`), ohne irgendetwas zu
persistieren. Das Übernehmen eines extern gelieferten Werts in ein
selbst gepflegtes Feld (`name`, `hostname`, `ip_address`, `notes` in
`systems`) bleibt dabei, wie bei jedem anderen Plugin hier,
ausschließlich eine bewusste, manuelle Aktion über
`get_netcup_system_details` plus eine spätere UI-Aktion; kein Kommando
hier schreibt automatisch in diese vier Felder.

### Frontend (`NetcupPluginSection.tsx`)

Strukturell an `IruPluginSection.tsx` angelehnt: eine flache, nicht
gruppierte Geräteliste je Verbindung (kein Level-artiges
Gruppen-Konzept), ein Kunde-Auswahlfeld im Anlage-Formular (Label plus
Kunde plus API-Token, kein Basis-URL-Feld, da netcup einen festen
API-Host hat), Filterung und 10-pro-Seite-Paginierung,
`j`/`k`/`Enter`/`l`/`u`-Tastaturnavigation, sowie der "Alle anlegen
(N)"-Sammel-Button neben der "Nicht verknüpft"-Überschrift, hier
verbindung-`id`-geschlüsselt (keine Gruppen-Ebene, siehe
`IruPluginSection.tsx`s `bulkCreateBusy` für dasselbe Muster). Das
Detail-Panel eines verknüpften Servers liest `get_netcup_system_details`s
Roh-JSON serverseitig aus: `serverLiveInfo.state` für eine
Status-Anzeige, `ipv4Addresses[0].ip` für die
Vergleichen/Übernehmen-Tabellenzeile "IP-Adresse", `nickname`/`hostname`/
`name` für "Name". `PluginsView.tsx` bindet die Sektion zwischen
`LevelPluginSection.tsx` und `NinjaPluginSection.tsx` ein: die
gerenderte Überschrift "netcup-Verbindungen" sortiert alphabetisch vor
"Ninja-Verbindungen" (der Buchstabe "e" steht vor "i"), nicht danach.

## Vultr-Plugin (`plugin::vultr`): siebzehnte echte Integration

`plugin/vultr.rs` implementiert `Plugin` für Vultrs öffentliche REST-API v2
über HTTPS (`https://api.vultr.com/v2`, feste Konstante, kein
selbstgehostetes, regionsabhängiges Host-Muster wie bei Tactical RMM oder
NinjaOne). Fakten unten verifiziert gegen Vultrs echten, offiziellen
Go-SDK-Quellcode (`govultr`, <https://github.com/vultr/govultr>) und
<https://www.vultr.com/api/> (die interaktive API-Referenz auf
docs.vultr.com). Anders als jede RMM- oder MDM-Integration hier: Vultr ist
reiner Server-Bestand (Cloud-VPS-Instanzen eines Kunden), keine
Geräteverwaltungs-Telemetrie, also kein Gruppen-, Site- oder
Organisationskonzept aufzulösen (anders als Levels Gruppen), und kein
Sicherungsstatus-Konzept wie bei Acronis. Die einfachste Plugin-Art in
dieser Codebasis.

- **Authentifizierung**: statischer API-Key im `Authorization`-Header, MIT
  `Bearer`-Präfix (anders als Level.ios Header ohne Präfix):
  `Authorization: Bearer <api-key>`. Wird im Kundenportal unter Account >
  API erzeugt (erfordert vorher "Enable API"), ein einmaliger,
  außerhalb dieser App liegender Einrichtungsschritt, den der Nutzer selbst
  durchführt, genau wie bei jedem anderen API-Key-Plugin hier.
- **Fester Host, kein `base_url`**: Vultrs API-Host ist immer
  `api.vultr.com`, wie `plugin::level::BASE_URL` und
  `plugin::intune::GRAPH_BASE_URL` eine Konstante statt eines
  konfigurierbaren `base_url`-Felds auf `VultrConnectionMeta`.
- **1:1 an einen Kunden gebunden**: bestätigt für das, was dieses Plugin
  braucht: ein Vultr-API-Key sieht ausschließlich die Instanzen genau eines
  Accounts ("List all instances on your account"). Vultr hat zwar ein
  echtes "Sub-Accounts"-Feature, aber jeder Unter-Account ist ein
  vollständig unabhängiger Vultr-Nutzer mit einem EIGENEN, separaten
  API-Key. Ein MSP mit mehreren Vultr-Unter-Accounts braucht also eine
  Verbindung (einen API-Key) pro Unter-Account, exakt dasselbe Muster wie
  Level.ios und Intunes "eine Verbindung = ein Kunde", KEINE
  Eltern-Account-sieht-alle-Kinder-Zuordnungstabelle wie NinjaOnes
  `NinjaOrgMapping`. `VultrConnectionMeta` trägt deshalb `customer_id`
  direkt, wie `LevelConnectionMeta` und `IntuneConnectionMeta`.
- **Instanzen**: `GET {BASE_URL}/instances` -> `{"instances": [...], "meta":
  {...}}`. Verifizierte Pro-Instanz-Felder: `id` (String, eine UUID, direkt
  als `external_id` verwendet), `label` (String, nutzergesetzter
  Anzeigename) und `hostname` (eigenes Feld): `label` gewinnt, wenn nicht
  leer, sonst `hostname`, sonst die externe ID, dasselbe Muster "menschlichen
  Namen bevorzugen, auf Rohkennung zurückfallen", das mehrere Plugins hier
  bereits verwenden (siehe `map_instance`); `main_ip` (String, IPv4, die
  primäre `ip_address`); `v6_main_ip` (String, IPv6, ein LEERER STRING,
  nicht `null`, wenn IPv6 für diese Instanz deaktiviert ist; wird genauso
  wie fehlend behandelt, nie als `Some("")` gespeichert, siehe
  `non_empty_str`); `plan` (String, die Größe beziehungsweise Ausstattung der
  Instanz, als `platform`-äquivalenter Anzeigestring verwendet, wie Datto
  RMMs `deviceClass`); `region` (String, z. B. `"ewr"`, rein informativ).
- **`status` gegenüber `power_status`**: ein echtes Instanzobjekt hat SOWOHL
  ein `status`-Feld (übergeordneter Lebenszyklus-Zustand, z. B. `"active"`
  oder `"pending"`) ALS AUCH ein separates `power_status`-Feld (Ein- oder
  Aus-Zustand), zwei unterschiedliche Felder, verifiziert in der echten
  SDK-Struktur. Dieses Plugin stellt bewusst `power_status` als
  `VultrInstance::status` dar (statt Vultrs eigenem `status`-Feld), weil das
  unmittelbar handlungsrelevantere Signal ist, ob die Instanz gerade läuft,
  was einen IT-Administrator auf den ersten Blick interessiert, während der
  Lebenszyklus-`status` hauptsächlich während Provisionierung oder Löschung
  eine Rolle spielt: ein deutlich kleinerer Ausschnitt der tatsächlichen
  Nutzung dieses Plugins.
- **Pagination**: cursor-basiert. Query-Parameter `per_page` (Standard 100,
  Maximum 500; dieses Plugin fragt immer das Maximum an, `PAGE_LIMIT =
  500`, um Roundtrips zu minimieren) und `cursor`. Antwort-Umschlag:
  `{"instances": [...], "meta": {"total": <int>, "links": {"next": "<cursor
  oder leerer String>", "prev": "..."}}}`. Weitere Seiten existieren,
  solange `meta.links.next` ein nicht leerer String ist; dieser String wird
  zum `cursor`-Query-Parameter der nächsten Anfrage. `fetch_all_instances`
  durchläuft alle Seiten intern und liefert eine einzige, bereits
  zusammengefügte Liste. `fetch_all_instances_via` ist bewusst generisch
  über die Seiten-Abruf-Funktion gehalten (statt einen echten HTTP-Aufruf
  fest zu verdrahten), sodass die Pagination-Zusammenführungslogik selbst
  als reine Funktion über vorab abgerufenen, hartkodierten JSON-Seiten
  testbar ist, ganz ohne echten Netzwerkzugriff, exakt dasselbe Muster wie
  `plugin::dattormm::fetch_all_pages`. Begrenzt auf `MAX_PAGES`, zum Schutz
  gegen eine sich falsch verhaltende Gegenstelle.
- **Einzelinstanz-Details**: `GET {BASE_URL}/instances/{id}` ->
  `{"instance": {...}}`, reicht die Antwort unverändert als
  `serde_json::Value` durch, exakt wie jedes andere Plugin hier.
- **Rate-Limits**: dokumentiert, 30 Anfragen pro Sekunde pro Quell-IP, `429`
  bei Überschreitung, keine dokumentierte `Retry-After`-Kopfzeile. Keine
  spezielle Wiederholungs- oder Backoff-Logik nötig: das Nutzungsmuster
  dieser App (manuelle, gelegentliche Synchronisationen) liegt bei Weitem
  unter 30 Anfragen pro Sekunde, wie bei jedem anderen Plugin hier.
- **Kein Dashboard-Deep-Link**: NICHT verifizierbar gegen die aktuelle
  v2-API: das einzige konkrete Vultr-Instanz-URL-Muster, das in
  öffentlichen Quellen gefunden wurde, verwendet eine veraltete
  v1-Zahlen-ID, nicht die v2-UUID `id`, die dieses Plugin tatsächlich hat.
  Bewusst vollständig ausgelassen, dasselbe Prinzip der ehrlichen Auslassung
  wie bei Tactical RMMs fehlendem Dashboard-Link.
- **HTTP-Client**: dieselbe `ureq`-3.4.1-Abhängigkeit wie jedes andere
  Plugin hier.
- **Zugangsdaten-Kodierung**: Vultr braucht nur einen einzigen Geheimwert
  (den API-Key), der 1:1 als `PluginCredentials.secret` durchgereicht wird,
  ganz ohne JSON-Kodierung, genau wie bei `plugin::level`.

### Vultr-Verbindungen sind 1:1 an einen Kunden gebunden

- Nicht-geheime Metadaten (`id`, `customer_id`, `label`) liegen als
  `VultrConnectionMeta` in `Config::vultr_connections` (`config.toml`,
  `#[serde(default)]`-kompatibel mit älteren Konfigurationen ohne dieses
  Feld).
- Verbindungs-`id`-Erzeugung (`slugify` + Millisekunden-Zeitstempel) und der
  vollqualifizierte `"vultr:<connection_id>"`-Bezeichner
  (Schlüsselspeicher-Konto UND `external_refs.plugin_id`) folgen exakt
  demselben Muster wie bei Level.io/Intune.
- Weil jede Verbindung genau eine `customer_id` trägt, braucht
  `sync_vultr_connection` keine Fallunterscheidung zwischen "zugeordnet"
  und "unzugeordnet": jede synchronisierte Instanz gehört automatisch zum
  Kunden der Verbindung, und `linked_system_id` wird für jede Instanz
  direkt gegen die `external_refs`-Zeilen dieses Kunden geprüft.

### Zwischenspeicher für Offline-Ansicht

Exakt dieselbe Konvention wie Level/Intune, ohne Gruppierung (Vultr kennt
kein Organisations-/Site-Konzept): `sync_vultr_connection` schreibt das
Ergebnis jedes Laufs zusätzlich als JSON nach
`data_dir/plugin-cache/vultr-<connection_id>.json` (`{"synced_at_utc":
"...", "instances": [...]}`). `get_cached_vultr_sync` liest ausschließlich
diese Datei (kein Netzwerkzugriff) und liefert `None`, wenn für eine
Verbindung noch nie synchronisiert wurde. `remove_vultr_connection` löscht
diese Cache-Datei (bestes Bemühen).

### Tauri-Kommandos (`commands::vultr`)

`test_vultr_connection`, `list_vultr_connections`, `add_vultr_connection`,
`remove_vultr_connection`, `sync_vultr_connection`, `get_cached_vultr_sync`,
`link_system_to_vultr`, `unlink_system_from_vultr`,
`get_vultr_system_details`: dünne Wrapper nach demselben Muster wie
`commands::level`, ohne Organisations-Zuordnungskommandos (kein
Vultr-Äquivalent zu `map_ninja_organization` oder
`unmap_ninja_organization` nötig, siehe oben).

`test_vultr_connection` prüft einen API-Key per leichtgewichtigem Aufruf
(eine Seite mit `per_page=1`), ohne irgendetwas zu persistieren. Das
Übernehmen eines extern gelieferten Werts in ein selbst gepflegtes Feld
(`name`, `hostname`, `ip_address`, `notes` in `systems`) bleibt dabei, wie
bei jedem anderen Plugin hier, ausschließlich eine bewusste, manuelle
Aktion über `get_vultr_system_details` plus eine spätere UI-Aktion; kein
Kommando hier schreibt automatisch in diese vier Felder.

### `collect_vultr` in `commands::external_directory`

Wie bei jedem anderen Plugin hier ist das Einbinden in
`list_unlinked_external_systems_for_customer_pure`
(`commands::external_directory`) ein PFLICHTSCHRITT, kein optionales
Verdrahten: genau der Fehler, der kürzlich für sechs andere Plugins
behoben wurde (siehe der Modul-Doc-Kommentar dort), wäre sonst für Vultr von
Anfang an reproduziert worden. `collect_vultr` folgt exakt demselben Muster
wie `collect_level`/`collect_intune` (keine Mandanten-Zuordnungstabelle,
`connection.customer_id` direkt geprüft), samt Tests, die
`level_device_appears_when_its_connection_is_bound_to_the_target_customer`/
`already_linked_level_device_is_excluded` spiegeln.

### Frontend (`VultrPluginSection.tsx`)

Strukturell an `IntunePluginSection.tsx` angelehnt: eine echte flache,
filterbare, 10-pro-Seite-paginierte Instanzliste (kein Gruppen-Layer, Vultr
kennt kein Organisations-/Site-Konzept), `j`/`k`/`Enter`/`l`/`u`-
Tastaturnavigation, die "Alle anlegen (N)"-Sammel-Anlegen-Schaltfläche
(geschlüsselt direkt über `connection.id`, wie bei jedem
1:1-Kunde-Plugin). Der Verbindungs-Anlage-Dialog hat wie
`LevelPluginSection.tsx`s nur ein Kunde-Auswahlfeld plus ein einzelnes
API-Schlüssel-Feld (`type="password"`), aber kein Base-URL-Feld (siehe
oben). `InstanceSummaryLine` zeigt Plan/Region/Status(power_status)/
IP-Adresse zusätzlich zum Namen an. Die Vergleichen/Übernehmen-Tabelle für
verknüpfte Instanzen bleibt bei den drei üblichen Feldern
(Name/Hostname/IP-Adresse); zusätzliche Vultr-Felder (Plan, Region, Status,
IPv6-Adresse) werden im Details-Panel rein informativ angezeigt, nicht
übernehmbar, analog zu Intunes Compliance- und Betriebssystem-Feldern.
`PluginsView.tsx` bindet die Sektion alphabetisch nach
`TacticalRmmPluginSection.tsx` ein (letzte Position, "V" kommt zuletzt).
