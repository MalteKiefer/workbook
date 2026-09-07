# Wartungsdoku — Phase 3b-2: Schnellerfassungsfenster — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A real, usable Schnellerfassungsfenster: compact second Tauri window, pre-created
hidden at boot (so showing it is fast), triggered for now via a new Tray-Menüeintrag
("Schnellerfassung" — deferred in Phase 3a because its target didn't exist yet). Title
autofocus, customer/system dropdowns pre-filled with the last selection, a `performed_at`
field that accepts absolute/relative text and shows a live formatted preview, `Strg+V`
pastes a clipboard image as an attachment, `Strg+S` saves and hides, `Esc` discards and
hides, closing via window chrome hides without discarding (draft survives — for free,
because hiding never unmounts the React tree).

**Phase 3b-3** (global hotkeys, `Strg+Alt+S` clipboard-to-new-entry via `arboard` for the
no-focused-webview case, focus-return-to-previous-application, context capture, Wayland
CLI fallback) follows once this is reviewed.

**Architecture:** In-window `Strg+V` uses the browser's native paste event
(`ClipboardEvent.clipboardData.items`) — synchronous image detection, no Rust clipboard
crate needed here. `arboard` is deferred to 3b-3, where it's actually required (reading
the clipboard when no webview has focus, i.e. from a global hotkey). The quick-capture
window is built once in `.setup()` and only shown/hidden afterward, never re-created —
that's what makes the < 150 ms requirement achievable at all (cold webview creation is
slow; showing an already-loaded hidden window is not).

**Tech Stack:** Vite multi-page build (`index.html` + `quick-capture.html`, each its own
React entry point). No new Rust dependencies in this phase.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
**Phase 3b-1 plan (done):** [docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase3b1.md](2026-09-07-wartungsdoku-plan-phase3b1.md)

## Global Constraints

- `performed_at` wird bei Aktivierung auf den Moment des Auslösens gesetzt — aber nur für
  einen *frischen* (leeren) Entwurf. Ein bereits angefangener Entwurf wird nicht überschrieben.
- Cursor steht bei jeder Aktivierung im Titelfeld.
- `Strg+S` speichert + schließt (versteckt) + Entwurf wird geleert. `Esc` verwirft (leert
  den Entwurf) + versteckt. Schließen über Fensterrahmen versteckt nur — Entwurf bleibt.
- Kein neues Feld `Kunde`/`System` als Freitext — beides sind IDs aus vorhandenen Datensätzen.
- IPC-Feldnamen: Tauri camelCased nur die Top-Level-Parameternamen eines Commands, nicht
  die Felder innerhalb einer als Parameter übergebenen Struktur — `NewEntry`/`EntryFilter`
  behalten ihre snake_case-Feldnamen 1:1 im JSON, das das Frontend schickt.

---

## Task 1: Letzte Auswahl in der Konfiguration

**Files:**
- Modify: `src-tauri/src/config.rs`

**Interfaces:**
- Produces: neue Felder `last_customer_id: Option<i64>`, `last_system_id: Option<i64>` auf `Config`

- [x] **Step 1: Felder ergänzen (mit Test)**

```rust
// src-tauri/src/config.rs — Config-Struct erweitern
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub data_dir: PathBuf,
    pub autostart_enabled: bool,
    pub context_capture_enabled: bool,
    pub late_entry_threshold_hours: i64,
    pub hotkeys: HotkeyConfig,
    pub last_customer_id: Option<i64>,
    pub last_system_id: Option<i64>,
}
```

```rust
// Default-Impl ergänzen
impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            autostart_enabled: true,
            context_capture_enabled: false,
            late_entry_threshold_hours: 24,
            hotkeys: HotkeyConfig::default(),
            last_customer_id: None,
            last_system_id: None,
        }
    }
}
```

Test ergänzen:

```rust
    #[test]
    fn save_then_load_roundtrips_last_selection() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.last_customer_id = Some(7);
        config.last_system_id = Some(3);

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.last_customer_id, Some(7));
        assert_eq!(loaded.last_system_id, Some(3));
    }
```

- [x] **Step 2: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test config:: && cd ..
```

Expected: 5 Tests grün (4 bestehende + 1 neuer).

- [x] **Step 3: Commit**

```bash
git add src-tauri/src/config.rs
git commit -m "feat: track last-used customer/system selection in config"
```

---

## Task 2: Zeitstempel-Anzeigeformat

**Files:**
- Modify: `src-tauri/src/time.rs`

**Interfaces:**
- Produces: `pub fn format_timestamp_for_display(utc: &str, tz_name: &str) -> Result<String, AppError>`

- [x] **Step 1: Funktion mit failing test ergänzen**

```rust
// src-tauri/src/time.rs — nach parse_temporal_input einfügen
pub fn format_timestamp_for_display(utc: &str, tz_name: &str) -> Result<String, AppError> {
    let parsed = DateTime::parse_from_rfc3339(utc)
        .map_err(|e| AppError::InvalidTimestamp(format!("{utc}: {e}")))?
        .with_timezone(&Utc);
    let tz: Tz = tz_name
        .parse()
        .map_err(|_| AppError::Timezone(format!("unbekannte Zone: {tz_name}")))?;
    Ok(parsed.with_timezone(&tz).format("%d.%m.%Y %H:%M %Z").to_string())
}
```

Test (im bestehenden `#[cfg(test)] mod tests` ergänzen):

```rust
    #[test]
    fn formats_utc_timestamp_in_target_zone_with_abbreviation() {
        let display = format_timestamp_for_display("2026-09-07T12:32:00.000Z", "Europe/Berlin").unwrap();
        // September = Sommerzeit (CEST). Abkürzung kommt aus chrono-tz auf Englisch,
        // nicht "MESZ" — bewusste Vereinfachung, siehe Ausführungsnotizen.
        assert!(display.starts_with("07.09.2026 14:32"));
    }

    #[test]
    fn rejects_invalid_timezone_name() {
        let result = format_timestamp_for_display("2026-09-07T12:32:00.000Z", "Not/AZone");
        assert!(matches!(result, Err(AppError::Timezone(_))));
    }
```

- [x] **Step 2: Tests laufen lassen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test time:: && cd ..
```

Expected: 9 Tests grün (7 bestehende + 2 neue).

- [x] **Step 3: Commit**

```bash
git add src-tauri/src/time.rs
git commit -m "feat: add human-readable timestamp display formatting"
```

---

## Task 3: Schnellerfassungsfenster erzeugen, Tray-Eintrag aktivieren

**Files:**
- Create: `src-tauri/src/quickcapture.rs`
- Modify: `src-tauri/src/window.rs`
- Modify: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `time::system_timezone`, `time::now_with_tz`
- Produces: `pub fn window::show_and_focus(window: &tauri::WebviewWindow)`,
  `pub fn window::install_hide_on_close_for(window: &tauri::WebviewWindow)`,
  `pub fn quickcapture::open(app: &tauri::AppHandle) -> Result<(), AppError>`,
  Tauri-Event `"quick-capture-activated"` mit Payload `{ performed_at_utc, performed_at_tz }`

- [x] **Step 1: `window.rs` verallgemeinern**

```rust
// src-tauri/src/window.rs — komplett ersetzen
use tauri::{AppHandle, Manager, WindowEvent};

pub fn show_and_focus(window: &tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn show_and_focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        show_and_focus(&window);
    } else {
        eprintln!("Hauptfenster nicht gefunden — konnte es nicht anzeigen");
    }
}

pub fn install_hide_on_close_for(window: &tauri::WebviewWindow) {
    let window_to_hide = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window_to_hide.hide();
        }
    });
}

pub fn install_hide_on_close(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(window) => install_hide_on_close_for(&window),
        None => eprintln!("Hauptfenster nicht gefunden — Schließen-Verhalten nicht verdrahtet"),
    }
}
```

- [x] **Step 2: `quickcapture.rs` schreiben**

```rust
// src-tauri/src/quickcapture.rs
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
```

- [x] **Step 3: Fenster in `lib.rs` beim Start erzeugen (versteckt)**

```rust
// src-tauri/src/lib.rs — Modul-Deklaration ergänzen
pub mod quickcapture;
```

```rust
// src-tauri/src/lib.rs — im .setup()-Callback, nach tray::build_tray(...) ergänzen
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
```

- [x] **Step 4: Tray-Menü um "Schnellerfassung" erweitern**

```rust
// src-tauri/src/tray.rs — build_tray() anpassen
pub fn build_tray(app: &AppHandle) -> Result<(), AppError> {
    let quick_capture_item = MenuItem::with_id(app, "quick_capture", "Schnellerfassung", true, None::<&str>)
        .map_err(|e| AppError::Config(format!("Tray-Menüeintrag konnte nicht erstellt werden: {e}")))?;
    let show_item = MenuItem::with_id(app, "show", "Fenster zeigen", true, None::<&str>)
        .map_err(|e| AppError::Config(format!("Tray-Menüeintrag konnte nicht erstellt werden: {e}")))?;
    let separator = PredefinedMenuItem::separator(app)
        .map_err(|e| AppError::Config(format!("Tray-Trennlinie konnte nicht erstellt werden: {e}")))?;
    let quit_item = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)
        .map_err(|e| AppError::Config(format!("Tray-Menüeintrag konnte nicht erstellt werden: {e}")))?;

    let menu = Menu::with_items(app, &[&quick_capture_item, &show_item, &separator, &quit_item])
        .map_err(|e| AppError::Config(format!("Tray-Menü konnte nicht erstellt werden: {e}")))?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| AppError::Config("Kein Standard-Icon für das Tray verfügbar".to_string()))?;

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Wartungsdoku")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quick_capture" => {
                if let Err(e) = crate::quickcapture::open(app) {
                    eprintln!("Schnellerfassung konnte nicht geöffnet werden: {e}");
                }
            }
            "show" => show_and_focus_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_and_focus_main(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| AppError::Config(format!("Tray-Icon konnte nicht erstellt werden: {e}")))?;

    Ok(())
}
```

- [x] **Step 5: Build verifizieren**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cargo test && cd ..
```

Expected: Build grün. Rust-Tests unverändert (kein Fenster-Code ist unit-testbar — echtes
`AppHandle` nötig; Verifikation folgt im End-to-End-Smoke-Test dieser Phase und in 3b-3).

Der Build referenziert `quick-capture.html`, das erst in Task 5 entsteht — Tauri prüft
diesen Pfad zur Laufzeit (`devUrl`/`frontendDist`), nicht zur Compile-Zeit, daher
kompiliert `cargo build` bereits jetzt; ein echter Start würde erst ab Task 5 die Datei
tatsächlich laden.

- [x] **Step 6: Commit**

```bash
git add src-tauri/src/window.rs src-tauri/src/quickcapture.rs src-tauri/src/tray.rs src-tauri/src/lib.rs
git commit -m "feat: create hidden quick-capture window, activate via tray menu"
```

---

## Task 4: Commands für Schnellerfassung

**Files:**
- Create: `src-tauri/src/commands/quickcapture.rs`
- Modify: `src-tauri/src/commands/entries.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `db::customers::get`, `db::systems::get` (Phase 2), `time::format_timestamp_for_display` (Task 2)
- Produces: `#[tauri::command] get_last_selection`, `#[tauri::command] format_timestamp_for_display`;
  `create_entry` merkt sich nach erfolgreichem Speichern `last_customer_id`/`last_system_id`

- [x] **Step 1: `commands/quickcapture.rs` schreiben**

```rust
// src-tauri/src/commands/quickcapture.rs
use tauri::State;

use crate::db::{customers, systems};
use crate::{AppError, AppState};

#[derive(Debug, Clone, serde::Serialize)]
pub struct LastSelection {
    pub customer: Option<customers::Customer>,
    pub system: Option<systems::System>,
}

#[tauri::command]
pub fn get_last_selection(state: State<AppState>) -> Result<LastSelection, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let (last_customer_id, last_system_id) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        (config.last_customer_id, config.last_system_id)
    };
    let customer = last_customer_id.and_then(|id| customers::get(&conn, id).ok());
    let system = last_system_id.and_then(|id| systems::get(&conn, id).ok());
    Ok(LastSelection { customer, system })
}
```

- [x] **Step 2: `format_timestamp_for_display`-Command in `commands/entries.rs` ergänzen**

```rust
// src-tauri/src/commands/entries.rs — ergänzen
#[tauri::command]
pub fn format_timestamp_for_display(utc: String, tz: String) -> Result<String, AppError> {
    time::format_timestamp_for_display(&utc, &tz)
}
```

- [x] **Step 3: `create_entry` merkt sich die letzte Auswahl**

```rust
// src-tauri/src/commands/entries.rs — create_entry ersetzen
#[tauri::command]
pub fn create_entry(state: State<AppState>, input: NewEntry) -> Result<Entry, AppError> {
    let conn = state.pool.get().map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    let data_dir = state.config.lock().expect("Config-Mutex vergiftet").data_dir.clone();
    let entry = entries::create(&conn, &data_dir, input, &tz)?;

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.last_customer_id = Some(entry.customer_id);
    config.last_system_id = entry.system_id;
    let config_path = config.data_dir.join("config.toml");
    if let Err(e) = config.save(&config_path) {
        eprintln!("Letzte Auswahl konnte nicht gespeichert werden: {e}");
    }

    Ok(entry)
}
```

- [x] **Step 4: Modul einhängen, Commands registrieren**

```rust
// src-tauri/src/commands/mod.rs — Zeile ergänzen
pub mod quickcapture;
```

```rust
// src-tauri/src/lib.rs — invoke_handler-Liste ergänzen
            commands::entries::format_timestamp_for_display,
            commands::quickcapture::get_last_selection,
```

- [x] **Step 5: Bauen und testen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cargo test && cd ..
```

Expected: Build grün, alle 58 Rust-Tests grün.

- [x] **Step 6: Commit**

```bash
git add src-tauri/src/commands/quickcapture.rs src-tauri/src/commands/entries.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat: add last-selection and timestamp-display commands, remember selection on save"
```

---

## Task 5: Frontend — zweiter Einstiegspunkt + Schnellerfassungs-Komponente

**Files:**
- Modify: `vite.config.ts`
- Create: `quick-capture.html`
- Create: `src/quick-capture/main.tsx`
- Create: `src/quick-capture/QuickCapture.tsx`

**Interfaces:**
- Consumes: Commands `list_customers`, `list_systems`, `get_last_selection`,
  `parse_temporal_input`, `format_timestamp_for_display`, `create_entry` (alle bereits vorhanden)
- Produces: eine per `npm run build` mitgebaute zweite Seite `quick-capture.html`

- [x] **Step 1: Vite auf Multi-Page umstellen**

```typescript
// vite.config.ts — komplett ersetzen
import { resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    outDir: "dist",
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        quickCapture: resolve(__dirname, "quick-capture.html"),
      },
    },
  },
});
```

- [x] **Step 2: `quick-capture.html` anlegen**

```html
<!-- quick-capture.html -->
<!doctype html>
<html lang="de">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Schnellerfassung</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/quick-capture/main.tsx"></script>
  </body>
</html>
```

- [x] **Step 3: `main.tsx` für das Fenster**

```tsx
// src/quick-capture/main.tsx
import React from "react";
import ReactDOM from "react-dom/client";
import QuickCapture from "./QuickCapture";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QuickCapture />
  </React.StrictMode>,
);
```

- [x] **Step 4: `QuickCapture.tsx` schreiben**

```tsx
// src/quick-capture/QuickCapture.tsx
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

interface System {
  id: number;
  customer_id: number;
  name: string;
}

interface PendingAttachment {
  token: string;
  bytesBase64: string;
  filename: string;
  mimeType: string;
}

const CATEGORIES: { value: string; label: string }[] = [
  { value: "wartung", label: "Wartung" },
  { value: "stoerung", label: "Störung" },
  { value: "aenderung", label: "Änderung" },
  { value: "installation", label: "Installation" },
  { value: "sonstiges", label: "Sonstiges" },
];

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

export default function QuickCapture() {
  const titleRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);

  const [customers, setCustomers] = useState<Customer[]>([]);
  const [systems, setSystems] = useState<System[]>([]);
  const [customerId, setCustomerId] = useState<number | "">("");
  const [systemId, setSystemId] = useState<number | "">("");
  const [title, setTitle] = useState("");
  const [bodyMd, setBodyMd] = useState("");
  const [category, setCategory] = useState("wartung");
  const [tagNames, setTagNames] = useState("");
  const [performedAtInput, setPerformedAtInput] = useState("");
  const [performedAtUtc, setPerformedAtUtc] = useState("");
  const [performedAtTz, setPerformedAtTz] = useState("");
  const [performedAtPreview, setPerformedAtPreview] = useState("");
  const [pendingAttachments, setPendingAttachments] = useState<PendingAttachment[]>([]);
  const [error, setError] = useState<string | null>(null);

  const resetDraft = useCallback(() => {
    setTitle("");
    setBodyMd("");
    setCategory("wartung");
    setTagNames("");
    setPendingAttachments([]);
    setPerformedAtInput("");
    setError(null);
  }, []);

  const refreshPreview = useCallback(async (utc: string, tz: string) => {
    try {
      const preview = await invoke<string>("format_timestamp_for_display", { utc, tz });
      setPerformedAtPreview(preview);
    } catch {
      setPerformedAtPreview("");
    }
  }, []);

  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  useEffect(() => {
    if (customerId === "") {
      setSystems([]);
      return;
    }
    invoke<System[]>("list_systems", { customerId, includeArchived: false }).then(setSystems);
  }, [customerId]);

  useEffect(() => {
    const unlisten = listen<{ performed_at_utc: string; performed_at_tz: string }>(
      "quick-capture-activated",
      async (event) => {
        if (title === "" && bodyMd === "") {
          const { performed_at_utc, performed_at_tz } = event.payload;
          setPerformedAtUtc(performed_at_utc);
          setPerformedAtTz(performed_at_tz);
          setPerformedAtInput("");
          await refreshPreview(performed_at_utc, performed_at_tz);

          const last = await invoke<{ customer: Customer | null; system: System | null }>("get_last_selection");
          if (last.customer) setCustomerId(last.customer.id);
          if (last.system) setSystemId(last.system.id);
        }
        titleRef.current?.focus();
      },
    );
    return () => {
      unlisten.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    const unlisten = currentWindow.onFocusChanged(({ payload: focused }) => {
      if (focused) titleRef.current?.focus();
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function handlePerformedAtBlur() {
    try {
      const result = await invoke<{ utc: string; tz: string }>("parse_temporal_input", { input: performedAtInput });
      setPerformedAtUtc(result.utc);
      setPerformedAtTz(result.tz);
      await refreshPreview(result.utc, result.tz);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  function insertAtCursor(text: string) {
    const el = bodyRef.current;
    if (!el) {
      setBodyMd((prev) => `${prev}\n${text}`);
      return;
    }
    const start = el.selectionStart ?? bodyMd.length;
    const end = el.selectionEnd ?? bodyMd.length;
    const next = `${bodyMd.slice(0, start)}${text}${bodyMd.slice(end)}`;
    setBodyMd(next);
    requestAnimationFrame(() => {
      el.selectionStart = el.selectionEnd = start + text.length;
      el.focus();
    });
  }

  async function handlePaste(e: React.ClipboardEvent<HTMLTextAreaElement>) {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith("image/")) {
        e.preventDefault();
        const file = item.getAsFile();
        if (!file) return;
        const buffer = await file.arrayBuffer();
        const bytesBase64 = arrayBufferToBase64(buffer);
        const token = `pending:${crypto.randomUUID()}`;
        insertAtCursor(`![Screenshot](${token})`);
        setPendingAttachments((prev) => [...prev, { token, bytesBase64, filename: "Screenshot.png", mimeType: item.type }]);
        return;
      }
    }
  }

  const save = useCallback(async () => {
    try {
      await invoke("create_entry", {
        input: {
          customer_id: customerId,
          system_id: systemId === "" ? null : systemId,
          title,
          body_md: bodyMd,
          category,
          performed_at_utc: performedAtUtc,
          performed_at_tz: performedAtTz,
          tag_names: tagNames.split(",").map((s) => s.trim()).filter(Boolean),
          pending_attachments: pendingAttachments.map((p) => ({
            placeholder_token: p.token,
            bytes_base64: p.bytesBase64,
            original_filename: p.filename,
            mime_type: p.mimeType,
          })),
        },
      });
      resetDraft();
      await getCurrentWindow().hide();
    } catch (e) {
      setError(String(e));
    }
  }, [customerId, systemId, title, bodyMd, category, performedAtUtc, performedAtTz, tagNames, pendingAttachments, resetDraft]);

  const discard = useCallback(async () => {
    resetDraft();
    await getCurrentWindow().hide();
  }, [resetDraft]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.ctrlKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      } else if (e.key === "Escape") {
        e.preventDefault();
        void discard();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [save, discard]);

  return (
    <main style={{ fontFamily: "sans-serif", padding: "0.75rem", display: "flex", flexDirection: "column", gap: "0.5rem" }}>
      <input
        ref={titleRef}
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        placeholder="Titel"
        style={{ fontSize: "1rem", padding: "0.4rem" }}
      />
      <div style={{ display: "flex", gap: "0.5rem" }}>
        <select value={customerId} onChange={(e) => setCustomerId(e.target.value === "" ? "" : Number(e.target.value))}>
          <option value="">Kunde wählen…</option>
          {customers.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name} ({c.short_code})
            </option>
          ))}
        </select>
        <select value={systemId} onChange={(e) => setSystemId(e.target.value === "" ? "" : Number(e.target.value))}>
          <option value="">Kein System</option>
          {systems.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
        <select value={category} onChange={(e) => setCategory(e.target.value)}>
          {CATEGORIES.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>
      </div>
      <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
        <input
          value={performedAtInput}
          onChange={(e) => setPerformedAtInput(e.target.value)}
          onBlur={handlePerformedAtBlur}
          placeholder="jetzt"
          style={{ width: "10rem" }}
        />
        <span style={{ fontFamily: "monospace", fontSize: "0.85rem" }}>{performedAtPreview}</span>
      </div>
      <textarea
        ref={bodyRef}
        value={bodyMd}
        onChange={(e) => setBodyMd(e.target.value)}
        onPaste={handlePaste}
        placeholder="Markdown…"
        style={{ flex: 1, minHeight: "10rem", fontFamily: "monospace" }}
      />
      <input value={tagNames} onChange={(e) => setTagNames(e.target.value)} placeholder="Tags, durch Komma getrennt" />
      {error && <p style={{ color: "crimson" }}>Fehler: {error}</p>}
    </main>
  );
}
```

- [x] **Step 5: Frontend bauen**

```bash
npm run build
```

Expected: `dist/index.html` und `dist/quick-capture.html` (samt zugehörigem JS-Bundle)
werden erzeugt, kein TypeScript-Fehler.

- [x] **Step 6: Commit**

```bash
git add vite.config.ts quick-capture.html src/quick-capture
git commit -m "feat: add quick-capture window frontend"
```

---

## Task 6: End-to-End-Verifikation

- [x] **Step 1: App real starten, Boot-Log prüfen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export WARTUNGSDOKU_DATA_DIR="$(pwd)/.smoke-test-data"
cd src-tauri
timeout 8 cargo run --bin wartungsdoku > ../smoke-test.log 2>&1
cd ..
cat smoke-test.log
```

Expected: kein `panic!`/`expect`-Abbruch; insbesondere kein Fehler beim Erzeugen des
`quick-capture`-Fensters oder beim Laden von `quick-capture.html`.

**Bekannte Grenze dieses Environments:** ich kann das native Fenster nicht interaktiv
bedienen (kein Tastatur-/Maus-Zugriff auf OS-Fenster, nur auf einen Web-Browser). Das
tatsächliche Tastaturverhalten (Tray-Klick öffnet das Fenster, Titel-Fokus, `Strg+V`
fügt einen Screenshot ein, `Strg+S`/`Esc` schließen korrekt) muss der Nutzer einmal
manuell prüfen — hier wird nur verifiziert, dass alles kompiliert, baut und die
Anwendung ohne Absturz hochfährt.

- [x] **Step 2: Aufräumen**

```bash
rm -rf .smoke-test-data smoke-test.log
```

- [x] **Step 3: Plan-Datei committen**

```bash
git add docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase3b2.md
git commit -m "docs: mark Phase 3b-2 plan tasks complete"
```

---

## Ausführungsnotizen

- **Task 5**: `vite.config.ts` brauchte `@types/node` (nicht im Plan erwähnt) für
  `node:path`/`__dirname` — nachinstalliert. `__dirname` außerdem durch
  `import.meta.dirname` ersetzt (Vite meldete es als in einer künftigen Major-Version
  entfernt).
- **Task 5**: Die geplante Pseudocode-Fassung des `"quick-capture-activated"`-Listeners
  prüfte `title === "" && bodyMd === ""` direkt in einem mit `[]` registrierten Effekt —
  das hätte wegen React-Closures immer die anfänglichen (leeren) Werte gesehen, nie den
  aktuellen Entwurfsstand. Behoben über eine per separatem Effekt aktuell gehaltene
  `draftIsEmptyRef`, die der Listener stattdessen abfragt.

## Self-Review (durchgeführt vor Ausführung)

- **Spec-Abdeckung**: kompaktes, mittig platziertes, nicht-Hauptfenster ✓; letzte
  Auswahl vorbelegt ✓; `performed_at` = Moment der Aktivierung bei frischem Entwurf ✓;
  `Strg+V` fügt Screenshot ein und schreibt Markdown-Referenz an Cursorposition ✓;
  `Strg+S`/`Esc` mit Entwurf-Leerung + Verstecken ✓; unvollendeter Entwurf überlebt
  Schließen ✓ (kostenlos durch hide-statt-close).
- **Bewusst vereinfacht/verschoben**: Kunde/System-Auswahl ist ein `<select>`, keine
  Fuzzy-Eingabe — die "dieselbe Fuzzy-Eingabe wie in der Command Palette" kommt erst mit
  Phase 4, wenn diese Komponente existiert; ein Wegwerf-Fuzzy-Input jetzt wäre
  doppelte Arbeit. Fokus-Rückgabe an die *zuvor aktive fremde Anwendung* (nicht nur
  internes Fenstermanagement) und `Strg+Alt+S` (Zwischenablage ohne fokussiertes
  Webview, braucht `arboard`) folgen in Phase 3b-3 zusammen mit den globalen Hotkeys,
  die diese Fenster-Aktivierung überhaupt erst von außerhalb der App auslösen.
- **Platzhalter-Scan**: keine TBD/TODO.
- **Typkonsistenz**: `QuickCaptureActivated`-Payload-Feldnamen (Task 3) exakt gleich im
  Frontend-Event-Listener (Task 5) verwendet; `NewEntry`-Feldnamen im `save()`-Aufruf
  (Task 5) entsprechen exakt den Rust-Feldnamen aus `db/entries.rs` (Phase 2/3b-1).
