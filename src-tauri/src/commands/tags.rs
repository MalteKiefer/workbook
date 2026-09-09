use tauri::State;

use crate::db::tags::{self, TagCount};
use crate::{AppError, AppState};

#[tauri::command]
pub fn list_tags(state: State<AppState>) -> Result<Vec<String>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    tags::list_all(&conn)
}

/// Every tag currently in use, with how many entries use it -- powers the tag
/// cloud in `JournalView` (Journal → Tag-Filter).
#[tauri::command]
pub fn list_tags_with_counts(state: State<AppState>) -> Result<Vec<TagCount>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    tags::list_all_with_counts(&conn)
}
