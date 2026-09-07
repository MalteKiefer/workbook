// X11 (EWMH `_NET_ACTIVE_WINDOW`) implementation of the context_capture surface.
//
// NOT COMPILED OR TESTED ON THIS HOST: this module lives behind
// `#[cfg(all(unix, not(target_os = "macos")))]`, which is always false on the
// Windows machine this was written on. Rust never parses this file here — it has
// not been checked by `rustc` at all, only written carefully against documented
// x11rb/EWMH usage. Treat it as "should work, pending verification on a real
// Linux/X11 checkout," not as "works."
//
// Every fallible step (connection, atom interning, property fetch, empty/malformed
// reply) degrades to `None`/no-op via `?`/`.ok()` chains — nothing here panics or
// blocks, which is also what keeps this safe to merely *attempt* under Wayland
// (where `x11rb::connect(None)` is expected to fail cleanly when there's no X11
// display, e.g. no XWayland or `$DISPLAY` unset). `platform_supports_context_capture`
// adds an explicit, cheap short-circuit so callers don't even bother making X11
// calls that would just fail slowly under a pure Wayland session.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, Window};

/// X11 window IDs are plain `u32` — already `Send + Sync`, so (unlike the Windows
/// module's `HWND`-as-`isize` workaround) no reinterpretation trick is needed here.
#[derive(Debug, Clone, Copy)]
pub struct ForegroundHandle(u32);

/// Shared helper: read the root window's `_NET_ACTIVE_WINDOW` property and return
/// the currently-focused window's id, or `None` on any failure (no X11 connection,
/// window manager without EWMH support, property absent/empty).
fn active_window_id() -> Option<u32> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let net_active_window = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;

    let reply = conn
        .get_property(false, root, net_active_window, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;

    let window_id = reply.value32()?.next()?;
    if window_id == 0 {
        None
    } else {
        Some(window_id)
    }
}

pub fn capture_foreground() -> Option<ForegroundHandle> {
    active_window_id().map(ForegroundHandle)
}

/// Requests the window manager activate `handle`'s window via the standard EWMH
/// `_NET_ACTIVE_WINDOW` ClientMessage protocol (sent to the root window), rather than
/// calling `SetInputFocus` directly — many window managers ignore or fight a direct
/// focus change for focus-stealing-prevention reasons, but respect this message.
pub fn restore_foreground(handle: &ForegroundHandle) {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
    let Some(root) = conn.setup().roots.get(screen_num).map(|screen| screen.root) else {
        return;
    };
    let Ok(atom_cookie) = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW") else {
        return;
    };
    let Ok(net_active_window) = atom_cookie.reply().map(|reply| reply.atom) else {
        return;
    };

    // data[0] = 1 => source indication: normal application (EWMH spec).
    // data[1] = 0 => timestamp: CurrentTime — acceptable here, we don't have the
    // original triggering event's timestamp available.
    let event = ClientMessageEvent::new(32, handle.0 as Window, net_active_window, [1u32, 0, 0, 0, 0]);

    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
    let _ = conn.flush();
}

pub fn foreground_window_title() -> Option<String> {
    let (conn, _screen_num) = x11rb::connect(None).ok()?;
    let window = active_window_id()?;

    // Prefer the modern EWMH `_NET_WM_NAME` (UTF8_STRING) property.
    if let Some(title) = read_utf8_property(&conn, window) {
        return Some(title);
    }

    // Fall back to the legacy ICCCM `WM_NAME` (STRING, ASCII/Latin-1) property —
    // not every application sets the newer EWMH property.
    read_legacy_wm_name(&conn, window)
}

fn read_utf8_property(conn: &impl Connection, window: u32) -> Option<String> {
    let utf8_string = conn.intern_atom(false, b"UTF8_STRING").ok()?.reply().ok()?.atom;
    let net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME").ok()?.reply().ok()?.atom;

    let reply = conn
        .get_property(false, window, net_wm_name, utf8_string, 0, 1024)
        .ok()?
        .reply()
        .ok()?;

    if reply.format != 8 || reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

fn read_legacy_wm_name(conn: &impl Connection, window: u32) -> Option<String> {
    let reply = conn
        .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
        .ok()?
        .reply()
        .ok()?;

    if reply.format != 8 || reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

/// Cheap runtime capability check: under Wayland (`WAYLAND_DISPLAY` set) X11 focus
/// APIs cannot provide real focus-tracking (no X11 display, unless XWayland happens
/// to be present, which still wouldn't reflect true compositor-level focus) — so
/// report unsupported without even attempting a connection. Not yet consumed by any
/// UI (no Settings screen exists in this project yet); this is the hook a future one
/// will need per the spec's "shown as disabled, not silently skipped" requirement.
pub fn platform_supports_context_capture() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_err()
}
