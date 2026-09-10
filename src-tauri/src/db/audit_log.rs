use chrono_tz::Tz;
use rusqlite::{params, Connection, Row};

use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditLogEntry {
    pub id: i64,
    pub entity_type: String,
    pub entity_id: i64,
    pub action: String,
    pub summary: String,
    pub at_utc: String,
    pub at_tz: String,
}

fn row_to_entry(row: &Row) -> rusqlite::Result<AuditLogEntry> {
    Ok(AuditLogEntry {
        id: row.get("id")?,
        entity_type: row.get("entity_type")?,
        entity_id: row.get("entity_id")?,
        action: row.get("action")?,
        summary: row.get("summary")?,
        at_utc: row.get("at_utc")?,
        at_tz: row.get("at_tz")?,
    })
}

pub fn record(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
    action: &str,
    summary: &str,
    tz: &Tz,
) -> Result<(), AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO audit_log (entity_type, entity_id, action, summary, at_utc, at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![entity_type, entity_id, action, summary, now_utc, now_tz],
    )?;
    Ok(())
}

pub fn list_for_entity(
    conn: &Connection,
    entity_type: &str,
    entity_id: i64,
) -> Result<Vec<AuditLogEntry>, AppError> {
    // Tiebreak on `id DESC` in addition to `at_utc DESC`: `at_utc` only has
    // millisecond resolution, so two records written in quick succession
    // (e.g. back-to-back calls in a CSV import loop, or just a fast test)
    // can share the same timestamp -- `id` (autoincrementing) is what
    // actually guarantees "newest first" in that case.
    let mut stmt = conn.prepare(
        "SELECT * FROM audit_log WHERE entity_type = ?1 AND entity_id = ?2 ORDER BY at_utc DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![entity_type, entity_id], row_to_entry)?;
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

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    #[test]
    fn record_then_list_roundtrips() {
        let conn = migrated_connection();
        record(
            &conn,
            "customer",
            1,
            "created",
            "Kunde \"ACME GmbH\" angelegt",
            &berlin(),
        )
        .unwrap();

        let entries = list_for_entity(&conn, "customer", 1).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.entity_type, "customer");
        assert_eq!(entry.entity_id, 1);
        assert_eq!(entry.action, "created");
        assert_eq!(entry.summary, "Kunde \"ACME GmbH\" angelegt");
        assert_eq!(entry.at_tz, "Europe/Berlin");
        assert!(!entry.at_utc.is_empty());
    }

    #[test]
    fn list_for_entity_orders_newest_first() {
        let conn = migrated_connection();
        record(&conn, "customer", 1, "created", "erster Eintrag", &berlin()).unwrap();
        record(
            &conn,
            "customer",
            1,
            "updated",
            "zweiter Eintrag",
            &berlin(),
        )
        .unwrap();
        record(
            &conn,
            "customer",
            1,
            "updated",
            "dritter Eintrag",
            &berlin(),
        )
        .unwrap();

        let entries = list_for_entity(&conn, "customer", 1).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].summary, "dritter Eintrag");
        assert_eq!(entries[1].summary, "zweiter Eintrag");
        assert_eq!(entries[2].summary, "erster Eintrag");
    }

    #[test]
    fn list_for_entity_filters_by_entity_type_and_id() {
        let conn = migrated_connection();
        record(&conn, "customer", 1, "created", "Kunde 1", &berlin()).unwrap();
        record(&conn, "customer", 2, "created", "Kunde 2", &berlin()).unwrap();
        record(&conn, "system", 1, "created", "System 1", &berlin()).unwrap();

        let entries = list_for_entity(&conn, "customer", 1).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].summary, "Kunde 1");
    }
}
