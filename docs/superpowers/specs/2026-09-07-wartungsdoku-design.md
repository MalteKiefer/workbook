# Wartungsdoku — Tastaturgesteuerte Wartungsdokumentation für IT-Dienstleister

Status: approved
Datum: 2026-09-07

## Zweck

Lokale Desktop-Anwendung für einen einzelnen IT-Administrator, um Tätigkeiten je
Kunde/System zeitstempelgenau zu dokumentieren. Oberstes Ziel: Erfassung darf die
eigentliche Arbeit nicht verlangsamen — vollständig tastaturbedienbar, ein Eintrag
inklusive Screenshot in unter 30 Sekunden, ohne die Hände von der Tastatur zu nehmen.

Kein Mehrbenutzerbetrieb, keine Synchronisation, keine Zeiterfassung/Abrechnung, keine
Vorlagen, kein Ticketsystem. Rein chronologisches Journal.

## Stack

| Bereich | Wahl | Anmerkung |
|---|---|---|
| App-Framework | Tauri 2 | Rust-Backend, Webview-Frontend |
| Tauri-Plugins | `tray-icon`, `global-shortcut`, `autostart`, `single-instance` | wie vorgegeben |
| Frontend | TypeScript + React, Build via Vite | Vite nicht im Auftrag genannt, Standardwahl für Tauri+React |
| Editor | CodeMirror 6, `@codemirror/lang-markdown` | |
| Command Palette | `cmdk` (React) | |
| DB | SQLite, FTS5, WAL, Foreign Keys ON | via `rusqlite` (bundled-Feature) + `r2d2_sqlite`-Pool |
| PDF-Export | **Typst** (`typst`-Crate, embedded) | **Abweichung vom Auftrag** (dort keine Engine genannt). Begründung: reines Rust, kein externes Binary/keine Laufzeitinstallation, native Unterstützung für TOC, Kopf-/Fußzeile, eingebettete Bilder, gute Typografie ohne manuelle Low-Level-Layoutberechnung |
| Zwischenablage-Bilder | `arboard`-Crate | |
| Fenstertitel-Kontext | Windows: `windows`-Crate (`GetForegroundWindow`/`GetWindowTextW`); Linux/X11: `x11rb`; Wayland: deaktiviert | |
| Secrets (Plugin-Ausbaustufe) | `keyring`-Crate | Windows Credential Manager / Secret Service |
| Zielplattformen | Windows, Linux, je Standalone-Binary | Tauri nutzt System-Webview (WebView2/WebKitGTK) statt eigener Runtime — erfüllt "ohne Laufzeitinstallation" im Sinne von: kein Node/Electron/JVM etc. muss installiert werden. WebView2 ist auf aktuellen Windows-10/11-Systemen vorinstalliert |

## Datenablage

```
<datadir>/
  wartungsdoku.db          # SQLite, WAL-Dateien (.db-wal, .db-shm) daneben
  attachments/
    a3/f2c9e1....png        # SHA-256, erste 2 Hex-Zeichen als Unterordner
  config.toml
```

Anhänge content-addressed im Dateisystem, nicht als BLOB in der DB. DB hält Hash,
Originaldateiname, MIME-Typ, Größe, Verknüpfung zum Eintrag. Identische Dateien
dedupliziert automatisch (gleicher Hash → gleicher Pfad, kein erneutes Schreiben).
Backup = Kopie des gesamten Datenverzeichnisses.

`datadir` ist konfigurierbar (`config.toml`), Default: plattformübliches
App-Datenverzeichnis (`%APPDATA%/wartungsdoku` bzw. `~/.local/share/wartungsdoku`).

## Datenmodell

Hierarchie: **Kunde → System → Eintrag**.

### Zeitstempel-Regel (verbindlich, ausnahmslos)

Kein Feld vom Typ `DATE`. Jede Zeitangabe = zwei Spalten:

- `<feld>_utc` — TEXT, ISO 8601 UTC mit Millisekunden (`2026-09-07T12:32:14.221Z`).
  Einzige Spalte für Sortierung, Filterung, Indizierung.
- `<feld>_tz` — TEXT, IANA-Zonenname zum Erfassungszeitpunkt (`Europe/Berlin`).

Kein reiner UTC-Offset — nicht eindeutig bei DST-Wechseln/rückwirkenden Zonenänderungen.

Regeln:
- Vergleich/Sortierung/Zeitraumfilter ausschließlich auf `_utc`-Wert
- Anzeige/Export in der gespeicherten Zone mit sichtbarer Zonenangabe
  (`07.09.2026 14:32 MESZ`), nie in der aktuellen Zone des Rechners
- Tätigkeitszeitpunkt und Erfassungszeitpunkt sind getrennte Felder, nie verrechnet
- Zeitstempel werden serverseitig in Rust erzeugt (`chrono` + `chrono-tz`), nie im Frontend

### Tabellen

**`customers`**: `id`, `name`, `short_code` (eindeutig, Index), `notes`,
`created_at_utc`, `created_at_tz`, `updated_at_utc`, `updated_at_tz`,
`archived_at_utc` (nullable), `archived_at_tz` (nullable)

**`systems`**: `id`, `customer_id` FK, `name`, `system_type` (freier Text),
`hostname`, `ip_address`, `notes`, `created_at_utc`, `created_at_tz`,
`updated_at_utc`, `updated_at_tz`, `archived_at_utc` (nullable), `archived_at_tz` (nullable)

**`entries`**: `id`, `customer_id` FK, `system_id` FK nullable (kundenweiter Eintrag),
`title`, `body_md`, `category` (Enum, siehe unten),
`performed_at_utc`, `performed_at_tz` — nachweisrelevanter Tätigkeitszeitpunkt,
nachträglich änderbar, Vorbelegung = aktueller Zeitpunkt in Systemzone bei Anlage,
`created_at_utc`, `created_at_tz` — Erfassungszeitpunkt, von der Anwendung gesetzt,
für den Nutzer nicht editierbar,
`updated_at_utc`, `updated_at_tz`

Weicht `created_at` deutlich (> konfigurierbarer Schwellwert, Default 24h) von
`performed_at` ab, zeigt UI "nachträglich erfasst" dezent an; Export weist in diesem
Fall beide Zeitpunkte aus.

`category` — im Code definierte, feste Liste: `wartung`, `stoerung`, `aenderung`,
`installation`, `sonstiges`.

**`tags`**: `id`, `name` (eindeutig)
**`entry_tags`**: `entry_id` FK, `tag_id` FK, PK(entry_id, tag_id)

**`attachments`**: `id`, `entry_id` FK, `sha256`, `original_filename`, `mime_type`,
`size_bytes`, `created_at_utc`, `created_at_tz`

**`entries_fts`**: FTS5-Virtualtabelle über `title`, `body_md`, per Trigger
(AFTER INSERT/UPDATE/DELETE on `entries`) synchron gehalten. Kunden- und Systemnamen
zusätzlich durchsuchbar über eine zweite FTS5-Tabelle (`directory_fts`) oder gemeinsame
Abfrage über beide Tabellen — Entscheidung fällt in der Implementierung, beide Wege
funktional gleichwertig.

**`external_refs`** (Plugin-Vorbereitung, jetzt angelegt, nicht befüllt): `id`,
`system_id` FK, `plugin_id`, `external_id`, `payload_json`, `synced_at_utc`,
`synced_at_tz`

**`schema_migrations`**: `version`, `applied_at_utc`, `applied_at_tz`

Alle FKs mit `ON DELETE CASCADE` wo fachlich sinnvoll (z. B. `entry_tags`,
`attachments` → `entries`), `ON DELETE RESTRICT` wo ein Datenverlust nicht gewollt ist
(z. B. `entries` → `customers`/`systems`, stattdessen Archivierung nutzen).

### Migrationen

Vorwärtsgerichtete, versionierte SQL-Dateien (`migrations/0001_init.sql`, …). Beim
Start: Schema-Version aus `schema_migrations` lesen, fehlende Migrationen sequenziell
anwenden, vorher Kopie der `.db`-Datei anlegen (`wartungsdoku.db.bak-<version>`).
Fehlschlag einer Migration → Abbruch mit verständlicher Fehlermeldung, keine
automatische Rückrollung (Backup-Kopie ist der Rückweg).

## Hintergrundbetrieb

- Autostart bei Anmeldung, in `config.toml` abschaltbar
- Tray-Icon (Windows Infobereich, Linux StatusNotifierItem/AppIndicator), Menü:
  Schnellerfassung, Suche, Fenster zeigen, Einstellungen, Beenden
- Fenster schließen (`X`) versteckt, beendet nicht. Beenden nur bewusst über
  Tray-Menü oder Palette-Befehl "Anwendung beenden"
- Single-Instance: zweiter Start beendet sich selbst, signalisiert laufender Instanz
  per Tauri-IPC/Event, Fenster zu zeigen (bzw. Schnellerfassung/Suche zu öffnen, wenn
  über `--quick-capture`/`--search` gestartet)

### Globale Hotkeys

| Taste | Wirkung |
|---|---|
| `Strg+Alt+Leertaste` | Schnellerfassung öffnen |
| `Strg+Alt+F` | Suche öffnen |
| `Strg+Alt+S` | Zwischenablage-Screenshot direkt als neuen Eintrag anlegen |

Alle Belegungen in `config.toml` änderbar. Registrierung über
`tauri-plugin-global-shortcut`.

**Linux/Wayland-Sonderfall**: Globale Registrierung nur via XDG-Portal
`org.freedesktop.portal.GlobalShortcuts`, nicht überall vorhanden. Fehlt es, zeigt die
Anwendung dies verständlich an und verweist auf CLI-Fallback:
`wartungsdoku --quick-capture`, `wartungsdoku --search`. Diese Argumente sind
unabhängig von der Portal-Verfügbarkeit immer implementiert (nutzbar auch unter X11
oder Windows für eigene Tastenkombinationen über Drittwerkzeuge).

### Schnellerfassungsfenster

Kompaktes, mittig platziertes Fenster (kein Hauptfenster), erscheint in < 150 ms,
sofort tastaturfokussiert. Cursor im Titelfeld, Kunde/System mit letzter Auswahl
vorbelegt (änderbar über dieselbe Fuzzy-Eingabe wie Command Palette). `performed_at`
= Moment des Hotkey-Drucks, nicht des Speicherns. `Strg+V` fügt Zwischenablage-
Screenshot ein. `Strg+S` speichert, schließt, gibt Fokus an vorher aktive Anwendung
zurück (Windows: `SetForegroundWindow` auf gemerktes HWND; X11: analog über
gemerkte Window-ID; Wayland: bestmöglicher Versuch, da fremder Fokuswechsel dort
eingeschränkt ist). `Esc` verwirft, gleiches Fokusverhalten. Unvollständiger Entwurf
überlebt Schließen (Zwischenspeicherung in `config`-nahem State, nicht in `entries`).

### Kontexterfassung (optional, Default aus)

Titel des zuvor aktiven Fensters wird bei Aktivierung als Notiz in den Entwurf
übernommen. Windows/X11 verfügbar, Wayland nicht (bewusst deaktiviert dargestellt,
nicht stillschweigend übergangen). Nutzer schaltet bewusst ein.

## Bedienkonzept

**Harte Anforderung**: jede Funktion ohne Maus erreichbar. Keine klick-exklusive
Aktion.

### Command Palette (`Strg+K`)

Primärer Einstiegspunkt, nicht Zusatzfeature: Fuzzy-Suche über Befehle, Sprung zu
Kunde/System (Kürzel oder Name), Volltextsuche über Einträge aus derselben
Eingabezeile, kontextabhängige Befehle je aktueller Ansicht, Shortcut-Anzeige neben
jedem Befehl.

### Tastaturbelegung

| Taste | Wirkung |
|---|---|
| `Strg+K` | Command Palette |
| `Strg+N` | Neuer Eintrag im aktuellen Kontext |
| `Strg+S` | Speichern |
| `/` | Suche fokussieren |
| `g c` | Zu Kundenliste |
| `g s` | Zu Systemliste des aktuellen Kunden |
| `g j` | Zum Journal |
| `j` / `k` | Liste abwärts / aufwärts |
| `Enter` | Auswählen/öffnen |
| `e` | Ausgewählten Eintrag bearbeiten |
| `Strg+V` | Screenshot aus Zwischenablage als Anhang |
| `Esc` | Abbrechen, Ebene zurück |
| `?` | Shortcut-Übersicht |

Vim-artige Sequenzen und Modifier-Shortcuts nebeneinander. Textfelder unterdrücken
Einzeltasten-Bindings, `Strg+K` bleibt immer aktiv. Fokus jederzeit sichtbar,
Tab-Reihenfolge folgt visueller Anordnung, Dialoge fangen Fokus ein und geben ihn beim
Schließen ans auslösende Element zurück.

## Kernworkflows

**Eintrag erfassen**: `Strg+K` → Kundenkürzel → Enter (Kontext gesetzt) → `Strg+N`
(Titelfeld fokussiert, `performed_at` vorbelegt, letztes System vorausgewählt) →
Zeitstempel bei Bedarf per Tastatur ändern (absolut `07.09.2026 14:32` oder relativ
`-2h`, `gestern 9:15`, sofort ausgeschrieben mit Zone angezeigt) → Tab in Markdown-
Body → `Strg+V` fügt Screenshot ein, legt ihn im Attachment-Store ab, schreibt
Markdown-Referenz an Cursorposition → `Strg+S`.

**Wiederfinden**: Volltextsuche mit Treffer-Hervorhebung und Kontextzeile, Filter nach
Kunde/System/Kategorie/Tag/Zeitraum (alle tastaturbedienbar), Journal-Ansicht
(chronologisch absteigend, filterbar), Systemansicht (Historie je System).

**Anhänge**: Einfügen per Zwischenablage/Dateiauswahl (tastaturbedienbar)/Drag&Drop.
Bilder im Markdown-Vorschaubereich gerendert, Nicht-Bilder als Liste am Eintrag.
Öffnen/Exportieren/Entfernen per Tastatur.

**Export** (pro Kunde, optional gefiltert nach System/Zeitraum):
- **Markdown**: Verzeichnis, eine Datei je System, Anhänge mitkopiert, relative Pfade
- **PDF**: durchgehendes Handbuch — Deckblatt, Inhaltsverzeichnis, Gliederung nach
  System, eingebettete Bilder, Kopf-/Fußzeile mit Kundenname und Erstellungsdatum

Jeder Eintrag trägt in beiden Formaten den vollständigen Tätigkeits-Zeitstempel mit
ausgeschriebener Zone; bei "nachträglich erfasst" zusätzlich den Erfassungszeitpunkt.
Export selbst trägt eigenen Erstellungszeitstempel im gleichen Format.

## Fehlerbehandlung & Transaktionen

Ein `AppError`-Enum (`thiserror`), an Frontend als `{code, message}` serialisiert,
dort als Banner/Toast sichtbar — nie stillschweigend verschluckt. Anhang-Import
(Datei-Kopie + DB-Insert) läuft als eine logische Transaktion: DB-Insert in
SQLite-Transaktion, Datei-Kopie davor abgeschlossen; schlägt der DB-Teil fehl, wird
die kopierte Datei wieder entfernt (außer sie war durch Dedup bereits vorhanden — dann
bleibt sie, da von anderem Eintrag referenziert oder referenzierbar). Kein Eintrag
ohne vollständigen Anhang-Datensatz. Verwaiste Dateien im Attachment-Store werden über
ein aufrufbares `cleanup_orphans`-Kommando erkannt/entfernt, nie automatisch im
Hintergrund.

## Plugin-Architektur (Vorbereitung, jetzt nicht befüllt)

```rust
trait Plugin {
    fn id(&self) -> &str;
    fn list_systems(&self, credentials: &PluginCredentials)
        -> Result<Vec<ExternalSystem>, PluginError>;
    fn get_system_details(&self, credentials: &PluginCredentials, external_id: &str)
        -> Result<serde_json::Value, PluginError>;
    fn link_system(&self, system_id: i64, external_id: &str) -> Result<(), PluginError>;
}
```

- Plugins laufen isoliert, kein direkter SQLite-Zugriff — nur über obiges Interface,
  Aufrufer (Anwendung) schreibt Ergebnis in `external_refs`
- Zugangsdaten im OS-Schlüsselspeicher (`keyring`-Crate), nie im Klartext in DB/Config
- Von Plugins gelieferte Daten sichtbar als "extern" markiert, überschreiben nie
  selbst gepflegte Inhalte (Merge-Regel: eigene Felder haben Vorrang, externe Felder
  nur ergänzend dargestellt)
- Jeder Plugin-Aufruf per Tastatur auslösbar (Command-Palette-Einträge)
- Dummy-Implementierung (`DummyPlugin`) als dokumentiertes Beispiel, liefert
  statische Testdaten

## Gestaltung

An Linear/VS Code orientiert, dicht aber lesbar, kein verschwendeter Weißraum.
Dunkles und helles Theme. Monospace für Hostnames/IPs/Code. Keine Animationen, die
Eingabe verzögern.

## Nicht-Ziele

Keine Mehrbenutzerfähigkeit/Rechteverwaltung/Synchronisation. Keine
Zeiterfassung/Abrechnung. Keine Vorlagen/Checklisten. Keine Wiki-Seiten neben dem
Journal. Kein Ticketsystem/Workflow-Status.

## Qualitätsanforderungen

Fehler verständlich angezeigt, nie stillschweigend verschluckt. Schreibvorgänge
transaktional. Verwaiste Anhang-Dateien nur über aufrufbare Bereinigungsfunktion.
Schnellerfassung < 150 ms nach Hotkey, sofort tippbereit. Bescheidener
Leerlauf-Speicherbedarf für dauerhaften Hintergrundbetrieb. Start < 1 s, flüssig
durchsuchbar bei mehreren zehntausend Einträgen. Tests für Datenschicht, Migrationen,
Suche (Rust-seitig, `cargo test`).

## Lieferumfang

1. Vollständiges, lauffähiges Projekt, Build-Anleitung Windows/Linux
2. DB-Schema als Migrationsdateien
3. `SHORTCUTS.md` mit vollständiger Tastaturbelegung
4. Architekturnotiz zur Plugin-Erweiterung

## Reihenfolge der Umsetzung

Datenmodell/Migrationen → Backend-Kommandos → Tray-Betrieb mit globalem Hotkey und
Schnellerfassung → Command Palette/Navigation → Editor/Anhänge → Export. Nach jedem
Abschnitt Zwischenstand zeigen.
