use tauri::AppHandle;

use crate::{quick_connect, AppError};

#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), AppError> {
    quick_connect::open_url(&app, &url)
}

#[tauri::command]
pub fn open_rdp(app: AppHandle, ip: String) -> Result<(), AppError> {
    quick_connect::open_rdp(&app, &ip)
}

#[tauri::command]
pub fn open_share(app: AppHandle, ip: String) -> Result<(), AppError> {
    quick_connect::open_share(&app, &ip)
}
