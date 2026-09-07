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
