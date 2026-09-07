use tauri::State;

use crate::db::systems::{self, NewSystem, System, UpdateSystem};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_systems(state: State<AppState>, customer_id: i64, include_archived: bool) -> Result<Vec<System>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    systems::list_by_customer(&conn, customer_id, include_archived)
}

#[tauri::command]
pub fn create_system(state: State<AppState>, input: NewSystem) -> Result<System, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_system(state: State<AppState>, id: i64, input: UpdateSystem) -> Result<System, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn archive_system(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::archive(&conn, id, &tz)
}
