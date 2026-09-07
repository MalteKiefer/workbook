use tauri::State;

use crate::db::search::{self, DirectoryHit, EntryHit};
use crate::{AppError, AppState};

#[tauri::command]
pub fn search_entries(state: State<AppState>, query: String, limit: i64) -> Result<Vec<EntryHit>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    search::search_entries(&conn, &query, limit)
}

#[tauri::command]
pub fn search_directory(state: State<AppState>, query: String, limit: i64) -> Result<Vec<DirectoryHit>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    search::search_directory(&conn, &query, limit)
}
