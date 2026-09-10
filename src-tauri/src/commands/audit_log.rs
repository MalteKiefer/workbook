use tauri::State;

use crate::db::audit_log::{self, AuditLogEntry};
use crate::{AppError, AppState};

#[tauri::command]
pub fn list_audit_log_for_entity(
    state: State<AppState>,
    entity_type: String,
    entity_id: i64,
) -> Result<Vec<AuditLogEntry>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    audit_log::list_for_entity(&conn, &entity_type, entity_id)
}
