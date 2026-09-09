//! General app settings that don't belong to a more specific module
//! (Settings -> General in the frontend). Currently just the theme
//! preference; further global settings can be added here.

use tauri::{AppHandle, Emitter, State};

use crate::config::ThemePreference;
use crate::{AppError, AppState};

#[tauri::command]
pub fn get_theme_preference(state: State<AppState>) -> ThemePreference {
    state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .theme_preference
}

#[tauri::command]
pub fn set_theme_preference(
    state: State<AppState>,
    app: AppHandle,
    preference: ThemePreference,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.theme_preference = preference;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    // App-wide broadcast (not just to the window that invoked the command) --
    // so an already-open quick capture window picks up the change live when
    // the setting is changed in the main window (and vice versa), see
    // `src/lib/theme.ts::listenForThemeChanges`.
    app.emit("theme-changed", preference)
        .map_err(|e| AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}")))?;

    Ok(())
}
