use tauri::State;

use crate::db::entries::{self, Entry, EntryFilter, NewEntry, UpdateEntry};
use crate::{time, AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TemporalPreview {
    pub utc: String,
    pub tz: String,
}

#[tauri::command]
pub fn list_entries(state: State<AppState>, filter: EntryFilter) -> Result<Vec<Entry>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    entries::list(&conn, &filter)
}

#[tauri::command]
pub fn get_entry(state: State<AppState>, id: i64) -> Result<Entry, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    entries::get(&conn, id)
}

#[tauri::command]
pub fn create_entry(state: State<AppState>, input: NewEntry) -> Result<Entry, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let had_pending_attachments = !input.pending_attachments.is_empty();
    let entry = entries::create(&conn, &data_dir, input, &tz)?;

    if had_pending_attachments {
        upload_new_entry_attachments_to_cloud_if_enabled(&state, &conn, &data_dir, entry.id);
    }

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.last_customer_id = Some(entry.customer_id);
    config.last_system_id = entry.system_id;
    let config_path = config.data_dir.join("config.toml");
    if let Err(e) = config.save(&config_path) {
        eprintln!("Letzte Auswahl konnte nicht gespeichert werden: {e}");
    }

    Ok(entry)
}

#[tauri::command]
pub fn format_timestamp_for_display(utc: String, tz: String) -> Result<String, AppError> {
    time::format_timestamp_for_display(&utc, &tz)
}

#[tauri::command]
pub fn update_entry(
    state: State<AppState>,
    id: i64,
    input: UpdateEntry,
) -> Result<Entry, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let had_pending_attachments = !input.pending_attachments.is_empty();
    let entry = entries::update(&conn, &data_dir, id, input, &tz)?;
    if had_pending_attachments {
        upload_new_entry_attachments_to_cloud_if_enabled(&state, &conn, &data_dir, entry.id);
    }
    Ok(entry)
}

/// Best-effort cloud upload for every attachment currently on `entry_id`,
/// called only right after a `create_entry`/`update_entry` call that
/// actually submitted new pending attachments (checked by the caller via
/// `had_pending_attachments`, before `input` was moved into
/// `entries::create`/`entries::update`). Lists ALL of the entry's
/// attachments rather than tracking exactly which ones were newly
/// resolved this call -- simpler, and re-uploading an unchanged
/// pre-existing attachment is harmless (same bytes, same content-addressed
/// key, no-op overwrite on the bucket side) -- see
/// `commands::attachments::upload_attachment_to_cloud_if_enabled`, reused
/// here per-attachment, for the actual per-file upload/skip logic and the
/// "never fails the caller" contract.
fn upload_new_entry_attachments_to_cloud_if_enabled(
    state: &State<AppState>,
    conn: &rusqlite::Connection,
    data_dir: &std::path::Path,
    entry_id: i64,
) {
    let attachments = match crate::db::attachments::list_for_entry(conn, entry_id) {
        Ok(list) => list,
        Err(e) => {
            eprintln!(
                "Cloud-Upload für neue Anhänge übersprungen (Anhänge konnten nicht geladen werden): {e}"
            );
            return;
        }
    };
    for attachment in &attachments {
        crate::commands::attachments::upload_attachment_to_cloud_if_enabled(
            state, data_dir, attachment,
        );
    }
}

#[tauri::command]
pub fn parse_temporal_input(input: String) -> Result<TemporalPreview, AppError> {
    let tz = time::system_timezone()?;
    let now = chrono::Utc::now();
    let (utc, tz_name) = time::parse_temporal_input(&input, &tz, now)?;
    Ok(TemporalPreview { utc, tz: tz_name })
}
