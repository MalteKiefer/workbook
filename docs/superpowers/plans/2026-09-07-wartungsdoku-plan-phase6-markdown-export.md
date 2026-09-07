# Phase 6 — Export: Markdown

## Ziel

Markdown-Export für einen Kunden (optional gefiltert nach System/Zeitraum, via
bestehender `EntryFilter`): ein Zielverzeichnis mit einer `.md`-Datei je System
(plus einer "Ohne System"-Datei für kundenweite Einträge ohne `system_id`),
Anhänge in identischer relativer Verzeichnisstruktur mitkopiert, sodass der
Export in jedem Markdown-Werkzeug lesbar bleibt (Spec, Abschnitt "Export" unter
Kernworkflows).

Jeder Eintrag trägt den vollständigen `performed_at`-Zeitstempel mit
ausgeschriebener Zone; weicht `created_at` um mehr als den konfigurierbaren
Schwellwert (`late_entry_threshold_hours`, Default 24h) ab, wird zusätzlich der
Erfassungszeitpunkt ausgewiesen ("nachträglich erfasst"). Der Export selbst
trägt einen eigenen Erstellungszeitstempel im gleichen Format.

## Die zentrale Vereinfachung: keine Pfad-Neuschreibung nötig

`attachments/store.rs::relative_path_for(sha256, original_filename)` erzeugt
`attachments/{erste 2 Hex-Zeichen}/{sha256}{.ext}`, relativ zum
App-Datenverzeichnis. Jede Anhang-Referenz, die bereits in `entry.body_md`
steht (z. B. `![Screenshot](attachments/a3/f2c9e1....png)`), verwendet exakt
dieses gleiche Schema. Kopiert der Export die referenzierten Dateien unter
derselben relativen Struktur in sein Zielverzeichnis, bleibt `body_md`
unverändert lesbar — keine Markdown-Text-Manipulation, kein Link-Rewriting,
nur ein `std::fs::copy` je Anhang an denselben relativen Pfad unterhalb des
Zielverzeichnisses.

## Bausteine

### 1. `src-tauri/src/export/markdown.rs` (neu, Modul `export` neu)

`pub fn export_markdown(conn, data_dir, dest_dir, customer_id, filter: &EntryFilter, late_entry_threshold_hours) -> Result<(), AppError>`:

- Lädt Kunde (`customers::get`) und alle Systeme des Kunden
  (`systems::list_by_customer(..., include_archived = true)`, damit auch
  Einträge an inzwischen archivierten Systemen im Export landen) für die
  System-Namens-Auflösung.
- Lädt Einträge über `entries::list(conn, filter)` (liefert `performed_at_utc
  DESC`), kehrt die Reihenfolge um für chronologischen Export (älteste zuerst).
- Gruppiert Einträge nach `system_id` (`None` → "Ohne System"), Dateiname pro
  Gruppe `sanitize_filename(system_label) + ".md"`.
- Je Gruppe: Markdown-Datei mit Überschrift, Export-Erstellungszeitstempel,
  dann je Eintrag Titel, Zeitpunkt/Kategorie/Tags, optionaler
  "Nachträglich erfasst"-Hinweis, `body_md` unverändert, danach die Anhänge
  kopiert (Zielpfad = `dest_dir.join(relative_path_for(...))`, Elternverzeichnis
  per `create_dir_all`).
- `category_label(Category) -> &'static str`: eigene kleine Funktion in diesem
  Modul (nicht in `db/entries.rs`, das bleibt unverändert), da `Category` keine
  öffentliche deutsche Label-Methode hat.
- `late_entry_note(entry, threshold_hours) -> Result<Option<String>, AppError>`:
  parst `performed_at_utc`/`created_at_utc` (beide RFC3339), Differenz in
  Stunden, ab Schwellwert ein `"Nachträglich erfasst: <formatiertes
  created_at>"` via `time::format_timestamp_for_display`.

### 2. `src-tauri/src/commands/export.rs` (neu, sofern nicht bereits von
   parallelem PDF-Export-Agenten angelegt — dann dort ergänzen statt
   überschreiben)

`#[tauri::command] pub fn export_markdown(state, customer_id, system_id,
from_utc, to_utc, dest_dir) -> Result<(), AppError>`: baut `EntryFilter` aus
den Parametern, holt `data_dir`/`late_entry_threshold_hours` aus
`state.config`, ruft `export::markdown::export_markdown`.

### 3. Nicht Teil dieses Bausteins

- Registrierung des neuen `pub mod export;` in `lib.rs` und des Kommandos im
  `invoke_handler!` — bleibt der Integrationsrunde vorbehalten (Boundaries der
  Aufgabenstellung).
- PDF-Export (`export/pdf.rs`, `export/markdown_to_typst.rs`) — separater
  paralleler Auftrag, hier nicht angefasst.
- Frontend-Anbindung (Export-Dialog-Aufruf) — reines Rust-Backend-Ticket.

## Tests (`#[cfg(test)] mod tests` in `export/markdown.rs`)

- Kunde + zwei Systeme + Einträge auf beiden Systemen + ein Eintrag ohne
  System anlegen, `export_markdown` aufrufen: erwartete `.md`-Dateien
  existieren im Ziel-Temp-Verzeichnis, enthalten die erwarteten Eintragstitel,
  "Ohne System"-Datei existiert für den systemlosen Eintrag.
- Eintrag mit Anhang (`attachments::store::attach_bytes_to_entry`) exportieren:
  Anhang-Datei liegt danach unter demselben relativen Pfad im Zielverzeichnis.
- `late_entry_note` direkt als Unit getestet: ein Eintrag kurz nach
  `performed_at` erstellt (kein Hinweis), einer deutlich später (Hinweis
  erwartet) — mit handgebauten `Entry`-Werten, da die Repository-Funktionen
  `created_at` nicht frei kontrollierbar machen.
