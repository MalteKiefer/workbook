//! Settings for the automatic update-check scheduler (Settings ->
//! Aktualisierung). The manual "Nach Updates suchen" check itself still
//! runs entirely in the frontend via `@tauri-apps/plugin-updater`'s JS
//! `check()` (see `UpdateSettingsView.tsx`) -- `record_update_check_result`
//! below only records that check's outcome the same way the background
//! scheduler (`updater::apply_update_check_result` in `lib.rs`) does, so
//! both paths keep `config.toml`, the tray tooltip, and every window's
//! badge in sync regardless of which one actually ran the check.

use tauri::{AppHandle, State};

use crate::config::AutoUpdateCheckFrequency;
use crate::{updater, AppError, AppState};

#[derive(Debug, serde::Serialize)]
pub struct UpdateCheckSettingsDto {
    pub enabled: bool,
    pub frequency: AutoUpdateCheckFrequency,
    pub last_run_utc: Option<String>,
    pub available_version: Option<String>,
}

#[tauri::command]
pub fn get_update_check_settings(state: State<AppState>) -> UpdateCheckSettingsDto {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    UpdateCheckSettingsDto {
        enabled: config.auto_update_check_enabled,
        frequency: config.auto_update_check_frequency,
        last_run_utc: config.auto_update_check_last_run_utc.clone(),
        available_version: config.auto_update_check_available_version.clone(),
    }
}

#[tauri::command]
pub fn set_auto_update_check_settings(
    state: State<AppState>,
    enabled: bool,
    frequency: AutoUpdateCheckFrequency,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.auto_update_check_enabled = enabled;
    config.auto_update_check_frequency = frequency;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

/// Called by `UpdateSettingsView.tsx` right after its own manual
/// `check()` resolves, with the found update's version (or `None` if
/// already up to date) -- see the module doc above for why this exists
/// instead of the frontend just updating local state itself.
#[tauri::command]
pub fn record_update_check_result(app: AppHandle, available_version: Option<String>) {
    updater::apply_update_check_result(&app, available_version);
}
