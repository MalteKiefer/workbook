use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::config::HotkeyConfig;
use crate::error::AppError;
use crate::{quickcapture, window};

pub fn register(app: &AppHandle, hotkeys: &HotkeyConfig) -> Result<(), AppError> {
    let quick_capture = parse_shortcut(&hotkeys.quick_capture, "Schnellerfassung");
    let search = parse_shortcut(&hotkeys.search, "Suche");
    let clipboard_screenshot =
        parse_shortcut(&hotkeys.clipboard_screenshot, "Zwischenablage-Screenshot");

    let quick_capture_for_handler = quick_capture;
    let search_for_handler = search;
    let clipboard_for_handler = clipboard_screenshot;

    app.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                if Some(shortcut) == quick_capture_for_handler.as_ref() {
                    if let Err(e) = quickcapture::open(app) {
                        eprintln!("Schnellerfassung (Hotkey) fehlgeschlagen: {e}");
                    }
                } else if Some(shortcut) == search_for_handler.as_ref() {
                    window::show_and_focus_main(app);
                } else if Some(shortcut) == clipboard_for_handler.as_ref() {
                    if let Err(e) = quickcapture::open_with_clipboard_screenshot(app) {
                        eprintln!("Zwischenablage-Screenshot (Hotkey) fehlgeschlagen: {e}");
                    }
                }
            })
            .build(),
    )
    .map_err(|e| {
        AppError::Config(format!(
            "Global-Shortcut-Plugin konnte nicht registriert werden: {e}"
        ))
    })?;

    // Each hotkey is registered individually (not via with_shortcuts() as one
    // batch registration), because the OS grants hotkeys exclusively: if one of
    // them is already taken by another application, that should only disable
    // that one, not take the other two down with it.
    let shortcut_manager = app.global_shortcut();
    for (shortcut, label) in [
        (quick_capture, "Schnellerfassung"),
        (search, "Suche"),
        (clipboard_screenshot, "Zwischenablage-Screenshot"),
    ] {
        if let Some(shortcut) = shortcut {
            if let Err(e) = shortcut_manager.register(shortcut) {
                eprintln!("Hotkey für \"{label}\" konnte nicht registriert werden (evtl. von einer anderen Anwendung belegt): {e}");
            }
        }
    }

    Ok(())
}

fn parse_shortcut(raw: &str, label: &str) -> Option<Shortcut> {
    match raw.parse::<Shortcut>() {
        Ok(shortcut) => Some(shortcut),
        Err(_) => {
            eprintln!("Hotkey für \"{label}\" (\"{raw}\") konnte nicht interpretiert werden — deaktiviert.");
            None
        }
    }
}
