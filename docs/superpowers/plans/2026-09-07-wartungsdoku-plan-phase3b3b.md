# Wartungsdoku — Phase 3b-3b: X11-Kontexterfassung, Wayland-Erkennung, CLI-Fallback — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the two items Phase 3b-3 explicitly deferred: a real X11 implementation
of `context_capture` (EWMH `_NET_ACTIVE_WINDOW`, matching the Windows module's public
shape), a runtime Wayland/X11 capability check (`platform_supports_context_capture`),
and the `--quick-capture`/`--search` CLI-argument fallback the spec requires
independent of global-shortcut/portal availability.

**Verification limitation, stated plainly — inherited unchanged from Phase 3b-3:** this
session runs on Windows only. `x11.rs` sits behind
`#[cfg(all(unix, not(target_os = "macos")))]`, which is false on this host, so Rust
never even parse-checks it here (a `cfg`'d-out module isn't just untested, it isn't
lexed against real trait/type definitions at all). Every line of `x11.rs` is written
from documented `x11rb`/EWMH usage patterns as carefully as possible, but it is
**unverified by compilation** on this machine. It should be treated as "should work,
pending verification on a real Linux checkout," never as "works." The `cli` module, by
contrast, is plain Rust with no platform-specific APIs — it compiles and its unit tests
run on Windows, and that is actually verified below.

**Architecture:**
- `context_capture` gets a third platform file, `x11.rs`, wired into `mod.rs`'s cfg
  split so the practical three-way outcome is: Windows → `windows.rs` (unchanged),
  everything else this repo targets (Linux, any non-macOS Unix) → `x11.rs`,
  true dead-end fallback → `unsupported.rs` (kept in the tree, no longer reachable on
  either currently-targeted platform, but still the documented "this platform truly
  can't do it" shape referenced by the spec's Wayland note).
- X11 focus tracking/restoration goes through the EWMH `_NET_ACTIVE_WINDOW` root-window
  property (read) and a `_NET_ACTIVE_WINDOW` `ClientMessage` sent to the root window
  (write/restore) — the standard mechanism respected by GNOME/Xorg, KDE Plasma X11,
  XFCE, i3, etc., and the one that cooperates with a WM's focus-stealing prevention
  instead of fighting it (unlike a raw `SetInputFocus`).
- A new `platform_supports_context_capture() -> bool` hook (one per platform module,
  re-exported from `mod.rs`) lets a future Settings screen show Wayland as "disabled",
  not silently skip it, without this phase having to build that screen. On X11/Linux it
  checks `WAYLAND_DISPLAY` at runtime (compile-time `cfg` cannot distinguish Wayland
  from X11 — both are the same target OS). Not yet consumed by any UI.
- A new `cli` module parses `--quick-capture`/`--search` out of `argv` into a
  `CliAction` enum and dispatches to the already-existing `quickcapture::open`/
  `window::show_and_focus_main`. It does not wire itself into `main.rs`/`lib.rs` —
  that integration (first-launch dispatch inside `.setup()`, and forwarding the
  second-instance argv through `tauri_plugin_single_instance::init`) is left to the
  human/agent doing final cross-phase integration, per this phase's boundaries; exact
  snippets are handed over in the completion report instead of applied directly.

**Tech Stack:** `x11rb` (X11 protocol bindings, non-Windows target-gated dependency).

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
**Phase 3b-3 plan (done, the pattern this phase extends):** [docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase3b3.md](2026-09-07-wartungsdoku-plan-phase3b3.md)

## Global Constraints

- Every `x11.rs` function degrades to `None`/no-op on any failure (no X11 display, WM
  without EWMH support, property absent, malformed reply) — never panics, never hangs.
  This is also what keeps Wayland sessions (no X11 display, unless XWayland happens to
  be present) safe by construction, on top of the explicit `WAYLAND_DISPLAY` short-circuit
  in `platform_supports_context_capture`.
- `ForegroundHandle` on X11 wraps a plain `u32` (X11 window IDs are `u32`, already
  `Send + Sync` — no `isize` reinterpretation trick needed, unlike the Windows module).
- `cli::parse_args`/`cli::dispatch` must not touch `main.rs`/`lib.rs` or any file another
  parallel agent owns (`quickcapture.rs`, `commands/quickcapture.rs`) — this phase adds
  the module and reports the integration snippets, it does not apply them.
- Commits scoped to exactly the files this phase touches (concurrency: other agents are
  editing other files in the same working tree right now).

---

## Task 1: `x11.rs` — real X11 EWMH implementation

**Files:**
- Modify: `src-tauri/Cargo.toml` (target-gated `x11rb` dependency)
- Create: `src-tauri/src/context_capture/x11.rs`
- Modify: `src-tauri/src/context_capture/mod.rs` (three-way cfg split)

**Interfaces:**
- Produces (same shape as `windows.rs`/`unsupported.rs`, plus one new function):
  `pub struct ForegroundHandle(u32)`,
  `pub fn capture_foreground() -> Option<ForegroundHandle>`,
  `pub fn restore_foreground(handle: &ForegroundHandle)`,
  `pub fn foreground_window_title() -> Option<String>`,
  `pub fn platform_supports_context_capture() -> bool`

- [ ] **Step 1: `x11rb` als target-gated Abhängigkeit hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo add x11rb --target 'cfg(all(unix, not(target_os = "macos")))'
cd ..
```

Landet unter `[target.'cfg(all(unix, not(target_os = "macos")))'.dependencies]` in
`Cargo.toml` — wird auf Windows nicht heruntergeladen/kompiliert.

- [ ] **Step 2: `x11.rs` schreiben**

Kernstruktur (EWMH `_NET_ACTIVE_WINDOW`):

```rust
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, Window,
};

#[derive(Debug, Clone, Copy)]
pub struct ForegroundHandle(u32);

fn active_window_id() -> Option<u32> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let net_active_window = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW").ok()?.reply().ok()?.atom;

    let reply = conn
        .get_property(false, root, net_active_window, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let window_id = reply.value32()?.next()?;
    if window_id == 0 { None } else { Some(window_id) }
}

pub fn capture_foreground() -> Option<ForegroundHandle> {
    active_window_id().map(ForegroundHandle)
}

pub fn restore_foreground(handle: &ForegroundHandle) {
    let Some((conn, screen_num)) = x11rb::connect(None).ok() else { return };
    let Some(root) = conn.setup().roots.get(screen_num).map(|s| s.root) else { return };
    let Ok(atom_cookie) = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW") else { return };
    let Ok(net_active_window) = atom_cookie.reply().map(|r| r.atom) else { return };

    let event = ClientMessageEvent::new(
        32,
        handle.0 as Window,
        net_active_window,
        [1u32, 0, 0, 0, 0], // source indication: normal application, timestamp: CurrentTime
    );
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
    let _ = conn.flush();
}

pub fn foreground_window_title() -> Option<String> {
    let (conn, _) = x11rb::connect(None).ok()?;
    let window = active_window_id()?;

    let utf8_string = conn.intern_atom(false, b"UTF8_STRING").ok()?.reply().ok()?.atom;
    let net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME").ok()?.reply().ok()?.atom;

    if let Ok(reply) = conn.get_property(false, window, net_wm_name, utf8_string, 0, 1024).and_then(|c| c.reply()) {
        if reply.format == 8 && !reply.value.is_empty() {
            return Some(String::from_utf8_lossy(&reply.value).into_owned());
        }
    }

    // Fallback: legacy WM_NAME (STRING, Latin-1/ASCII)
    if let Ok(reply) = conn
        .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
        .and_then(|c| c.reply())
    {
        if reply.format == 8 && !reply.value.is_empty() {
            return Some(String::from_utf8_lossy(&reply.value).into_owned());
        }
    }

    None
}

pub fn platform_supports_context_capture() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_err()
}
```

Jede Fehlerquelle (keine Verbindung, kein EWMH, Property fehlt, leerer Rückgabewert)
degradiert über `?`/`.ok()`-Ketten zu `None`/No-op, nie zu einem `panic!`. Das erfüllt
strukturell die Wayland-Anforderung ("kein Absturz, kein Hänger"), zusätzlich zu der
expliziten `WAYLAND_DISPLAY`-Kurzschluss-Prüfung in
`platform_supports_context_capture`.

**Kann auf diesem Windows-Host nicht kompiliert werden** — Zieldatei liegt hinter
`cfg(all(unix, not(target_os = "macos")))`, hier stets `false`.

- [ ] **Step 3: `mod.rs` auf Drei-Wege-Weiche umstellen**

```rust
// src-tauri/src/context_capture/mod.rs
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{capture_foreground, foreground_window_title, platform_supports_context_capture, restore_foreground, ForegroundHandle};

#[cfg(all(unix, not(target_os = "macos")))]
mod x11;
#[cfg(all(unix, not(target_os = "macos")))]
pub use x11::{capture_foreground, foreground_window_title, platform_supports_context_capture, restore_foreground, ForegroundHandle};

#[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
mod unsupported;
#[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
pub use unsupported::{capture_foreground, foreground_window_title, platform_supports_context_capture, restore_foreground, ForegroundHandle};
```

`unsupported.rs` bekommt eine vierte Funktion (`platform_supports_context_capture() -> false`)
und bleibt im Baum, ist aber auf beiden aktuell anvisierten Plattformen (Windows, Linux)
nicht mehr erreichbar — bewusst so, siehe Architekturabschnitt oben. `windows.rs`
bekommt dieselbe vierte Funktion (`-> true`).

- [ ] **Step 4: Windows-Build verifizieren**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cd ..
```

Expected: kompiliert (Windows-Zweig aktiv; `x11.rs` und `unsupported.rs` werden auf
diesem Host von `cfg` ausgeschlossen, nicht mitkompiliert).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/context_capture
git commit -- src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/context_capture -m "feat: add X11 EWMH context-capture implementation and Wayland detection hook"
```

---

## Task 2: CLI-Argument-Fallback (`--quick-capture`/`--search`)

**Files:**
- Create: `src-tauri/src/cli.rs`

**Interfaces:**
- Produces: `pub enum CliAction { QuickCapture, Search, None }`,
  `pub fn parse_args(args: &[String]) -> CliAction`,
  `pub fn dispatch(app: &AppHandle, action: CliAction)`
- Consumes (calls, does not modify): `quickcapture::open`, `window::show_and_focus_main`

- [ ] **Step 1: `cli.rs` schreiben (reines Rust, keine Plattform-APIs)**

```rust
use tauri::AppHandle;

use crate::{quickcapture, window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAction {
    QuickCapture,
    Search,
    None,
}

pub fn parse_args(args: &[String]) -> CliAction {
    if args.iter().any(|a| a == "--quick-capture") {
        CliAction::QuickCapture
    } else if args.iter().any(|a| a == "--search") {
        CliAction::Search
    } else {
        CliAction::None
    }
}

pub fn dispatch(app: &AppHandle, action: CliAction) {
    match action {
        CliAction::QuickCapture => {
            if let Err(e) = quickcapture::open(app) {
                eprintln!("Schnellerfassung (CLI) fehlgeschlagen: {e}");
            }
        }
        CliAction::Search => window::show_and_focus_main(app),
        CliAction::None => {}
    }
}
```

Bewusst **nicht** in `lib.rs` als `pub mod cli;` eingehängt und **nicht** in
`main.rs`/`lib.rs` verdrahtet — das ist Grenzbereich dieser Phase, siehe
Abschlussbericht für die exakten Integrations-Snippets, die ein Mensch/Parallel-Agent
außerhalb dieser Phase einträgt.

- [ ] **Step 2: Unit-Tests (reine Logik, ohne Plattform-API — hier tatsächlich lauffähig)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quick_capture_flag() {
        assert_eq!(parse_args(&["wartungsdoku".to_string(), "--quick-capture".to_string()]), CliAction::QuickCapture);
    }

    #[test]
    fn parses_search_flag() {
        assert_eq!(parse_args(&["wartungsdoku".to_string(), "--search".to_string()]), CliAction::Search);
    }

    #[test]
    fn no_recognized_flag_is_none() {
        assert_eq!(parse_args(&["wartungsdoku".to_string()]), CliAction::None);
    }
}
```

- [ ] **Step 3: Bauen, testen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo test cli::
cargo build --bin wartungsdoku
cd ..
```

Da `cli.rs` nicht als Modul eingehängt ist, kompiliert/testet dieser Schritt die Datei
zunächst nicht mit — siehe Ausführungsnotizen unten für das tatsächliche Vorgehen
(temporäres Einhängen ausschließlich zur Verifikation, danach wieder entfernt, damit
`lib.rs` unangetastet im Commit bleibt).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/cli.rs
git commit -- src-tauri/src/cli.rs -m "feat: add CLI-argument fallback for quick-capture/search"
```

---

## Bekannte Grenzen

- `x11.rs` ist auf diesem Windows-Host **nicht kompilierbar** (cfg schließt es aus) —
  unverifiziert, muss auf echtem Linux (X11-Session) geprüft werden: Verbindungsaufbau,
  Atom-Interning, `get_property`-Antwortformat (`value32()` vs. manuelles
  `u32::from_ne_bytes`-Chunking), `ClientMessageEvent`-Feldreihenfolge, Verhalten unter
  verschiedenen Fenstermanagern (GNOME/Xorg, KDE Plasma X11, XFCE, i3).
- `platform_supports_context_capture` ist noch an keine UI angebunden (keine
  Settings-Ansicht in diesem Projektstand) — reine Fähigkeitsabfrage für eine künftige
  Phase.
- `cli.rs` ist bewusst nicht in `lib.rs`/`main.rs` eingehängt — Integration (Dispatch im
  ersten Start innerhalb von `.setup()`, Weiterleitung des zweiten Start-`argv` über
  `tauri_plugin_single_instance::init`) bleibt einem Menschen/Parallel-Agent
  überlassen, der mehrere Phasen zusammenführt.
