# Wartungsdoku — Phase 1: Datenmodell & Migrationen — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pure Rust data layer for the Wartungsdoku app — SQLite schema (all tables from
the spec), a forward-only migration runner with pre-migration backup, a WAL/foreign-keys
connection pool, timestamp generation/parsing, and config.toml load/save. No Tauri
wiring yet — that is Phase 2.

**Architecture:** A standalone Rust library crate at `src-tauri/` (no `[[bin]]` target
yet). Later phases add the Tauri binary on top of this same crate without restructuring
it. Every timestamp-producing function lives in `time.rs` and is called from
everywhere else — no other module formats a timestamp by hand.

**Tech Stack:** Rust, `rusqlite` (bundled SQLite, gives FTS5 — confirmed via
`libsqlite3-sys` build script, which passes `-DSQLITE_ENABLE_FTS5` unconditionally for
bundled builds), `r2d2`/`r2d2_sqlite` pool, `chrono`/`chrono-tz`/`iana-time-zone`,
`thiserror`, `serde`/`toml`, `dirs`, `regex`, `tempfile` (dev-dependency).

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)

**This is Phase 1 of 6** (order fixed by the user): Datenmodell/Migrationen (this plan)
→ Backend-Kommandos → Tray/Hotkey/Schnellerfassung → Command Palette/Navigation →
Editor/Anhänge → Export. Each later phase gets its own plan, written after this one is
reviewed — per the user's explicit "pause after each section" instruction.

## Global Constraints

- Kein Feld vom Typ `DATE`. Jede Zeitangabe = `<feld>_utc` (TEXT, ISO 8601 UTC, Millisekunden, `...Z`) + `<feld>_tz` (TEXT, IANA-Zonenname).
- Sortierung/Filterung/Vergleich ausschließlich über die `_utc`-Spalte.
- Zeitstempel werden ausschließlich serverseitig in Rust erzeugt, nie im Frontend.
- `performed_at_*` (Tätigkeitszeitpunkt) und `created_at_*` (Erfassungszeitpunkt) sind getrennte Felder, nie miteinander verrechnet.
- Migrationen sind vorwärtsgerichtet; vor jeder Anwendung wird die `.db`-Datei gesichert.
- `PRAGMA foreign_keys = ON` und `PRAGMA journal_mode = WAL` gelten für jede Connection.
- `entries.category` ist eine feste Liste: `wartung`, `stoerung`, `aenderung`, `installation`, `sonstiges`.
- Anhänge werden content-addressed (SHA-256) im Dateisystem gespeichert, nicht als BLOB in der DB (Schema jetzt, Speicherlogik folgt in Phase 5).
- Fehler werden nie stillschweigend verschluckt — jeder Fehlerpfad liefert eine `AppError`-Variante mit verständlicher Meldung.

---

## Task 1: Crate-Grundgerüst + Fehlertyp

**Files:**
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/lib.rs`
- Create: `src-tauri/src/error.rs`

**Interfaces:**
- Produces: `pub enum AppError { Config(String), Database(String), Migration(String), InvalidTimestamp(String), Timezone(String), Io(String) }`, `impl From<rusqlite::Error> for AppError`, `impl From<std::io::Error> for AppError`, `impl serde::Serialize for AppError` (fields `code: &str`, `message: String`)

- [ ] **Step 1: Verzeichnis und Cargo.toml anlegen**

```bash
mkdir -p src-tauri/src
cat > src-tauri/Cargo.toml <<'EOF'
[package]
name = "wartungsdoku"
version = "0.1.0"
edition = "2021"

[lib]
name = "wartungsdoku_lib"
path = "src/lib.rs"
EOF
```

- [ ] **Step 2: Abhängigkeiten hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo add rusqlite --features bundled
cargo add r2d2
cargo add r2d2_sqlite
cargo add chrono --features serde
cargo add chrono-tz
cargo add iana-time-zone
cargo add thiserror
cargo add serde --features derive
cargo add toml
cargo add dirs
cargo add regex
cargo add tempfile --dev
cd ..
```

- [ ] **Step 3: `error.rs` schreiben (inkl. Unit-Tests)**

```rust
// src-tauri/src/error.rs
use thiserror::Error;

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
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Config(_) => "config",
            AppError::Database(_) => "database",
            AppError::Migration(_) => "migration",
            AppError::InvalidTimestamp(_) => "invalid_timestamp",
            AppError::Timezone(_) => "timezone",
            AppError::Io(_) => "io",
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Database(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_error_has_database_code_and_readable_message() {
        let err = AppError::Database("UNIQUE constraint failed".to_string());
        assert_eq!(err.code(), "database");
        assert_eq!(err.to_string(), "Datenbankfehler: UNIQUE constraint failed");
    }

    #[test]
    fn serializes_to_code_and_message_json() {
        let err = AppError::InvalidTimestamp("07.09.2026".to_string());
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(
            json,
            r#"{"code":"invalid_timestamp","message":"Ungültiger Zeitstempel: 07.09.2026"}"#
        );
    }

    #[test]
    fn io_error_converts_from_std_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "datei fehlt");
        let app_err: AppError = io_err.into();
        assert_eq!(app_err.code(), "io");
    }
}
```

The serialization test needs `serde_json` as a dev-dependency too:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo add serde_json --dev && cd ..
```

- [ ] **Step 4: `lib.rs` schreiben**

```rust
// src-tauri/src/lib.rs
pub mod error;

pub use error::AppError;
```

- [ ] **Step 5: Bauen und Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test && cd ..
```

Expected: 3 tests passed (`database_error_has_database_code_and_readable_message`,
`serializes_to_code_and_message_json`, `io_error_converts_from_std_io_error`).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/src/error.rs
git commit -m "feat: scaffold Rust data-layer crate with AppError"
```

---

## Task 2: Zeitmodul (Erzeugung + Parsing)

**Files:**
- Create: `src-tauri/src/time.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `AppError` from Task 1
- Produces: `pub fn system_timezone() -> Result<chrono_tz::Tz, AppError>`,
  `pub fn now_with_tz(tz: &chrono_tz::Tz) -> (String, String)` (returns
  `(utc_iso_ms, tz_name)`),
  `pub fn parse_temporal_input(input: &str, tz: &chrono_tz::Tz, now_utc: chrono::DateTime<chrono::Utc>) -> Result<(String, String), AppError>`

- [ ] **Step 1: Failing tests schreiben**

```rust
// src-tauri/src/time.rs
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;

use crate::error::AppError;

pub fn system_timezone() -> Result<Tz, AppError> {
    let name = iana_time_zone::get_timezone()
        .map_err(|e| AppError::Timezone(e.to_string()))?;
    name.parse::<Tz>()
        .map_err(|_| AppError::Timezone(format!("unbekannte Zone: {name}")))
}

pub fn now_with_tz(tz: &Tz) -> (String, String) {
    format_utc_with_tz(Utc::now(), tz)
}

fn format_utc_with_tz(dt: DateTime<Utc>, tz: &Tz) -> (String, String) {
    (
        dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        tz.name().to_string(),
    )
}

pub fn parse_temporal_input(
    input: &str,
    tz: &Tz,
    now_utc: DateTime<Utc>,
) -> Result<(String, String), AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(format_utc_with_tz(now_utc, tz));
    }

    if let Some(caps) = relative_pattern().captures(trimmed) {
        let amount: i64 = caps[1].parse().unwrap();
        let unit = &caps[2];
        let delta = match unit {
            "h" => Duration::hours(amount),
            "m" => Duration::minutes(amount),
            "d" => Duration::days(amount),
            _ => unreachable!(),
        };
        return Ok(format_utc_with_tz(now_utc - delta, tz));
    }

    if let Some(caps) = yesterday_pattern().captures(trimmed) {
        let hour: u32 = caps[1].parse().unwrap();
        let minute: u32 = caps[2].parse().unwrap();
        let local_now = now_utc.with_timezone(tz);
        let yesterday_date = local_now.date_naive() - Duration::days(1);
        return localize(yesterday_date, hour, minute, tz, trimmed);
    }

    if let Some(caps) = absolute_pattern().captures(trimmed) {
        let day: u32 = caps[1].parse().unwrap();
        let month: u32 = caps[2].parse().unwrap();
        let year: i32 = caps[3].parse().unwrap();
        let hour: u32 = caps[4].parse().unwrap();
        let minute: u32 = caps[5].parse().unwrap();
        let date = NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| AppError::InvalidTimestamp(trimmed.to_string()))?;
        return localize(date, hour, minute, tz, trimmed);
    }

    Err(AppError::InvalidTimestamp(trimmed.to_string()))
}

fn localize(
    date: NaiveDate,
    hour: u32,
    minute: u32,
    tz: &Tz,
    original_input: &str,
) -> Result<(String, String), AppError> {
    let naive = date
        .and_hms_opt(hour, minute, 0)
        .ok_or_else(|| AppError::InvalidTimestamp(original_input.to_string()))?;
    let local: NaiveDateTime = naive;
    match tz.from_local_datetime(&local).single() {
        Some(dt) => Ok(format_utc_with_tz(dt.with_timezone(&Utc), tz)),
        None => Err(AppError::InvalidTimestamp(format!(
            "{original_input} (mehrdeutig oder ungültig durch Zeitumstellung)"
        ))),
    }
}

fn relative_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^-(\d+)([hmd])$").unwrap())
}

fn yesterday_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^gestern\s+(\d{1,2}):(\d{2})$").unwrap())
}

fn absolute_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^(\d{2})\.(\d{2})\.(\d{4})\s+(\d{1,2}):(\d{2})$").unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    fn fixed_now() -> DateTime<Utc> {
        // 2026-09-07T12:00:00.000Z == 2026-09-07 14:00 Europe/Berlin (CEST, UTC+2)
        Utc.with_ymd_and_hms(2026, 9, 7, 12, 0, 0).unwrap()
    }

    #[test]
    fn empty_input_returns_now() {
        let (utc, tz) = parse_temporal_input("", &berlin(), fixed_now()).unwrap();
        assert_eq!(utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(tz, "Europe/Berlin");
    }

    #[test]
    fn relative_hours_subtracts_from_now() {
        let (utc, _) = parse_temporal_input("-2h", &berlin(), fixed_now()).unwrap();
        assert_eq!(utc, "2026-09-07T10:00:00.000Z");
    }

    #[test]
    fn relative_days_subtracts_from_now() {
        let (utc, _) = parse_temporal_input("-1d", &berlin(), fixed_now()).unwrap();
        assert_eq!(utc, "2026-09-06T12:00:00.000Z");
    }

    #[test]
    fn yesterday_with_time_resolves_to_local_day_before() {
        let (utc, tz) = parse_temporal_input("gestern 9:15", &berlin(), fixed_now()).unwrap();
        // 2026-09-06 09:15 Europe/Berlin (CEST, UTC+2) == 2026-09-06T07:15:00Z
        assert_eq!(utc, "2026-09-06T07:15:00.000Z");
        assert_eq!(tz, "Europe/Berlin");
    }

    #[test]
    fn absolute_date_and_time_parses_in_given_zone() {
        let (utc, tz) = parse_temporal_input("07.09.2026 14:32", &berlin(), fixed_now()).unwrap();
        assert_eq!(utc, "2026-09-07T12:32:00.000Z");
        assert_eq!(tz, "Europe/Berlin");
    }

    #[test]
    fn garbage_input_is_rejected() {
        let result = parse_temporal_input("nicht ein datum", &berlin(), fixed_now());
        assert!(matches!(result, Err(AppError::InvalidTimestamp(_))));
    }

    #[test]
    fn invalid_calendar_date_is_rejected() {
        let result = parse_temporal_input("31.02.2026 10:00", &berlin(), fixed_now());
        assert!(matches!(result, Err(AppError::InvalidTimestamp(_))));
    }
}
```

- [ ] **Step 2: `time`-Modul in `lib.rs` einhängen**

```rust
// src-tauri/src/lib.rs
pub mod error;
pub mod time;

pub use error::AppError;
```

- [ ] **Step 3: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test time:: && cd ..
```

Expected: 7 tests passed, 0 failed.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/time.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add timestamp generation and quick-entry temporal parsing"
```

---

## Task 3: Konfiguration (`config.toml`)

**Files:**
- Create: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `AppError` from Task 1
- Produces: `pub struct HotkeyConfig { quick_capture, search, clipboard_screenshot: String }`,
  `pub struct Config { data_dir: PathBuf, autostart_enabled: bool, context_capture_enabled: bool, late_entry_threshold_hours: i64, hotkeys: HotkeyConfig }`,
  `impl Config { pub fn load_or_default(path: &Path) -> Result<Config, AppError>; pub fn save(&self, path: &Path) -> Result<(), AppError>; }`

- [ ] **Step 1: Failing tests schreiben**

```rust
// src-tauri/src/config.rs
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub quick_capture: String,
    pub search: String,
    pub clipboard_screenshot: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            quick_capture: "Ctrl+Alt+Space".to_string(),
            search: "Ctrl+Alt+F".to_string(),
            clipboard_screenshot: "Ctrl+Alt+S".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub data_dir: PathBuf,
    pub autostart_enabled: bool,
    pub context_capture_enabled: bool,
    pub late_entry_threshold_hours: i64,
    pub hotkeys: HotkeyConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            autostart_enabled: true,
            context_capture_enabled: false,
            late_entry_threshold_hours: 24,
            hotkeys: HotkeyConfig::default(),
        }
    }
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wartungsdoku")
}

impl Config {
    pub fn load_or_default(config_path: &Path) -> Result<Self, AppError> {
        if !config_path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(config_path)
            .map_err(|e| AppError::Config(format!("config.toml lesen fehlgeschlagen: {e}")))?;
        toml::from_str(&text)
            .map_err(|e| AppError::Config(format!("config.toml ungültig: {e}")))
    }

    pub fn save(&self, config_path: &Path) -> Result<(), AppError> {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| AppError::Config(format!("config.toml serialisieren fehlgeschlagen: {e}")))?;
        std::fs::write(config_path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_or_default_returns_defaults_when_file_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config::load_or_default(&path).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.hotkeys.quick_capture, "Ctrl+Alt+Space");
        assert!(config.autostart_enabled);
        assert!(!config.context_capture_enabled);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let mut config = Config::default();
        config.autostart_enabled = false;
        config.context_capture_enabled = true;
        config.hotkeys.search = "Ctrl+Shift+F".to_string();

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn invalid_toml_returns_config_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not = [valid toml").unwrap();

        let result = Config::load_or_default(&path);
        assert!(matches!(result, Err(AppError::Config(_))));
    }
}
```

- [ ] **Step 2: `config`-Modul einhängen**

```rust
// src-tauri/src/lib.rs
pub mod config;
pub mod error;
pub mod time;

pub use error::AppError;
```

- [ ] **Step 3: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test config:: && cd ..
```

Expected: 3 tests passed.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/config.rs src-tauri/src/lib.rs
git commit -m "feat: add config.toml load/save with defaults"
```

---

## Task 4: DB-Connection-Pool (WAL + Foreign Keys)

**Files:**
- Create: `src-tauri/src/db/mod.rs`
- Create: `src-tauri/src/db/pool.rs`

**Interfaces:**
- Consumes: `AppError` from Task 1
- Produces: `pub type DbPool = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>`,
  `pub fn build_pool(db_path: &Path) -> Result<DbPool, AppError>`

- [ ] **Step 1: Failing test schreiben**

```rust
// src-tauri/src/db/pool.rs
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;

use crate::error::AppError;

pub type DbPool = Pool<SqliteConnectionManager>;

pub fn build_pool(db_path: &Path) -> Result<DbPool, AppError> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Ok(())
    });
    Pool::new(manager).map_err(|e| AppError::Database(format!("Connection-Pool: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn pool_connections_have_foreign_keys_and_wal_enabled() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = build_pool(&db_path).unwrap();
        let conn = pool.get().unwrap();

        let fk_enabled: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!(fk_enabled, 1);

        let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(journal_mode, "wal");
    }

    #[test]
    fn build_pool_creates_missing_parent_directory() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("nested").join("deep").join("test.db");
        let pool = build_pool(&db_path).unwrap();
        assert!(pool.get().is_ok());
        assert!(db_path.parent().unwrap().exists());
    }
}
```

- [ ] **Step 2: `db/mod.rs` schreiben, in `lib.rs` einhängen**

```rust
// src-tauri/src/db/mod.rs
pub mod pool;
```

```rust
// src-tauri/src/lib.rs
pub mod config;
pub mod db;
pub mod error;
pub mod time;

pub use error::AppError;
```

- [ ] **Step 3: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test db::pool:: && cd ..
```

Expected: 2 tests passed.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/db/mod.rs src-tauri/src/db/pool.rs src-tauri/src/lib.rs
git commit -m "feat: add SQLite connection pool with WAL and foreign keys"
```

---

## Task 5: Schema-Migration (vollständiges Datenmodell) + Runner

**Files:**
- Create: `src-tauri/migrations/0001_init.sql`
- Create: `src-tauri/src/db/migrations.rs`
- Modify: `src-tauri/src/db/mod.rs`

**Interfaces:**
- Consumes: `AppError` from Task 1, `time::now_with_tz` from Task 2
- Produces: `pub fn current_version(conn: &rusqlite::Connection) -> Result<i64, AppError>`,
  `pub fn run_migrations(conn: &mut rusqlite::Connection, db_path: &Path, system_tz: &chrono_tz::Tz) -> Result<(), AppError>`

- [ ] **Step 1: Migrations-SQL schreiben**

```sql
-- src-tauri/migrations/0001_init.sql

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_utc TEXT NOT NULL,
    applied_at_tz TEXT NOT NULL
);

CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    short_code TEXT NOT NULL UNIQUE,
    notes TEXT NOT NULL DEFAULT '',
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL,
    archived_at_utc TEXT,
    archived_at_tz TEXT
);

CREATE INDEX idx_customers_short_code ON customers(short_code);

CREATE TABLE systems (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    system_type TEXT NOT NULL DEFAULT '',
    hostname TEXT NOT NULL DEFAULT '',
    ip_address TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL,
    archived_at_utc TEXT,
    archived_at_tz TEXT
);

CREATE INDEX idx_systems_customer_id ON systems(customer_id);

CREATE TABLE entries (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    system_id INTEGER REFERENCES systems(id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    body_md TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL CHECK (category IN ('wartung', 'stoerung', 'aenderung', 'installation', 'sonstiges')),
    performed_at_utc TEXT NOT NULL,
    performed_at_tz TEXT NOT NULL,
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL
);

CREATE INDEX idx_entries_customer_id ON entries(customer_id);
CREATE INDEX idx_entries_system_id ON entries(system_id);
CREATE INDEX idx_entries_performed_at_utc ON entries(performed_at_utc);

CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE entry_tags (
    entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (entry_id, tag_id)
);

CREATE TABLE attachments (
    id INTEGER PRIMARY KEY,
    entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL,
    original_filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL
);

CREATE INDEX idx_attachments_entry_id ON attachments(entry_id);
CREATE INDEX idx_attachments_sha256 ON attachments(sha256);

CREATE TABLE external_refs (
    id INTEGER PRIMARY KEY,
    system_id INTEGER NOT NULL REFERENCES systems(id) ON DELETE CASCADE,
    plugin_id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    synced_at_utc TEXT NOT NULL,
    synced_at_tz TEXT NOT NULL
);

CREATE INDEX idx_external_refs_system_id ON external_refs(system_id);

CREATE VIRTUAL TABLE entries_fts USING fts5(
    title,
    body_md,
    content = 'entries',
    content_rowid = 'id'
);

CREATE TRIGGER entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, title, body_md) VALUES (new.id, new.title, new.body_md);
END;

CREATE TRIGGER entries_ad AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, title, body_md) VALUES ('delete', old.id, old.title, old.body_md);
END;

CREATE TRIGGER entries_au AFTER UPDATE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, title, body_md) VALUES ('delete', old.id, old.title, old.body_md);
    INSERT INTO entries_fts(rowid, title, body_md) VALUES (new.id, new.title, new.body_md);
END;
```

- [ ] **Step 2: Failing tests für den Runner schreiben**

```rust
// src-tauri/src/db/migrations.rs
use chrono_tz::Tz;
use rusqlite::Connection;
use std::path::Path;

use crate::error::AppError;
use crate::time::now_with_tz;

struct Migration {
    version: i64,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("../../migrations/0001_init.sql"),
}];

pub fn current_version(conn: &Connection) -> Result<i64, AppError> {
    let table_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |row| row.get(0),
    )?;
    if table_count == 0 {
        return Ok(0);
    }
    let version: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

pub fn run_migrations(
    conn: &mut Connection,
    db_path: &Path,
    system_tz: &Tz,
) -> Result<(), AppError> {
    let current = current_version(conn)?;
    let pending: Vec<&Migration> = MIGRATIONS.iter().filter(|m| m.version > current).collect();
    if pending.is_empty() {
        return Ok(());
    }

    if db_path.exists() {
        let backup_path = db_path.with_extension(format!("db.bak-{current}"));
        std::fs::copy(db_path, &backup_path)?;
    }

    for migration in pending {
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)
            .map_err(|e| AppError::Migration(format!("Migration {}: {e}", migration.version)))?;
        let (applied_utc, applied_tz) = now_with_tz(system_tz);
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at_utc, applied_at_tz) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, applied_utc, applied_tz],
        )?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    #[test]
    fn fresh_database_starts_at_version_zero() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn run_migrations_creates_all_tables_and_records_version() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let mut conn = Connection::open(&db_path).unwrap();

        run_migrations(&mut conn, &db_path, &berlin()).unwrap();

        assert_eq!(current_version(&conn).unwrap(), 1);

        for table in [
            "customers", "systems", "entries", "tags", "entry_tags",
            "attachments", "external_refs", "entries_fts",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "Tabelle {table} fehlt");
        }
    }

    #[test]
    fn running_migrations_twice_is_a_no_op() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let mut conn = Connection::open(&db_path).unwrap();

        run_migrations(&mut conn, &db_path, &berlin()).unwrap();
        run_migrations(&mut conn, &db_path, &berlin()).unwrap();

        let row_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(row_count, 1);
    }

    #[test]
    fn migration_backs_up_existing_database_file() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        {
            // Version-0-Datei anlegen, damit ein Backup entstehen kann.
            Connection::open(&db_path).unwrap();
        }
        let mut conn = Connection::open(&db_path).unwrap();
        run_migrations(&mut conn, &db_path, &berlin()).unwrap();

        let backup_path = db_path.with_extension("db.bak-0");
        assert!(backup_path.exists());
    }

    #[test]
    fn foreign_keys_are_enforced_after_migration() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let mut conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&mut conn, &db_path, &berlin()).unwrap();

        let result = conn.execute(
            "INSERT INTO systems (id, customer_id, name, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 999, 'Ghost', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        );
        assert!(result.is_err(), "FK-Verletzung hätte fehlschlagen müssen");
    }

    #[test]
    fn entries_fts_stays_in_sync_via_triggers() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let mut conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations(&mut conn, &db_path, &berlin()).unwrap();

        conn.execute(
            "INSERT INTO customers (id, name, short_code, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 'ACME GmbH', 'ACME', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO entries (id, customer_id, title, body_md, category, performed_at_utc, performed_at_tz, created_at_utc, created_at_tz, updated_at_utc, updated_at_tz)
             VALUES (1, 1, 'Exchange Update', 'Kumulatives Update eingespielt', 'wartung',
                     '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin', '2026-09-07T12:00:00.000Z', 'Europe/Berlin')",
            [],
        ).unwrap();

        let hit_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entries_fts WHERE entries_fts MATCH 'kumulatives'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hit_count, 1);

        conn.execute("DELETE FROM entries WHERE id = 1", []).unwrap();
        let hit_count_after_delete: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entries_fts WHERE entries_fts MATCH 'kumulatives'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hit_count_after_delete, 0);
    }
}
```

- [ ] **Step 3: `migrations`-Modul einhängen**

```rust
// src-tauri/src/db/mod.rs
pub mod migrations;
pub mod pool;
```

- [ ] **Step 4: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test db:: && cd ..
```

Expected: 8 tests passed (2 aus `pool.rs` + 6 aus `migrations.rs`), 0 failed.

- [ ] **Step 5: Gesamten Testlauf der Phase verifizieren**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test && cd ..
```

Expected: alle Tests aus Task 1–5 grün (3 + 7 + 3 + 2 + 6 = 21 Tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/migrations/0001_init.sql src-tauri/src/db/migrations.rs src-tauri/src/db/mod.rs
git commit -m "feat: add full schema migration with FTS5 sync triggers and forward-only runner"
```

---

## Self-Review (durchgeführt vor Ausführung)

- **Spec-Abdeckung**: alle Tabellen aus dem Datenmodell (customers, systems, entries,
  tags, entry_tags, attachments, entries_fts, external_refs, schema_migrations) sind in
  Task 5 enthalten. Zeitstempel-Regel (Task 2), Migration mit Backup (Task 5),
  WAL/Foreign-Keys (Task 4), Config (Task 3) — jeweils mit Tests abgedeckt.
- **Bewusst auf spätere Phasen verschoben**: Repository-Funktionen für
  customers/systems/entries/tags/attachments/search (gehören zu "Backend-Kommandos",
  Phase 2), Verzeichnis-Suche über Kunden-/Systemnamen (ebenfalls Phase 2 — Schema
  dafür ist in Task 5 bereits vorhanden).
  Kein Tauri-Wiring, kein `[[bin]]`-Target — kommt in Phase 2, ohne dass diese Struktur
  umgebaut werden muss.
- **Platzhalter-Scan**: keine TBD/TODO, jeder Schritt enthält lauffähigen Code.
- **Typkonsistenz**: `AppError` (Task 1) wird identisch in Task 2–5 verwendet;
  `time::now_with_tz` (Task 2) wird in Task 5 mit exakt dieser Signatur aufgerufen;
  `DbPool`/`build_pool` (Task 4) wird von `run_migrations` (Task 5) nicht direkt
  gebraucht (der Runner arbeitet auf einer rohen `Connection`, die die aufrufende Stelle
  in Phase 2 aus dem Pool zieht) — bewusst entkoppelt, damit Migrationen auch beim
  App-Start vor dem ersten Pool-Checkout laufen können.
