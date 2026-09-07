# Wartungsdoku — Phase 3b-1: Attachment-Store — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Content-addressed file storage for attachments (SHA-256, dedup, transactional
rollback), plus wiring `entries::create`/`update` to accept pending attachments (raw
bytes pasted before the entry exists) and resolve their markdown references once the
entry is saved. This is the backend primitive the quick-capture window's `Strg+V` and
the `Strg+Alt+S` global hotkey both need — pulled forward from Phase 5 ("Editor und
Anhänge") because both Phase-3 features are meaningless without it, and building it once
here means Phase 5's full editor reuses this unchanged instead of re-deriving it.

**Architecture:** `attachments/store.rs` owns the filesystem side (hash, path naming,
write, dedup) and is Tauri-independent (pure `&Path`/`&[u8]` in, testable with
`tempfile`). `db/attachments.rs` owns the `attachments` table. A pending attachment
travels from frontend to backend as `{ placeholder_token, bytes_base64, original_filename,
mime_type }`; `entries::create`/`update` insert the entry first (FK requires
`entry_id`), then for each pending attachment: save file, insert its row, and replace
its placeholder token in `body_md` with the real content-addressed relative path — all
before returning the saved `Entry`.

**Tech Stack:** `sha2` (hashing), `base64` (frontend sends bytes as base64 over IPC).

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
**Phase 3a plan (done):** [docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase3a.md](2026-09-07-wartungsdoku-plan-phase3a.md)

## Global Constraints

- Anhänge content-addressed (SHA-256) im Dateisystem, DB hält nur Hash/Name/MIME/Größe/Verknüpfung.
- Identische Dateien dedupliziert automatisch (gleicher Hash → gleicher Pfad, kein erneutes Schreiben).
- Anhang-Import ist transaktional: schlägt der DB-Insert fehl, wird eine *neu geschriebene* Datei wieder entfernt; eine bereits vorhandene (Dedup-Treffer) bleibt unangetastet, da sie von anderer Stelle referenziert sein kann.
- Kein Eintrag ohne vollständigen Anhang-Datensatz.
- Fehler nie stillschweigend verschluckt.

---

## Task 1: Content-addressed Dateispeicher

**Files:**
- Create: `src-tauri/src/attachments/store.rs`
- Create: `src-tauri/src/attachments/mod.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `pub struct SavedFile { sha256, relative_path: String, size_bytes: i64, newly_written: bool }`,
  `pub fn relative_path_for(sha256: &str, original_filename: &str) -> String`,
  `pub fn save_content_addressed(data_dir: &Path, bytes: &[u8], original_filename: &str) -> Result<SavedFile, AppError>`

- [x] **Step 1: Abhängigkeit hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo add sha2 && cd ..
```

- [x] **Step 2: `store.rs` mit failing tests schreiben**

```rust
// src-tauri/src/attachments/store.rs
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq)]
pub struct SavedFile {
    pub sha256: String,
    pub relative_path: String,
    pub size_bytes: i64,
    pub newly_written: bool,
}

pub fn relative_path_for(sha256: &str, original_filename: &str) -> String {
    let ext = Path::new(original_filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let prefix = &sha256[0..2];
    format!("attachments/{prefix}/{sha256}{ext}")
}

pub fn save_content_addressed(data_dir: &Path, bytes: &[u8], original_filename: &str) -> Result<SavedFile, AppError> {
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let relative_path = relative_path_for(&sha256, original_filename);
    let absolute_path = data_dir.join(&relative_path);
    let newly_written = !absolute_path.exists();
    if newly_written {
        if let Some(parent) = absolute_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&absolute_path, bytes)?;
    }
    Ok(SavedFile { sha256, relative_path, size_bytes: bytes.len() as i64, newly_written })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn saves_file_at_deterministic_content_addressed_path() {
        let dir = tempdir().unwrap();
        let saved = save_content_addressed(dir.path(), b"hello world", "screenshot.png").unwrap();
        assert!(saved.newly_written);
        assert_eq!(saved.size_bytes, 11);
        assert!(saved.relative_path.starts_with("attachments/"));
        assert!(saved.relative_path.ends_with(".png"));
        assert!(dir.path().join(&saved.relative_path).exists());
    }

    #[test]
    fn identical_bytes_deduplicate_without_rewriting() {
        let dir = tempdir().unwrap();
        let first = save_content_addressed(dir.path(), b"same content", "a.png").unwrap();
        assert!(first.newly_written);
        let second = save_content_addressed(dir.path(), b"same content", "b.png").unwrap();
        assert!(!second.newly_written);
        assert_eq!(first.sha256, second.sha256);
        // Extension kommt vom *ersten* Schreiben — Pfad ist über den Hash deterministisch,
        // nicht vom zweiten Aufruf abhängig.
        assert_eq!(first.relative_path, second.relative_path);
    }

    #[test]
    fn different_bytes_produce_different_hashes_and_paths() {
        let dir = tempdir().unwrap();
        let a = save_content_addressed(dir.path(), b"content a", "a.png").unwrap();
        let b = save_content_addressed(dir.path(), b"content b", "b.png").unwrap();
        assert_ne!(a.sha256, b.sha256);
        assert_ne!(a.relative_path, b.relative_path);
    }

    #[test]
    fn missing_extension_is_handled() {
        let dir = tempdir().unwrap();
        let saved = save_content_addressed(dir.path(), b"no extension", "README").unwrap();
        assert!(!saved.relative_path.contains('.') || saved.relative_path.matches('.').count() == 0);
    }
}
```

- [x] **Step 3: `mod.rs` schreiben, in `lib.rs` einhängen**

```rust
// src-tauri/src/attachments/mod.rs
pub mod store;
```

```rust
// src-tauri/src/lib.rs — Modul-Deklaration ergänzen
pub mod attachments;
```

- [x] **Step 4: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test attachments:: && cd ..
```

Expected: 4 Tests grün.

- [x] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/attachments src-tauri/src/lib.rs
git commit -m "feat: add content-addressed attachment file store"
```

---

## Task 2: Repository `attachments`

**Files:**
- Create: `src-tauri/src/db/attachments.rs`
- Modify: `src-tauri/src/db/mod.rs`

**Interfaces:**
- Produces: `pub struct Attachment { id, entry_id, sha256, original_filename, mime_type, size_bytes: i64, created_at_utc, created_at_tz }`,
  `pub fn create(conn, entry_id: i64, sha256: &str, original_filename: &str, mime_type: &str, size_bytes: i64, tz: &Tz) -> Result<Attachment, AppError>`,
  `pub fn list_for_entry(conn, entry_id: i64) -> Result<Vec<Attachment>, AppError>`

- [x] **Step 1: `attachments.rs` mit failing tests schreiben**

```rust
// src-tauri/src/db/attachments.rs
use chrono_tz::Tz;
use rusqlite::{params, Connection, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Attachment {
    pub id: i64,
    pub entry_id: i64,
    pub sha256: String,
    pub original_filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub created_at_utc: String,
    pub created_at_tz: String,
}

fn row_to_attachment(row: &Row) -> rusqlite::Result<Attachment> {
    Ok(Attachment {
        id: row.get("id")?,
        entry_id: row.get("entry_id")?,
        sha256: row.get("sha256")?,
        original_filename: row.get("original_filename")?,
        mime_type: row.get("mime_type")?,
        size_bytes: row.get("size_bytes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
    })
}

pub fn create(
    conn: &Connection,
    entry_id: i64,
    sha256: &str,
    original_filename: &str,
    mime_type: &str,
    size_bytes: i64,
    tz: &Tz,
) -> Result<Attachment, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO attachments (entry_id, sha256, original_filename, mime_type, size_bytes, created_at_utc, created_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![entry_id, sha256, original_filename, mime_type, size_bytes, now_utc, now_tz],
    )?;
    let id = conn.last_insert_rowid();
    conn.query_row("SELECT * FROM attachments WHERE id = ?1", params![id], row_to_attachment)
        .map_err(Into::into)
}

pub fn list_for_entry(conn: &Connection, entry_id: i64) -> Result<Vec<Attachment>, AppError> {
    let mut stmt = conn.prepare("SELECT * FROM attachments WHERE entry_id = ?1 ORDER BY created_at_utc ASC")?;
    let rows = stmt.query_map(params![entry_id], row_to_attachment)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::customers::{self, NewCustomer};
    use crate::db::entries::{self, Category, NewEntry};
    use crate::db::test_support::migrated_connection;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    fn seed_entry(conn: &Connection) -> i64 {
        let customer_id = customers::create(conn, NewCustomer { name: "ACME".into(), short_code: "ACME".into(), notes: "".into() }, &berlin()).unwrap().id;
        entries::create(
            conn,
            NewEntry {
                customer_id,
                system_id: None,
                title: "Titel".into(),
                body_md: "".into(),
                category: Category::Wartung,
                performed_at_utc: "2026-09-07T12:00:00.000Z".into(),
                performed_at_tz: "Europe/Berlin".into(),
                tag_names: vec![],
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    #[test]
    fn create_then_list_roundtrips() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn);
        let created = create(&conn, entry_id, "abc123", "screenshot.png", "image/png", 42, &berlin()).unwrap();
        assert_eq!(created.entry_id, entry_id);
        assert_eq!(list_for_entry(&conn, entry_id).unwrap(), vec![created]);
    }

    #[test]
    fn list_for_entry_without_attachments_is_empty() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn);
        assert!(list_for_entry(&conn, entry_id).unwrap().is_empty());
    }

    #[test]
    fn create_rejects_unknown_entry() {
        let conn = migrated_connection();
        let result = create(&conn, 999, "abc123", "x.png", "image/png", 1, &berlin());
        assert!(matches!(result, Err(AppError::Database(_))));
    }
}
```

`entries::create` wird hier bereits mit der *bisherigen* Signatur aus Phase 2 benutzt
(`create(conn, input, tz)`, drei Argumente) — Task 4 erweitert diese Signatur um
`data_dir` und `pending_attachments`. Bis dahin bleibt sie unverändert, dieser Test läuft
schon jetzt gegen den aktuellen Stand.

- [x] **Step 2: Modul einhängen**

```rust
// src-tauri/src/db/mod.rs — Zeile ergänzen
pub mod attachments;
```

- [x] **Step 3: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test db::attachments:: && cd ..
```

Expected: 3 Tests grün.

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/db/attachments.rs src-tauri/src/db/mod.rs
git commit -m "feat: add attachments repository"
```

---

## Task 3: Transaktionale Verknüpfung (Datei + DB-Zeile)

**Files:**
- Modify: `src-tauri/src/attachments/store.rs`

**Interfaces:**
- Consumes: `save_content_addressed` (Task 1), `db::attachments::create` (Task 2)
- Produces: `pub fn attach_bytes_to_entry(conn, data_dir: &Path, entry_id: i64, bytes: &[u8], original_filename: &str, mime_type: &str, tz: &Tz) -> Result<crate::db::attachments::Attachment, AppError>`

- [x] **Step 1: Funktion mit failing tests ergänzen**

```rust
// src-tauri/src/attachments/store.rs — ans Dateiende anfügen
use chrono_tz::Tz;
use rusqlite::Connection;

use crate::db::attachments::{self, Attachment};

pub fn attach_bytes_to_entry(
    conn: &Connection,
    data_dir: &Path,
    entry_id: i64,
    bytes: &[u8],
    original_filename: &str,
    mime_type: &str,
    tz: &Tz,
) -> Result<Attachment, AppError> {
    let saved = save_content_addressed(data_dir, bytes, original_filename)?;
    match attachments::create(conn, entry_id, &saved.sha256, original_filename, mime_type, saved.size_bytes, tz) {
        Ok(attachment) => Ok(attachment),
        Err(e) => {
            if saved.newly_written {
                let _ = std::fs::remove_file(data_dir.join(&saved.relative_path));
            }
            Err(e)
        }
    }
}
```

Tests (im selben `#[cfg(test)] mod tests` Block ergänzen):

```rust
    use crate::db::customers::{self, NewCustomer};
    use crate::db::entries::{self, Category, NewEntry};

    fn seed_entry(conn: &Connection, tz: &Tz) -> i64 {
        let customer_id = customers::create(conn, NewCustomer { name: "ACME".into(), short_code: "ACME".into(), notes: "".into() }, tz).unwrap().id;
        entries::create(
            conn,
            NewEntry {
                customer_id,
                system_id: None,
                title: "Titel".into(),
                body_md: "".into(),
                category: Category::Wartung,
                performed_at_utc: "2026-09-07T12:00:00.000Z".into(),
                performed_at_tz: "Europe/Berlin".into(),
                tag_names: vec![],
            },
            tz,
        )
        .unwrap()
        .id
    }

    #[test]
    fn attach_bytes_to_entry_writes_file_and_row_together() {
        let dir = tempdir().unwrap();
        let conn = crate::db::test_support::migrated_connection();
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let entry_id = seed_entry(&conn, &tz);

        let attachment = attach_bytes_to_entry(&conn, dir.path(), entry_id, b"png bytes", "shot.png", "image/png", &tz).unwrap();

        assert_eq!(attachment.entry_id, entry_id);
        let relative_path = relative_path_for(&attachment.sha256, "shot.png");
        assert!(dir.path().join(&relative_path).exists());
    }

    #[test]
    fn attach_bytes_to_entry_rolls_back_newly_written_file_on_db_failure() {
        let dir = tempdir().unwrap();
        let conn = crate::db::test_support::migrated_connection();
        let tz: Tz = "Europe/Berlin".parse().unwrap();

        // entry_id 999 existiert nicht -> FK-Verletzung -> DB-Insert schlägt fehl
        let result = attach_bytes_to_entry(&conn, dir.path(), 999, b"orphan bytes", "shot.png", "image/png", &tz);
        assert!(result.is_err());

        let sha256 = format!("{:x}", Sha256::digest(b"orphan bytes"));
        let relative_path = relative_path_for(&sha256, "shot.png");
        assert!(!dir.path().join(&relative_path).exists(), "neu geschriebene Datei hätte entfernt werden müssen");
    }

    #[test]
    fn attach_bytes_to_entry_keeps_deduplicated_file_on_db_failure() {
        let dir = tempdir().unwrap();
        let conn = crate::db::test_support::migrated_connection();
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let entry_id = seed_entry(&conn, &tz);

        // Erster Aufruf schreibt die Datei erfolgreich (gültiger entry_id).
        let first = attach_bytes_to_entry(&conn, dir.path(), entry_id, b"shared bytes", "shot.png", "image/png", &tz).unwrap();
        let relative_path = relative_path_for(&first.sha256, "shot.png");
        assert!(dir.path().join(&relative_path).exists());

        // Zweiter Aufruf mit identischen Bytes, aber ungültigem entry_id -> DB-Insert schlägt fehl,
        // Datei existierte aber schon (Dedup) -> darf NICHT gelöscht werden.
        let result = attach_bytes_to_entry(&conn, dir.path(), 999, b"shared bytes", "shot.png", "image/png", &tz);
        assert!(result.is_err());
        assert!(dir.path().join(&relative_path).exists(), "über Dedup geteilte Datei darf nicht gelöscht werden");
    }
```

- [x] **Step 2: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test attachments::store:: && cd ..
```

Expected: 7 Tests grün (4 aus Task 1 + 3 neue).

- [x] **Step 3: Commit**

```bash
git add src-tauri/src/attachments/store.rs
git commit -m "feat: transactionally link attachment file and database row"
```

---

## Task 4: `entries::create`/`update` um Pending-Attachments erweitern

**Files:**
- Modify: `src-tauri/src/db/entries.rs`
- Modify: `src-tauri/src/commands/entries.rs`

**Interfaces:**
- Consumes: `attachments::store::attach_bytes_to_entry` (Task 3)
- Produces: `pub struct PendingAttachment { placeholder_token, bytes_base64, original_filename, mime_type: String }`,
  neues Feld `pending_attachments: Vec<PendingAttachment>` auf `NewEntry`/`UpdateEntry`,
  neue Signatur `create(conn, data_dir: &Path, input: NewEntry, tz: &Tz)` und
  `update(conn, data_dir: &Path, id: i64, input: UpdateEntry, tz: &Tz)`

- [x] **Step 1: `PendingAttachment` + Feld auf `NewEntry`/`UpdateEntry` ergänzen**

```rust
// src-tauri/src/db/entries.rs — nach der EntryFilter-Definition einfügen
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PendingAttachment {
    pub placeholder_token: String,
    pub bytes_base64: String,
    pub original_filename: String,
    pub mime_type: String,
}
```

```rust
// NewEntry und UpdateEntry — jeweils letztes Feld ergänzen
    pub tag_names: Vec<String>,
    #[serde(default)]
    pub pending_attachments: Vec<PendingAttachment>,
```

- [x] **Step 2: `create` und `update` um `data_dir` und Anhang-Auflösung erweitern**

```rust
// src-tauri/src/db/entries.rs — create() ersetzen
pub fn create(conn: &Connection, data_dir: &std::path::Path, input: NewEntry, tz: &Tz) -> Result<Entry, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO entries (customer_id, system_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?8, ?9)",
        params![
            input.customer_id,
            input.system_id,
            input.title,
            input.body_md,
            input.category,
            input.performed_at_utc,
            input.performed_at_tz,
            now_utc,
            now_tz,
        ],
    )?;
    let id = conn.last_insert_rowid();
    tags::set_tags_for_entry(conn, id, &input.tag_names)?;
    resolve_pending_attachments(conn, data_dir, id, &input.pending_attachments, tz)?;
    get(conn, id)
}
```

```rust
// src-tauri/src/db/entries.rs — update() ersetzen
pub fn update(conn: &Connection, data_dir: &std::path::Path, id: i64, input: UpdateEntry, tz: &Tz) -> Result<Entry, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE entries SET system_id = ?1, title = ?2, body_md = ?3, category = ?4, performed_at_utc = ?5, performed_at_tz = ?6, updated_at_utc = ?7, updated_at_tz = ?8 WHERE id = ?9",
        params![
            input.system_id,
            input.title,
            input.body_md,
            input.category,
            input.performed_at_utc,
            input.performed_at_tz,
            now_utc,
            now_tz,
            id,
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Eintrag {id} nicht gefunden")));
    }
    tags::set_tags_for_entry(conn, id, &input.tag_names)?;
    resolve_pending_attachments(conn, data_dir, id, &input.pending_attachments, tz)?;
    get(conn, id)
}

fn resolve_pending_attachments(
    conn: &Connection,
    data_dir: &std::path::Path,
    entry_id: i64,
    pending: &[PendingAttachment],
    tz: &Tz,
) -> Result<(), AppError> {
    if pending.is_empty() {
        return Ok(());
    }
    use base64::prelude::*;
    let mut body_md: String = conn.query_row("SELECT body_md FROM entries WHERE id = ?1", params![entry_id], |r| r.get(0))?;
    for item in pending {
        let bytes = BASE64_STANDARD
            .decode(&item.bytes_base64)
            .map_err(|e| AppError::Config(format!("Anhang konnte nicht dekodiert werden: {e}")))?;
        let attachment = crate::attachments::store::attach_bytes_to_entry(
            conn, data_dir, entry_id, &bytes, &item.original_filename, &item.mime_type, tz,
        )?;
        let relative_path = crate::attachments::store::relative_path_for(&attachment.sha256, &item.original_filename);
        body_md = body_md.replace(&item.placeholder_token, &relative_path);
    }
    conn.execute("UPDATE entries SET body_md = ?1 WHERE id = ?2", params![body_md, entry_id])?;
    Ok(())
}
```

- [x] **Step 3: Abhängigkeit hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo add base64 && cd ..
```

- [x] **Step 4: Alle Aufrufstellen von `entries::create`/`update` auf die neue Signatur anpassen**

Betroffene bestehende Aufrufe (Phase 2, Task 4 und Task 2 dieser Datei) bekommen ein
zusätzliches `dir.path()`-Argument in ihren Tests:

```rust
// src-tauri/src/db/entries.rs — im eigenen #[cfg(test)] mod tests: jeden Aufruf
// `create(&conn, new_entry(...), &berlin())` ersetzen durch
// `create(&conn, tempfile::tempdir().unwrap().path(), new_entry(...), &berlin())`
```

Praktikabler: eine gemeinsame Test-Hilfsfunktion, die einen `TempDir` einmal pro Test
offen hält (der `TempDir`-Wert muss im Scope bleiben, sonst wird das Verzeichnis vor
Testende gelöscht):

```rust
// src-tauri/src/db/entries.rs — im #[cfg(test)] mod tests, vor den bestehenden Tests
    fn temp_data_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }
```

Jeder bestehende Testkörper bekommt zu Beginn `let data_dir = temp_data_dir();` und jeder
`create(&conn, ...)`/`update(&conn, ...)`-Aufruf wird um `data_dir.path()` als zweites
Argument ergänzt (z. B. `create(&conn, data_dir.path(), new_entry(...), &berlin())`).
Gleiches gilt für `src-tauri/src/db/attachments.rs::tests::seed_entry` (Task 2) und
`src-tauri/src/attachments/store.rs::tests::seed_entry` (Task 3) — beide rufen
`entries::create` auf und bekommen ebenfalls ein `data_dir.path()`-Argument; da diese
Tests bereits ein `dir = tempdir()` besitzen, wird genau dieses `dir.path()`
weitergereicht statt eines zweiten.

```rust
// src-tauri/src/commands/entries.rs — create_entry und update_entry anpassen
#[tauri::command]
pub fn create_entry(state: State<AppState>, input: NewEntry) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let data_dir = state.config.lock().expect("Config-Mutex vergiftet").data_dir.clone();
    entries::create(&conn, &data_dir, input, &tz)
}

#[tauri::command]
pub fn update_entry(state: State<AppState>, id: i64, input: UpdateEntry) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let data_dir = state.config.lock().expect("Config-Mutex vergiftet").data_dir.clone();
    entries::update(&conn, &data_dir, id, input, &tz)
}
```

- [x] **Step 5: Neuen Test für Anhang-Auflösung beim Anlegen ergänzen**

```rust
// src-tauri/src/db/entries.rs — im #[cfg(test)] mod tests ergänzen
    #[test]
    fn create_resolves_pending_attachment_placeholder_in_body() {
        use base64::prelude::*;
        let conn = migrated_connection();
        let data_dir = temp_data_dir();
        let customer_id = seed_customer(&conn);

        let mut input = new_entry(customer_id, "Mit Screenshot", "2026-09-07T12:00:00.000Z", Category::Wartung);
        input.body_md = "Vorher\n![Screenshot](pending:tok1)\nNachher".into();
        input.pending_attachments = vec![PendingAttachment {
            placeholder_token: "pending:tok1".into(),
            bytes_base64: BASE64_STANDARD.encode(b"fake png bytes"),
            original_filename: "screenshot.png".into(),
            mime_type: "image/png".into(),
        }];

        let created = create(&conn, data_dir.path(), input, &berlin()).unwrap();

        assert!(!created.body_md.contains("pending:tok1"));
        assert!(created.body_md.contains("attachments/"));
        assert!(created.body_md.contains(".png"));

        let sha256 = format!("{:x}", sha2::Sha256::digest(b"fake png bytes"));
        let relative_path = crate::attachments::store::relative_path_for(&sha256, "screenshot.png");
        assert!(data_dir.path().join(&relative_path).exists());
    }
```

`sha2` muss dafür auch als reguläre (nicht nur transitive) Abhängigkeit sichtbar sein —
ist sie bereits seit Task 1 (`cargo add sha2` lief ohne `--dev`).

- [x] **Step 6: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test db::entries:: db::attachments:: attachments::store:: && cd ..
```

`cargo test` akzeptiert nur ein Filterargument — stattdessen einfach den gesamten
Testlauf ausführen:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test && cd ..
```

Expected: alle bisherigen Tests weiterhin grün, plus der neue
`create_resolves_pending_attachment_placeholder_in_body`-Test.

- [x] **Step 7: Bin-Target bauen (Commands geändert)**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cd ..
```

- [x] **Step 8: Commit**

```bash
git add src-tauri/src/db/entries.rs src-tauri/src/db/attachments.rs src-tauri/src/attachments/store.rs src-tauri/src/commands/entries.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: resolve pending attachments into entry body on create/update"
```

---

## Ausführungsnotizen

- **Task 1**: `sha2` v0.11 (hybrid-array statt generic-array) implementiert `LowerHex`
  nicht mehr auf dem Digest-Ausgabetyp — `format!("{:x}", Sha256::digest(bytes))` aus
  dem Plan kompiliert nicht. Ersetzt durch eine manuelle `sha256_hex()`-Hilfsfunktion
  (`digest.as_ref()` → `&[u8]` → Byte-für-Byte-Hex), zusätzlich öffentlich gemacht, damit
  Task 4 sie in Tests wiederverwenden kann, statt die Hash-Logik zu duplizieren.
- **Task 4**: `base64` 0.23 (statt der im Plan angenommenen 0.22-Beispielsyntax) — die
  `base64::prelude::*` + `BASE64_STANDARD`-API war unverändert gültig, keine Anpassung nötig.

## Self-Review (durchgeführt vor Ausführung)

- **Spec-Abdeckung**: content-addressed Speicherung mit Dedup ✓, transaktionale Verknüpfung mit Rollback-Regel (Dedup-Dateien bleiben erhalten) ✓, Grundlage für "Strg+V fügt Screenshot ein, legt ihn im Attachment-Store ab, schreibt die Markdown-Referenz an die Cursorposition" ✓ (Cursor-Positionierung selbst ist Frontend/Editor-Sache, Phase 3b-2/5 — hier wird nur die Backend-Auflösung fertiggestellt).
- **Bewusst verschoben**: tatsächliches Lesen der Zwischenablage (Phase 3b-2, braucht `arboard` + die Schnellerfassungsfenster-UI, die den Cursor kennt), Anhänge öffnen/exportieren/entfernen und Nicht-Bild-Dateiliste (Phase 5), verwaiste-Dateien-Bereinigung (Phase 5, explizit "nie automatisch im Hintergrund").
- **Platzhalter-Scan**: keine TBD/TODO.
- **Typkonsistenz**: `attach_bytes_to_entry` (Task 3) nimmt exakt die `Attachment`-Struct-Felder, die `db::attachments::create` (Task 2) zurückgibt; `PendingAttachment` (Task 4) und `resolve_pending_attachments` verwenden `attachments::store::relative_path_for`/`attach_bytes_to_entry` (Task 1/3) mit identischen Parameternamen und -typen.
