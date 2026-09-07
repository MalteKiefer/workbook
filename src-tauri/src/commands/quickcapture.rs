use tauri::State;

use crate::db::{customers, systems};
use crate::{AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct LastSelection {
    pub customer: Option<customers::Customer>,
    pub system: Option<systems::System>,
}

#[tauri::command]
pub fn get_last_selection(state: State<AppState>) -> Result<LastSelection, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let (last_customer_id, last_system_id) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.last_customer_id, config.last_system_id)
    };
    let customer = last_customer_id.and_then(|id| customers::get(&conn, id).ok());
    let system = last_system_id.and_then(|id| systems::get(&conn, id).ok());
    Ok(LastSelection { customer, system })
}
