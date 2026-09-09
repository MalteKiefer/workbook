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
            "customers",
            "systems",
            "entries",
            "tags",
            "entry_tags",
            "attachments",
            "external_refs",
            "entries_fts",
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
            // Create a version-0 file so a backup can be produced.
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

        conn.execute("DELETE FROM entries WHERE id = 1", [])
            .unwrap();
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
