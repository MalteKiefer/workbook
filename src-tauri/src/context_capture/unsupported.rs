#[derive(Debug, Clone, Copy)]
pub struct ForegroundHandle;

pub fn capture_foreground() -> Option<ForegroundHandle> {
    None
}

pub fn restore_foreground(_handle: &ForegroundHandle) {}

pub fn foreground_window_title() -> Option<String> {
    None
}
