use tauri::State;

use crate::db::customers::{self, Customer, NewCustomer, UpdateCustomer};
use crate::import::{self, ImportSummary};
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

#[derive(Debug, serde::Serialize)]
pub struct BulkArchiveSummary {
    pub archived: usize,
    pub errors: Vec<String>,
}

/// Archives every id in `ids`, one at a time. One id that fails to
/// archive (e.g. already archived, or doesn't exist) is recorded in
/// `errors` and does not stop the rest -- same "one bad item doesn't
/// abort the whole batch" philosophy as `import_customers_from_csv`.
#[tauri::command]
pub fn archive_customers(
    state: State<AppState>,
    ids: Vec<i64>,
) -> Result<BulkArchiveSummary, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;

    let mut archived = 0;
    let mut errors = Vec::new();
    for id in ids {
        match customers::archive(&conn, id, &tz) {
            Ok(()) => archived += 1,
            Err(e) => errors.push(format!("Kunde #{id}: {e}")),
        }
    }
    Ok(BulkArchiveSummary { archived, errors })
}

/// Bulk-creates customers from a CSV file at `csv_path` (picked via the
/// frontend's native file dialog, see `CustomerListView.tsx`). One bad row
/// -- a missing required field, or a `short_code` collision with an
/// existing customer -- is recorded in `ImportSummary.errors` and skipped;
/// it never aborts the rest of the file. See `import::parse_customers_csv`
/// for the column-header matching rules.
#[tauri::command]
pub fn import_customers_from_csv(
    state: State<AppState>,
    csv_path: String,
) -> Result<ImportSummary, AppError> {
    let content = std::fs::read_to_string(&csv_path)?;
    let rows = import::parse_customers_csv(&content).map_err(AppError::Import)?;

    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;

    let mut imported = 0;
    let mut errors = Vec::new();
    for (row, parsed) in rows {
        match parsed {
            Ok(new_customer) => match customers::create(&conn, new_customer, &tz) {
                Ok(_) => imported += 1,
                Err(e) => errors.push(import::ImportRowError {
                    row,
                    message: friendly_customer_error(&e),
                }),
            },
            Err(message) => errors.push(import::ImportRowError { row, message }),
        }
    }

    Ok(ImportSummary { imported, errors })
}

/// Rewrites the raw SQLite `UNIQUE constraint failed: customers.short_code`
/// message into something a non-technical reader recognizes, without
/// hiding any other database error's real message.
fn friendly_customer_error(e: &AppError) -> String {
    let message = e.to_string();
    if message.contains("UNIQUE constraint failed") && message.contains("short_code") {
        "Kürzel bereits vergeben.".to_string()
    } else {
        message
    }
}
