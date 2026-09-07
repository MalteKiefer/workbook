# Wartungsdoku — Phase 3a: Hintergrundbetrieb (Tray, Single-Instance, Autostart) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** App runs as a background process: tray icon with a minimal working menu,
closing the window hides it instead of quitting, a second launch focuses the existing
instance, autostart state syncs with `config.toml` at boot. This is **Phase 3a** — a
split of the original "Tray-Betrieb mit globalem Hotkey und Schnellerfassung" phase.
**Phase 3b** (global hotkeys, quick-capture window, clipboard-screenshot attachment,
context capture, Wayland CLI fallback) follows as its own plan once this is reviewed.

**Architecture:** `tray-icon` is a **Cargo feature flag on the `tauri` crate itself in
Tauri v2, not a separate plugin** (the spec listed it alongside the plugins — this is a
naming correction, not a scope change). `tauri-plugin-single-instance` must be the
*first* plugin registered in the builder chain (documented requirement). Window-hide
and tray both operate on the `"main"` window via `app.get_webview_window("main")`.

**Tech Stack:** `tauri` (+ `tray-icon` feature), `tauri-plugin-single-instance`,
`tauri-plugin-autostart`.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
**Phase 2 plan (done):** [docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase2.md](2026-09-07-wartungsdoku-plan-phase2.md)

## Global Constraints

- Fenster schließen (`X`) versteckt die Anwendung, beendet sie nicht. Beenden nur bewusst über Tray-Menü.
- Single-Instance: zweiter Start beendet sich, signalisiert der laufenden Instanz, das Fenster zu zeigen.
- Autostart ist standardmäßig an, in `config.toml` (`autostart_enabled`) abschaltbar.
- Fehler nie stillschweigend verschluckt — auch eine fehlschlagende Plugin-Registrierung muss sichtbar sein (`eprintln!` bei Tray/Autostart-Fehlern, kein `.expect()`-Absturz für Nicht-kritisches).

---

## Task 1: Abhängigkeiten hinzufügen

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [x] **Step 1: `tray-icon`-Feature und Plugins hinzufügen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri
cargo add tauri --features tray-icon
cargo add tauri-plugin-single-instance
cargo add tauri-plugin-autostart
cd ..
```

- [x] **Step 2: Build verifizieren**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cd ..
```

Expected: kompiliert (Plugins noch nicht registriert, nur Abhängigkeiten vorhanden).

- [x] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add tray-icon feature and single-instance/autostart plugin dependencies"
```

---

## Task 2: Fenster-Verwaltung (Schließen versteckt statt beendet)

**Files:**
- Create: `src-tauri/src/window.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `pub fn show_and_focus_main(app: &tauri::AppHandle)`, `pub fn install_hide_on_close(app: &tauri::AppHandle)`

- [x] **Step 1: `window.rs` schreiben**

```rust
// src-tauri/src/window.rs
use tauri::{AppHandle, Manager, WindowEvent};

pub fn show_and_focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        eprintln!("Hauptfenster nicht gefunden — konnte es nicht anzeigen");
    }
}

pub fn install_hide_on_close(app: &AppHandle) {
    let Some(main_window) = app.get_webview_window("main") else {
        eprintln!("Hauptfenster nicht gefunden — Schließen-Verhalten nicht verdrahtet");
        return;
    };
    let window_to_hide = main_window.clone();
    main_window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window_to_hide.hide();
        }
    });
}
```

Kein isolierter Unit-Test möglich (braucht echtes `AppHandle`/Fenster) — Verifikation
erfolgt in Task 5 über den echten App-Start.

- [x] **Step 2: In `lib.rs` im `.setup()`-Callback verdrahten**

```rust
// src-tauri/src/lib.rs — Modul-Deklaration ergänzen
pub mod window;
```

```rust
// src-tauri/src/lib.rs — im run(), vor .invoke_handler(...) einfügen
        .setup(|app| {
            window::install_hide_on_close(app.handle());
            Ok(())
        })
```

- [x] **Step 3: Build verifizieren**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cd ..
```

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/window.rs src-tauri/src/lib.rs
git commit -m "feat: hide main window on close instead of quitting"
```

---

## Task 3: Tray-Icon mit Minimal-Menü

**Files:**
- Create: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `window::show_and_focus_main` (Task 2)
- Produces: `pub fn build_tray(app: &tauri::AppHandle) -> Result<(), AppError>`

- [x] **Step 1: `tray.rs` schreiben**

```rust
// src-tauri/src/tray.rs
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::error::AppError;
use crate::window::show_and_focus_main;

pub fn build_tray(app: &AppHandle) -> Result<(), AppError> {
    let show_item = MenuItem::with_id(app, "show", "Fenster zeigen", true, None::<&str>)
        .map_err(|e| AppError::Config(format!("Tray-Menüeintrag konnte nicht erstellt werden: {e}")))?;
    let separator = PredefinedMenuItem::separator(app)
        .map_err(|e| AppError::Config(format!("Tray-Trennlinie konnte nicht erstellt werden: {e}")))?;
    let quit_item = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)
        .map_err(|e| AppError::Config(format!("Tray-Menüeintrag konnte nicht erstellt werden: {e}")))?;

    let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])
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

- [x] **Step 2: In `lib.rs` einhängen**

```rust
// src-tauri/src/lib.rs — Modul-Deklaration ergänzen
pub mod tray;
```

```rust
// src-tauri/src/lib.rs — im .setup()-Callback ergänzen
        .setup(|app| {
            window::install_hide_on_close(app.handle());
            if let Err(e) = tray::build_tray(app.handle()) {
                eprintln!("Tray konnte nicht eingerichtet werden: {e}");
            }
            Ok(())
        })
```

- [x] **Step 3: Build verifizieren**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cd ..
```

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/tray.rs src-tauri/src/lib.rs
git commit -m "feat: add tray icon with show/quit menu"
```

---

## Task 4: Single-Instance + Autostart-Synchronisation

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `window::show_and_focus_main` (Task 2), `config::Config.autostart_enabled` (Phase 1)

- [x] **Step 1: Single-Instance-Plugin als erstes Plugin registrieren**

```rust
// src-tauri/src/lib.rs — .manage(...) direkt VORAUSGEHEND einfügen (muss vor jedem
// anderen .plugin(...)-Aufruf stehen, falls später welche folgen)
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            window::show_and_focus_main(app);
        }))
```

Die vollständige `tauri::Builder`-Kette in `run()` sieht danach so aus (Reihenfolge
beachten — `single_instance` zuerst, `manage` und `setup` danach):

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            window::show_and_focus_main(app);
        }))
        .manage(AppState { pool, config: Mutex::new(app_config) })
        .setup(|app| {
            window::install_hide_on_close(app.handle());
            if let Err(e) = tray::build_tray(app.handle()) {
                eprintln!("Tray konnte nicht eingerichtet werden: {e}");
            }

            let autostart_enabled = {
                let state = app.state::<AppState>();
                state.config.lock().expect("Config-Mutex vergiftet").autostart_enabled
            };
            use tauri_plugin_autostart::ManagerExt;
            let autolaunch = app.autolaunch();
            let sync_result = if autostart_enabled {
                autolaunch.enable()
            } else {
                autolaunch.disable()
            };
            if let Err(e) = sync_result {
                eprintln!("Autostart konnte nicht synchronisiert werden: {e}");
            }

            Ok(())
        })
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
```

(Der `.plugin(tauri_plugin_autostart::Builder::new().build())`-Aufruf muss **vor** dem
`.setup()`-Callback in der Builder-Kette stehen, damit `app.autolaunch()` dort bereits
verfügbar ist — Tauri führt `.plugin(...)`-Registrierungen in Aufrufreihenfolge aus,
danach erst `.setup()`. Der Codeblock oben zeigt die *gewünschte Logik*; beim
Einfügen in `lib.rs` steht `.plugin(tauri_plugin_autostart::Builder::new().build())`
direkt nach `.manage(...)`, vor `.setup(...)`.)

- [x] **Step 2: Endgültige Reihenfolge in `run()` herstellen**

```rust
// src-tauri/src/lib.rs — vollständiger run()-Körper ab tauri::Builder::default()
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            window::show_and_focus_main(app);
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(AppState { pool, config: Mutex::new(app_config) })
        .setup(|app| {
            window::install_hide_on_close(app.handle());
            if let Err(e) = tray::build_tray(app.handle()) {
                eprintln!("Tray konnte nicht eingerichtet werden: {e}");
            }

            let autostart_enabled = {
                let state = app.state::<AppState>();
                state.config.lock().expect("Config-Mutex vergiftet").autostart_enabled
            };
            use tauri_plugin_autostart::ManagerExt;
            let autolaunch = app.autolaunch();
            let sync_result = if autostart_enabled { autolaunch.enable() } else { autolaunch.disable() };
            if let Err(e) = sync_result {
                eprintln!("Autostart konnte nicht synchronisiert werden: {e}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::customers::list_customers,
            commands::customers::create_customer,
            commands::customers::update_customer,
            commands::customers::archive_customer,
            commands::systems::list_systems,
            commands::systems::create_system,
            commands::systems::update_system,
            commands::systems::archive_system,
            commands::tags::list_tags,
            commands::entries::list_entries,
            commands::entries::get_entry,
            commands::entries::create_entry,
            commands::entries::update_entry,
            commands::entries::parse_temporal_input,
            commands::search::search_entries,
            commands::search::search_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
```

- [x] **Step 3: Build verifizieren**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build --bin wartungsdoku && cargo test && cd ..
```

Expected: Build grün; alle 45 Rust-Tests weiterhin grün (an der Datenschicht ändert
sich nichts).

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: register single-instance plugin and sync autostart with config"
```

---

## Task 5: End-to-End-Verifikation

**Files:** keine neuen.

- [x] **Step 1: App real starten, Boot-Log prüfen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(pwd)"
export WARTUNGSDOKU_DATA_DIR="$(pwd)/.smoke-test-data"
cd src-tauri
timeout 8 cargo run --bin wartungsdoku > ../smoke-test.log 2>&1
cd ..
cat smoke-test.log
```

Expected: kein `panic!`/`expect`-Abbruch; kein `"Tray konnte nicht eingerichtet
werden"` oder `"Autostart konnte nicht synchronisiert werden"` im Log (beides wäre
sichtbar, aber nicht fatal, falls doch — dann Ursache klären statt ignorieren).

- [x] **Step 2: Aufräumen**

```bash
rm -rf .smoke-test-data smoke-test.log
```

- [x] **Step 3: Plan-Datei mit abgehaktem Stand committen**

```bash
git add docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase3a.md
git commit -m "docs: mark Phase 3a plan tasks complete"
```

---

## Self-Review (durchgeführt vor Ausführung)

- **Spec-Abdeckung**: Tray-Icon mit Menü ✓, Fenster-schließen-versteckt ✓, Single-Instance mit Fokus-Signal ✓, Autostart synchronisiert mit Konfiguration ✓.
- **Bewusst verschoben (Phase 3b)**: globale Hotkeys, Schnellerfassungsfenster, Zwischenablage-Screenshot-Hotkey, Kontexterfassung, Wayland-CLI-Fallback (`--quick-capture`/`--search`). Die Tray-Menüeinträge "Schnellerfassung" und "Suche" aus der Spec-Tabelle kommen bewusst erst in Phase 3b hinzu, sobald ihre Ziel-Fenster existieren — ein Menüeintrag ohne funktionierendes Ziel wäre ein Platzhalter.
- **Namenskorrektur**: `tray-icon` ist in Tauri 2 ein Cargo-Feature auf dem `tauri`-Crate, kein eigenständiges Plugin — im Auftrag als Plugin gelistet, hier explizit richtiggestellt.
- **Platzhalter-Scan**: keine TBD/TODO. `eprintln!`-Fehlerpfade sind bewusste, sichtbare Nicht-Abstürze für nicht-kritische Subsysteme (Tray/Autostart), keine verschluckten Fehler.
- **Typkonsistenz**: `AppError::Config` wird für Tray-/Autostart-Fehler wiederverwendet (passender Code als jede neue Variante); `AppHandle`-Signatur von `show_and_focus_main`/`install_hide_on_close` (Task 2) identisch in `tray.rs` (Task 3) und `lib.rs` (Task 4) verwendet.
