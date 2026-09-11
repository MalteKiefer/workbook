use std::path::Path;

use tauri::State;

use crate::db::attachments::{self, Attachment, AttachmentStorageSummary};
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn list_attachments_for_entry(
    state: State<AppState>,
    entry_id: i64,
) -> Result<Vec<Attachment>, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    attachments::list_for_entry(&conn, entry_id)
}

#[tauri::command]
pub fn add_attachment_to_entry(
    state: State<AppState>,
    entry_id: i64,
    bytes_base64: String,
    original_filename: String,
    mime_type: String,
) -> Result<Attachment, AppError> {
    use base64::prelude::*;
    let bytes = BASE64_STANDARD
        .decode(&bytes_base64)
        .map_err(|e| AppError::Config(format!("Anhang konnte nicht dekodiert werden: {e}")))?;
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let tz = time::system_timezone()?;
    let attachment = crate::attachments::store::attach_bytes_to_entry(
        &conn,
        &data_dir,
        entry_id,
        &bytes,
        &original_filename,
        &mime_type,
        &tz,
    )?;
    upload_attachment_to_cloud_if_enabled(&state, &data_dir, &attachment);
    Ok(attachment)
}

/// Best-effort: uploads one attachment to the configured cloud bucket if
/// `cloud_storage_enabled` and fully configured. Never fails the caller
/// -- the local attachment that was just saved is already safe on disk;
/// a cloud-upload problem (network, bad credentials, misconfigured
/// bucket) is logged and otherwise ignored, same convention
/// `commands::backup::upload_backup_to_cloud_if_enabled` already
/// follows for full backups. The S3 key reuses the exact same
/// content-addressed relative path the local store already uses
/// (`attachments/<sha256 prefix>/<sha256><ext>`, via
/// `attachments::store::relative_path_for`), so a cloud bucket mirrors
/// the same layout as the local `attachments/` folder.
pub(crate) fn upload_attachment_to_cloud_if_enabled(
    state: &State<AppState>,
    data_dir: &Path,
    attachment: &crate::db::attachments::Attachment,
) {
    let enabled = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .cloud_storage_enabled;
    if !enabled {
        return;
    }
    let (settings, secret_key) =
        match crate::commands::cloud_storage::current_cloud_storage_settings(state) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "Cloud-Upload für Anhang übersprungen (Konfiguration unvollständig): {e}"
                );
                return;
            }
        };
    let relative_path = crate::attachments::store::relative_path_for(
        &attachment.sha256,
        &attachment.original_filename,
    );
    let local_path = data_dir.join(&relative_path);
    if let Err(e) =
        crate::cloud_storage::upload_file(&settings, &secret_key, &local_path, &relative_path)
    {
        eprintln!("Cloud-Upload für Anhang fehlgeschlagen: {e}");
    }
}

#[tauri::command]
pub fn remove_attachment(state: State<AppState>, attachment_id: i64) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    attachments::delete(&conn, attachment_id)
}

#[tauri::command]
pub fn read_attachment_data_url(
    state: State<AppState>,
    attachment_id: i64,
) -> Result<String, AppError> {
    use base64::prelude::*;
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let attachment = attachments::get(&conn, attachment_id)?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let relative_path = crate::attachments::store::relative_path_for(
        &attachment.sha256,
        &attachment.original_filename,
    );
    let bytes = std::fs::read(data_dir.join(&relative_path))
        .map_err(|e| AppError::Io(format!("Anhang konnte nicht gelesen werden: {e}")))?;
    Ok(format!(
        "data:{};base64,{}",
        attachment.mime_type,
        BASE64_STANDARD.encode(&bytes)
    ))
}

#[tauri::command]
pub fn copy_attachment_to(
    state: State<AppState>,
    attachment_id: i64,
    dest_path: String,
) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let attachment = attachments::get(&conn, attachment_id)?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let relative_path = crate::attachments::store::relative_path_for(
        &attachment.sha256,
        &attachment.original_filename,
    );
    std::fs::copy(data_dir.join(&relative_path), &dest_path)
        .map_err(|e| AppError::Io(format!("Anhang konnte nicht exportiert werden: {e}")))?;
    Ok(())
}

/// Result of an explicit cleanup run. Triggered exclusively manually via
/// `cleanup_orphans`, never automatically in the background (see spec,
/// section "Error handling & transactions").
#[derive(Debug, Clone, serde::Serialize)]
pub struct CleanupResult {
    pub removed_count: u32,
    pub removed_bytes: u64,
}

#[tauri::command]
pub fn cleanup_orphans(state: State<AppState>) -> Result<CleanupResult, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let referenced = attachments::all_referenced_hashes(&conn)?;
    let stored = crate::attachments::store::list_all_stored_files(&data_dir)?;
    let mut removed_count = 0u32;
    let mut removed_bytes = 0u64;
    for file in stored {
        if !referenced.contains(&file.sha256) && std::fs::remove_file(&file.absolute_path).is_ok() {
            removed_count += 1;
            removed_bytes += file.size_bytes;
        }
    }
    Ok(CleanupResult {
        removed_count,
        removed_bytes,
    })
}

#[tauri::command]
pub fn get_attachment_storage_summary(
    state: State<AppState>,
) -> Result<AttachmentStorageSummary, AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    attachments::storage_summary(&conn)
}

#[tauri::command]
pub fn open_attachment(
    app: tauri::AppHandle,
    state: State<AppState>,
    attachment_id: i64,
) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let attachment = attachments::get(&conn, attachment_id)?;
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let relative_path = crate::attachments::store::relative_path_for(
        &attachment.sha256,
        &attachment.original_filename,
    );
    let absolute_path = data_dir.join(&relative_path);
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(absolute_path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| AppError::Io(format!("Anhang konnte nicht geöffnet werden: {e}")))
}
