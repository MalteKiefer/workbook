use tauri::State;

use crate::db::entry_revisions::{self, EntryRevision};
use crate::{AppError, AppState};

#[tauri::command]
pub fn list_entry_revisions(
    state: State<AppState>,
    entry_id: i64,
) -> Result<Vec<EntryRevision>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    entry_revisions::list_for_entry(&conn, entry_id)
}
