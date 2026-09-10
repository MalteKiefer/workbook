use tauri::State;

use crate::db::systems::{self, NewSystem, System, UpdateSystem};
use crate::import::{self, ImportSummary};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_systems(
    state: State<AppState>,
    customer_id: i64,
    include_archived: bool,
) -> Result<Vec<System>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    systems::list_by_customer(&conn, customer_id, include_archived)
}

#[tauri::command]
pub fn create_system(state: State<AppState>, input: NewSystem) -> Result<System, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::create(&conn, input, &tz)
}

#[tauri::command]
pub fn update_system(
    state: State<AppState>,
    id: i64,
    input: UpdateSystem,
) -> Result<System, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::update(&conn, id, input, &tz)
}

#[tauri::command]
pub fn archive_system(state: State<AppState>, id: i64) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    systems::archive(&conn, id, &tz)
}

/// Bulk-creates systems from a CSV file at `csv_path`, all under the same
/// `customer_id` -- picked via the frontend's native file dialog while
/// looking at one customer's system list, see `SystemListView.tsx`. A bad
/// row (missing name) is recorded in `ImportSummary.errors` and skipped;
/// it never aborts the rest of the file. Unlike customers, there's no
/// unique-constraint collision to worry about here -- systems have no
/// unique column. See `import::parse_systems_csv` for the column-header
/// matching rules.
#[tauri::command]
pub fn import_systems_from_csv(
    state: State<AppState>,
    customer_id: i64,
    csv_path: String,
) -> Result<ImportSummary, AppError> {
    let content = std::fs::read_to_string(&csv_path)?;
    let rows = import::parse_systems_csv(&content, customer_id).map_err(AppError::Import)?;

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;

    let mut imported = 0;
    let mut errors = Vec::new();
    for (row, parsed) in rows {
        match parsed {
            Ok(new_system) => match systems::create(&conn, new_system, &tz) {
                Ok(_) => imported += 1,
                Err(e) => errors.push(import::ImportRowError {
                    row,
                    message: e.to_string(),
                }),
            },
            Err(message) => errors.push(import::ImportRowError { row, message }),
        }
    }

    Ok(ImportSummary { imported, errors })
}
