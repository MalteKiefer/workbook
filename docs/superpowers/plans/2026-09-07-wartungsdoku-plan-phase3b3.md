# Wartungsdoku — Phase 3b-3: Globale Hotkeys, Zwischenablage-Hotkey, Fokus-Rückgabe — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The three global hotkeys from the spec, `Strg+Alt+S`'s clipboard-to-new-entry
path (via `arboard`, needed because there's no focused webview to catch a native paste
event), and focus returning to whichever application was active before the hotkey fired
— on Windows, fully implemented and verified via real builds on this machine. Linux/X11
gets the same module structure with a working stub for now (Wayland's permanent
unavailability and Linux's temporary one look identical from the caller's side); real
X11 support is **Phase 3b-3b**, together with the Wayland CLI fallback
(`--quick-capture`/`--search`) — both need testing on an actual Linux box, which this
Windows-only session cannot do.

**Verification limitation, stated plainly:** this session runs on Windows only. Any
`#[cfg(not(target_os = "windows"))]` code compiles on *no* target here — Rust doesn't
even parse-check a `cfg`'d-out module. So `context_capture`'s non-Windows stub in this
plan is structurally trivial (no platform APIs, mirrors the Windows module's public
shape) but genuinely unverified. Say so if asked, don't claim more confidence than a
real compile gives.

**Architecture:** `context_capture` gets one file per platform behind `cfg`, all
exposing the same three functions (`capture_foreground`, `restore_foreground`,
`foreground_window_title`) so callers (`quickcapture::open`, the new
`quick_capture_close` command) never branch on platform themselves. Global hotkeys are
registered dynamically inside `.setup()` (not the top-level builder chain) because they
need the already-loaded `Config`; each configured hotkey string is parsed once into a
`Shortcut` and compared by value in the handler, so mis-configured hotkeys degrade
gracefully (skipped, logged) instead of crashing startup.

**Tech Stack:** `tauri-plugin-global-shortcut`, `windows` (Win32 Foundation +
WindowsAndMessaging, Windows-only), `arboard` + `image` (clipboard image → PNG).

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
**Phase 3b-2 plan (done):** [docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase3b2.md](2026-09-07-wartungsdoku-plan-phase3b2.md)

## Global Constraints

- Alle drei Hotkeys sind über `config.toml` (`hotkeys.*`) änderbar; ein nicht parsbarer
  Hotkey deaktiviert nur sich selbst, nie den App-Start.
- Fokus-Rückgabe: der Fokus geht an die Anwendung zurück, die *vor* dem Öffnen der
  Schnellerfassung aktiv war — erfasst, bevor irgendein eigenes Fenster fokussiert wird;
  wiederhergestellt beim Schließen über `Strg+S` oder `Esc`.
- Kontexterfassung bleibt Standard-aus (`config.context_capture_enabled`), wird nur bei
  aktivem Flag *und* verfügbarer Plattform-API abgefragt — nie stillschweigend auf
  falschen Daten basierend, sondern `None`, wenn nicht verfügbar.
- Fehler nie stillschweigend verschluckt — auch hier: `eprintln!` für nicht-fatale
  Subsysteme (fehlgeschlagene Hotkey-Registrierung, leere Zwischenablage), kein Absturz.

---

## Task 1: `context_capture`-Modul (Windows real, andere Plattformen als Stub)

**Files:**
- Create: `src-tauri/src/context_capture/mod.rs`
- Create: `src-tauri/src/context_capture/windows.rs`
- Create: `src-tauri/src/context_capture/unsupported.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces (identisch auf jeder Plattform): `pub struct ForegroundHandle`,
  `pub fn capture_foreground() -> Option<ForegroundHandle>`,
  `pub fn restore_foreground(handle: &ForegroundHandle)`,
  `pub fn foreground_window_title() -> Option<String>`

- [ ] **Step 1: `windows`-Crate hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo add windows --features Win32_Foundation,Win32_UI_WindowsAndMessaging
cd ..
```

- [ ] **Step 2: Windows-Implementierung schreiben**

```rust
// src-tauri/src/context_capture/windows.rs
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, SetForegroundWindow};

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
```

`ForegroundHandle` hält bewusst ein `isize`, nicht das `HWND` selbst (das einen
`*mut c_void` kapselt und damit nicht `Send`/`Sync` ist) — `AppState` muss zwischen
Tauri-Threads wandern können.

- [ ] **Step 3: Stub für andere Plattformen schreiben**

```rust
// src-tauri/src/context_capture/unsupported.rs
#[derive(Debug, Clone, Copy)]
pub struct ForegroundHandle;

pub fn capture_foreground() -> Option<ForegroundHandle> {
    None
}

pub fn restore_foreground(_handle: &ForegroundHandle) {}

pub fn foreground_window_title() -> Option<String> {
    None
}
```

- [ ] **Step 4: `mod.rs` mit Plattform-Weiche**

```rust
// src-tauri/src/context_capture/mod.rs
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{capture_foreground, foreground_window_title, restore_foreground, ForegroundHandle};

#[cfg(not(target_os = "windows"))]
mod unsupported;
#[cfg(not(target_os = "windows"))]
pub use unsupported::{capture_foreground, foreground_window_title, restore_foreground, ForegroundHandle};
```

- [ ] **Step 5: In `lib.rs` einhängen, Build verifizieren**

```rust
// src-tauri/src/lib.rs — Modul-Deklaration ergänzen
pub mod context_capture;
```

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cd ..
```

Expected: kompiliert (Windows-Zweig aktiv auf diesem Rechner; `unsupported.rs` wird auf
diesem Host nicht mitkompiliert — das ist die oben genannte Grenze).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/context_capture src-tauri/src/lib.rs
git commit -m "feat: add platform foreground-window capture (Windows real, others stubbed)"
```

---

## Task 2: Fokus-Rückgabe + optionale Kontexterfassung verdrahten

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/quickcapture.rs`
- Modify: `src-tauri/src/commands/quickcapture.rs`
- Modify: `src-tauri/src/commands/mod.rs` *(keine Änderung nötig — Modul schon vorhanden)*
- Modify: `src/quick-capture/QuickCapture.tsx`

**Interfaces:**
- Produces: neues `AppState`-Feld `previous_foreground: Mutex<Option<context_capture::ForegroundHandle>>`,
  `#[tauri::command] quick_capture_close`, `QuickCaptureActivated.context_note: Option<String>`

- [ ] **Step 1: `AppState` erweitern**

```rust
// src-tauri/src/lib.rs — Struct-Definition
pub struct AppState {
    pub pool: DbPool,
    pub config: Mutex<Config>,
    pub previous_foreground: Mutex<Option<context_capture::ForegroundHandle>>,
}
```

```rust
// src-tauri/src/lib.rs — .manage(...)-Aufruf anpassen
        .manage(AppState { pool, config: Mutex::new(app_config), previous_foreground: Mutex::new(None) })
```

- [ ] **Step 2: `quickcapture::open` um Fokus-Erfassung und Kontext-Notiz erweitern**

```rust
// src-tauri/src/quickcapture.rs — komplett ersetzen
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
```

- [ ] **Step 3: `quick_capture_close`-Command ergänzen**

```rust
// src-tauri/src/commands/quickcapture.rs — ergänzen
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn quick_capture_close(app: AppHandle, state: State<AppState>) -> Result<(), AppError> {
    if let Some(window) = app.get_webview_window("quick-capture") {
        let _ = window.hide();
    }
    let previous = state.previous_foreground.lock().expect("Foreground-Mutex vergiftet").take();
    if let Some(handle) = previous {
        crate::context_capture::restore_foreground(&handle);
    }
    Ok(())
}
```

- [ ] **Step 4: Command registrieren**

```rust
// src-tauri/src/lib.rs — invoke_handler-Liste ergänzen
            commands::quickcapture::get_last_selection,
            commands::quickcapture::quick_capture_close,
```

- [ ] **Step 5: Frontend auf den neuen Command umstellen, Kontext-Notiz übernehmen**

```tsx
// src/quick-capture/QuickCapture.tsx — Ersetzungen:
// 1) Event-Payload-Typ erweitern:
//    listen<{ performed_at_utc: string; performed_at_tz: string; context_note: string | null }>
// 2) Im Listener, im "if (draftIsEmptyRef.current)"-Zweig, nach dem Setzen von
//    performedAt* ergänzen:
if (event.payload.context_note) {
  setBodyMd(`_Kontext: ${event.payload.context_note}_\n\n`);
}
// 3) In save() und discard(): "await getCurrentWindow().hide();" jeweils ersetzen durch
await invoke("quick_capture_close");
// 4) Der Import von getCurrentWindow wird dadurch nur noch für den
//    onFocusChanged-Listener gebraucht — Import bleibt bestehen.
```

- [ ] **Step 6: Bauen, testen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cargo test && cd ..
npm run build
```

Expected: Build grün, alle 59 Rust-Tests grün, Frontend-Build grün.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/quickcapture.rs src-tauri/src/commands/quickcapture.rs src/quick-capture/QuickCapture.tsx
git commit -m "feat: capture and restore foreground focus, wire optional context note"
```

---

## Task 3: Zwischenablage-Screenshot ohne fokussiertes Fenster

**Files:**
- Create: `src-tauri/src/clipboard.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/quickcapture.rs`
- Modify: `src/quick-capture/QuickCapture.tsx`

**Interfaces:**
- Produces: `pub fn read_image_as_png_base64() -> Result<(String, String), AppError>`
  (gibt `(bytes_base64, mime_type)` zurück),
  `pub fn quickcapture::open_with_clipboard_screenshot(app: &AppHandle) -> Result<(), AppError>`,
  Tauri-Event `"quick-capture-paste-image"` mit Payload `{ bytes_base64, mime_type }`

- [ ] **Step 1: Abhängigkeiten hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo add arboard
cargo add image
cd ..
```

- [ ] **Step 2: `clipboard.rs` schreiben**

```rust
// src-tauri/src/clipboard.rs
use arboard::Clipboard;
use base64::prelude::*;
use image::{ImageBuffer, Rgba};

use crate::error::AppError;

pub fn read_image_as_png_base64() -> Result<(String, String), AppError> {
    let mut clipboard = Clipboard::new().map_err(|e| AppError::Config(format!("Zwischenablage nicht verfügbar: {e}")))?;
    let image_data = clipboard
        .get_image()
        .map_err(|e| AppError::NotFound(format!("Kein Bild in der Zwischenablage: {e}")))?;

    let width = image_data.width as u32;
    let height = image_data.height as u32;
    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, image_data.bytes.into_owned())
        .ok_or_else(|| AppError::Config("Zwischenablage-Bild hat unerwartetes Format".to_string()))?;

    let mut png_bytes: Vec<u8> = Vec::new();
    buffer
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| AppError::Io(format!("PNG-Kodierung fehlgeschlagen: {e}")))?;

    Ok((BASE64_STANDARD.encode(&png_bytes), "image/png".to_string()))
}
```

Kein automatisierter Test möglich — braucht eine echte, mit einem Bild gefüllte
OS-Zwischenablage. Verifikation erfolgt manuell (Nutzer kopiert ein Bild, drückt
`Strg+Alt+S`).

- [ ] **Step 3: `quickcapture::open_with_clipboard_screenshot` ergänzen**

```rust
// src-tauri/src/quickcapture.rs — ergänzen
#[derive(Clone, serde::Serialize)]
pub struct ClipboardPasteImage {
    pub bytes_base64: String,
    pub mime_type: String,
}

pub fn open_with_clipboard_screenshot(app: &AppHandle) -> Result<(), AppError> {
    open(app)?;
    match crate::clipboard::read_image_as_png_base64() {
        Ok((bytes_base64, mime_type)) => {
            let window = app
                .get_webview_window("quick-capture")
                .ok_or_else(|| AppError::Config("Schnellerfassungsfenster nicht gefunden".to_string()))?;
            window
                .emit("quick-capture-paste-image", ClipboardPasteImage { bytes_base64, mime_type })
                .map_err(|e| AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}")))?;
        }
        Err(e) => eprintln!("Kein Bild in der Zwischenablage: {e}"),
    }
    Ok(())
}
```

- [ ] **Step 4: Modul einhängen**

```rust
// src-tauri/src/lib.rs — Modul-Deklaration ergänzen
pub mod clipboard;
```

- [ ] **Step 5: Frontend-Listener für automatisch eingefügtes Bild**

```tsx
// src/quick-capture/QuickCapture.tsx — neuer Effekt, z. B. nach dem
// "quick-capture-activated"-Effekt
useEffect(() => {
  const unlisten = listen<{ bytes_base64: string; mime_type: string }>("quick-capture-paste-image", (event) => {
    const { bytes_base64, mime_type } = event.payload;
    const token = `pending:${crypto.randomUUID()}`;
    const markdown = `![Screenshot](${token})`;
    setBodyMd((prev) => (prev ? `${prev}\n${markdown}` : markdown));
    setPendingAttachments((prev) => [...prev, { token, bytesBase64: bytes_base64, filename: "Screenshot.png", mimeType: mime_type }]);
  });
  return () => {
    unlisten.then((f) => f());
  };
}, []);
```

- [ ] **Step 6: Bauen, testen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cargo test && cd ..
npm run build
```

Expected: Build grün, 59 Rust-Tests weiterhin grün, Frontend-Build grün.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/clipboard.rs src-tauri/src/lib.rs src-tauri/src/quickcapture.rs src/quick-capture/QuickCapture.tsx
git commit -m "feat: add clipboard-screenshot capture for the no-focused-webview case"
```

---

## Task 4: Globale Hotkeys registrieren

**Files:**
- Create: `src-tauri/src/hotkeys.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `config::HotkeyConfig`, `quickcapture::open`, `quickcapture::open_with_clipboard_screenshot`, `window::show_and_focus_main`
- Produces: `pub fn register(app: &tauri::AppHandle, hotkeys: &config::HotkeyConfig) -> Result<(), AppError>`

- [ ] **Step 1: Abhängigkeit hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo add tauri-plugin-global-shortcut && cd ..
```

- [ ] **Step 2: `hotkeys.rs` schreiben**

```rust
// src-tauri/src/hotkeys.rs
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{Shortcut, ShortcutState};

use crate::config::HotkeyConfig;
use crate::error::AppError;
use crate::{quickcapture, window};

pub fn register(app: &AppHandle, hotkeys: &HotkeyConfig) -> Result<(), AppError> {
    let quick_capture = parse_shortcut(&hotkeys.quick_capture, "Schnellerfassung");
    let search = parse_shortcut(&hotkeys.search, "Suche");
    let clipboard_screenshot = parse_shortcut(&hotkeys.clipboard_screenshot, "Zwischenablage-Screenshot");

    let registered: Vec<Shortcut> = [&quick_capture, &search, &clipboard_screenshot]
        .into_iter()
        .filter_map(|s| s.clone())
        .collect();

    let quick_capture_for_handler = quick_capture.clone();
    let search_for_handler = search.clone();
    let clipboard_for_handler = clipboard_screenshot.clone();

    app.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_shortcuts(registered)
            .map_err(|e| AppError::Config(format!("Hotkeys konnten nicht vorbereitet werden: {e}")))?
            .with_handler(move |app, shortcut, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                if Some(shortcut) == quick_capture_for_handler.as_ref() {
                    if let Err(e) = quickcapture::open(app) {
                        eprintln!("Schnellerfassung (Hotkey) fehlgeschlagen: {e}");
                    }
                } else if Some(shortcut) == search_for_handler.as_ref() {
                    window::show_and_focus_main(app);
                } else if Some(shortcut) == clipboard_for_handler.as_ref() {
                    if let Err(e) = quickcapture::open_with_clipboard_screenshot(app) {
                        eprintln!("Zwischenablage-Screenshot (Hotkey) fehlgeschlagen: {e}");
                    }
                }
            })
            .build(),
    )
    .map_err(|e| AppError::Config(format!("Global-Shortcut-Plugin konnte nicht registriert werden: {e}")))?;

    Ok(())
}

fn parse_shortcut(raw: &str, label: &str) -> Option<Shortcut> {
    match raw.parse::<Shortcut>() {
        Ok(shortcut) => Some(shortcut),
        Err(_) => {
            eprintln!("Hotkey für \"{label}\" (\"{raw}\") konnte nicht interpretiert werden — deaktiviert.");
            None
        }
    }
}
```

`app.plugin(...)` statt `app.handle().plugin(...)`: innerhalb von `.setup()` ist `app`
bereits ein `&mut App`/`&AppHandle`-kompatibler Typ mit derselben Methode — beide Formen
sind in der offiziellen Tauri-Dokumentation für dynamische Plugin-Registrierung im
Setup-Callback belegt.

- [ ] **Step 3: In `lib.rs` einhängen und mit der geladenen Konfiguration aufrufen**

```rust
// src-tauri/src/lib.rs — Modul-Deklaration ergänzen
pub mod hotkeys;
```

```rust
// src-tauri/src/lib.rs — run(): hotkey_config VOR der Builder-Kette aus app_config klonen
    let hotkey_config = app_config.hotkeys.clone();
```

```rust
// src-tauri/src/lib.rs — .setup(...) auf einen `move`-Closure umstellen und
// hotkeys::register(...) am Ende ergänzen (vor `Ok(())`)
        .setup(move |app| {
            window::install_hide_on_close(app.handle());
            if let Err(e) = tray::build_tray(app.handle()) {
                eprintln!("Tray konnte nicht eingerichtet werden: {e}");
            }

            let quick_capture_window = tauri::WebviewWindowBuilder::new(
                app,
                "quick-capture",
                tauri::WebviewUrl::App("quick-capture.html".into()),
            )
            .title("Schnellerfassung")
            .inner_size(560.0, 420.0)
            .center()
            .resizable(false)
            .visible(false)
            .build()?;
            window::install_hide_on_close_for(&quick_capture_window);

            let state = app.state::<AppState>();
            let autostart_enabled = state.config.lock().expect("Config-Mutex vergiftet").autostart_enabled;
            use tauri_plugin_autostart::ManagerExt;
            let autolaunch = app.autolaunch();
            let sync_result = if autostart_enabled { autolaunch.enable() } else { autolaunch.disable() };
            if let Err(e) = sync_result {
                eprintln!("Autostart konnte nicht synchronisiert werden: {e}");
            }

            if let Err(e) = hotkeys::register(app.handle(), &hotkey_config) {
                eprintln!("Globale Hotkeys konnten nicht registriert werden: {e}");
            }

            Ok(())
        })
```

- [ ] **Step 4: Bauen, testen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cargo test && cd ..
```

Expected: Build grün, alle 59 Rust-Tests grün (Hotkey-Registrierung selbst ist nicht
unit-testbar — braucht einen echten `AppHandle` mit laufendem Event-Loop; Verifikation
im End-to-End-Smoke-Test, Task 5).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/hotkeys.rs src-tauri/src/lib.rs
git commit -m "feat: register configurable global hotkeys for quick-capture, search, clipboard-screenshot"
```

---

## Task 5: End-to-End-Verifikation

- [ ] **Step 1: App real starten, Boot-Log prüfen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export WARTUNGSDOKU_DATA_DIR="$(pwd)/.smoke-test-data"
cd src-tauri
timeout 8 cargo run --bin wartungsdoku > ../smoke-test.log 2>&1
cd ..
cat smoke-test.log
```

Expected: kein `panic!`/`expect`-Abbruch; insbesondere keine Meldung
"Globale Hotkeys konnten nicht registriert werden" (das hieße, die Default-Hotkey-Strings
aus `HotkeyConfig::default()` lassen sich nicht parsen — dann Format prüfen, nicht
ignorieren).

- [ ] **Step 2: Aufräumen, Plan committen**

```bash
rm -rf .smoke-test-data smoke-test.log
git add docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase3b3.md
git commit -m "docs: mark Phase 3b-3 plan tasks complete"
```

**Bekannte Grenzen, die ein manueller Test durch den Nutzer schließen muss:**
Tastaturverhalten der drei Hotkeys von außerhalb der App (`Strg+Alt+Leertaste`,
`Strg+Alt+F`, `Strg+Alt+S` inklusive eines echten Bildes in der Zwischenablage) sowie
Fokus-Rückgabe an die zuvor aktive Anwendung — dieses Environment kann native
OS-Fenster nicht interaktiv bedienen.

---

## Self-Review (durchgeführt vor Ausführung)

- **Spec-Abdeckung**: drei konfigurierbare globale Hotkeys ✓ (Suche zeigt vorerst nur
  das Hauptfenster — echte Such-UI ist Phase 4); `Strg+Alt+S` liest Zwischenablage ohne
  fokussiertes Fenster über `arboard` ✓; Fokus-Rückgabe an die zuvor aktive fremde
  Anwendung ✓ (Windows real, andere Plattformen strukturell vorbereitet); optionale
  Kontexterfassung (Fenstertitel als Notiz) ✓, Standard aus, respektiert
  `config.context_capture_enabled`.
- **Bewusst verschoben (Phase 3b-3b)**: echte X11-Implementierung von
  `context_capture` (aktuell im "andere Plattformen"-Zweig, der auch Wayland abdeckt —
  fachlich korrekt für Wayland, für X11 nur vorübergehend), Wayland-CLI-Fallback
  (`--quick-capture`/`--search`), Weiterleitung dieser Argumente über die
  Single-Instance-Callback an eine bereits laufende Instanz.
- **Verifikationsgrenze offen benannt**: dieses Environment ist Windows-only; jeder
  `cfg(not(target_os = "windows"))`-Codepfad kompiliert hier nicht mit — das gilt für
  den kompletten "andere Plattformen"-Zweig aus Task 1, nicht nur für zukünftigen
  X11-Code.
- **Platzhalter-Scan**: keine TBD/TODO. Die "andere Plattformen"-Stubs sind korrektes,
  beabsichtigtes Verhalten (liefern `None`, genau wie von jedem Aufrufer erwartet), kein
  unfertiger Code.
- **Typkonsistenz**: `ForegroundHandle` (Task 1) wird in `AppState` (Task 2),
  `quickcapture::open`/`open_with_clipboard_screenshot` (Task 2/3) und
  `commands::quickcapture::quick_capture_close` (Task 2) identisch verwendet;
  `QuickCaptureActivated`/`ClipboardPasteImage`-Feldnamen (Task 2/3) exakt wie im
  Frontend erwartet (Task 2/3), analog zum bereits in Phase 3b-2 etablierten Muster.
