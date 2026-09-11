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
    conn.query_row(
        "SELECT * FROM attachments WHERE id = ?1",
        params![id],
        row_to_attachment,
    )
    .map_err(Into::into)
}

pub fn list_for_entry(conn: &Connection, entry_id: i64) -> Result<Vec<Attachment>, AppError> {
    let mut stmt =
        conn.prepare("SELECT * FROM attachments WHERE entry_id = ?1 ORDER BY created_at_utc ASC")?;
    let rows = stmt.query_map(params![entry_id], row_to_attachment)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn get(conn: &Connection, id: i64) -> Result<Attachment, AppError> {
    conn.query_row(
        "SELECT * FROM attachments WHERE id = ?1",
        params![id],
        row_to_attachment,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("Anhang {id} nicht gefunden")))
}

/// Deletes only the database row. The associated file in the content-addressed
/// store is left untouched -- it may be referenced by other attachment rows via
/// dedup. Cleanup of no-longer-referenced files runs exclusively through the
/// separate, explicitly invoked `cleanup_orphans` functionality, never
/// automatically.
pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    let changed = conn.execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("Anhang {id} nicht gefunden")));
    }
    Ok(())
}

/// All content hashes currently referenced, as the basis for orphan
/// detection in the attachment store.
pub fn all_referenced_hashes(
    conn: &Connection,
) -> Result<std::collections::HashSet<String>, AppError> {
    let mut stmt = conn.prepare("SELECT DISTINCT sha256 FROM attachments")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut result = std::collections::HashSet::new();
    for row in rows {
        result.insert(row?);
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct AttachmentStorageSummary {
    /// Number of DISTINCT files on disk (by sha256), not the number of
    /// attachment rows/links -- two entries sharing one screenshot count
    /// as one file here, matching actual disk usage.
    pub distinct_file_count: i64,
    /// Total bytes across those distinct files -- see the module-level
    /// note on this function about why a naive SUM(size_bytes) over every
    /// row would overcount.
    pub total_size_bytes: i64,
}

/// Distinct-file count and total disk size across all attachments,
/// deduplicated by `sha256` exactly like `all_referenced_hashes` already
/// does for orphan detection -- the same physical file can be linked from
/// multiple attachment rows (one per entry it's attached to), and each row
/// redundantly stores that file's own size, so grouping by `sha256` first
/// is required to avoid counting a shared file's size more than once.
pub fn storage_summary(conn: &Connection) -> Result<AttachmentStorageSummary, AppError> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM (
             SELECT sha256, size_bytes FROM attachments GROUP BY sha256
         )",
        [],
        |row| {
            Ok(AttachmentStorageSummary {
                distinct_file_count: row.get(0)?,
                total_size_bytes: row.get(1)?,
            })
        },
    )
    .map_err(Into::into)
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
        let customer_id = customers::create(
            conn,
            NewCustomer {
                name: "ACME".into(),
                short_code: "ACME".into(),
                notes: "".into(),
            },
            &berlin(),
        )
        .unwrap()
        .id;
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
        let created = create(
            &conn,
            entry_id,
            "abc123",
            "screenshot.png",
            "image/png",
            42,
            &berlin(),
        )
        .unwrap();
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
        let created = create(
            &conn,
            entry_id,
            "abc123",
            "screenshot.png",
            "image/png",
            42,
            &berlin(),
        )
        .unwrap();
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
        let created = create(
            &conn,
            entry_id,
            "abc123",
            "screenshot.png",
            "image/png",
            42,
            &berlin(),
        )
        .unwrap();
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
        let relative_path =
            crate::attachments::store::relative_path_for(&attachment.sha256, "shot.png");
        assert!(dir.path().join(&relative_path).exists());

        delete(&conn, attachment.id).unwrap();

        assert!(
            dir.path().join(&relative_path).exists(),
            "delete darf die Datei im Store nicht anfassen"
        );
    }

    #[test]
    fn all_referenced_hashes_returns_distinct_hashes() {
        let conn = migrated_connection();
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        create(
            &conn,
            entry_id,
            "hash-a",
            "a.png",
            "image/png",
            1,
            &berlin(),
        )
        .unwrap();
        create(
            &conn,
            entry_id,
            "hash-a",
            "a2.png",
            "image/png",
            1,
            &berlin(),
        )
        .unwrap();
        create(
            &conn,
            entry_id,
            "hash-b",
            "b.png",
            "image/png",
            1,
            &berlin(),
        )
        .unwrap();

        let hashes = all_referenced_hashes(&conn).unwrap();

        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains("hash-a"));
        assert!(hashes.contains("hash-b"));
    }

    #[test]
    fn storage_summary_with_no_attachments_is_zero() {
        let conn = migrated_connection();
        let summary = storage_summary(&conn).unwrap();
        assert_eq!(summary.distinct_file_count, 0);
        assert_eq!(summary.total_size_bytes, 0);
    }

    #[test]
    fn storage_summary_sums_distinct_files() {
        let conn = migrated_connection();
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        create(
            &conn,
            entry_id,
            "hash-a",
            "a.png",
            "image/png",
            100,
            &berlin(),
        )
        .unwrap();
        create(
            &conn,
            entry_id,
            "hash-b",
            "b.png",
            "image/png",
            250,
            &berlin(),
        )
        .unwrap();

        let summary = storage_summary(&conn).unwrap();

        assert_eq!(summary.distinct_file_count, 2);
        assert_eq!(summary.total_size_bytes, 350);
    }

    /// Proves the dedup logic: the same physical file (same sha256) attached
    /// to two different entries must be counted ONCE, not twice, both for
    /// distinct_file_count and total_size_bytes -- see the module-level note
    /// on `storage_summary` about why a naive SUM(size_bytes) over every row
    /// would overcount actual disk usage.
    #[test]
    fn storage_summary_deduplicates_shared_file_across_rows() {
        let conn = migrated_connection();
        let dir = tempfile::tempdir().unwrap();
        let entry_id = seed_entry(&conn, dir.path());
        // Same sha256, attached via two separate attachment rows (e.g. the
        // same screenshot attached to two different journal entries).
        create(
            &conn,
            entry_id,
            "shared-hash",
            "shot.png",
            "image/png",
            777,
            &berlin(),
        )
        .unwrap();
        create(
            &conn,
            entry_id,
            "shared-hash",
            "shot-copy.png",
            "image/png",
            777,
            &berlin(),
        )
        .unwrap();

        let summary = storage_summary(&conn).unwrap();

        assert_eq!(
            summary.distinct_file_count, 1,
            "shared file must be counted once, not once per attachment row"
        );
        assert_eq!(
            summary.total_size_bytes, 777,
            "shared file's size must not be double-counted"
        );
    }
}
