use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::AppError;
use crate::window::show_and_focus_main;

/// Id passed to `TrayIconBuilder::with_id` so the tray icon can be looked up
/// again later (`app.tray_by_id`) from `sync_tray_tooltip`, without having to
/// thread a `TrayIcon` handle through `AppState`.
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

/// Reflects both live tray-worthy conditions (a pending app update, and any
/// overdue systems) on the tray icon's tooltip in one string, since a tray
/// tooltip can only ever show one message and the two conditions are set by
/// independent call sites (`updater::apply_update_check_result` for the
/// first, `run_maintenance_check` in `lib.rs` for the second) that don't
/// know about each other. Reads both pieces of state itself via `AppState`
/// rather than taking them as parameters, so either caller can just call
/// `sync_tray_tooltip(app)` after updating its own piece of state, with no
/// risk of one call's tooltip write clobbering the other's. Silently does
/// nothing if the tray icon can't be found (e.g. platforms without tray
/// support) -- this is a courtesy hint, not load-bearing.
pub fn sync_tray_tooltip(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let state = app.state::<crate::AppState>();
    let available_version = state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .auto_update_check_available_version
        .clone();
    let overdue_count = *state
        .overdue_systems_count
        .lock()
        .expect("Overdue-Mutex vergiftet");

    let tooltip = compose_tooltip(available_version.as_deref(), overdue_count);
    if let Err(e) = tray.set_tooltip(Some(tooltip)) {
        eprintln!("Tray-Tooltip konnte nicht aktualisiert werden: {e}");
    }
}

/// Pure composition of the tray tooltip string from both tray-worthy
/// conditions, extracted out of `sync_tray_tooltip` so it can be unit
/// tested without a running Tauri app / tray icon.
fn compose_tooltip(available_version: Option<&str>, overdue_count: usize) -> String {
    let mut parts = Vec::new();
    if let Some(version) = available_version {
        parts.push(format!("Update {version} verfügbar"));
    }
    if overdue_count > 0 {
        parts.push(format!(
            "{overdue_count} System{plural} überfällig",
            plural = if overdue_count == 1 { "" } else { "e" }
        ));
    }
    if parts.is_empty() {
        "Wartungsdoku".to_string()
    } else {
        format!("Wartungsdoku — {}", parts.join(" · "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_tooltip_with_neither_condition() {
        assert_eq!(compose_tooltip(None, 0), "Wartungsdoku");
    }

    #[test]
    fn compose_tooltip_with_only_update_available() {
        assert_eq!(
            compose_tooltip(Some("1.2.3"), 0),
            "Wartungsdoku — Update 1.2.3 verfügbar"
        );
    }

    #[test]
    fn compose_tooltip_with_only_overdue_singular() {
        assert_eq!(compose_tooltip(None, 1), "Wartungsdoku — 1 System überfällig");
    }

    #[test]
    fn compose_tooltip_with_only_overdue_plural() {
        assert_eq!(compose_tooltip(None, 3), "Wartungsdoku — 3 Systeme überfällig");
    }

    #[test]
    fn compose_tooltip_with_both_conditions() {
        assert_eq!(
            compose_tooltip(Some("1.2.3"), 2),
            "Wartungsdoku — Update 1.2.3 verfügbar · 2 Systeme überfällig"
        );
    }
}
