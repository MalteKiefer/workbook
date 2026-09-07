use tauri::State;

use crate::db::entries::{self, Entry, EntryFilter, NewEntry, UpdateEntry};
use crate::{time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TemporalPreview {
    pub utc: String,
    pub tz: String,
}

#[tauri::command]
pub fn list_entries(state: State<AppState>, filter: EntryFilter) -> Result<Vec<Entry>, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    entries::list(&conn, &filter)
}

#[tauri::command]
pub fn get_entry(state: State<AppState>, id: i64) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    entries::get(&conn, id)
}

#[tauri::command]
pub fn create_entry(state: State<AppState>, input: NewEntry) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    entries::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_entry(state: State<AppState>, id: i64, input: UpdateEntry) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    entries::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn parse_temporal_input(input: String) -> Result<TemporalPreview, AppError> {
    let tz = time::system_timezone()?;
    let now = chrono::Utc::now();
    let (utc, tz_name) = time::parse_temporal_input(&input, &tz, now)?;
    Ok(TemporalPreview { utc, tz: tz_name })
}
