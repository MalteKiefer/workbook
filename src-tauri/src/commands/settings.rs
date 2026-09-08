//! Allgemeine App-Einstellungen, die zu keinem spezifischeren Modul gehören
//! (Einstellungen → Allgemein im Frontend). Aktuell nur die Theme-Präferenz;
//! weitere globale Einstellungen können hier ergänzt werden.

use tauri::{AppHandle, Emitter, State};

use crate::config::ThemePreference;
use crate::{AppError, AppState};

#[tauri::command]
pub fn get_theme_preference(state: State<AppState>) -> ThemePreference {
    state.config.lock().expect("Config-Mutex vergiftet").theme_preference
}

#[tauri::command]
pub fn set_theme_preference(state: State<AppState>, app: AppHandle, preference: ThemePreference) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.theme_preference = preference;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    // App-weites Broadcast (nicht nur an das Fenster, das den Command
    // aufgerufen hat) -- damit ein bereits offenes Schnellerfassungsfenster
    // live nachzieht, wenn die Einstellung im Hauptfenster geändert wird (und
    // umgekehrt), siehe `src/lib/theme.ts::listenForThemeChanges`.
    app.emit("theme-changed", preference)
        .map_err(|e| AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}")))?;

    Ok(())
}
