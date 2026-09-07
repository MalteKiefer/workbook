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

pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(bytes);
    let digest_bytes: &[u8] = digest.as_ref();
    let mut out = String::with_capacity(digest_bytes.len() * 2);
    for byte in digest_bytes {
        write!(out, "{byte:02x}").expect("Schreiben in String kann nicht fehlschlagen");
    }
    out
}

pub fn save_content_addressed(data_dir: &Path, bytes: &[u8], original_filename: &str) -> Result<SavedFile, AppError> {
    let sha256 = sha256_hex(bytes);
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
        assert_eq!(saved.relative_path.matches('.').count(), 0);
    }

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

        let result = attach_bytes_to_entry(&conn, dir.path(), 999, b"orphan bytes", "shot.png", "image/png", &tz);
        assert!(result.is_err());

        let sha256 = sha256_hex(b"orphan bytes");
        let relative_path = relative_path_for(&sha256, "shot.png");
        assert!(!dir.path().join(&relative_path).exists(), "neu geschriebene Datei hätte entfernt werden müssen");
    }

    #[test]
    fn attach_bytes_to_entry_keeps_deduplicated_file_on_db_failure() {
        let dir = tempdir().unwrap();
        let conn = crate::db::test_support::migrated_connection();
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let entry_id = seed_entry(&conn, &tz);

        let first = attach_bytes_to_entry(&conn, dir.path(), entry_id, b"shared bytes", "shot.png", "image/png", &tz).unwrap();
        let relative_path = relative_path_for(&first.sha256, "shot.png");
        assert!(dir.path().join(&relative_path).exists());

        let result = attach_bytes_to_entry(&conn, dir.path(), 999, b"shared bytes", "shot.png", "image/png", &tz);
        assert!(result.is_err());
        assert!(dir.path().join(&relative_path).exists(), "über Dedup geteilte Datei darf nicht gelöscht werden");
    }
}
