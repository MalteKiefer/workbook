# Wartungsdoku — Phase 2: Backend-Kommandos — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repository layer (customers/systems/tags/entries/search) on top of the Phase 1
schema, a minimal Tauri app shell that boots with that data layer managed as state, a
minimal React/Vite frontend just sufficient to prove the IPC round-trip, and
`#[tauri::command]` wrappers exposing every repository function. No tray, no global
hotkeys, no quick-capture window, no real UI yet — those are Phase 3/4.

**Architecture:** Repository modules live under `src-tauri/src/db/` next to
`pool.rs`/`migrations.rs` (one file per table they own). Each repository function takes
`&rusqlite::Connection` and, where it writes a timestamp, `&chrono_tz::Tz` — it never
resolves the system timezone itself, that's the caller's job (kept easy to unit-test
with a fixed zone). `src-tauri/src/commands/` holds thin `#[tauri::command]` wrappers
that pull a connection from the pool, resolve the system timezone once, and delegate.
`AppState` (in `lib.rs`) holds the `DbPool` and the loaded `Config`.

**Tech Stack:** Same Rust stack as Phase 1, plus `tauri` 2.x, `tauri-build` 2.x
(build-dependency). Frontend: Vite + React + TypeScript, `@tauri-apps/api` for `invoke`.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
**Phase 1 plan (done):** [docs/superpowers/plans/2026-09-07-wartungsdoku-plan.md](2026-09-07-wartungsdoku-plan.md)

**This is Phase 2 of 6.** Next: Tray/Hotkey/Schnellerfassung → Command Palette/Navigation
→ Editor/Anhänge → Export.

## Global Constraints

(Carried over from Phase 1, still binding for every task below)
- Kein Feld vom Typ `DATE`; jede Zeitangabe = `_utc` (ISO 8601 UTC, ms) + `_tz` (IANA-Zone).
- Sortierung/Filterung ausschließlich über `_utc`. Zeitstempel entstehen ausschließlich in Rust.
- `performed_at_*` und `created_at_*` bleiben getrennt — `created_at` wird nach dem Anlegen nie verändert.
- `entries.category` bleibt exakt: `wartung`, `stoerung`, `aenderung`, `installation`, `sonstiges`.
- Fehler nie stillschweigend verschluckt — jede Repository-Funktion liefert `Result<T, AppError>`.

**Neu in dieser Phase:**
- Jede Repository-Funktion ist rein (nimmt `&Connection`, keine eigene Pool-Beschaffung) — Tests laufen ohne Tauri-Kontext.
- `AppError` bekommt eine `NotFound`-Variante für "existiert nicht"-Fälle (404-Äquivalent).
- Datenverzeichnis-Auflösung: `WARTUNGSDOKU_DATA_DIR`-Umgebungsvariable override, sonst Plattform-Default. `config.toml` liegt immer in genau diesem Verzeichnis — das Verzeichnis, in dem `config.toml` gefunden wird, *ist* das Datenverzeichnis (kein zirkulärer Verweis).

---

## Task 1: `AppError::NotFound` + Repository `customers`

**Files:**
- Modify: `src-tauri/src/error.rs`
- Create: `src-tauri/src/db/test_support.rs`
- Create: `src-tauri/src/db/customers.rs`
- Modify: `src-tauri/src/db/mod.rs`

**Interfaces:**
- Consumes: `AppError`, `time::now_with_tz` (Phase 1)
- Produces: `AppError::NotFound(String)` (code `"not_found"`);
  `crate::db::test_support::migrated_connection() -> rusqlite::Connection` (test-only);
  `pub struct Customer { id, name, short_code, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz, archived_at_utc: Option<String>, archived_at_tz: Option<String> }`,
  `pub struct NewCustomer { name, short_code, notes: String }`,
  `pub struct UpdateCustomer { name, short_code, notes: String }`,
  `pub fn create/get/list/update/archive(...)`

- [x] **Step 1: `NotFound`-Variante ergänzen**

```rust
// src-tauri/src/error.rs — Enum-Definition erweitern
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Konfigurationsfehler: {0}")]
    Config(String),
    #[error("Datenbankfehler: {0}")]
    Database(String),
    #[error("Migrationsfehler: {0}")]
    Migration(String),
    #[error("Ungültiger Zeitstempel: {0}")]
    InvalidTimestamp(String),
    #[error("Zeitzone konnte nicht ermittelt werden: {0}")]
    Timezone(String),
    #[error("I/O-Fehler: {0}")]
    Io(String),
    #[error("Nicht gefunden: {0}")]
    NotFound(String),
}
```

Und in `impl AppError::code`:

```rust
            AppError::Io(_) => "io",
            AppError::NotFound(_) => "not_found",
```

Test ergänzen in `error.rs`:

```rust
    #[test]
    fn not_found_has_not_found_code() {
        let err = AppError::NotFound("Kunde 42".to_string());
        assert_eq!(err.code(), "not_found");
    }
```

- [x] **Step 2: Test-Helper für migrierte Connection**

```rust
// src-tauri/src/db/test_support.rs
#![cfg(test)]
use rusqlite::Connection;

use crate::db::migrations::run_migrations;

pub fn migrated_connection() -> Connection {
    let tz: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    run_migrations(&mut conn, std::path::Path::new(":memory-marker:"), &tz).unwrap();
    conn
}
```

- [x] **Step 3: `customers.rs` mit failing tests schreiben**

```rust
// src-tauri/src/db/customers.rs
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Customer {
    pub id: i64,
    pub name: String,
    pub short_code: String,
    pub notes: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
    pub archived_at_utc: Option<String>,
    pub archived_at_tz: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewCustomer {
    pub name: String,
    pub short_code: String,
    pub notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateCustomer {
    pub name: String,
    pub short_code: String,
    pub notes: String,
}

fn row_to_customer(row: &Row) -> rusqlite::Result<Customer> {
    Ok(Customer {
        id: row.get("id")?,
        name: row.get("name")?,
        short_code: row.get("short_code")?,
        notes: row.get("notes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
        archived_at_utc: row.get("archived_at_utc")?,
        archived_at_tz: row.get("archived_at_tz")?,
    })
}

pub fn create(conn: &Connection, input: NewCustomer, tz: &Tz) -> Result<Customer, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO customers (name, short_code, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?5)",
        params![input.name, input.short_code, input.notes, now_utc, now_tz],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Customer, AppError> {
    conn.query_row("SELECT * FROM customers WHERE id = ?1", params![id], row_to_customer)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("Kunde {id} nicht gefunden")))
}

pub fn list(conn: &Connection, include_archived: bool) -> Result<Vec<Customer>, AppError> {
    let sql = if include_archived {
        "SELECT * FROM customers ORDER BY name COLLATE NOCASE"
    } else {
        "SELECT * FROM customers WHERE archived_at_utc IS NULL ORDER BY name COLLATE NOCASE"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], row_to_customer)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(conn: &Connection, id: i64, input: UpdateCustomer, tz: &Tz) -> Result<Customer, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE customers SET name = ?1, short_code = ?2, notes = ?3, updated_at_utc = ?4, updated_at_tz = ?5 WHERE id = ?6",
        params![input.name, input.short_code, input.notes, now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Kunde {id} nicht gefunden")));
    }
    get(conn, id)
}

pub fn archive(conn: &Connection, id: i64, tz: &Tz) -> Result<(), AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE customers SET archived_at_utc = ?1, archived_at_tz = ?2 WHERE id = ?3",
        params![now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Kunde {id} nicht gefunden")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::migrated_connection;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let created = create(
            &conn,
            NewCustomer { name: "ACME GmbH".into(), short_code: "ACME".into(), notes: "".into() },
            &berlin(),
        )
        .unwrap();
        assert_eq!(created.name, "ACME GmbH");
        assert_eq!(created.created_at_utc, created.updated_at_utc);
        assert!(created.archived_at_utc.is_none());
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn get_missing_customer_returns_not_found() {
        let conn = migrated_connection();
        assert!(matches!(get(&conn, 999), Err(AppError::NotFound(_))));
    }

    #[test]
    fn list_excludes_archived_by_default() {
        let conn = migrated_connection();
        let a = create(&conn, NewCustomer { name: "Aktiv".into(), short_code: "AKT".into(), notes: "".into() }, &berlin()).unwrap();
        let b = create(&conn, NewCustomer { name: "Archiviert".into(), short_code: "ARC".into(), notes: "".into() }, &berlin()).unwrap();
        archive(&conn, b.id, &berlin()).unwrap();

        let active = list(&conn, false).unwrap();
        assert_eq!(active.iter().map(|c| c.id).collect::<Vec<_>>(), vec![a.id]);
        assert_eq!(list(&conn, true).unwrap().len(), 2);
    }

    #[test]
    fn update_changes_fields_and_keeps_created_at() {
        let conn = migrated_connection();
        let created = create(&conn, NewCustomer { name: "Alt".into(), short_code: "ALT".into(), notes: "".into() }, &berlin()).unwrap();
        let updated = update(
            &conn,
            created.id,
            UpdateCustomer { name: "Neu".into(), short_code: "NEU".into(), notes: "geändert".into() },
            &berlin(),
        )
        .unwrap();
        assert_eq!(updated.name, "Neu");
        assert_eq!(updated.short_code, "NEU");
        assert_eq!(updated.created_at_utc, created.created_at_utc);
    }

    #[test]
    fn duplicate_short_code_is_rejected() {
        let conn = migrated_connection();
        create(&conn, NewCustomer { name: "Erster".into(), short_code: "DUP".into(), notes: "".into() }, &berlin()).unwrap();
        let result = create(&conn, NewCustomer { name: "Zweiter".into(), short_code: "DUP".into(), notes: "".into() }, &berlin());
        assert!(matches!(result, Err(AppError::Database(_))));
    }
}
```

- [x] **Step 4: Module einhängen**

```rust
// src-tauri/src/db/mod.rs
pub mod customers;
pub mod migrations;
pub mod pool;

#[cfg(test)]
pub mod test_support;
```

- [x] **Step 5: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test error:: db::customers:: && cd ..
```

Expected: 4 Tests aus `error.rs` + 5 Tests aus `db::customers` grün.

- [x] **Step 6: Commit**

```bash
git add src-tauri/src/error.rs src-tauri/src/db/test_support.rs src-tauri/src/db/customers.rs src-tauri/src/db/mod.rs
git commit -m "feat: add NotFound error and customers repository"
```

---

## Task 2: Repository `systems`

**Files:**
- Create: `src-tauri/src/db/systems.rs`
- Modify: `src-tauri/src/db/mod.rs`

**Interfaces:**
- Consumes: `customers::create` (Task 1, for test fixtures), `AppError`, `time::now_with_tz`
- Produces: `pub struct System { id, customer_id, name, system_type, hostname, ip_address, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz, archived_at_utc: Option<String>, archived_at_tz: Option<String> }`,
  `pub struct NewSystem`, `pub struct UpdateSystem`,
  `pub fn create/get/list_by_customer/update/archive(...)`

- [x] **Step 1: `systems.rs` mit failing tests schreiben**

```rust
// src-tauri/src/db/systems.rs
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct System {
    pub id: i64,
    pub customer_id: i64,
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
    pub archived_at_utc: Option<String>,
    pub archived_at_tz: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewSystem {
    pub customer_id: i64,
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateSystem {
    pub name: String,
    pub system_type: String,
    pub hostname: String,
    pub ip_address: String,
    pub notes: String,
}

fn row_to_system(row: &Row) -> rusqlite::Result<System> {
    Ok(System {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        name: row.get("name")?,
        system_type: row.get("system_type")?,
        hostname: row.get("hostname")?,
        ip_address: row.get("ip_address")?,
        notes: row.get("notes")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
        archived_at_utc: row.get("archived_at_utc")?,
        archived_at_tz: row.get("archived_at_tz")?,
    })
}

pub fn create(conn: &Connection, input: NewSystem, tz: &Tz) -> Result<System, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO systems (customer_id, name, system_type, hostname, ip_address, notes, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7, ?8)",
        params![input.customer_id, input.name, input.system_type, input.hostname, input.ip_address, input.notes, now_utc, now_tz],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<System, AppError> {
    conn.query_row("SELECT * FROM systems WHERE id = ?1", params![id], row_to_system)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("System {id} nicht gefunden")))
}

pub fn list_by_customer(conn: &Connection, customer_id: i64, include_archived: bool) -> Result<Vec<System>, AppError> {
    let sql = if include_archived {
        "SELECT * FROM systems WHERE customer_id = ?1 ORDER BY name COLLATE NOCASE"
    } else {
        "SELECT * FROM systems WHERE customer_id = ?1 AND archived_at_utc IS NULL ORDER BY name COLLATE NOCASE"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![customer_id], row_to_system)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn update(conn: &Connection, id: i64, input: UpdateSystem, tz: &Tz) -> Result<System, AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE systems SET name = ?1, system_type = ?2, hostname = ?3, ip_address = ?4, notes = ?5, updated_at_utc = ?6, updated_at_tz = ?7 WHERE id = ?8",
        params![input.name, input.system_type, input.hostname, input.ip_address, input.notes, now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("System {id} nicht gefunden")));
    }
    get(conn, id)
}

pub fn archive(conn: &Connection, id: i64, tz: &Tz) -> Result<(), AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    let changed = conn.execute(
        "UPDATE systems SET archived_at_utc = ?1, archived_at_tz = ?2 WHERE id = ?3",
        params![now_utc, now_tz, id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("System {id} nicht gefunden")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::customers::{self, NewCustomer};
    use crate::db::test_support::migrated_connection;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    fn seed_customer(conn: &Connection) -> i64 {
        customers::create(conn, NewCustomer { name: "ACME".into(), short_code: "ACME".into(), notes: "".into() }, &berlin())
            .unwrap()
            .id
    }

    #[test]
    fn create_then_get_roundtrips() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(
            &conn,
            NewSystem { customer_id, name: "FS01".into(), system_type: "Server".into(), hostname: "fs01.acme.local".into(), ip_address: "10.0.0.5".into(), notes: "".into() },
            &berlin(),
        )
        .unwrap();
        assert_eq!(created.hostname, "fs01.acme.local");
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn create_rejects_unknown_customer() {
        let conn = migrated_connection();
        let result = create(
            &conn,
            NewSystem { customer_id: 999, name: "Ghost".into(), system_type: "".into(), hostname: "".into(), ip_address: "".into(), notes: "".into() },
            &berlin(),
        );
        assert!(matches!(result, Err(AppError::Database(_))));
    }

    #[test]
    fn list_by_customer_excludes_archived_by_default() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let a = create(&conn, NewSystem { customer_id, name: "Aktiv".into(), system_type: "".into(), hostname: "".into(), ip_address: "".into(), notes: "".into() }, &berlin()).unwrap();
        let b = create(&conn, NewSystem { customer_id, name: "Alt".into(), system_type: "".into(), hostname: "".into(), ip_address: "".into(), notes: "".into() }, &berlin()).unwrap();
        archive(&conn, b.id, &berlin()).unwrap();

        let active = list_by_customer(&conn, customer_id, false).unwrap();
        assert_eq!(active.iter().map(|s| s.id).collect::<Vec<_>>(), vec![a.id]);
        assert_eq!(list_by_customer(&conn, customer_id, true).unwrap().len(), 2);
    }

    #[test]
    fn update_changes_fields_and_keeps_created_at() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, NewSystem { customer_id, name: "Alt".into(), system_type: "".into(), hostname: "".into(), ip_address: "".into(), notes: "".into() }, &berlin()).unwrap();
        let updated = update(
            &conn,
            created.id,
            UpdateSystem { name: "Neu".into(), system_type: "Firewall".into(), hostname: "fw.acme.local".into(), ip_address: "10.0.0.1".into(), notes: "".into() },
            &berlin(),
        )
        .unwrap();
        assert_eq!(updated.name, "Neu");
        assert_eq!(updated.created_at_utc, created.created_at_utc);
    }
}
```

- [x] **Step 2: Modul einhängen**

```rust
// src-tauri/src/db/mod.rs — Zeile ergänzen
pub mod systems;
```

- [x] **Step 3: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test db::systems:: && cd ..
```

Expected: 4 Tests grün.

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/db/systems.rs src-tauri/src/db/mod.rs
git commit -m "feat: add systems repository"
```

---

## Task 3: Repository `tags`

**Files:**
- Create: `src-tauri/src/db/tags.rs`
- Modify: `src-tauri/src/db/mod.rs`

**Interfaces:**
- Consumes: `AppError`
- Produces: `pub fn find_or_create(conn, name: &str) -> Result<i64, AppError>`,
  `pub fn set_tags_for_entry(conn, entry_id: i64, tag_names: &[String]) -> Result<(), AppError>`,
  `pub fn tags_for_entry(conn, entry_id: i64) -> Result<Vec<String>, AppError>`,
  `pub fn list_all(conn) -> Result<Vec<String>, AppError>`

Tests seed `customers`/`entries` rows via raw SQL (the `entries` repository module does
not exist yet — it is built in Task 4 and will *consume* this module).

- [x] **Step 1: `tags.rs` mit failing tests schreiben**

```rust
// src-tauri/src/db/tags.rs
use rusqlite::{params, Connection};

use crate::error::AppError;

pub fn find_or_create(conn: &Connection, name: &str) -> Result<i64, AppError> {
    conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", params![name])?;
    conn.query_row("SELECT id FROM tags WHERE name = ?1", params![name], |r| r.get(0))
        .map_err(Into::into)
}

pub fn set_tags_for_entry(conn: &Connection, entry_id: i64, tag_names: &[String]) -> Result<(), AppError> {
    conn.execute("DELETE FROM entry_tags WHERE entry_id = ?1", params![entry_id])?;
    for name in tag_names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tag_id = find_or_create(conn, trimmed)?;
        conn.execute(
            "INSERT OR IGNORE INTO entry_tags (entry_id, tag_id) VALUES (?1, ?2)",
            params![entry_id, tag_id],
        )?;
    }
    Ok(())
}

pub fn tags_for_entry(conn: &Connection, entry_id: i64) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT t.name FROM tags t JOIN entry_tags et ON et.tag_id = t.id WHERE et.entry_id = ?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(params![entry_id], |r| r.get::<_, String>(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn list_all(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare("SELECT name FROM tags ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::migrated_connection;

    fn seed_entry(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO customers (id, name, short_code, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 'ACME', 'ACME', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO entries (id, customer_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 1, 'Titel', '', 'wartung', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        )
        .unwrap();
        1
    }

    #[test]
    fn find_or_create_is_idempotent() {
        let conn = migrated_connection();
        let first = find_or_create(&conn, "exchange").unwrap();
        let second = find_or_create(&conn, "exchange").unwrap();
        assert_eq!(first, second);
        assert_eq!(list_all(&conn).unwrap(), vec!["exchange".to_string()]);
    }

    #[test]
    fn set_tags_for_entry_attaches_and_replaces() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn);

        set_tags_for_entry(&conn, entry_id, &["update".into(), "exchange".into()]).unwrap();
        assert_eq!(tags_for_entry(&conn, entry_id).unwrap(), vec!["exchange".to_string(), "update".to_string()]);

        set_tags_for_entry(&conn, entry_id, &["firewall".into()]).unwrap();
        assert_eq!(tags_for_entry(&conn, entry_id).unwrap(), vec!["firewall".to_string()]);
    }

    #[test]
    fn set_tags_for_entry_ignores_blank_names() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn);
        set_tags_for_entry(&conn, entry_id, &["  ".into(), "real".into()]).unwrap();
        assert_eq!(tags_for_entry(&conn, entry_id).unwrap(), vec!["real".to_string()]);
    }
}
```

- [x] **Step 2: Modul einhängen**

```rust
// src-tauri/src/db/mod.rs — Zeile ergänzen
pub mod tags;
```

- [x] **Step 3: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test db::tags:: && cd ..
```

Expected: 3 Tests grün.

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/db/tags.rs src-tauri/src/db/mod.rs
git commit -m "feat: add tags repository with entry association"
```

---

## Task 4: Repository `entries` (Kategorie + CRUD + Filter)

**Files:**
- Create: `src-tauri/src/db/entries.rs`
- Modify: `src-tauri/src/db/mod.rs`

**Interfaces:**
- Consumes: `customers::create` (Task 1), `tags::set_tags_for_entry`/`tags_for_entry` (Task 3), `AppError`, `time::now_with_tz`
- Produces: `pub enum Category { Wartung, Stoerung, Aenderung, Installation, Sonstiges }` (implements `ToSql`/`FromSql`/`Serialize`/`Deserialize`),
  `pub struct Entry { id, customer_id, system_id: Option<i64>, title, body_md, category: Category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz, tags: Vec<String> }`,
  `pub struct NewEntry`, `pub struct UpdateEntry`, `pub struct EntryFilter { customer_id, system_id: Option<i64>, category: Option<Category>, tag: Option<String>, from_utc: Option<String>, to_utc: Option<String> }`,
  `pub fn create/get/update/list(...)`

- [x] **Step 1: `entries.rs` mit failing tests schreiben**

```rust
// src-tauri/src/db/entries.rs
use chrono_tz::Tz;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::tags;
use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Wartung,
    Stoerung,
    Aenderung,
    Installation,
    Sonstiges,
}

impl Category {
    fn as_db_str(&self) -> &'static str {
        match self {
            Category::Wartung => "wartung",
            Category::Stoerung => "stoerung",
            Category::Aenderung => "aenderung",
            Category::Installation => "installation",
            Category::Sonstiges => "sonstiges",
        }
    }

    fn from_db_str(s: &str) -> Result<Self, AppError> {
        match s {
            "wartung" => Ok(Category::Wartung),
            "stoerung" => Ok(Category::Stoerung),
            "aenderung" => Ok(Category::Aenderung),
            "installation" => Ok(Category::Installation),
            "sonstiges" => Ok(Category::Sonstiges),
            other => Err(AppError::Database(format!("unbekannte category: {other}"))),
        }
    }
}

impl ToSql for Category {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_db_str()))
    }
}

impl FromSql for Category {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        Category::from_db_str(s).map_err(|_| FromSqlError::InvalidType)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Entry {
    pub id: i64,
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub title: String,
    pub body_md: String,
    pub category: Category,
    pub performed_at_utc: String,
    pub performed_at_tz: String,
    pub created_at_utc: String,
    pub created_at_tz: String,
    pub updated_at_utc: String,
    pub updated_at_tz: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewEntry {
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub title: String,
    pub body_md: String,
    pub category: Category,
    pub performed_at_utc: String,
    pub performed_at_tz: String,
    pub tag_names: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateEntry {
    pub system_id: Option<i64>,
    pub title: String,
    pub body_md: String,
    pub category: Category,
    pub performed_at_utc: String,
    pub performed_at_tz: String,
    pub tag_names: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct EntryFilter {
    pub customer_id: Option<i64>,
    pub system_id: Option<i64>,
    pub category: Option<Category>,
    pub tag: Option<String>,
    pub from_utc: Option<String>,
    pub to_utc: Option<String>,
}

fn row_to_entry(row: &Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get("id")?,
        customer_id: row.get("customer_id")?,
        system_id: row.get("system_id")?,
        title: row.get("title")?,
        body_md: row.get("body_md")?,
        category: row.get("category")?,
        performed_at_utc: row.get("performed_at_utc")?,
        performed_at_tz: row.get("performed_at_tz")?,
        created_at_utc: row.get("created_at_utc")?,
        created_at_tz: row.get("created_at_tz")?,
        updated_at_utc: row.get("updated_at_utc")?,
        updated_at_tz: row.get("updated_at_tz")?,
        tags: Vec::new(),
    })
}

pub fn create(conn: &Connection, input: NewEntry, tz: &Tz) -> Result<Entry, AppError> {
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
    get(conn, id)
}

pub fn get(conn: &Connection, id: i64) -> Result<Entry, AppError> {
    let mut entry = conn
        .query_row("SELECT * FROM entries WHERE id = ?1", params![id], row_to_entry)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("Eintrag {id} nicht gefunden")))?;
    entry.tags = tags::tags_for_entry(conn, id)?;
    Ok(entry)
}

pub fn update(conn: &Connection, id: i64, input: UpdateEntry, tz: &Tz) -> Result<Entry, AppError> {
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
    get(conn, id)
}

pub fn list(conn: &Connection, filter: &EntryFilter) -> Result<Vec<Entry>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM entries e
         WHERE (?1 IS NULL OR customer_id = ?1)
           AND (?2 IS NULL OR system_id = ?2)
           AND (?3 IS NULL OR category = ?3)
           AND (?4 IS NULL OR performed_at_utc >= ?4)
           AND (?5 IS NULL OR performed_at_utc <= ?5)
           AND (?6 IS NULL OR EXISTS (
                 SELECT 1 FROM entry_tags et JOIN tags t ON t.id = et.tag_id
                 WHERE et.entry_id = e.id AND t.name = ?6
               ))
         ORDER BY performed_at_utc DESC",
    )?;
    let category_str = filter.category.map(|c| c.as_db_str());
    let rows = stmt.query_map(
        params![filter.customer_id, filter.system_id, category_str, filter.from_utc, filter.to_utc, filter.tag],
        row_to_entry,
    )?;
    let mut result = Vec::new();
    for row in rows {
        let mut entry = row?;
        entry.tags = tags::tags_for_entry(conn, entry.id)?;
        result.push(entry);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::customers::{self, NewCustomer};
    use crate::db::test_support::migrated_connection;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    fn seed_customer(conn: &Connection) -> i64 {
        customers::create(conn, NewCustomer { name: "ACME".into(), short_code: "ACME".into(), notes: "".into() }, &berlin())
            .unwrap()
            .id
    }

    fn new_entry(customer_id: i64, title: &str, performed_at_utc: &str, category: Category) -> NewEntry {
        NewEntry {
            customer_id,
            system_id: None,
            title: title.into(),
            body_md: "Inhalt".into(),
            category,
            performed_at_utc: performed_at_utc.into(),
            performed_at_tz: "Europe/Berlin".into(),
            tag_names: vec!["exchange".into()],
        }
    }

    #[test]
    fn create_then_get_includes_tags() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, new_entry(customer_id, "Update", "2026-09-07T12:00:00.000Z", Category::Wartung), &berlin()).unwrap();
        assert_eq!(created.tags, vec!["exchange".to_string()]);
        assert_eq!(created.created_at_utc, created.updated_at_utc);
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn update_replaces_tags_and_keeps_created_at() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let created = create(&conn, new_entry(customer_id, "Update", "2026-09-07T12:00:00.000Z", Category::Wartung), &berlin()).unwrap();

        let updated = update(
            &conn,
            created.id,
            UpdateEntry {
                system_id: None,
                title: "Update v2".into(),
                body_md: "Neuer Inhalt".into(),
                category: Category::Stoerung,
                performed_at_utc: "2026-09-06T09:00:00.000Z".into(),
                performed_at_tz: "Europe/Berlin".into(),
                tag_names: vec!["firewall".into()],
            },
            &berlin(),
        )
        .unwrap();

        assert_eq!(updated.title, "Update v2");
        assert_eq!(updated.category, Category::Stoerung);
        assert_eq!(updated.tags, vec!["firewall".to_string()]);
        assert_eq!(updated.created_at_utc, created.created_at_utc);
    }

    #[test]
    fn list_orders_by_performed_at_descending() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let older = create(&conn, new_entry(customer_id, "Älter", "2026-09-01T10:00:00.000Z", Category::Wartung), &berlin()).unwrap();
        let newer = create(&conn, new_entry(customer_id, "Neuer", "2026-09-07T10:00:00.000Z", Category::Wartung), &berlin()).unwrap();

        let result = list(&conn, &EntryFilter::default()).unwrap();
        assert_eq!(result.iter().map(|e| e.id).collect::<Vec<_>>(), vec![newer.id, older.id]);
    }

    #[test]
    fn list_filters_by_category_and_date_range() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        create(&conn, new_entry(customer_id, "Wartung", "2026-09-05T10:00:00.000Z", Category::Wartung), &berlin()).unwrap();
        let stoerung = create(&conn, new_entry(customer_id, "Störung", "2026-09-06T10:00:00.000Z", Category::Stoerung), &berlin()).unwrap();

        let by_category = list(&conn, &EntryFilter { category: Some(Category::Stoerung), ..Default::default() }).unwrap();
        assert_eq!(by_category.iter().map(|e| e.id).collect::<Vec<_>>(), vec![stoerung.id]);

        let by_range = list(
            &conn,
            &EntryFilter { from_utc: Some("2026-09-06T00:00:00.000Z".into()), to_utc: Some("2026-09-07T00:00:00.000Z".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(by_range.iter().map(|e| e.id).collect::<Vec<_>>(), vec![stoerung.id]);
    }

    #[test]
    fn list_filters_by_tag() {
        let conn = migrated_connection();
        let customer_id = seed_customer(&conn);
        let with_tag = create(&conn, new_entry(customer_id, "Mit Tag", "2026-09-05T10:00:00.000Z", Category::Wartung), &berlin()).unwrap();
        create(
            &conn,
            NewEntry { tag_names: vec!["anderes".into()], ..new_entry(customer_id, "Ohne passendes Tag", "2026-09-06T10:00:00.000Z", Category::Wartung) },
            &berlin(),
        )
        .unwrap();

        let result = list(&conn, &EntryFilter { tag: Some("exchange".into()), ..Default::default() }).unwrap();
        assert_eq!(result.iter().map(|e| e.id).collect::<Vec<_>>(), vec![with_tag.id]);
    }
}
```

- [x] **Step 2: Modul einhängen**

```rust
// src-tauri/src/db/mod.rs — Zeile ergänzen
pub mod entries;
```

- [x] **Step 3: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test db::entries:: && cd ..
```

Expected: 5 Tests grün.

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/db/entries.rs src-tauri/src/db/mod.rs
git commit -m "feat: add entries repository with category, tags and filtered listing"
```

---

## Task 5: Repository `search` (Volltext + Verzeichnis)

**Files:**
- Create: `src-tauri/src/db/search.rs`
- Modify: `src-tauri/src/db/mod.rs`

**Interfaces:**
- Consumes: `AppError`
- Produces: `pub struct EntryHit { entry_id, customer_id, system_id: Option<i64>, title, snippet, performed_at_utc, performed_at_tz }`,
  `pub fn search_entries(conn, query: &str, limit: i64) -> Result<Vec<EntryHit>, AppError>`,
  `pub enum DirectoryKind { Customer, System }`, `pub struct DirectoryHit { kind, id, customer_id, label }`,
  `pub fn search_directory(conn, query: &str, limit: i64) -> Result<Vec<DirectoryHit>, AppError>`

- [x] **Step 1: `search.rs` mit failing tests schreiben**

```rust
// src-tauri/src/db/search.rs
use rusqlite::{params, Connection};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EntryHit {
    pub entry_id: i64,
    pub customer_id: i64,
    pub system_id: Option<i64>,
    pub title: String,
    pub snippet: String,
    pub performed_at_utc: String,
    pub performed_at_tz: String,
}

pub fn search_entries(conn: &Connection, query: &str, limit: i64) -> Result<Vec<EntryHit>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.customer_id, e.system_id, e.title, e.performed_at_utc, e.performed_at_tz,
                snippet(entries_fts, 1, '<mark>', '</mark>', '…', 12) AS snippet
         FROM entries_fts
         JOIN entries e ON e.id = entries_fts.rowid
         WHERE entries_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit], |row| {
        Ok(EntryHit {
            entry_id: row.get(0)?,
            customer_id: row.get(1)?,
            system_id: row.get(2)?,
            title: row.get(3)?,
            performed_at_utc: row.get(4)?,
            performed_at_tz: row.get(5)?,
            snippet: row.get(6)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryKind {
    Customer,
    System,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DirectoryHit {
    pub kind: DirectoryKind,
    pub id: i64,
    pub customer_id: i64,
    pub label: String,
}

pub fn search_directory(conn: &Connection, query: &str, limit: i64) -> Result<Vec<DirectoryHit>, AppError> {
    let sanitized: String = query.chars().filter(|c| *c != '%' && *c != '_').collect();
    let like = format!("%{sanitized}%");
    let mut stmt = conn.prepare(
        "SELECT 'customer' AS kind, id, id AS customer_id, name || ' (' || short_code || ')' AS label
         FROM customers
         WHERE archived_at_utc IS NULL AND (name LIKE ?1 COLLATE NOCASE OR short_code LIKE ?1 COLLATE NOCASE)
         UNION ALL
         SELECT 'system' AS kind, id, customer_id, name
         FROM systems
         WHERE archived_at_utc IS NULL AND (name LIKE ?1 COLLATE NOCASE OR hostname LIKE ?1 COLLATE NOCASE)
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![like, limit], |row| {
        let kind_str: String = row.get(0)?;
        Ok(DirectoryHit {
            kind: if kind_str == "customer" { DirectoryKind::Customer } else { DirectoryKind::System },
            id: row.get(1)?,
            customer_id: row.get(2)?,
            label: row.get(3)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::migrated_connection;

    fn seed(conn: &Connection) {
        conn.execute(
            "INSERT INTO customers (id, name, short_code, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 'ACME GmbH', 'ACME', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin'),
                    (2, 'Beispiel AG', 'BSP', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        ).unwrap();
        conn.execute(
            "UPDATE customers SET archived_at_utc = '2026-09-07T12:00:00.000Z', archived_at_tz = 'Europe/Berlin' WHERE id = 2",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO systems (id, customer_id, name, hostname, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 1, 'Fileserver', 'fs01.acme.local', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO entries (id, customer_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 1, 'Exchange Update', 'Kumulatives Update auf Server eingespielt', 'wartung',
                     '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        ).unwrap();
    }

    #[test]
    fn search_entries_finds_match_with_snippet() {
        let conn = migrated_connection();
        seed(&conn);
        let hits = search_entries(&conn, "kumulatives", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry_id, 1);
        assert!(hits[0].snippet.contains("<mark>"));
    }

    #[test]
    fn search_directory_finds_customer_by_short_code() {
        let conn = migrated_connection();
        seed(&conn);
        let hits = search_directory(&conn, "ACME", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, DirectoryKind::Customer);
        assert_eq!(hits[0].id, 1);
    }

    #[test]
    fn search_directory_excludes_archived_customers() {
        let conn = migrated_connection();
        seed(&conn);
        let hits = search_directory(&conn, "Beispiel", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_directory_finds_system_by_hostname_fragment() {
        let conn = migrated_connection();
        seed(&conn);
        let hits = search_directory(&conn, "fs01", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, DirectoryKind::System);
        assert_eq!(hits[0].customer_id, 1);
    }
}
```

- [x] **Step 2: Modul einhängen**

```rust
// src-tauri/src/db/mod.rs — Zeile ergänzen
pub mod search;
```

- [x] **Step 3: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test db::search:: && cd ..
```

Expected: 4 Tests grün.

- [x] **Step 4: Gesamten Rust-Testlauf verifizieren**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test && cd ..
```

Expected: alle Tests aus Phase 1 + Task 1–5 grün (21 + 1 + 5 + 4 + 3 + 5 + 4 = 43 Tests).

- [x] **Step 5: Commit**

```bash
git add src-tauri/src/db/search.rs src-tauri/src/db/mod.rs
git commit -m "feat: add entries full-text search and customer/system directory search"
```

---

## Task 6: Frontend-Grundgerüst (Vite + React + TypeScript)

**Files:**
- Create: `package.json`, `vite.config.ts`, `tsconfig.json`, `tsconfig.node.json`, `index.html`
- Create: `src/main.tsx`, `src/App.tsx`

**Interfaces:**
- Produces: ein per `npm run build` baubares Frontend unter `dist/`, das `invoke("list_customers", { includeArchived: false })` aus `@tauri-apps/api/core` aufruft (Konsument von `commands::customers::list_customers`, Task 8).

- [x] **Step 1: `package.json` anlegen und Pakete installieren**

```bash
cat > package.json <<'EOF'
{
  "name": "wartungsdoku",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview"
  }
}
EOF
npm install react react-dom
npm install -D typescript vite @vitejs/plugin-react @types/react @types/react-dom @tauri-apps/api @tauri-apps/cli
```

- [x] **Step 2: Vite-/TS-Konfiguration schreiben**

```typescript
// vite.config.ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    outDir: "dist",
  },
});
```

```json
// tsconfig.json
{
  "compilerOptions": {
    "target": "ES2021",
    "useDefineForClassFields": true,
    "lib": ["ES2021", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

```json
// tsconfig.node.json
{
  "compilerOptions": {
    "composite": true,
    "skipLibCheck": true,
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true
  },
  "include": ["vite.config.ts"]
}
```

- [x] **Step 3: `index.html` und React-Einstieg schreiben**

```html
<!-- index.html -->
<!doctype html>
<html lang="de">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Wartungsdoku</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

```tsx
// src/main.tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

```tsx
// src/App.tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

export default function App() {
  const [customers, setCustomers] = useState<Customer[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false })
      .then(setCustomers)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <main style={{ fontFamily: "sans-serif", padding: "2rem" }}>
      <h1>Wartungsdoku</h1>
      <p>Backend-Kommandos verdrahtet. Command Palette und Editor folgen in späteren Phasen.</p>
      {error && <p style={{ color: "crimson" }}>Fehler: {error}</p>}
      {customers && <p>Kunden in der Datenbank: {customers.length}</p>}
    </main>
  );
}
```

- [x] **Step 4: Build verifizieren** (schlägt hier noch fehl, da `@tauri-apps/api` zur Laufzeit `invoke` nur innerhalb einer Tauri-Webview auflöst — der TypeScript-Build selbst muss dennoch grün sein)

```bash
npm run build
```

Expected: `dist/index.html` und `dist/assets/*.js` werden erzeugt, kein TypeScript-Fehler.

- [x] **Step 5: Commit**

```bash
git add package.json package-lock.json vite.config.ts tsconfig.json tsconfig.node.json index.html src/main.tsx src/App.tsx
git commit -m "feat: add minimal Vite/React frontend scaffold with IPC smoke test"
```

---

## Task 7: Tauri-App-Verdrahtung

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/config.rs`
- Create: `src-tauri/build.rs`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/capabilities/default.json`
- Create: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `config::Config`, `db::pool::build_pool`, `db::migrations::run_migrations`, `time::system_timezone` (Phase 1/2)
- Produces: `pub fn config::resolve_data_dir() -> PathBuf`, `pub struct AppState { pool: DbPool, config: Mutex<Config> }`, `pub fn run()` (Tauri-Einstiegspunkt, noch ohne registrierte Commands — die kommen in Task 8)

- [x] **Step 1: `resolve_data_dir` ergänzen (löst den zirkulären Verweis config.toml↔data_dir auf)**

```rust
// src-tauri/src/config.rs — nach `fn default_data_dir` einfügen
pub fn resolve_data_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("WARTUNGSDOKU_DATA_DIR") {
        return PathBuf::from(override_dir);
    }
    default_data_dir()
}
```

Test ergänzen:

```rust
    #[test]
    fn resolve_data_dir_honours_env_override() {
        // SAFETY: Tests laufen sequenziell innerhalb dieses Prozesses für diese eine Variable.
        std::env::set_var("WARTUNGSDOKU_DATA_DIR", "/tmp/wartungsdoku-test-override");
        let resolved = resolve_data_dir();
        std::env::remove_var("WARTUNGSDOKU_DATA_DIR");
        assert_eq!(resolved, PathBuf::from("/tmp/wartungsdoku-test-override"));
    }
```

- [x] **Step 2: Tauri-Abhängigkeiten hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo add tauri
cargo add tauri-build --build
cargo add serde_json
cd ..
```

- [x] **Step 3: `build.rs` schreiben**

```rust
// src-tauri/build.rs
fn main() {
    tauri_build::build();
}
```

- [x] **Step 4: `tauri.conf.json` schreiben**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Wartungsdoku",
  "version": "0.1.0",
  "identifier": "de.maltekiefer.wartungsdoku",
  "build": {
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build",
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "label": "main",
        "title": "Wartungsdoku",
        "width": 1200,
        "height": 800,
        "minWidth": 800,
        "minHeight": 600
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": []
  }
}
```

- [x] **Step 5: Capability-Datei schreiben**

```json
// src-tauri/capabilities/default.json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main-capability",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:path:default",
    "core:event:default",
    "core:window:default",
    "core:app:default",
    "core:resources:default",
    "core:menu:default",
    "core:tray:default",
    "core:window:allow-set-title"
  ]
}
```

- [x] **Step 6: `main.rs` schreiben**

```rust
// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    wartungsdoku_lib::run();
}
```

- [x] **Step 7: `lib.rs` mit `AppState` und `run()` erweitern**

```rust
// src-tauri/src/lib.rs
pub mod config;
pub mod db;
pub mod error;
pub mod time;

pub use error::AppError;

use std::sync::Mutex;

use crate::config::Config;
use crate::db::pool::{build_pool, DbPool};

pub struct AppState {
    pub pool: DbPool,
    pub config: Mutex<Config>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let data_dir = config::resolve_data_dir();
    let config_path = data_dir.join("config.toml");
    let mut app_config = Config::load_or_default(&config_path).expect("Konfiguration konnte nicht geladen werden");
    app_config.data_dir = data_dir.clone();
    app_config.save(&config_path).expect("Konfiguration konnte nicht gespeichert werden");

    let db_path = data_dir.join("wartungsdoku.db");
    let pool = build_pool(&db_path).expect("Datenbank-Pool konnte nicht erstellt werden");
    {
        let system_tz = time::system_timezone().expect("Systemzeitzone konnte nicht ermittelt werden");
        let mut conn = pool.get().expect("Keine Datenbankverbindung verfügbar");
        db::migrations::run_migrations(&mut conn, &db_path, &system_tz).expect("Migration fehlgeschlagen");
    }

    tauri::Builder::default()
        .manage(AppState { pool, config: Mutex::new(app_config) })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

(Noch ohne `invoke_handler` — Commands und ihre Registrierung folgen in Task 8, weil sie
selbst noch nicht existieren.)

- [x] **Step 8: Bibliothek weiter testen, Binary bauen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo test config::
cargo build --bin wartungsdoku
cd ..
```

Expected: `resolve_data_dir_honours_env_override` grün; `cargo build --bin wartungsdoku`
kompiliert ohne Fehler (lädt dabei `tauri`, `tauri-build` u. a. neu herunter — erster
Lauf kann mehrere Minuten dauern).

- [x] **Step 9: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/build.rs src-tauri/tauri.conf.json src-tauri/capabilities/default.json src-tauri/src/main.rs src-tauri/src/lib.rs src-tauri/src/config.rs
git commit -m "feat: wire minimal Tauri app shell with managed data-layer state"
```

---

## Task 8: `#[tauri::command]`-Wrapper für alle Repository-Funktionen

**Files:**
- Create: `src-tauri/src/commands/mod.rs`
- Create: `src-tauri/src/commands/customers.rs`
- Create: `src-tauri/src/commands/systems.rs`
- Create: `src-tauri/src/commands/tags.rs`
- Create: `src-tauri/src/commands/entries.rs`
- Create: `src-tauri/src/commands/search.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: alle Repository-Funktionen aus Task 1–5, `AppState`, `time::system_timezone`, `time::parse_temporal_input`
- Produces: registrierte Tauri-Commands `list_customers`, `create_customer`, `update_customer`, `archive_customer`, `list_systems`, `create_system`, `update_system`, `archive_system`, `list_tags`, `list_entries`, `get_entry`, `create_entry`, `update_entry`, `parse_temporal_input`, `search_entries`, `search_directory`

- [x] **Step 1: `commands/customers.rs` schreiben**

```rust
// src-tauri/src/commands/customers.rs
use tauri::State;

use crate::db::customers::{self, Customer, NewCustomer, UpdateCustomer};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_customers(state: State<AppState>, include_archived: bool) -> Result<Vec<Customer>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    customers::list(&conn, include_archived)
}

#[tauri::command]
pub fn create_customer(state: State<AppState>, input: NewCustomer) -> Result<Customer, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    customers::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_customer(state: State<AppState>, id: i64, input: UpdateCustomer) -> Result<Customer, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    customers::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn archive_customer(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    customers::archive(&conn, id, &tz)
}
```

- [x] **Step 2: `commands/systems.rs` schreiben**

```rust
// src-tauri/src/commands/systems.rs
use tauri::State;

use crate::db::systems::{self, NewSystem, System, UpdateSystem};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_systems(state: State<AppState>, customer_id: i64, include_archived: bool) -> Result<Vec<System>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    systems::list_by_customer(&conn, customer_id, include_archived)
}

#[tauri::command]
pub fn create_system(state: State<AppState>, input: NewSystem) -> Result<System, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_system(state: State<AppState>, id: i64, input: UpdateSystem) -> Result<System, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn archive_system(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::archive(&conn, id, &tz)
}
```

- [x] **Step 3: `commands/tags.rs` schreiben**

```rust
// src-tauri/src/commands/tags.rs
use tauri::State;

use crate::db::tags;
use crate::{AppError, AppState};

#[tauri::command]
pub fn list_tags(state: State<AppState>) -> Result<Vec<String>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    tags::list_all(&conn)
}
```

- [x] **Step 4: `commands/entries.rs` schreiben (inkl. Zeitstempel-Parser-Command)**

```rust
// src-tauri/src/commands/entries.rs
use tauri::State;

use crate::db::entries::{self, Entry, EntryFilter, NewEntry, UpdateEntry};
use crate::{time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TemporalPreview {
    pub utc: String,
    pub tz: String,
}

#[tauri::command]
pub fn list_entries(state: State<AppState>, filter: EntryFilter) -> Result<Vec<Entry>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    entries::list(&conn, &filter)
}

#[tauri::command]
pub fn get_entry(state: State<AppState>, id: i64) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    entries::get(&conn, id)
}

#[tauri::command]
pub fn create_entry(state: State<AppState>, input: NewEntry) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    entries::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_entry(state: State<AppState>, id: i64, input: UpdateEntry) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    entries::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn parse_temporal_input(input: String) -> Result<TemporalPreview, AppError> {
    let tz = time::system_timezone()?;
    let now = chrono::Utc::now();
    let (utc, tz_name) = time::parse_temporal_input(&input, &tz, now)?;
    Ok(TemporalPreview { utc, tz: tz_name })
}
```

- [x] **Step 5: `commands/search.rs` schreiben**

```rust
// src-tauri/src/commands/search.rs
use tauri::State;

use crate::db::search::{self, DirectoryHit, EntryHit};
use crate::{AppError, AppState};

#[tauri::command]
pub fn search_entries(state: State<AppState>, query: String, limit: i64) -> Result<Vec<EntryHit>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    search::search_entries(&conn, &query, limit)
}

#[tauri::command]
pub fn search_directory(state: State<AppState>, query: String, limit: i64) -> Result<Vec<DirectoryHit>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    search::search_directory(&conn, &query, limit)
}
```

- [x] **Step 6: `commands/mod.rs` schreiben**

```rust
// src-tauri/src/commands/mod.rs
pub mod customers;
pub mod entries;
pub mod search;
pub mod systems;
pub mod tags;
```

- [x] **Step 7: In `lib.rs` einhängen und im `invoke_handler` registrieren**

```rust
// src-tauri/src/lib.rs — Modul-Deklarationen ergänzen
pub mod commands;
pub mod config;
pub mod db;
pub mod error;
pub mod time;
```

```rust
// src-tauri/src/lib.rs — im run(): .manage(...) direkt gefolgt von
        .invoke_handler(tauri::generate_handler![
            commands::customers::list_customers,
            commands::customers::create_customer,
            commands::customers::update_customer,
            commands::customers::archive_customer,
            commands::systems::list_systems,
            commands::systems::create_system,
            commands::systems::update_system,
            commands::systems::archive_system,
            commands::tags::list_tags,
            commands::entries::list_entries,
            commands::entries::get_entry,
            commands::entries::create_entry,
            commands::entries::update_entry,
            commands::entries::parse_temporal_input,
            commands::search::search_entries,
            commands::search::search_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
```

- [x] **Step 8: Bauen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo build --bin wartungsdoku
cargo test
cd ..
```

Expected: Binary kompiliert; alle Rust-Tests (43 aus Task 1–5, unverändert) weiterhin grün — `commands/` hat keine eigenen Tests, es sind reine Wrapper, deren Typkorrektheit der Compiler prüft.

- [x] **Step 9: Commit**

```bash
git add src-tauri/src/commands src-tauri/src/lib.rs
git commit -m "feat: expose repository layer as Tauri commands"
```

---

## Task 9: End-to-End-Verifikation

**Files:** keine neuen — nur Ausführung.

- [x] **Step 1: Frontend bauen**

```bash
npm run build
```

Expected: `dist/` aktuell, kein Fehler.

- [x] **Step 2: App im Hintergrund starten und Boot-Verhalten prüfen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export WARTUNGSDOKU_DATA_DIR="$(pwd)/.smoke-test-data"
cd src-tauri
timeout 8 cargo run --bin wartungsdoku > ../smoke-test.log 2>&1
cd ..
cat smoke-test.log
```

Expected: kein Rust-`panic!`/`expect`-Abbruch im Log (Migration lief durch, Pool wurde
erstellt, Fenster wurde erzeugt). Ein Abbruch durch `timeout` nach 8s ist normal (die App
läuft dauerhaft) und kein Fehlschlag.

- [x] **Step 3: Datenverzeichnis der Smoke-Test-Instanz prüfen**

```bash
ls -la .smoke-test-data
```

Expected: `wartungsdoku.db`, `wartungsdoku.db-wal`, `wartungsdoku.db-shm`, `config.toml`
vorhanden — bestätigt, dass `resolve_data_dir` + Migration + Pool beim echten App-Start
zusammenspielen.

- [x] **Step 4: Smoke-Test-Artefakte aufräumen (nicht committen)**

```bash
rm -rf .smoke-test-data smoke-test.log
```

- [x] **Step 5: Plan-Datei mit abgehaktem Stand committen**

```bash
git add docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase2.md
git commit -m "docs: mark Phase 2 plan tasks complete"
```

---

## Ausführungsnotizen (während der Umsetzung entstanden)

- **Task 5**: `search_directory_finds_customer_by_short_code` schlug beim ersten Lauf fehl
  (erwartet 1 Treffer, tatsächlich 2) — Testannahme war falsch, nicht die Implementierung:
  die Abfrage `"ACME"` matcht korrekt sowohl den Kunden (Name/Kürzel) als auch das System
  (Hostname `fs01.acme.local` enthält `acme`, `COLLATE NOCASE`). Test in zwei Fälle
  aufgeteilt: ein eindeutiger Treffer (`"GmbH"`) und ein Fall, der bewusst beide Treffer
  prüft. Damit hat Phase 2 einen Test mehr als ursprünglich vorhergesagt (44 statt 43 nach
  Task 5, 45 nach Task 7 durch `resolve_data_dir_honours_env_override`).
- **Task 7**: `cargo build --bin wartungsdoku` schlug beim ersten Versuch fehl —
  `icons/icon.ico` fehlte (unter Windows für die Ressourcen-Datei zwingend, auch für
  Debug-Builds, nicht nur fürs Bundling). Mit `tauri icon` aus einem generierten
  Platzhalter-Monogramm (`src-tauri/icons-source/icon-source.png`) ein Icon-Set erzeugt;
  mobile/Windows-Store-Assets, die der Generator zusätzlich anlegt, wieder entfernt (kein
  Ziel laut Spec). Danach kompilierte der Bin-Zielsatz sauber.
- **Task 9**: Echter App-Start (`cargo run`, 8s über `timeout` begrenzt) lief ohne
  `panic!`/`expect`-Abbruch durch; die einzige Log-Zeile
  (`Failed to unregister class Chrome_WidgetWin_0`) ist ein bekannter, harmloser
  WebView2-Cleanup-Hinweis beim harten Prozessabbruch. Datenverzeichnis enthielt danach
  `wartungsdoku.db`, `.db-wal`, `.db-shm`, `config.toml` und `wartungsdoku.db.bak-0` —
  Letzteres bestätigt, dass SQLite die Datei bereits beim Verbindungsaufbau anlegt (noch
  vor der ersten Migration), weshalb selbst der allererste Start ein (leeres) Backup
  erzeugt. Erwartetes, unschädliches Verhalten, kein Fehler.

## Self-Review (durchgeführt vor Ausführung)

- **Spec-Abdeckung**: Kunden-/System-CRUD mit Archivierung ✓, Einträge mit Kategorie/Tags/Zeitstempel-Trennung ✓, Volltextsuche mit Snippet/Hervorhebung ✓, Verzeichnis-Suche über Kunden-/Systemnamen ✓ (Ambiguität aus dem Spec-Dokument jetzt konkret als LIKE-Abfrage entschieden), Zeitstempel-Parser als Command exponiert (wird von der Schnellerfassung in Phase 3 und dem Editor in Phase 5 wiederverwendet) ✓, Tauri-App bootet mit verwaltetem State ✓.
- **Bewusst verschoben**: Tray/Hotkeys/Schnellerfassungsfenster (Phase 3), Command Palette/Navigation (Phase 4), Anhang-Dateispeicher — die `attachments`-Tabelle existiert bereits (Phase 1), aber keine Repository-Funktionen dafür (Phase 5, da inhaltsadressierte Ablage eng mit dem Editor-Workflow verzahnt ist), PDF/Markdown-Export (Phase 6).
- **Platzhalter-Scan**: keine TBD/TODO; jeder Schritt enthält lauffähigen Code.
- **Typkonsistenz**: `Category` (Task 4) wird identisch in `commands/entries.rs` (Task 8) verwendet; `EntryFilter`-Feldnamen (`customer_id`, `system_id`, `category`, `tag`, `from_utc`, `to_utc`) stimmen zwischen Repository (Task 4) und Command-Parameter (Task 8) überein; `AppState { pool, config }` (Task 7) wird in jedem `commands/*.rs`-Wrapper (Task 8) identisch destrukturiert.
