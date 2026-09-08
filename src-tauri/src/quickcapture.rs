use tauri::{AppHandle, Emitter, Manager};

use crate::context_capture;
use crate::error::AppError;
use crate::time;
use crate::window;
use crate::AppState;

#[derive(Clone, serde::Serialize)]
pub struct QuickCaptureActivated {
    pub performed_at_utc: String,
    pub performed_at_tz: String,
    pub context_note: Option<String>,
    pub override_customer_id: Option<i64>,
    pub override_system_id: Option<i64>,
}

pub fn open(app: &AppHandle) -> Result<(), AppError> {
    open_internal(app, None, None)
}

pub fn open_with_context(
    app: &AppHandle,
    customer_id: Option<i64>,
    system_id: Option<i64>,
) -> Result<(), AppError> {
    open_internal(app, customer_id, system_id)
}

fn open_internal(
    app: &AppHandle,
    override_customer_id: Option<i64>,
    override_system_id: Option<i64>,
) -> Result<(), AppError> {
    let window = app
        .get_webview_window("quick-capture")
        .ok_or_else(|| AppError::Config("Schnellerfassungsfenster nicht gefunden".to_string()))?;

    let previous = context_capture::capture_foreground();
    {
        let state = app.state::<AppState>();
        *state
            .previous_foreground
            .lock()
            .expect("Foreground-Mutex vergiftet") = previous;
    }

    let context_note = {
        let state = app.state::<AppState>();
        let enabled = state
            .config
            .lock()
            .expect("Config-Mutex vergiftet")
            .context_capture_enabled;
        if enabled {
            context_capture::foreground_window_title()
        } else {
            None
        }
    };

    let tz = time::system_timezone()?;
    let (performed_at_utc, performed_at_tz) = time::now_with_tz(&tz);
    window
        .emit(
            "quick-capture-activated",
            QuickCaptureActivated {
                performed_at_utc,
                performed_at_tz,
                context_note,
                override_customer_id,
                override_system_id,
            },
        )
        .map_err(|e| AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}")))?;

    window::show_and_focus(&window);
    Ok(())
}

#[derive(Clone, serde::Serialize)]
pub struct ClipboardPasteImage {
    pub bytes_base64: String,
    pub mime_type: String,
}

pub fn open_with_clipboard_screenshot(app: &AppHandle) -> Result<(), AppError> {
    open(app)?;
    match crate::clipboard::read_image_as_png_base64() {
        Ok((bytes_base64, mime_type)) => {
            let window = app.get_webview_window("quick-capture").ok_or_else(|| {
                AppError::Config("Schnellerfassungsfenster nicht gefunden".to_string())
            })?;
            window
                .emit(
                    "quick-capture-paste-image",
                    ClipboardPasteImage {
                        bytes_base64,
                        mime_type,
                    },
                )
                .map_err(|e| {
                    AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}"))
                })?;
        }
        Err(e) => eprintln!("Kein Bild in der Zwischenablage: {e}"),
    }
    Ok(())
}
