use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use crate::error::AppError;
use crate::window::show_and_focus_main;

pub fn build_tray(app: &AppHandle) -> Result<(), AppError> {
    let quick_capture_item = MenuItem::with_id(app, "quick_capture", "Schnellerfassung", true, None::<&str>)
        .map_err(|e| AppError::Config(format!("Tray-Menüeintrag konnte nicht erstellt werden: {e}")))?;
    let show_item = MenuItem::with_id(app, "show", "Fenster zeigen", true, None::<&str>)
        .map_err(|e| AppError::Config(format!("Tray-Menüeintrag konnte nicht erstellt werden: {e}")))?;
    let separator = PredefinedMenuItem::separator(app)
        .map_err(|e| AppError::Config(format!("Tray-Trennlinie konnte nicht erstellt werden: {e}")))?;
    let quit_item = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)
        .map_err(|e| AppError::Config(format!("Tray-Menüeintrag konnte nicht erstellt werden: {e}")))?;

    let menu = Menu::with_items(app, &[&quick_capture_item, &show_item, &separator, &quit_item])
        .map_err(|e| AppError::Config(format!("Tray-Menü konnte nicht erstellt werden: {e}")))?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| AppError::Config("Kein Standard-Icon für das Tray verfügbar".to_string()))?;

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Wartungsdoku")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quick_capture" => {
                if let Err(e) = crate::quickcapture::open(app) {
                    eprintln!("Schnellerfassung konnte nicht geöffnet werden: {e}");
                }
            }
            "show" => show_and_focus_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_and_focus_main(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| AppError::Config(format!("Tray-Icon konnte nicht erstellt werden: {e}")))?;

    Ok(())
}
