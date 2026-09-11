use tauri::State;

use crate::db::entry_templates::{self, EntryTemplate, NewEntryTemplate, UpdateEntryTemplate};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_entry_templates(state: State<AppState>) -> Result<Vec<EntryTemplate>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    entry_templates::list_all(&conn)
}

#[tauri::command]
pub fn create_entry_template(
    state: State<AppState>,
    input: NewEntryTemplate,
) -> Result<EntryTemplate, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    entry_templates::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_entry_template(
    state: State<AppState>,
    id: i64,
    input: UpdateEntryTemplate,
) -> Result<EntryTemplate, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    entry_templates::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn delete_entry_template(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    entry_templates::delete(&conn, id)
}
