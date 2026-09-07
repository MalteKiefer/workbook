use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Row};

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

pub fn get(conn: &Connection, id: i64) -> Result<Attachment, AppError> {
    conn.query_row("SELECT * FROM attachments WHERE id = ?1", params![id], row_to_attachment)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("Anhang {id} nicht gefunden")))
}

/// Löscht nur die Datenbank-Zeile. Die zugehörige Datei im Content-Addressed-Store
/// bleibt unangetastet — sie kann per Dedup von anderen Anhang-Zeilen referenziert
/// sein. Bereinigung nicht mehr referenzierter Dateien läuft ausschließlich über
/// die separate, explizit aufzurufende `cleanup_orphans`-Funktionalität, nie
/// automatisch.
pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    let changed = conn.execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Anhang {id} nicht gefunden")));
    }
    Ok(())
}

/// Alle im Moment referenzierten Content-Hashes, als Grundlage für die
/// Orphan-Erkennung im Attachment-Store.
pub fn all_referenced_hashes(conn: &Connection) -> Result<std::collections::HashSet<String>, AppError> {
    let mut stmt = conn.prepare("SELECT DISTINCT sha256 FROM attachments")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut result = std::collections::HashSet::new();
    for row in rows {
        result.insert(row?);
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

    #[test]
    fn get_returns_existing_attachment() {
        let conn = migrated_connection();
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        let created = create(&conn, entry_id, "abc123", "screenshot.png", "image/png", 42, &berlin()).unwrap();
        assert_eq!(get(&conn, created.id).unwrap(), created);
    }

    #[test]
    fn get_returns_not_found_for_unknown_id() {
        let conn = migrated_connection();
        let result = get(&conn, 999);
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[test]
    fn delete_removes_row() {
        let conn = migrated_connection();
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        let created = create(&conn, entry_id, "abc123", "screenshot.png", "image/png", 42, &berlin()).unwrap();
        delete(&conn, created.id).unwrap();
        assert!(matches!(get(&conn, created.id), Err(AppError::NotFound(_))));
    }

    #[test]
    fn delete_returns_not_found_for_unknown_id() {
        let conn = migrated_connection();
        let result = delete(&conn, 999);
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[test]
    fn delete_does_not_touch_file_on_disk() {
        let conn = migrated_connection();
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        let attachment = crate::attachments::store::attach_bytes_to_entry(
            &conn,
            dir.path(),
            entry_id,
            b"png bytes",
            "shot.png",
            "image/png",
            &berlin(),
        )
        .unwrap();
        let relative_path = crate::attachments::store::relative_path_for(&attachment.sha256, "shot.png");
        assert!(dir.path().join(&relative_path).exists());

        delete(&conn, attachment.id).unwrap();

        assert!(dir.path().join(&relative_path).exists(), "delete darf die Datei im Store nicht anfassen");
    }

    #[test]
    fn all_referenced_hashes_returns_distinct_hashes() {
        let conn = migrated_connection();
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        create(&conn, entry_id, "hash-a", "a.png", "image/png", 1, &berlin()).unwrap();
        create(&conn, entry_id, "hash-a", "a2.png", "image/png", 1, &berlin()).unwrap();
        create(&conn, entry_id, "hash-b", "b.png", "image/png", 1, &berlin()).unwrap();

        let hashes = all_referenced_hashes(&conn).unwrap();

        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains("hash-a"));
        assert!(hashes.contains("hash-b"));
    }
}
