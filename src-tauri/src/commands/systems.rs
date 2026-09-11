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

#[derive(Debug, serde::Serialize)]
pub struct BulkArchiveSummary {
    pub archived: usize,
    pub errors: Vec<String>,
}

/// Archives every id in `ids`, one at a time. One id that fails to
/// archive (e.g. already archived, or doesn't exist) is recorded in
/// `errors` and does not stop the rest -- same "one bad item doesn't
/// abort the whole batch" philosophy as `import_systems_from_csv`.
#[tauri::command]
pub fn archive_systems(
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
        match systems::archive(&conn, id, &tz) {
            Ok(()) => archived += 1,
            Err(e) => errors.push(format!("System #{id}: {e}")),
        }
    }
    Ok(BulkArchiveSummary { archived, errors })
}

/// A `System` plus its computed maintenance status -- whether it's overdue
/// per `maintenance::is_overdue` and the timestamp that status was
/// computed from (`None` if it has never had an entry). An ADDITIONAL
/// command alongside `list_systems` -- it doesn't replace it, other code
/// still uses the plain list.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemWithMaintenanceStatus {
    #[serde(flatten)]
    pub system: System,
    pub overdue: bool,
    pub last_performed_at_utc: Option<String>,
}

#[tauri::command]
pub fn list_systems_with_maintenance_status(
    state: State<AppState>,
    customer_id: i64,
    include_archived: bool,
) -> Result<Vec<SystemWithMaintenanceStatus>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let systems = systems::list_by_customer(&conn, customer_id, include_archived)?;
    let now = chrono::Utc::now();
    systems
        .into_iter()
        .map(|system| {
            let last_performed_at_utc = systems::latest_performed_at(&conn, system.id)?;
            let overdue = crate::maintenance::is_overdue(
                system.maintenance_interval_days,
                last_performed_at_utc.as_deref(),
                &system.created_at_utc,
                now,
            );
            Ok(SystemWithMaintenanceStatus {
                system,
                overdue,
                last_performed_at_utc,
            })
        })
        .collect()
}

/// A single overdue system for the Dashboard's cross-customer overview,
/// carrying its customer's name alongside it since `Dashboard` has no other
/// way to resolve `customer_id` -> name without a second round trip per row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OverdueSystemDto {
    pub system_id: i64,
    pub system_name: String,
    pub customer_id: i64,
    pub customer_name: String,
    pub last_performed_at_utc: Option<String>,
}

/// Overdue systems across every active customer, for the Dashboard view.
/// Reuses the exact same `maintenance::is_overdue` logic as
/// `list_systems_with_maintenance_status`, just fanned out over every
/// customer instead of one -- this app's expected data volumes (one
/// admin, at most a few hundred systems) make the nested loop below fine
/// without a dedicated SQL join.
#[tauri::command]
pub fn list_overdue_systems(state: State<AppState>) -> Result<Vec<OverdueSystemDto>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let customers = crate::db::customers::list(&conn, false)?;
    let now = chrono::Utc::now();

    let mut result = Vec::new();
    for customer in customers {
        let customer_systems = systems::list_by_customer(&conn, customer.id, false)?;
        for system in customer_systems {
            let last_performed_at_utc = systems::latest_performed_at(&conn, system.id)?;
            let overdue = crate::maintenance::is_overdue(
                system.maintenance_interval_days,
                last_performed_at_utc.as_deref(),
                &system.created_at_utc,
                now,
            );
            if overdue {
                result.push(OverdueSystemDto {
                    system_id: system.id,
                    system_name: system.name,
                    customer_id: customer.id,
                    customer_name: customer.name.clone(),
                    last_performed_at_utc,
                });
            }
        }
    }
    Ok(result)
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
    let content = import::read_csv_file(&csv_path)?;
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
