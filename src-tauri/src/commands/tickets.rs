use tauri::State;

use crate::db::tickets::{self, NewTicket, Ticket, UpdateTicket};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_tickets_for_customer(
    state: State<AppState>,
    customer_id: i64,
) -> Result<Vec<Ticket>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    tickets::list_for_customer(&conn, customer_id)
}

#[tauri::command]
pub fn create_ticket(state: State<AppState>, input: NewTicket) -> Result<Ticket, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    tickets::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_ticket(
    state: State<AppState>,
    id: i64,
    input: UpdateTicket,
) -> Result<Ticket, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    tickets::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn delete_ticket(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    tickets::delete(&conn, id)
}
