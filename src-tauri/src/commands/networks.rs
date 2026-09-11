use tauri::State;

use crate::db::networks::{self, Network, NewNetwork, UpdateNetwork};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_networks_for_customer(
    state: State<AppState>,
    customer_id: i64,
) -> Result<Vec<Network>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    networks::list_for_customer(&conn, customer_id)
}

#[tauri::command]
pub fn create_network(state: State<AppState>, input: NewNetwork) -> Result<Network, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    networks::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_network(
    state: State<AppState>,
    id: i64,
    input: UpdateNetwork,
) -> Result<Network, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    networks::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn delete_network(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    networks::delete(&conn, id)
}
