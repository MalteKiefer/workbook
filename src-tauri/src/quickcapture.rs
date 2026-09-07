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
}

pub fn open(app: &AppHandle) -> Result<(), AppError> {
    let window = app
        .get_webview_window("quick-capture")
        .ok_or_else(|| AppError::Config("Schnellerfassungsfenster nicht gefunden".to_string()))?;

    let previous = context_capture::capture_foreground();
    {
        let state = app.state::<AppState>();
        *state.previous_foreground.lock().expect("Foreground-Mutex vergiftet") = previous;
    }

    let context_note = {
        let state = app.state::<AppState>();
        let enabled = state.config.lock().expect("Config-Mutex vergiftet").context_capture_enabled;
        if enabled { context_capture::foreground_window_title() } else { None }
    };

    let tz = time::system_timezone()?;
    let (performed_at_utc, performed_at_tz) = time::now_with_tz(&tz);
    window
        .emit("quick-capture-activated", QuickCaptureActivated { performed_at_utc, performed_at_tz, context_note })
        .map_err(|e| AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}")))?;

    window::show_and_focus(&window);
    Ok(())
}
