use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};

use crate::backup::crypto;
use crate::config::AutoBackupFrequency;
use crate::{backup, plugin::secrets, AppError, AppState};

/// Exports a full backup (database snapshot + attachments) to `dest_path` as
/// a single zip file, or -- if backup encryption is enabled in settings -- as
/// an encrypted file requiring the passphrase set via
/// `set_backup_encryption_passphrase`. See `backup::create_backup` for how
/// the database snapshot is taken safely from the live WAL-mode pool.
#[tauri::command]
pub fn create_backup(state: State<AppState>, dest_path: String) -> Result<(), AppError> {
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let (data_dir, encryption_enabled) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.data_dir.clone(), config.backup_encryption_enabled)
    };
    let dest_path = Path::new(&dest_path);

    if encryption_enabled {
        let passphrase = secrets::load_secret(crypto::SECRET_ID)?.ok_or_else(|| {
            AppError::Backup(
                "Verschlüsselung ist aktiviert, aber es ist noch kein Passwort gesetzt. In den Einstellungen unter Backup ein Passwort festlegen.".to_string(),
            )
        })?;
        backup::create_backup_encrypted(&conn, &data_dir, dest_path, &passphrase)
    } else {
        backup::create_backup(&conn, &data_dir, dest_path)
    }
}

/// Whether the file at `path` was produced by this app's backup encryption --
/// used by the frontend to decide whether to prompt for a passphrase before
/// calling `restore_backup`.
#[tauri::command]
pub fn is_backup_file_encrypted(path: String) -> Result<bool, AppError> {
    crypto::is_encrypted_file(Path::new(&path))
}

/// Stages a restore from the backup file at `source_path`, then restarts the
/// app so the actual file swap happens at the next startup, before any
/// database pool exists -- see `backup::apply_pending_restore_if_present`.
///
/// `passphrase` is required when `source_path` is an encrypted backup (the
/// frontend checks this upfront via `is_backup_file_encrypted`); ignored for
/// a plain zip.
///
/// On success this never actually returns to the caller: `AppHandle::restart`
/// tears down and relaunches the process. On failure (e.g. an invalid zip or
/// wrong passphrase), it returns normally with an `AppError` the frontend can
/// show.
#[tauri::command]
pub fn restore_backup(
    app: AppHandle,
    state: State<AppState>,
    source_path: String,
    passphrase: Option<String>,
) -> Result<(), AppError> {
    let data_dir = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .data_dir
        .clone();
    let source_path = Path::new(&source_path);

    if crypto::is_encrypted_file(source_path)? {
        let passphrase = passphrase.ok_or_else(|| {
            AppError::Backup(
                "Diese Backup-Datei ist verschlüsselt -- ein Passwort ist erforderlich".to_string(),
            )
        })?;
        backup::stage_restore_encrypted(&data_dir, source_path, &passphrase)?;
    } else {
        backup::stage_restore(&data_dir, source_path)?;
    }
    app.restart();
}

/// Snapshot of every backup-related setting, read together so the frontend
/// only needs one round-trip to populate its Backup settings view.
#[derive(Debug, serde::Serialize)]
pub struct BackupSettingsDto {
    pub auto_backup_enabled: bool,
    pub auto_backup_dir: Option<String>,
    pub auto_backup_frequency: AutoBackupFrequency,
    pub auto_backup_last_run_utc: Option<String>,
    pub encryption_enabled: bool,
    pub has_encryption_passphrase: bool,
}

#[tauri::command]
pub fn get_backup_settings(state: State<AppState>) -> Result<BackupSettingsDto, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(BackupSettingsDto {
        auto_backup_enabled: config.auto_backup_enabled,
        auto_backup_dir: config
            .auto_backup_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        auto_backup_frequency: config.auto_backup_frequency,
        auto_backup_last_run_utc: config.auto_backup_last_run_utc.clone(),
        encryption_enabled: config.backup_encryption_enabled,
        has_encryption_passphrase: secrets::load_secret(crypto::SECRET_ID)?.is_some(),
    })
}

#[tauri::command]
pub fn set_auto_backup_settings(
    state: State<AppState>,
    enabled: bool,
    dir: Option<String>,
    frequency: AutoBackupFrequency,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.auto_backup_enabled = enabled;
    config.auto_backup_dir = dir.map(PathBuf::from);
    config.auto_backup_frequency = frequency;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn set_backup_encryption_enabled(
    state: State<AppState>,
    enabled: bool,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.backup_encryption_enabled = enabled;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

/// Sets (or replaces) the passphrase used for backup encryption, stored only
/// in the OS keyring -- never in `config.toml` or the database. Changing it
/// has no effect on backups already created with the previous passphrase.
#[tauri::command]
pub fn set_backup_encryption_passphrase(passphrase: String) -> Result<(), AppError> {
    secrets::store_secret(crypto::SECRET_ID, &passphrase)
}
