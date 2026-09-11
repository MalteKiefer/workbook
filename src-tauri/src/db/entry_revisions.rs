use chrono_tz::Tz;
use rusqlite::{params, Connection, Row};

use crate::db::entries::Category;
use crate::error::AppError;
use crate::time::now_with_tz;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EntryRevision {
    pub id: i64,
    pub entry_id: i64,
    pub title: String,
    pub body_md: String,
    pub category: Category,
    pub revised_at_utc: String,
    pub revised_at_tz: String,
}

fn row_to_revision(row: &Row) -> rusqlite::Result<EntryRevision> {
    Ok(EntryRevision {
        id: row.get("id")?,
        entry_id: row.get("entry_id")?,
        title: row.get("title")?,
        body_md: row.get("body_md")?,
        category: row.get("category")?,
        revised_at_utc: row.get("revised_at_utc")?,
        revised_at_tz: row.get("revised_at_tz")?,
    })
}

/// Snapshots an entry's current `title`/`body_md`/`category` as a new
/// revision, timestamped now. Called from `db::entries::update` with the
/// PRE-update values, right before the actual `UPDATE entries` runs --
/// see Task 3.
pub fn create(
    conn: &Connection,
    entry_id: i64,
    title: &str,
    body_md: &str,
    category: Category,
    tz: &Tz,
) -> Result<(), AppError> {
    let (now_utc, now_tz) = now_with_tz(tz);
    conn.execute(
        "INSERT INTO entry_revisions (entry_id, title, body_md, category, revised_at_utc, revised_at_tz)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![entry_id, title, body_md, category, now_utc, now_tz],
    )?;
    Ok(())
}

/// Newest-first, same "id DESC tiebreak on equal timestamps" reasoning as
/// `db::audit_log::list_for_entity` (a fast edit loop, or a test, can
/// share a millisecond-resolution timestamp).
pub fn list_for_entry(conn: &Connection, entry_id: i64) -> Result<Vec<EntryRevision>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM entry_revisions WHERE entry_id = ?1 ORDER BY revised_at_utc DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![entry_id], row_to_revision)?;
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
    use crate::db::entries::{self, NewEntry};
    use crate::db::test_support::migrated_connection;
    use tempfile::tempdir;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    // `short_code` is caller-supplied (not hardcoded to "ACME") because
    // `customers.short_code` is UNIQUE -- a test that seeds two separate
    // entries (e.g. `list_for_entry_excludes_other_entries`) needs two
    // distinct customers, and reusing the same short_code for both would
    // trip that constraint.
    fn seed_entry(conn: &Connection, short_code: &str) -> i64 {
        let customer_id = customers::create(
            conn,
            NewCustomer {
                name: "ACME GmbH".to_string(),
                short_code: short_code.to_string(),
                notes: "".to_string(),
            },
            &berlin(),
        )
        .unwrap()
        .id;
        let data_dir = tempdir().unwrap();
        entries::create(
            conn,
            data_dir.path(),
            NewEntry {
                customer_id,
                system_id: None,
                title: "Erste Version".to_string(),
                body_md: "Ursprünglicher Text".to_string(),
                category: Category::Wartung,
                performed_at_utc: "2026-09-07T12:00:00.000Z".to_string(),
                performed_at_tz: "Europe/Berlin".to_string(),
                tag_names: vec![],
                pending_attachments: vec![],
            },
            &berlin(),
        )
        .unwrap()
        .id
    }

    #[test]
    fn create_then_list_roundtrips() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn, "ACME");
        create(
            &conn,
            entry_id,
            "Erste Version",
            "Ursprünglicher Text",
            Category::Wartung,
            &berlin(),
        )
        .unwrap();

        let revisions = list_for_entry(&conn, entry_id).unwrap();
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].title, "Erste Version");
        assert_eq!(revisions[0].body_md, "Ursprünglicher Text");
    }

    #[test]
    fn list_orders_newest_first() {
        let conn = migrated_connection();
        let entry_id = seed_entry(&conn, "ACME");
        create(
            &conn,
            entry_id,
            "v1",
            "text v1",
            Category::Wartung,
            &berlin(),
        )
        .unwrap();
        create(
            &conn,
            entry_id,
            "v2",
            "text v2",
            Category::Wartung,
            &berlin(),
        )
        .unwrap();

        let revisions = list_for_entry(&conn, entry_id).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].title, "v2");
        assert_eq!(revisions[1].title, "v1");
    }

    #[test]
    fn list_for_entry_excludes_other_entries() {
        let conn = migrated_connection();
        let entry_id_a = seed_entry(&conn, "ACME-A");
        let entry_id_b = seed_entry(&conn, "ACME-B");
        create(
            &conn,
            entry_id_a,
            "A",
            "text A",
            Category::Wartung,
            &berlin(),
        )
        .unwrap();
        create(
            &conn,
            entry_id_b,
            "B",
            "text B",
            Category::Wartung,
            &berlin(),
        )
        .unwrap();

        let revisions = list_for_entry(&conn, entry_id_a).unwrap();
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].title, "A");
    }
}
