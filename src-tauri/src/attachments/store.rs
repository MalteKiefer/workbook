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

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").expect("Schreiben in String kann nicht fehlschlagen");
    }
    out
}

pub fn save_content_addressed(data_dir: &Path, bytes: &[u8], original_filename: &str) -> Result<SavedFile, AppError> {
    let sha256 = to_hex(Sha256::digest(bytes).as_ref());
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
}
