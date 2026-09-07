use tauri::State;

use crate::db::tags;
use crate::{AppError, AppState};

#[tauri::command]
pub fn list_tags(state: State<AppState>) -> Result<Vec<String>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    tags::list_all(&conn)
}
