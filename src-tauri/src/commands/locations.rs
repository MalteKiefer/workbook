use tauri::State;

use crate::db::locations::{self, Location, NewLocation, UpdateLocation};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_locations_for_customer(
    state: State<AppState>,
    customer_id: i64,
) -> Result<Vec<Location>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    locations::list_for_customer(&conn, customer_id)
}

#[tauri::command]
pub fn create_location(state: State<AppState>, input: NewLocation) -> Result<Location, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    locations::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_location(
    state: State<AppState>,
    id: i64,
    input: UpdateLocation,
) -> Result<Location, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    locations::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn delete_location(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    locations::delete(&conn, id)
}
