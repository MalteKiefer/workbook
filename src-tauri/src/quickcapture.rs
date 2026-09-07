use tauri::{AppHandle, Emitter, Manager};

use crate::error::AppError;
use crate::time;
use crate::window;

#[derive(Clone, serde::Serialize)]
pub struct QuickCaptureActivated {
    pub performed_at_utc: String,
    pub performed_at_tz: String,
}

pub fn open(app: &AppHandle) -> Result<(), AppError> {
    let window = app
        .get_webview_window("quick-capture")
        .ok_or_else(|| AppError::Config("Schnellerfassungsfenster nicht gefunden".to_string()))?;

    let tz = time::system_timezone()?;
    let (performed_at_utc, performed_at_tz) = time::now_with_tz(&tz);
    window
        .emit("quick-capture-activated", QuickCaptureActivated { performed_at_utc, performed_at_tz })
        .map_err(|e| AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}")))?;

    window::show_and_focus(&window);
    Ok(())
}
