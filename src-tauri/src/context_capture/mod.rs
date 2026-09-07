#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{capture_foreground, foreground_window_title, restore_foreground, ForegroundHandle};

#[cfg(not(target_os = "windows"))]
mod unsupported;
#[cfg(not(target_os = "windows"))]
pub use unsupported::{capture_foreground, foreground_window_title, restore_foreground, ForegroundHandle};
