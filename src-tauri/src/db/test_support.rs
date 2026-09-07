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
