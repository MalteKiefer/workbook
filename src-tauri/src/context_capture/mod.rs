#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{
    capture_foreground, foreground_window_title, platform_supports_context_capture,
    restore_foreground, ForegroundHandle,
};

// Linux, and any other non-macOS Unix — this repo only targets Windows and Linux
// (see spec), so in practice this is the Linux branch. Cannot distinguish X11 from
// Wayland at compile time (both run on the same target OS); that split happens at
// runtime via `platform_supports_context_capture`.
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;
#[cfg(all(unix, not(target_os = "macos")))]
pub use x11::{
    capture_foreground, foreground_window_title, platform_supports_context_capture,
    restore_foreground, ForegroundHandle,
};

// True dead-end fallback for any platform that is neither Windows nor a non-macOS
// Unix (e.g. macOS, which the spec does not target). Not reachable on either
// currently-targeted platform (Windows, Linux) — kept in the tree anyway, since it's
// the documented "this platform truly can't do it" shape the spec's Wayland note
// conceptually depends on existing somewhere.
#[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
mod unsupported;
#[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
#[allow(unused_imports)]
pub use unsupported::{
    capture_foreground, foreground_window_title, platform_supports_context_capture,
    restore_foreground, ForegroundHandle,
};
