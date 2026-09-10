//! Custom keyboard bindings (Settings -> Tastaturbelegung). Two separate
//! config sections with different semantics:
//! - `KeymapConfig` (get_keymap/set_keymap): in-app shortcuts, checked by
//!   JS keydown handlers (see `src/lib/keymap.ts`), apply immediately --
//!   `set_keymap` broadcasts a `"keymap-changed"` event so an already-open
//!   window (e.g. quick capture) picks up the change live.
//! - `HotkeyConfig` (get_hotkeys/set_hotkeys): OS-registered global
//!   hotkeys via tauri-plugin-global-shortcut (`src-tauri/src/hotkeys.rs`),
//!   only read at startup -- a change here needs an app restart, which the
//!   frontend surfaces explicitly rather than pretending it's live.

use tauri::{AppHandle, Emitter, State};

use crate::config::{HotkeyConfig, KeymapConfig};
use crate::{AppError, AppState};

#[tauri::command]
pub fn get_keymap(state: State<AppState>) -> KeymapConfig {
    state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .keymap
        .clone()
}

#[tauri::command]
pub fn set_keymap(
    state: State<AppState>,
    app: AppHandle,
    keymap: KeymapConfig,
) -> Result<(), AppError> {
    validate_keymap(&keymap)?;
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.keymap = keymap.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    app.emit("keymap-changed", keymap)
        .map_err(|e| AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}")))?;
    Ok(())
}

#[tauri::command]
pub fn get_hotkeys(state: State<AppState>) -> HotkeyConfig {
    state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .hotkeys
        .clone()
}

#[tauri::command]
pub fn set_hotkeys(state: State<AppState>, hotkeys: HotkeyConfig) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.hotkeys = hotkeys;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

/// Rejects an empty binding, two actions sharing an identical binding
/// (case-insensitively -- Shift changes a letter's case but not the
/// physical key), or the three `goto_*` two-key sequences not sharing the
/// same prefix word (the frontend's pending-prefix state machine only
/// tracks one prefix at a time, see `src/hooks/useGlobalHotkeys.ts`).
fn validate_keymap(keymap: &KeymapConfig) -> Result<(), AppError> {
    let entries: [(&str, &str); 9] = [
        ("Command Palette", keymap.command_palette.as_str()),
        ("Schnellerfassung", keymap.quick_capture.as_str()),
        ("Speichern", keymap.save.as_str()),
        ("Zu Kundenliste", keymap.goto_customers.as_str()),
        ("Zu Systemliste", keymap.goto_systems.as_str()),
        ("Zum Journal", keymap.goto_journal.as_str()),
        ("Liste: nächster Eintrag", keymap.list_next.as_str()),
        ("Liste: vorheriger Eintrag", keymap.list_prev.as_str()),
        ("Ausgewähltes bearbeiten", keymap.edit_selected.as_str()),
    ];

    for (label, binding) in entries {
        if binding.trim().is_empty() {
            return Err(AppError::Config(format!(
                "\"{label}\" braucht eine Tastenkombination"
            )));
        }
    }

    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            if entries[i].1.eq_ignore_ascii_case(entries[j].1) {
                return Err(AppError::Config(format!(
                    "\"{}\" und \"{}\" haben dieselbe Tastenkombination (\"{}\")",
                    entries[i].0, entries[j].0, entries[i].1
                )));
            }
        }
    }

    let customers_prefix = keymap.goto_customers.split(' ').next().unwrap_or("");
    let systems_prefix = keymap.goto_systems.split(' ').next().unwrap_or("");
    let journal_prefix = keymap.goto_journal.split(' ').next().unwrap_or("");
    if customers_prefix != systems_prefix || customers_prefix != journal_prefix {
        return Err(AppError::Config(
            "\"Zu Kundenliste\", \"Zu Systemliste\" und \"Zum Journal\" müssen mit derselben ersten Taste beginnen".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_keymap() -> KeymapConfig {
        KeymapConfig::default()
    }

    #[test]
    fn default_keymap_passes_validation() {
        assert!(validate_keymap(&valid_keymap()).is_ok());
    }

    #[test]
    fn empty_binding_is_rejected() {
        let mut keymap = valid_keymap();
        keymap.save = "   ".to_string();
        let result = validate_keymap(&keymap);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn duplicate_binding_is_rejected() {
        let mut keymap = valid_keymap();
        keymap.save = keymap.command_palette.clone();
        let result = validate_keymap(&keymap);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn duplicate_binding_is_rejected_case_insensitively() {
        let mut keymap = valid_keymap();
        keymap.list_next = "E".to_string();
        keymap.edit_selected = "e".to_string();
        let result = validate_keymap(&keymap);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn mismatched_goto_prefixes_are_rejected() {
        let mut keymap = valid_keymap();
        keymap.goto_systems = "x s".to_string();
        let result = validate_keymap(&keymap);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn matching_goto_prefixes_with_a_different_letter_are_accepted() {
        let mut keymap = valid_keymap();
        keymap.goto_customers = "x c".to_string();
        keymap.goto_systems = "x s".to_string();
        keymap.goto_journal = "x j".to_string();
        assert!(validate_keymap(&keymap).is_ok());
    }
}
