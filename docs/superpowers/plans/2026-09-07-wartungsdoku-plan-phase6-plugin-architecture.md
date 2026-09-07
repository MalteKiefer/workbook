# Phase 6 — Plugin-Architektur (Erweiterungspunkt, nicht befüllt)

## Ziel

Spec-Abschnitt "Plugin-Architektur (Vorbereitung, jetzt nicht befüllt)" liefern:
Trait-Definition, `external_refs`-Anbindung (Tabelle existiert bereits seit
Phase 1, `migrations/0001_init.sql`, hier nur verifiziert, nicht verändert),
und eine dokumentierte Dummy-Beispielimplementierung. Kein echtes Plugin, kein
UI, kein Tauri-Kommando — reiner Erweiterungspunkt für eine spätere
Ausbaustufe.

Verbindliche Vorgaben aus der Spec, wörtlich übernommen:
- Plugins laufen isoliert, kein direkter SQLite-Zugriff — nur über das
  Trait-Interface; der Aufrufer (Anwendung) schreibt Ergebnisse in
  `external_refs`.
- Zugangsdaten ausschließlich im OS-Schlüsselspeicher (`keyring`-Crate,
  Windows Credential Manager / Secret Service unter Linux), nie im Klartext in
  DB oder `config.toml`.
- Von Plugins gelieferte Daten sind als "extern" markiert und überschreiben
  nie selbst gepflegte Inhalte.
- Jeder Plugin-Aufruf muss später per Tastatur auslösbar sein — jetzt nicht
  relevant (kein UI), aber die Trait-Form darf nichts vorwegnehmen, das später
  eine Maus-exklusive Bedienung erzwingen würde (z. B. keine Rückgabetypen,
  die zwingend interaktive Dialoge voraussetzen).

## Bausteine

### 1. `src-tauri/src/plugin/mod.rs` (neu, Modul `plugin` neu, Top-Level wie
   `db`/`error`, nicht unter `commands`)

- `ExternalSystem { external_id, name, hostname: Option<String> }` — minimal,
  generisch; reichhaltige, werkzeugspezifische Daten gehören in
  `external_refs.payload_json`, nicht hierher.
- `PluginCredentials { secret: String }` — opaque, das Crate interpretiert
  den Inhalt nicht, holt ihn nur unversehrt aus dem Schlüsselspeicher.
- `PluginError` (`thiserror`, bereits Abhängigkeit über `AppError`):
  `Authentication`, `Unreachable`, `UnexpectedResponse`.
- `trait Plugin`: `id`, `list_systems`, `get_system_details`, `link_system` —
  Signaturen wie im Spec-Codeblock, `link_system` schreibt selbst nichts in
  die DB (das übernimmt der Aufrufer via `db::external_refs::upsert`).
- `pub mod dummy;` und `pub mod secrets;`.

### 2. `src-tauri/src/plugin/dummy.rs` (neu)

`DummyPlugin`, dokumentiert als Referenzimplementierung: liefert zwei feste
`ExternalSystem`-Einträge, `get_system_details` echot die `external_id` in
einem JSON-Objekt, `link_system` loggt nur (`println!`) und gibt `Ok(())`
zurück — kein echter Netzwerkzugriff, komplett in-memory. Tests decken alle
drei Methoden ab (Anzahl/Inhalt der Systeme, `external_id` im Detail-JSON,
`link_system` erfolgreich).

### 3. `src-tauri/src/plugin/secrets.rs` (neu)

Dünner Wrapper um die `keyring`-Crate (neue Abhängigkeit, `cargo add
keyring`): `store_secret(plugin_id, secret) -> Result<(), AppError>`,
`load_secret(plugin_id) -> Result<Option<String>, AppError>`, Service-Name
`"wartungsdoku"`, Konto = `plugin_id`. Fehler werden auf `AppError::Config`
gemappt (kein passenderer bestehender `AppError`-Fall vorhanden). Kein
automatisierter Test gegen den echten OS-Schlüsselspeicher: das würde auf
diesem Rechner einen verwaisten, schwer aufzuräumenden Credential-Eintrag
hinterlassen, und Sandbox-/CI-Umgebungen haben oft gar keinen echten
Schlüsselspeicher. Korrektheit stützt sich auf die Testsuite der
`keyring`-Crate selbst plus sauberes Kompilieren/Typchecking gegen die reale
API dieser Version — deshalb kein `#[cfg(test)]`-Block in dieser Datei.

### 4. `src-tauri/src/db/external_refs.rs` (neu, `pub mod external_refs;` in
   `db/mod.rs` ergänzt — einzige Änderung an bestehenden Dateien)

Minimales Repository nach etabliertem Muster (`db/attachments.rs`): eine
Zeile je `(system_id, plugin_id)`.

- `ExternalRef { id, system_id, plugin_id, external_id, payload_json,
  synced_at_utc, synced_at_tz }`.
- `upsert(conn, system_id, plugin_id, external_id, payload_json, tz)` —
  sucht bestehende Zeile über `(system_id, plugin_id)`, aktualisiert sie
  (inkl. `synced_at_*` via `time::now_with_tz`) oder legt sie neu an.
  Wiederholtes Sync überschreibt in derselben Zeile statt Duplikate
  anzusammeln.
- `get(conn, id)` — `NotFound` bei fehlender Zeile.
- `list_for_system(conn, system_id)` — sortiert nach `plugin_id`.

Tests: zweimaliger `upsert` mit unterschiedlichem `payload_json` für
dasselbe `(system_id, plugin_id)` → gleiche `id`, aktualisiertes
`payload_json`, weiterhin genau eine Zeile; `upsert` mit zweitem `plugin_id`
auf demselben System → `list_for_system` liefert zwei Zeilen; `get` mit
unbekannter `id` → `AppError::NotFound`. Seed von Kunde+System über
bestehende `customers::create`/`systems::create`.

### Nicht Teil dieses Bausteins

- Keine Tauri-Kommandos, kein `invoke_handler!`-Eintrag — nichts im Frontend
  konsumiert dies.
- Keine Registrierung von `pub mod plugin;` in `lib.rs` — bleibt der
  Integrationsrunde vorbehalten (Boundary der Aufgabenstellung), wird im
  Abschlussbericht als exakte Zeile mitgeliefert.
- `migrations/0001_init.sql` bleibt unangetastet (Tabelle existiert bereits,
  vorwärtsgerichtete Migrationen).

## Architekturnotiz

`docs/PLUGIN_ARCHITECTURE.md` (wenige hundert Wörter): Isolationsprinzip,
Credential-Prinzip, "externe Daten überschreiben nie selbst gepflegte
Inhalte" (mit expliziter Aussage: `systems`-Felder `name`/`hostname`/
`ip_address`/`notes` haben in einer künftigen UI immer Vorrang, externe
Payload nur ergänzend/read-only dargestellt, nie automatisch in diese Felder
zurückgeschrieben), kurze Anleitung "eigenes Plugin ergänzen" anhand von
`DummyPlugin` als Vorlage.

## Build/Test

```
cd src-tauri && cargo build --bin wartungsdoku && cargo test plugin:: && cargo test external_refs::
```
