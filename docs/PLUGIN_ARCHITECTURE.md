# Plugin-Architektur

Status: Erweiterungspunkt vorbereitet, nicht befüllt. Kein echtes Plugin, kein
UI-Aufruf — Spec-Abschnitt "Plugin-Architektur (spätere Ausbaustufe)".

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

Noch nicht Teil dieser Ausbaustufe: ein Plugin-Registry/-Loader, Tauri-
Kommandos, die ein Plugin aus dem Frontend ansteuerbar machen, und
Command-Palette-Einträge dafür.
