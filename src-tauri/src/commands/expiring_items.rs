use tauri::State;

use crate::db::expiring_items::{self, ExpiringItem, NewExpiringItem, UpdateExpiringItem};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_expiring_items(state: State<AppState>) -> Result<Vec<ExpiringItem>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    expiring_items::list_all(&conn)
}

#[tauri::command]
pub fn create_expiring_item(
    state: State<AppState>,
    input: NewExpiringItem,
) -> Result<ExpiringItem, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    expiring_items::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_expiring_item(
    state: State<AppState>,
    id: i64,
    input: UpdateExpiringItem,
) -> Result<ExpiringItem, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    expiring_items::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn delete_expiring_item(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    expiring_items::delete(&conn, id)
}
