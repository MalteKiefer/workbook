# Phase 5 — Anhang-Lebenszyklus: Backend-Kommandos

## Ziel

Backend-Unterstützung für den vollständigen Lebenszyklus von Anhängen an bereits
gespeicherten Einträgen: auflisten, hinzufügen, lesen (als Data-URL für Vorschau),
exportieren (Datei-Kopie an Zielpfad), entfernen (nur DB-Zeile) und im Dateisystem
öffnen. Zusätzlich eine explizit aufrufbare, niemals automatische Bereinigung
verwaister Dateien im Attachment-Store (`cleanup_orphans`), wie in der Spec unter
"Fehlerbehandlung & Transaktionen" gefordert: „Verwaiste Dateien im Attachment-Store
werden über ein aufrufbares `cleanup_orphans`-Kommando erkannt/entfernt, nie
automatisch im Hintergrund."

Das ist bewusst getrennt vom bestehenden Pending-Attachment-Flow der
Schnellerfassung (`db::entries` löst `pending_attachments` beim Speichern eines
*neuen* Eintrags auf) — dieser Phase-5-Baustein deckt Anhänge an einem *bereits
existierenden* Eintrag ab (Editor-Ansicht).

## Bausteine

### 1. `db/attachments.rs` — Repository-Erweiterungen

- `get(conn, id) -> Result<Attachment, AppError>`: `SELECT * ... WHERE id = ?1`,
  `.optional()?.ok_or_else(NotFound)`, exakt im Stil von `db::customers::get`.
- `delete(conn, id) -> Result<(), AppError>`: löscht nur die DB-Zeile
  (`DELETE FROM attachments WHERE id = ?1`), `NotFound` wenn `changed == 0`. Rührt
  die Datei im Content-Addressed-Store nicht an — Dedup bedeutet, dass eine Datei
  von mehreren Anhang-Zeilen referenziert sein kann; Datei-Bereinigung ist bewusst
  eine separate Funktion (siehe Punkt 2/3).
- `all_referenced_hashes(conn) -> Result<HashSet<String>, AppError>`: alle
  `DISTINCT sha256` aus `attachments`, Basis für Orphan-Erkennung.
- Unit-Tests im bestehenden `#[cfg(test)] mod tests`-Block: `get` (Treffer und
  `NotFound`), `delete` (entfernt Zeile, `NotFound` bei unbekannter ID, UND belegt
  per Test, dass die zugehörige Datei auf der Platte nach `delete` weiterhin
  existiert).

### 2. `attachments/store.rs` — Enumeration des Dateisystem-Stores

- `StoredFile { sha256, absolute_path, size_bytes }` und
  `list_all_stored_files(data_dir) -> Result<Vec<StoredFile>, AppError>`: iteriert
  `data_dir/attachments/<prefix>/<hash><ext>`, liest den Hash aus dem Dateinamen
  (Stem), keine erneute Hash-Berechnung nötig. Kein leeres Verzeichnis → leeres
  Ergebnis statt Fehler. Nicht-hexadecimal wirkende Dateinamen werden bewusst nicht
  gefiltert — der `attachments/`-Ordner gehört exklusiv dieser App, ein
  Fremdkörper darin soll konservativ als "nicht referenziert" gelten.
- Unit-Test mit `tempfile::tempdir()`: zwei Dateien anlegen, eine davon zusätzlich
  per DB-Zeile referenzieren, `list_all_stored_files` muss beide mit korrekter
  Größe finden.

### 3. `commands/attachments.rs` — neue Tauri-Kommandos (7 Stück)

Alle folgen dem Stil aus `commands/entries.rs`
(`state.pool.get().map_err(...)`, `state.config.lock().expect("Config-Mutex vergiftet")`).

1. `list_attachments_for_entry(entry_id) -> Vec<Attachment>`
2. `add_attachment_to_entry(entry_id, bytes_base64, original_filename, mime_type) -> Attachment`
   — dekodiert Base64 (gleiches Muster wie `db/entries.rs`s Pending-Attachment-Auflösung),
   ruft `attachments::store::attach_bytes_to_entry` (bereits vorhandene transaktionale
   Save-then-Insert-Logik mit Rollback) wieder.
3. `remove_attachment(attachment_id) -> ()` — nur DB-Zeile, siehe oben.
4. `read_attachment_data_url(attachment_id) -> String` — liest Datei von Platte,
   liefert `data:<mime>;base64,<...>` für Inline-Vorschau im Frontend.
5. `copy_attachment_to(attachment_id, dest_path) -> ()` — Datei-Export an
   Nutzer-gewählten Zielpfad (Dialog wird von anderem Agenten im Frontend gebaut).
6. `cleanup_orphans() -> CleanupResult { removed_count, removed_bytes }` — Differenz
   aus `list_all_stored_files` und `all_referenced_hashes`, löscht nur unreferenzierte
   Dateien. Ausschließlich manuell aufrufbar, keine Aufrufstelle in Hintergrund-Timern
   o.ä.
7. `open_attachment(attachment_id) -> ()` — öffnet die Datei mit dem
   Betriebssystem-Standardprogramm über `tauri-plugin-opener`.

### 4. Neue Abhängigkeiten

- `tauri-plugin-opener` (Cargo) + Capability `"opener:default"` — für
  `open_attachment`.
- `tauri-plugin-dialog` (Cargo + `@tauri-apps/plugin-dialog` npm) + Capability
  `"dialog:default"` — für den "Speichern unter"-Dialog beim Export; wird von
  einem anderen Agenten im Frontend verdrahtet, dieser Baustein macht das Plugin
  nur verfügbar/registrierbar.

### 5. Nicht Teil dieses Bausteins

- Registrierung der neuen Kommandos und Plugins in `lib.rs`s `invoke_handler!`
  bzw. `.plugin(...)`-Kette — das bleibt einer separaten Integrationsrunde
  vorbehalten (siehe Abgrenzung/Boundaries in der Aufgabenstellung). Der
  erfolgreiche `cargo build`/`cargo test` dieses Bausteins bestätigt nur, dass die
  neuen Funktionen für sich genommen typprüfen.
- Frontend-Anbindung (Editor-UI für Anhänge, Export-Dialog-Aufruf) — reines
  Rust-Backend-Ticket.

## Tests

- `db/attachments.rs`: `get_returns_existing_attachment`,
  `get_returns_not_found_for_unknown_id`, `delete_removes_row`,
  `delete_returns_not_found_for_unknown_id`,
  `delete_does_not_touch_file_on_disk`.
- `attachments/store.rs`: `list_all_stored_files_finds_referenced_and_unreferenced_files`
  (oder ähnlich benannt).
- Bestehende Suite (62 Tests zuletzt) bleibt grün, plus die neuen Tests.
