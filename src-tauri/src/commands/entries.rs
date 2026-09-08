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
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    entries::list(&conn, &filter)
}

#[tauri::command]
pub fn get_entry(state: State<AppState>, id: i64) -> Result<Entry, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    entries::get(&conn, id)
}

#[tauri::command]
pub fn create_entry(state: State<AppState>, input: NewEntry) -> Result<Entry, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let entry = entries::create(&conn, &data_dir, input, &tz)?;

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.last_customer_id = Some(entry.customer_id);
    config.last_system_id = entry.system_id;
    let config_path = config.data_dir.join("config.toml");
    if let Err(e) = config.save(&config_path) {
        eprintln!("Letzte Auswahl konnte nicht gespeichert werden: {e}");
    }

    Ok(entry)
}

#[tauri::command]
pub fn format_timestamp_for_display(utc: String, tz: String) -> Result<String, AppError> {
    time::format_timestamp_for_display(&utc, &tz)
}

#[tauri::command]
pub fn update_entry(
    state: State<AppState>,
    id: i64,
    input: UpdateEntry,
) -> Result<Entry, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    entries::update(&conn, &data_dir, id, input, &tz)
}

#[tauri::command]
pub fn parse_temporal_input(input: String) -> Result<TemporalPreview, AppError> {
    let tz = time::system_timezone()?;
    let now = chrono::Utc::now();
    let (utc, tz_name) = time::parse_temporal_input(&input, &tz, now)?;
    Ok(TemporalPreview { utc, tz: tz_name })
}
