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

    fn seed_entry(conn: &Connection, data_dir: &std::path::Path) -> i64 {
        let customer_id = customers::create(conn, NewCustomer { name: "ACME".into(), short_code: "ACME".into(), notes: "".into() }, &berlin()).unwrap().id;
        entries::create(
            conn,
            data_dir,
            NewEntry {
                customer_id,
                system_id: None,
                title: "Titel".into(),
                body_md: "".into(),
                category: Category::Wartung,
                performed_at_utc: "2026-09-07T12:00:00.000Z".into(),
                performed_at_tz: "Europe/Berlin".into(),
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
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        let created = create(&conn, entry_id, "abc123", "screenshot.png", "image/png", 42, &berlin()).unwrap();
        assert_eq!(created.entry_id, entry_id);
        assert_eq!(list_for_entry(&conn, entry_id).unwrap(), vec![created]);
    }

    #[test]
    fn list_for_entry_without_attachments_is_empty() {
        let conn = migrated_connection();
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        assert!(list_for_entry(&conn, entry_id).unwrap().is_empty());
    }

    #[test]
    fn create_rejects_unknown_entry() {
        let conn = migrated_connection();
        let result = create(&conn, 999, "abc123", "x.png", "image/png", 1, &berlin());
        assert!(matches!(result, Err(AppError::Database(_))));
    }
}
