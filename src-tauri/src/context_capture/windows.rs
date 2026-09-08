use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextW, SetForegroundWindow,
};

#[derive(Debug, Clone, Copy)]
pub struct ForegroundHandle(isize);

pub fn capture_foreground() -> Option<ForegroundHandle> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        None
    } else {
        Some(ForegroundHandle(hwnd.0 as isize))
    }
}

pub fn restore_foreground(handle: &ForegroundHandle) {
    let hwnd = HWND(handle.0 as *mut std::ffi::c_void);
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn foreground_window_title() -> Option<String> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }
    let mut buffer = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

pub fn platform_supports_context_capture() -> bool {
    true
}
