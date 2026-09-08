use tauri::State;

use crate::db::customers::{self, Customer, NewCustomer, UpdateCustomer};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_customers(
    state: State<AppState>,
    include_archived: bool,
) -> Result<Vec<Customer>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    customers::list(&conn, include_archived)
}

#[tauri::command]
pub fn create_customer(state: State<AppState>, input: NewCustomer) -> Result<Customer, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    customers::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_customer(
    state: State<AppState>,
    id: i64,
    input: UpdateCustomer,
) -> Result<Customer, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    customers::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn archive_customer(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    customers::archive(&conn, id, &tz)
}
