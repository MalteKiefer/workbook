use tauri::State;

use crate::db::entries::EntryFilter;
use crate::{AppError, AppState};

#[tauri::command]
pub fn export_markdown(
    state: State<AppState>,
    customer_id: i64,
    system_id: Option<i64>,
    from_utc: Option<String>,
    to_utc: Option<String>,
    dest_dir: String,
) -> Result<(), AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let (data_dir, late_entry_threshold_hours) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.data_dir.clone(), config.late_entry_threshold_hours)
    };
    let filter = EntryFilter {
        customer_id: Some(customer_id),
        system_id,
        category: None,
        tag: None,
        from_utc,
        to_utc,
    };
    crate::export::markdown::export_markdown(
        &conn,
        &data_dir,
        std::path::Path::new(&dest_dir),
        customer_id,
        &filter,
        late_entry_threshold_hours,
    )
}
