use tauri::{AppHandle, State};

use crate::{backup, AppError, AppState};

/// Exports a full backup (database snapshot + attachments) to `dest_path` as
/// a single zip file. See `backup::create_backup` for how the database
/// snapshot is taken safely from the live WAL-mode pool.
#[tauri::command]
pub fn create_backup(state: State<AppState>, dest_path: String) -> Result<(), AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let data_dir = state.config.lock().expect("Config-Mutex vergiftet").data_dir.clone();
    backup::create_backup(&conn, &data_dir, std::path::Path::new(&dest_path))
}

/// Stages a restore from the backup zip at `source_path`, then restarts the
/// app so the actual file swap happens at the next startup, before any
/// database pool exists -- see `backup::apply_pending_restore_if_present`.
///
/// On success this never actually returns to the caller: `AppHandle::restart`
/// tears down and relaunches the process. On failure (e.g. an invalid zip),
/// it returns normally with an `AppError` the frontend can show.
#[tauri::command]
pub fn restore_backup(app: AppHandle, state: State<AppState>, source_path: String) -> Result<(), AppError> {
    let data_dir = state.config.lock().expect("Config-Mutex vergiftet").data_dir.clone();
    backup::stage_restore(&data_dir, std::path::Path::new(&source_path))?;
    app.restart();
}
