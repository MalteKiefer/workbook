use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter};

use crate::error::AppError;
use crate::window::show_and_focus_main;

/// Id passed to `TrayIconBuilder::with_id` so the tray icon can be looked up
/// again later (`app.tray_by_id`) from `sync_update_indicator`, without
/// having to thread a `TrayIcon` handle through `AppState`.
const TRAY_ID: &str = "main";

pub fn build_tray(app: &AppHandle) -> Result<(), AppError> {
    let quick_capture_item =
        MenuItem::with_id(app, "quick_capture", "Schnellerfassung", true, None::<&str>).map_err(
            |e| {
                AppError::Config(format!(
                    "Tray-Menüeintrag konnte nicht erstellt werden: {e}"
                ))
            },
        )?;
    let show_item =
        MenuItem::with_id(app, "show", "Fenster zeigen", true, None::<&str>).map_err(|e| {
            AppError::Config(format!(
                "Tray-Menüeintrag konnte nicht erstellt werden: {e}"
            ))
        })?;
    let update_item = MenuItem::with_id(
        app,
        "update_available",
        "Nach Updates suchen",
        true,
        None::<&str>,
    )
    .map_err(|e| {
        AppError::Config(format!(
            "Tray-Menüeintrag konnte nicht erstellt werden: {e}"
        ))
    })?;
    let separator = PredefinedMenuItem::separator(app).map_err(|e| {
        AppError::Config(format!("Tray-Trennlinie konnte nicht erstellt werden: {e}"))
    })?;
    let quit_item = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>).map_err(|e| {
        AppError::Config(format!(
            "Tray-Menüeintrag konnte nicht erstellt werden: {e}"
        ))
    })?;

    let menu = Menu::with_items(
        app,
        &[
            &quick_capture_item,
            &show_item,
            &update_item,
            &separator,
            &quit_item,
        ],
    )
    .map_err(|e| AppError::Config(format!("Tray-Menü konnte nicht erstellt werden: {e}")))?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| AppError::Config("Kein Standard-Icon für das Tray verfügbar".to_string()))?;

    TrayIconBuilder::with_id(TRAY_ID)
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
            "update_available" => {
                show_and_focus_main(app);
                if let Err(e) = app.emit("open-update-settings", ()) {
                    eprintln!(
                        "Ereignis \"open-update-settings\" konnte nicht gesendet werden: {e}"
                    );
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_and_focus_main(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| AppError::Config(format!("Tray-Icon konnte nicht erstellt werden: {e}")))?;

    Ok(())
}

/// Reflects the outcome of the most recent update check (automatic or
/// manual, see `updater::apply_update_check_result`) on the tray icon's
/// tooltip -- the "Tray-Hinweis" half of the update-check feature, the
/// other half being the Settings-nav badge in the frontend (driven by the
/// `update-check-completed` event this same call site's caller emits).
/// Silently does nothing if the tray icon can't be found (e.g. platforms
/// without tray support) -- this is a courtesy hint, not load-bearing.
pub fn sync_update_indicator(app: &AppHandle, available_version: Option<&str>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let tooltip = match available_version {
        Some(version) => format!("Wartungsdoku — Update {version} verfügbar"),
        None => "Wartungsdoku".to_string(),
    };
    if let Err(e) = tray.set_tooltip(Some(tooltip)) {
        eprintln!("Tray-Tooltip konnte nicht aktualisiert werden: {e}");
    }
}
