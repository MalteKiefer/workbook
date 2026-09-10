# Keyboard Shortcut Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Tastaturbelegung" tab to Settings where every keyboard
shortcut (3 global OS hotkeys + 9 in-app actions) can be viewed and
rebound, replacing the current mix of no-UI-at-all (global hotkeys) and
hardcoded-in-11-files (in-app shortcuts).

**Architecture:** A new `KeymapConfig` struct in `src-tauri/src/config.rs`
(mirrors the existing `HotkeyConfig` pattern exactly) persists the 9 in-app
bindings to `config.toml`. Two new Tauri command pairs
(`get_keymap`/`set_keymap`, `get_hotkeys`/`set_hotkeys`) read/write it and
`HotkeyConfig`. A new frontend module `src/lib/keymap.ts` holds a
module-level singleton (same non-React-state pattern `src/lib/theme.ts`
already uses) that every keydown handler across the app reads
synchronously via `matchesBinding(e, keymap.<action>)` instead of a literal
comparison. A new `KeymapSettingsView.tsx` lets the user record and save
new bindings.

**Tech Stack:** Rust (serde/toml, existing `AppState`/`AppError`
conventions), React/TypeScript (Zustand only where noted — most of this
feature deliberately does NOT touch the Zustand store, see Global
Constraints), Tauri commands/events (existing `get_theme_preference`/
`set_theme_preference` pair is the template for all four new commands).

**Spec:** `docs/superpowers/specs/2026-09-10-keyboard-shortcut-settings-design.md`

## Global Constraints

- Every `KeymapConfig`/`HotkeyConfig` field is a plain `String`; `#[serde(default)]` on the struct (not needing per-field attributes, matching `HotkeyConfig`'s own existing style) keeps every pre-existing `config.toml` loading correctly once this field is added to `Config`.
- **No JS test runner exists in this project** (`package.json` has no `test` script, no vitest/jest dependency) — every frontend "test" step in this plan is a manual verification via the Browser tool's `javascript_tool` (matching how prior work this session, e.g. `buildExportFilename`, was verified), never an automated unit test file. Do not add a test framework as a side effect of this plan.
- Matching is **exact**, no "Ctrl also matches Cmd" aliasing: a stored `"Ctrl+K"` requires `e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey`. The macOS-appropriate default (`"Cmd+K"`) is what makes Cmd+K work out of the box on a Mac, not runtime aliasing (see spec's self-review note).
- A binding with no recognized modifier token (e.g. `"j"`) requires all four modifier flags to be `false` at match time — this is a deliberate, minor tightening vs. today's shipped `e.key === "j"` check (which technically also fired on Ctrl+J/Alt+J/Meta+J, an existing oversight, not a documented feature). Call this out in the relevant task's commit message, don't treat it as silently out of scope.
- The three `goto_*` bindings (`goto_customers`, `goto_systems`, `goto_journal`) are two-key sequences ("prefix key" then "follow-up key", space-separated, e.g. `"g c"`) because that's the actual underlying mechanism (`useGlobalHotkeys.ts`'s 800ms pending-prefix state machine) — not three independent combinations. `set_keymap` validates all three share the same prefix word; the frontend state machine reads that shared prefix once.
- Excluded from remapping entirely (see spec's Scope section for why): Ctrl+V/Cmd+V (native paste event), Enter, Esc. None of these gain a `KeymapConfig` field.
- `formatShortcut` in `src/lib/platform.ts` (added in commit `01e2e6b`) stays and keeps its one remaining caller (the Ctrl+V row in `ShortcutOverview.tsx`) — it is NOT the same thing as this plan's new `formatBindingForDisplay` in `src/lib/keymap.ts`, which formats an arbitrary *stored, possibly user-customized* binding string rather than reconstructing a hardcoded default from a bare letter.

---

### Task 1: `KeymapConfig` data model

**Files:**
- Modify: `src-tauri/src/config.rs`

**Interfaces:**
- Produces: `pub struct KeymapConfig { pub command_palette: String, pub quick_capture: String, pub save: String, pub goto_customers: String, pub goto_systems: String, pub goto_journal: String, pub list_next: String, pub list_prev: String, pub edit_selected: String }`, its `impl Default`, and a new `pub keymap: KeymapConfig` field on `Config`.

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block at the bottom of `src-tauri/src/config.rs` (near the existing `save_then_load_roundtrips_ninja_org_mappings`/`config_without_ninja_org_mappings_field_defaults_to_empty` pair — copy that exact two-test shape):

```rust
    #[test]
    fn keymap_defaults_use_platform_appropriate_primary_modifier() {
        let config = Config::default();
        let expected_primary = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
        assert_eq!(config.keymap.command_palette, format!("{expected_primary}+K"));
        assert_eq!(config.keymap.quick_capture, format!("{expected_primary}+N"));
        assert_eq!(config.keymap.save, format!("{expected_primary}+S"));
        assert_eq!(config.keymap.goto_customers, "g c");
        assert_eq!(config.keymap.goto_systems, "g s");
        assert_eq!(config.keymap.goto_journal, "g j");
        assert_eq!(config.keymap.list_next, "j");
        assert_eq!(config.keymap.list_prev, "k");
        assert_eq!(config.keymap.edit_selected, "e");
    }

    #[test]
    fn save_then_load_roundtrips_keymap() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.keymap.command_palette = "Ctrl+Shift+K".to_string();
        config.keymap.list_next = "n".to_string();

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.keymap.command_palette, "Ctrl+Shift+K");
        assert_eq!(loaded.keymap.list_next, "n");
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_keymap_field_defaults_to_platform_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simulates a config.toml from before this feature existed -- the
        // field is entirely missing and must fall back gracefully to
        // KeymapConfig::default() thanks to `#[serde(default)]` instead of
        // making loading fail.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.keymap, KeymapConfig::default());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run (PowerShell, `src-tauri` directory): `cargo test --lib config:: 2>&1 | tail -40`
Expected: FAIL to compile — `KeymapConfig` and `Config.keymap` don't exist yet.

- [ ] **Step 3: Implement `KeymapConfig` and wire it into `Config`**

Add right after the existing `HotkeyConfig`/`impl Default for HotkeyConfig` block (after line 61, before `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n#[serde(default)]\npub struct Config {`):

```rust
/// In-app keyboard shortcuts (Settings -> Tastaturbelegung), as opposed to
/// `HotkeyConfig`'s 3 OS-registered global hotkeys. Checked by JS keydown
/// handlers (see `src/lib/keymap.ts::matchesBinding`), not the OS, so a
/// change here takes effect immediately -- no restart, unlike `HotkeyConfig`.
///
/// `goto_customers`/`goto_systems`/`goto_journal` are two-key sequences
/// ("prefix key" then "follow-up key", space-separated, e.g. `"g c"`) since
/// that is the actual mechanism (`useGlobalHotkeys.ts`'s 800ms
/// pending-prefix state machine), not three independent key combinations --
/// `commands::keymap::set_keymap` validates all three share the same
/// prefix word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    pub command_palette: String,
    pub quick_capture: String,
    pub save: String,
    pub goto_customers: String,
    pub goto_systems: String,
    pub goto_journal: String,
    pub list_next: String,
    pub list_prev: String,
    pub edit_selected: String,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        // macOS's shipped default is the literal "Cmd+..." string, not
        // "Ctrl+..." matched loosely against both Ctrl and Cmd at runtime --
        // matching is exact (see src/lib/keymap.ts::matchesBinding), so the
        // platform-correct literal here is what makes Cmd+K work out of the
        // box on a Mac.
        let primary = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
        Self {
            command_palette: format!("{primary}+K"),
            quick_capture: format!("{primary}+N"),
            save: format!("{primary}+S"),
            goto_customers: "g c".to_string(),
            goto_systems: "g s".to_string(),
            goto_journal: "g j".to_string(),
            list_next: "j".to_string(),
            list_prev: "k".to_string(),
            edit_selected: "e".to_string(),
        }
    }
}
```

Then add `pub keymap: KeymapConfig,` to the `Config` struct, directly under the existing `pub hotkeys: HotkeyConfig,` field (around line 70), and add `keymap: KeymapConfig::default(),` next to the existing `hotkeys: HotkeyConfig::default(),` in `impl Default for Config` (around line 216).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib config:: 2>&1 | tail -40`
Expected: PASS, all `config::tests::*` green including the 3 new ones.

- [ ] **Step 5: Full backend verification and commit**

Run (from `src-tauri`): `cargo build --lib && cargo test --lib && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check`
Expected: all four succeed with no warnings/diffs.

```bash
git add src-tauri/src/config.rs
git commit -m "feat: add KeymapConfig for customizable in-app keyboard shortcuts"
```

---

### Task 2: `get_keymap`/`set_keymap`/`get_hotkeys`/`set_hotkeys` commands

**Files:**
- Create: `src-tauri/src/commands/keymap.rs`
- Modify: `src-tauri/src/commands/mod.rs:15` (add `pub mod keymap;` alphabetically after `pub mod external_directory;`... actually alphabetically it goes between `pub mod intune;` and `pub mod iru;` — insert as its own line `pub mod keymap;` in the existing alphabetical list)
- Modify: `src-tauri/src/lib.rs` (register the 4 new commands in `tauri::generate_handler![...]`)

**Interfaces:**
- Consumes: `crate::config::{Config, HotkeyConfig, KeymapConfig}` (Task 1), `crate::{AppError, AppState}` (existing).
- Produces: `get_keymap(state) -> KeymapConfig`, `set_keymap(state, app, keymap: KeymapConfig) -> Result<(), AppError>`, `get_hotkeys(state) -> HotkeyConfig`, `set_hotkeys(state, hotkeys: HotkeyConfig) -> Result<(), AppError>`. `set_keymap` emits a `"keymap-changed"` event with the new `KeymapConfig` as payload — Task 3's frontend listens for exactly this event name/payload shape.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/commands/keymap.rs` with just the validation logic and its tests first (the two Tauri commands need `AppState`, which isn't constructible in a plain unit test — `validate_keymap` is tested directly, the commands themselves are exercised for real once Task 11's Settings UI calls them end-to-end):

```rust
//! Custom keyboard bindings (Settings -> Tastaturbelegung). Two separate
//! config sections with different semantics:
//! - `KeymapConfig` (get_keymap/set_keymap): in-app shortcuts, checked by
//!   JS keydown handlers (see `src/lib/keymap.ts`), apply immediately --
//!   `set_keymap` broadcasts a `"keymap-changed"` event so an already-open
//!   window (e.g. quick capture) picks up the change live.
//! - `HotkeyConfig` (get_hotkeys/set_hotkeys): OS-registered global
//!   hotkeys via tauri-plugin-global-shortcut (`src-tauri/src/hotkeys.rs`),
//!   only read at startup -- a change here needs an app restart, which the
//!   frontend surfaces explicitly rather than pretending it's live.

use tauri::{AppHandle, Emitter, State};

use crate::config::{HotkeyConfig, KeymapConfig};
use crate::{AppError, AppState};

#[tauri::command]
pub fn get_keymap(state: State<AppState>) -> KeymapConfig {
    state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .keymap
        .clone()
}

#[tauri::command]
pub fn set_keymap(
    state: State<AppState>,
    app: AppHandle,
    keymap: KeymapConfig,
) -> Result<(), AppError> {
    validate_keymap(&keymap)?;
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.keymap = keymap.clone();
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    app.emit("keymap-changed", keymap)
        .map_err(|e| AppError::Config(format!("Ereignis konnte nicht gesendet werden: {e}")))?;
    Ok(())
}

#[tauri::command]
pub fn get_hotkeys(state: State<AppState>) -> HotkeyConfig {
    state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .hotkeys
        .clone()
}

#[tauri::command]
pub fn set_hotkeys(state: State<AppState>, hotkeys: HotkeyConfig) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.hotkeys = hotkeys;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

/// Rejects an empty binding, two actions sharing an identical binding
/// (case-insensitively -- Shift changes a letter's case but not the
/// physical key), or the three `goto_*` two-key sequences not sharing the
/// same prefix word (the frontend's pending-prefix state machine only
/// tracks one prefix at a time, see `src/hooks/useGlobalHotkeys.ts`).
fn validate_keymap(keymap: &KeymapConfig) -> Result<(), AppError> {
    let entries: [(&str, &str); 9] = [
        ("Command Palette", keymap.command_palette.as_str()),
        ("Schnellerfassung", keymap.quick_capture.as_str()),
        ("Speichern", keymap.save.as_str()),
        ("Zu Kundenliste", keymap.goto_customers.as_str()),
        ("Zu Systemliste", keymap.goto_systems.as_str()),
        ("Zum Journal", keymap.goto_journal.as_str()),
        ("Liste: nächster Eintrag", keymap.list_next.as_str()),
        ("Liste: vorheriger Eintrag", keymap.list_prev.as_str()),
        ("Ausgewähltes bearbeiten", keymap.edit_selected.as_str()),
    ];

    for (label, binding) in entries {
        if binding.trim().is_empty() {
            return Err(AppError::Config(format!(
                "\"{label}\" braucht eine Tastenkombination"
            )));
        }
    }

    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            if entries[i].1.eq_ignore_ascii_case(entries[j].1) {
                return Err(AppError::Config(format!(
                    "\"{}\" und \"{}\" haben dieselbe Tastenkombination (\"{}\")",
                    entries[i].0, entries[j].0, entries[i].1
                )));
            }
        }
    }

    let customers_prefix = keymap.goto_customers.split(' ').next().unwrap_or("");
    let systems_prefix = keymap.goto_systems.split(' ').next().unwrap_or("");
    let journal_prefix = keymap.goto_journal.split(' ').next().unwrap_or("");
    if customers_prefix != systems_prefix || customers_prefix != journal_prefix {
        return Err(AppError::Config(
            "\"Zu Kundenliste\", \"Zu Systemliste\" und \"Zum Journal\" müssen mit derselben ersten Taste beginnen".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_keymap() -> KeymapConfig {
        KeymapConfig::default()
    }

    #[test]
    fn default_keymap_passes_validation() {
        assert!(validate_keymap(&valid_keymap()).is_ok());
    }

    #[test]
    fn empty_binding_is_rejected() {
        let mut keymap = valid_keymap();
        keymap.save = "   ".to_string();
        let result = validate_keymap(&keymap);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn duplicate_binding_is_rejected() {
        let mut keymap = valid_keymap();
        keymap.save = keymap.command_palette.clone();
        let result = validate_keymap(&keymap);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn duplicate_binding_is_rejected_case_insensitively() {
        let mut keymap = valid_keymap();
        keymap.list_next = "E".to_string();
        keymap.edit_selected = "e".to_string();
        let result = validate_keymap(&keymap);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn mismatched_goto_prefixes_are_rejected() {
        let mut keymap = valid_keymap();
        keymap.goto_systems = "x s".to_string();
        let result = validate_keymap(&keymap);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn matching_goto_prefixes_with_a_different_letter_are_accepted() {
        let mut keymap = valid_keymap();
        keymap.goto_customers = "x c".to_string();
        keymap.goto_systems = "x s".to_string();
        keymap.goto_journal = "x j".to_string();
        assert!(validate_keymap(&keymap).is_ok());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib commands::keymap 2>&1 | tail -40`
Expected: FAIL to compile — `src-tauri/src/commands/keymap.rs` isn't registered as a module yet.

- [ ] **Step 3: Register the module and the 4 commands**

In `src-tauri/src/commands/mod.rs`, add `pub mod keymap;` into the existing alphabetical list (between `pub mod jamf;` and `pub mod level;`).

In `src-tauri/src/lib.rs`, add these 4 lines to the `tauri::generate_handler![...]` list, directly after the existing `commands::settings::get_theme_preference,`/`commands::settings::set_theme_preference,` pair:

```rust
            commands::keymap::get_keymap,
            commands::keymap::set_keymap,
            commands::keymap::get_hotkeys,
            commands::keymap::set_hotkeys,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib commands::keymap 2>&1 | tail -40`
Expected: PASS, all 6 `commands::keymap::tests::*` green.

- [ ] **Step 5: Full backend verification and commit**

Run (from `src-tauri`): `cargo build --lib && cargo test --lib && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check`
Expected: all four succeed.

```bash
git add src-tauri/src/commands/keymap.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat: add get/set commands for the in-app keymap and global hotkeys"
```

---

### Task 3: `src/lib/keymap.ts` — pure functions and the live singleton

**Files:**
- Create: `src/lib/keymap.ts`
- Modify: `src/main.tsx`, `src/quick-capture/main.tsx`

**Interfaces:**
- Consumes: `isMac` from `src/lib/platform.ts` (existing, from commit `01e2e6b`). Backend commands `get_keymap` and event `"keymap-changed"` (Task 2).
- Produces: `interface Keymap { command_palette: string; quick_capture: string; save: string; goto_customers: string; goto_systems: string; goto_journal: string; list_next: string; list_prev: string; edit_selected: string }`, `matchesBinding(e: KeyboardEvent, binding: string): boolean`, `formatBinding(e: KeyboardEvent): string`, `formatBindingForDisplay(binding: string): string`, `defaultKeymap(): Keymap`, `getKeymap(): Keymap`, `loadKeymap(): Promise<void>`, `listenForKeymapChanges(): void`. Every later task that checks a shortcut imports `getKeymap`/`matchesBinding` from here.

- [ ] **Step 1: Create the file**

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isMac } from "./platform";

// Kept in sync with src-tauri/src/config.rs::KeymapConfig field-for-field.
export interface Keymap {
  command_palette: string;
  quick_capture: string;
  save: string;
  goto_customers: string;
  goto_systems: string;
  goto_journal: string;
  list_next: string;
  list_prev: string;
  edit_selected: string;
}

interface ParsedBinding {
  ctrl: boolean;
  meta: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
}

function parseBinding(binding: string): ParsedBinding {
  const parts = binding.split("+").map((p) => p.trim());
  const key = parts[parts.length - 1].toLowerCase();
  const mods = parts.slice(0, -1).map((p) => p.toLowerCase());
  return {
    ctrl: mods.includes("ctrl"),
    meta: mods.includes("cmd") || mods.includes("meta"),
    alt: mods.includes("alt"),
    shift: mods.includes("shift"),
    key,
  };
}

// Exact match: a stored "Ctrl+K" requires e.ctrlKey and NOT e.metaKey/altKey/
// shiftKey, never "Ctrl or Cmd" aliasing (see the design spec's self-review
// note — the platform-appropriate literal is what KeymapConfig::default()
// seeds instead). A binding with no modifier token (e.g. "j") requires
// every modifier flag to be false.
export function matchesBinding(e: KeyboardEvent, binding: string): boolean {
  const parsed = parseBinding(binding);
  return (
    e.key.toLowerCase() === parsed.key &&
    e.ctrlKey === parsed.ctrl &&
    e.metaKey === parsed.meta &&
    e.altKey === parsed.alt &&
    e.shiftKey === parsed.shift
  );
}

// Formats a KeyboardEvent's modifiers + key back into the same shape
// matchesBinding parses, e.g. Cmd+Shift+K -> "Cmd+Shift+K". Used by the
// KeyRecorder settings control (Task 12) — matchesBinding and this function
// must stay exact inverses for every combination the recorder can produce.
export function formatBinding(e: KeyboardEvent): string {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.metaKey) mods.push("Cmd");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  return [...mods, key].join("+");
}

// Display label for a stored binding, e.g. "Cmd+K" -> "⌘+K", "Ctrl+K" ->
// "Strg+K". Distinct from src/lib/platform.ts's formatShortcut, which
// formats a hardcoded default from a bare letter rather than an arbitrary
// (possibly user-customized) stored binding string.
export function formatBindingForDisplay(binding: string): string {
  return binding
    .split("+")
    .map((part) => (part === "Cmd" ? "⌘" : part === "Ctrl" ? "Strg" : part))
    .join("+");
}

// Mirrors src-tauri/src/config.rs::KeymapConfig::default() exactly — used
// as this module's initial value before loadKeymap()'s async fetch
// resolves, and by the Settings UI's "Auf Standard zurücksetzen" button.
export function defaultKeymap(): Keymap {
  const primary = isMac() ? "Cmd" : "Ctrl";
  return {
    command_palette: `${primary}+K`,
    quick_capture: `${primary}+N`,
    save: `${primary}+S`,
    goto_customers: "g c",
    goto_systems: "g s",
    goto_journal: "g j",
    list_next: "j",
    list_prev: "k",
    edit_selected: "e",
  };
}

// Module-level singleton, same non-React-state pattern as
// src/lib/theme.ts's DOM attribute — every keydown handler reads this
// synchronously via getKeymap(), not through a React subscription. Each
// window (main, quick capture) is a separate JS bundle/module instance
// (see src/main.tsx / src/quick-capture/main.tsx), so each keeps its own
// copy, kept in sync via loadKeymap()/listenForKeymapChanges() below —
// exactly like applyPersistedTheme()/listenForThemeChanges() already do.
let currentKeymap: Keymap = defaultKeymap();

export function getKeymap(): Keymap {
  return currentKeymap;
}

export async function loadKeymap(): Promise<void> {
  try {
    currentKeymap = await invoke<Keymap>("get_keymap");
  } catch {
    // Keep the platform-appropriate default from defaultKeymap() above
    // rather than crash startup over a failed keymap lookup — same
    // fail-open convention as src/lib/theme.ts::applyPersistedTheme.
  }
}

export function listenForKeymapChanges(): void {
  void listen<Keymap>("keymap-changed", (event) => {
    currentKeymap = event.payload;
  });
}
```

- [ ] **Step 2: Wire loading into both entry points**

In `src/main.tsx`, add the import and calls alongside the existing theme ones:

```ts
import { applyPersistedTheme, listenForThemeChanges } from "./lib/theme";
import { loadKeymap, listenForKeymapChanges } from "./lib/keymap";

void applyPersistedTheme();
listenForThemeChanges();
void loadKeymap();
listenForKeymapChanges();
```

Same two-line addition, same import-path style (`../lib/keymap`), in `src/quick-capture/main.tsx`.

- [ ] **Step 3: Verify types and lint**

Run (repo root): `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed (no `.ts` errors, no unused-import warnings — `loadKeymap`/`listenForKeymapChanges` are called, not just imported).

- [ ] **Step 4: Manual verification in the Browser tool**

Start the dev server (`preview_start` with the `vite-dev` launch config), navigate to it, then run in `javascript_tool`:

```js
const mod = await import('/src/lib/keymap.ts');
JSON.stringify({
  defaultCombo: mod.defaultKeymap().command_palette,
  formatsBack: mod.formatBinding({ ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, key: "k" }),
  matchesCtrlK: mod.matchesBinding({ ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, key: "k" }, "Ctrl+K"),
  rejectsCmdKforCtrlK: mod.matchesBinding({ ctrlKey: false, metaKey: true, altKey: false, shiftKey: false, key: "k" }, "Ctrl+K"),
  bareJNoModifiers: mod.matchesBinding({ ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, key: "j" }, "j"),
  bareJRejectsCtrlJ: mod.matchesBinding({ ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, key: "j" }, "j"),
  displayCmd: mod.formatBindingForDisplay("Cmd+K"),
  displayCtrl: mod.formatBindingForDisplay("Ctrl+Shift+K"),
})
```

Expected JSON: `defaultCombo` is `"Ctrl+K"` (this session's dev machine is Windows), `formatsBack` is `"Ctrl+K"`, `matchesCtrlK` is `true`, `rejectsCmdKforCtrlK` is `false`, `bareJNoModifiers` is `true`, `bareJRejectsCtrlJ` is `false`, `displayCmd` is `"⌘+K"`, `displayCtrl` is `"Strg+Shift+K"`. Stop the preview server after.

- [ ] **Step 5: Commit**

```bash
git add src/lib/keymap.ts src/main.tsx src/quick-capture/main.tsx
git commit -m "feat: add src/lib/keymap.ts, the live keymap singleton"
```

---

### Task 4: Refactor `useGlobalHotkeys.ts`

**Files:**
- Modify: `src/hooks/useGlobalHotkeys.ts`

**Interfaces:**
- Consumes: `getKeymap`, `matchesBinding` from `src/lib/keymap.ts` (Task 3).

- [ ] **Step 1: Replace the hardcoded Ctrl+N check and the "g" prefix mechanism**

Replace the full contents of `src/hooks/useGlobalHotkeys.ts` with:

```ts
import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { getKeymap, matchesBinding } from "../lib/keymap";

export function isTypingTarget(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || (el as HTMLElement).isContentEditable;
}

export function useGlobalHotkeys() {
  const pendingPrefixRef = useRef<string | null>(null);
  const pendingTimeoutRef = useRef<number | null>(null);

  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const goToSystems = useAppStore((s) => s.goToSystems);
  const goToJournal = useAppStore((s) => s.goToJournal);
  const formOpen = useAppStore((s) => s.formOpen);
  const closeForm = useAppStore((s) => s.closeForm);
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const selectedSystemId = useAppStore((s) => s.selectedSystemId);
  const view = useAppStore((s) => s.view);

  useEffect(() => {
    function clearPrefix() {
      pendingPrefixRef.current = null;
      if (pendingTimeoutRef.current !== null) {
        window.clearTimeout(pendingTimeoutRef.current);
        pendingTimeoutRef.current = null;
      }
    }

    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (formOpen) {
          e.preventDefault();
          closeForm();
        } else if (view === "systems") {
          goToCustomers();
        }
        clearPrefix();
        return;
      }

      if (matchesBinding(e, getKeymap().quick_capture)) {
        e.preventDefault();
        void invoke("open_quick_capture_with_context", {
          customerId: selectedCustomerId,
          systemId: selectedSystemId,
        });
        return;
      }

      if (isTypingTarget(document.activeElement) || e.ctrlKey || e.metaKey || e.altKey) {
        return;
      }

      const keymap = getKeymap();
      const [prefix, customersFollow] = keymap.goto_customers.split(" ");
      const systemsFollow = keymap.goto_systems.split(" ")[1];
      const journalFollow = keymap.goto_journal.split(" ")[1];

      if (pendingPrefixRef.current === prefix) {
        clearPrefix();
        if (e.key === customersFollow) {
          e.preventDefault();
          goToCustomers();
        } else if (e.key === systemsFollow && selectedCustomerId !== null) {
          e.preventDefault();
          goToSystems();
        } else if (e.key === journalFollow) {
          e.preventDefault();
          goToJournal();
        }
        return;
      }

      if (e.key === prefix) {
        pendingPrefixRef.current = prefix;
        pendingTimeoutRef.current = window.setTimeout(clearPrefix, 800);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      clearPrefix();
    };
  }, [formOpen, view, selectedCustomerId, selectedSystemId, goToCustomers, goToSystems, goToJournal, closeForm]);
}
```

`isTypingTarget` is unchanged (still exported, still used by `CustomerListView.tsx`/`SystemListView.tsx`/`JournalView.tsx`/`ShortcutOverview.tsx`) — only the body of `useGlobalHotkeys` changed.

- [ ] **Step 2: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed.

- [ ] **Step 3: Manual verification in the Browser tool**

Start the dev server, navigate to it, and confirm the app still loads (screenshot) and `g` then `c`/`s`/`j` still navigate between Kunden/Systeme/Journal (use `computer` key presses: `g`, then `c`; check the sidebar's active tab changed). Also confirm Ctrl+N still triggers `open_quick_capture_with_context` — check `read_network_requests`/console isn't necessary, a `read_console_messages` call after pressing Ctrl+N showing no thrown error is enough (the actual quick-capture window won't visibly open in this headless preview, but the invoke call itself not throwing confirms the binding matched). Stop the preview server after.

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useGlobalHotkeys.ts
git commit -m "refactor: read Ctrl+N and g-prefix navigation from the keymap"
```

---

### Task 5: Refactor `CommandPalette.tsx`

**Files:**
- Modify: `src/components/CommandPalette.tsx:322` (the `isModK` check), `:111-112` (quick-capture shortcut display), `:122`/`:141`/`:160` (the three `goto_*` shortcut displays)

**Interfaces:**
- Consumes: `getKeymap`, `matchesBinding`, `formatBindingForDisplay` from `src/lib/keymap.ts` (Task 3).

- [ ] **Step 1: Add the import**

Add to the top of `src/components/CommandPalette.tsx`, after the existing `import { formatInvokeError } from "../lib/errors";`:

```ts
import { getKeymap, matchesBinding, formatBindingForDisplay } from "../lib/keymap";
```

- [ ] **Step 2: Replace the isModK check**

Replace:

```ts
      const isModK = (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k";
```

with:

```ts
      const isModK = matchesBinding(e, getKeymap().command_palette);
```

- [ ] **Step 3: Replace the 4 hardcoded shortcut display strings**

At the top of the `staticCommands` `useMemo` body (right after `const cmds: StaticCommand[] = [` would normally start — add the line just before it instead), add `const keymap = getKeymap();`, then:

- Replace `shortcut: formatShortcut("N"),` with `shortcut: formatBindingForDisplay(keymap.quick_capture),` (and remove the now-unused `import { formatShortcut } from "../lib/platform";` line if this was CommandPalette's only use of it — confirm with `grep -n formatShortcut src/components/CommandPalette.tsx` after this edit; if the grep only shows the import line, delete that import line too).
- Replace `shortcut: "g c",` (the `goto-customers` command) with `shortcut: keymap.goto_customers,`
- Replace `shortcut: "g s",` (the `goto-systems` command, inside the `if (selectedCustomerId !== null)` block) with `shortcut: keymap.goto_systems,`
- Replace `shortcut: "g j",` (the `goto-journal` command) with `shortcut: keymap.goto_journal,`

- [ ] **Step 4: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed, no unused-import warning for `formatShortcut`.

- [ ] **Step 5: Manual verification in the Browser tool**

Start the dev server, navigate to it, press Ctrl+K (via `computer` key action), confirm the palette opens (`find` for "Schnellerfassung öffnen") and its shortcut hint reads "Strg+N" (Windows dev machine). Press Ctrl+K again to close it. Stop the preview server after.

- [ ] **Step 6: Commit**

```bash
git add src/components/CommandPalette.tsx
git commit -m "refactor: read Command Palette shortcut hints from the keymap"
```

---

### Task 6: Refactor Save (`EntryEditor.tsx`, `QuickCapture.tsx`)

**Files:**
- Modify: `src/components/EntryEditor.tsx:618`
- Modify: `src/quick-capture/QuickCapture.tsx:521`, `:758`

**Interfaces:**
- Consumes: `getKeymap`, `matchesBinding`, `formatBindingForDisplay` from `src/lib/keymap.ts` (Task 3).

- [ ] **Step 1: `EntryEditor.tsx`**

Add to the imports (after `import { formatInvokeError } from "../lib/errors";`):

```ts
import { getKeymap, matchesBinding } from "../lib/keymap";
```

Replace:

```ts
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
```

with:

```ts
      if (matchesBinding(e, getKeymap().save)) {
```

- [ ] **Step 2: `QuickCapture.tsx`**

Replace the import `import { formatShortcut } from "../lib/platform";` with:

```ts
import { getKeymap, matchesBinding, formatBindingForDisplay } from "../lib/keymap";
```

Replace:

```ts
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
```

with:

```ts
      if (matchesBinding(e, getKeymap().save)) {
```

Replace:

```ts
          Speichern ({formatShortcut("S")})
```

with:

```ts
          Speichern ({formatBindingForDisplay(getKeymap().save)})
```

- [ ] **Step 3: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed.

- [ ] **Step 4: Manual verification in the Browser tool**

Start the dev server, navigate to it, open the Command Palette (Ctrl+K) and select "Neuer Eintrag (im Hauptfenster)" to open the entry editor; confirm via `find` that a save-related element is present, then press Ctrl+S (`computer` key action) and confirm via `read_console_messages`/`read_network_requests` that a `create_entry`/`update_entry` invoke fired (or at minimum no thrown JS error) rather than the browser's own save-page dialog appearing. Stop the preview server after.

- [ ] **Step 5: Commit**

```bash
git add src/components/EntryEditor.tsx src/quick-capture/QuickCapture.tsx
git commit -m "refactor: read Ctrl+S save binding from the keymap"
```

---

### Task 7: Refactor list navigation in the 3 main views

**Files:**
- Modify: `src/components/CustomerListView.tsx:44-63`
- Modify: `src/components/SystemListView.tsx:66-78`
- Modify: `src/components/JournalView.tsx:152-170`

**Interfaces:**
- Consumes: `getKeymap`, `matchesBinding` from `src/lib/keymap.ts` (Task 3).

- [ ] **Step 1: `CustomerListView.tsx`**

Add to the imports (after `import { isTypingTarget } from "../hooks/useGlobalHotkeys";`):

```ts
import { getKeymap, matchesBinding } from "../lib/keymap";
```

Replace the whole `onKeyDown` function body:

```ts
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, customers.length - 1));
      } else if (matchesBinding(e, keymap.list_prev)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          selectCustomer(customer.id);
          goToSystems(customer.id);
        }
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          openCustomerEditor(customer.id);
        }
      }
    }
```

(`Enter` is intentionally left as a literal `e.key === "Enter"` check — Enter is excluded from remapping, see Global Constraints.)

- [ ] **Step 2: `SystemListView.tsx`**

Same import addition. Replace the `onKeyDown` body:

```ts
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, systems.length - 1));
      } else if (matchesBinding(e, keymap.list_prev)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const system = systems[selectedIndex];
        if (system && selectedCustomerId !== null) {
          e.preventDefault();
          openSystemEditor(system.id, selectedCustomerId);
        }
      }
    }
```

- [ ] **Step 3: `JournalView.tsx`**

Same import addition (into whatever the existing import block is — add `import { getKeymap, matchesBinding } from "../lib/keymap";` near the top). Replace the `onKeyDown` body:

```ts
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, entries.length - 1));
      } else if (matchesBinding(e, keymap.list_prev)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const entry = entries[selectedIndex];
        if (entry) {
          e.preventDefault();
          openEntryEditor(entry.id);
        }
      } else if (e.key === "Enter") {
        const entry = entries[selectedIndex];
        if (entry) {
          e.preventDefault();
          openEntryDetail(entry.id);
        }
      }
    }
```

- [ ] **Step 4: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed.

- [ ] **Step 5: Manual verification in the Browser tool**

Start the dev server, navigate to it (Kunden view), and if any customers exist, press `j`/`k` (`computer` key actions) and confirm the selected row's highlight moves (screenshot before/after, or `read_page` to inspect the highlighted `<li>`'s style). If no customers exist in this dev DB, skip the visual check and rely on Steps 1-4 (build+lint) — note that in the plan's completion notes. Stop the preview server after.

- [ ] **Step 6: Commit**

```bash
git add src/components/CustomerListView.tsx src/components/SystemListView.tsx src/components/JournalView.tsx
git commit -m "refactor: read j/k/e list navigation from the keymap in the 3 main views"
```

---

### Task 8: Refactor list navigation in the 8 plugin sections

**Files:**
- Modify: `src/components/NinjaPluginSection.tsx:867-872`
- Modify: `src/components/LevelPluginSection.tsx:676-681`
- Modify: `src/components/SnipeitPluginSection.tsx:919-924`
- Modify: `src/components/IntunePluginSection.tsx:567-572`
- Modify: `src/components/IruPluginSection.tsx:550-555`
- Modify: `src/components/JamfPluginSection.tsx:943-948`
- Modify: `src/components/AbmPluginSection.tsx:569-574`
- Modify: `src/components/TacticalRmmPluginSection.tsx:930-935`

**Interfaces:**
- Consumes: `getKeymap`, `matchesBinding` from `src/lib/keymap.ts` (Task 3).

All 8 files share the exact same 6-line pattern (only the `j`/`k`/`ArrowDown`/`ArrowUp` block; the `Enter`/`l`/`u` lines right after are untouched and excluded from this feature). For **each** of the 8 files:

- [ ] **Step 1: Add the import**

These files live in `src/components/`, so the correct relative path to `src/lib/keymap.ts` is `"../lib/keymap"`. Add:

```ts
import { getKeymap, matchesBinding } from "../lib/keymap";
```

placed alongside that file's other relative imports (each of these 8 files already imports from `"../lib/..."` or `"./..."` for other things — put it near the top import block, exact line doesn't matter as long as it's a top-level import).

- [ ] **Step 2: Replace the j/k check**

Replace (the two conditions differ only in which local `setSelectedIndexByGroup`/index variables are in scope, per file — the `j`/`k` half of the condition is identical across all 8):

```ts
      if (e.key === "j" || e.key === "ArrowDown") {
```

with:

```ts
      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next) || e.key === "ArrowDown") {
```

and:

```ts
      } else if (e.key === "k" || e.key === "ArrowUp") {
```

with:

```ts
      } else if (matchesBinding(e, keymap.list_prev) || e.key === "ArrowUp") {
```

(`ArrowDown`/`ArrowUp` stay as literal fallbacks in every file, per the spec — only the `j`/`k` half is keymap-driven. Do not touch the `Enter`/`l`/`u` branches that follow.)

- [ ] **Step 3: Repeat Steps 1-2 for all 8 files listed above**

Each file's `j`/`k` block is structurally identical to the snippet shown; locate it via `grep -n 'e.key === "j" || e.key === "ArrowDown"'` in that file if the line numbers above have drifted since this plan was written.

- [ ] **Step 4: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed, across all 8 files.

- [ ] **Step 5: Manual verification in the Browser tool**

Start the dev server, navigate to Einstellungen → Plugins, open any one plugin section that has at least one configured connection with cached devices (if none exist in this dev DB, skip the visual check and rely on Step 4 — note that in the completion notes), press `j`/`k` and confirm the selected device row's highlight moves. One section is representative of all 8 given the identical mechanical change; no need to test all 8 individually. Stop the preview server after.

- [ ] **Step 6: Commit**

```bash
git add src/components/NinjaPluginSection.tsx src/components/LevelPluginSection.tsx src/components/SnipeitPluginSection.tsx src/components/IntunePluginSection.tsx src/components/IruPluginSection.tsx src/components/JamfPluginSection.tsx src/components/AbmPluginSection.tsx src/components/TacticalRmmPluginSection.tsx
git commit -m "refactor: read j/k list navigation from the keymap in all 8 plugin sections"
```

---

### Task 9: Refactor display labels (`App.tsx`, `ShortcutOverview.tsx`)

**Files:**
- Modify: `src/App.tsx:14`, `:92`
- Modify: `src/components/ShortcutOverview.tsx`

**Interfaces:**
- Consumes: `getKeymap`, `formatBindingForDisplay` from `src/lib/keymap.ts` (Task 3); `formatShortcut` from `src/lib/platform.ts` stays for the one Ctrl+V row.

- [ ] **Step 1: `App.tsx`**

Replace:

```ts
import { formatShortcut } from "./lib/platform";
```

with:

```ts
import { getKeymap, formatBindingForDisplay } from "./lib/keymap";
```

Replace:

```tsx
          <span>{formatShortcut("K")} · Befehle</span>
```

with:

```tsx
          <span>{formatBindingForDisplay(getKeymap().command_palette)} · Befehle</span>
```

- [ ] **Step 2: `ShortcutOverview.tsx`**

Add the new import alongside the existing `import { formatShortcut } from "../lib/platform";` (both stay — `formatShortcut` is still used for the Ctrl+V row):

```ts
import { getKeymap, formatBindingForDisplay } from "../lib/keymap";
```

Replace the whole `shortcuts()` function body:

```ts
function shortcuts(): ShortcutRow[] {
  const keymap = getKeymap();
  return [
    { keys: formatBindingForDisplay(keymap.command_palette), description: "Command Palette öffnen" },
    {
      keys: formatBindingForDisplay(keymap.quick_capture),
      description: `Schnellerfassungsfenster öffnen — für einen Eintrag im Hauptfenster: ${formatBindingForDisplay(keymap.command_palette)} → „Neuer Eintrag“`,
    },
    { keys: formatBindingForDisplay(keymap.save), description: "Speichern (im geöffneten Editor)" },
    { keys: "/", description: "Noch nicht gebunden" },
    { keys: keymap.goto_customers, description: "Zu Kundenliste" },
    { keys: keymap.goto_systems, description: "Zu Systemliste des aktuellen Kunden" },
    { keys: keymap.goto_journal, description: "Zum Journal" },
    { keys: `${keymap.list_next} / ${keymap.list_prev}`, description: "Liste abwärts / aufwärts" },
    { keys: "Enter", description: "Auswählen/öffnen — wirkt je nach Ansicht unterschiedlich (siehe SHORTCUTS.md)" },
    { keys: keymap.edit_selected, description: "Ausgewählten Eintrag/Kunde/System bearbeiten" },
    { keys: formatShortcut("V"), description: "Screenshot aus Zwischenablage als Anhang" },
    { keys: "Esc", description: "Abbrechen, Ebene zurück" },
    { keys: "?", description: "Diese Shortcut-Übersicht" },
  ];
}
```

Then add a note below the table in the render output. Replace:

```tsx
      </table>
    </Modal>
```

with:

```tsx
      </table>
      <p style={{ margin: "0.75rem 0 0", fontSize: "0.78rem", color: "var(--text-muted)" }}>
        Anpassbar unter Einstellungen → Tastaturbelegung.
      </p>
    </Modal>
```

- [ ] **Step 3: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed.

- [ ] **Step 4: Manual verification in the Browser tool**

Start the dev server, navigate to it, press `?` (`computer` key action, same approach as verified earlier this session — a plain "shift+slash" key combo did not register, `key` action with the literal text `"?"` did), confirm via `read_page` that the table shows "Strg+K"/"Strg+N"/"Strg+S" and the new footer note, and that the sidebar footer (`find` for "· Befehle") also reads "Strg+K". Stop the preview server after.

- [ ] **Step 5: Commit**

```bash
git add src/App.tsx src/components/ShortcutOverview.tsx
git commit -m "refactor: read shortcut display labels from the keymap"
```

---

### Task 10: New "Tastaturbelegung" Settings tab (wiring only)

**Files:**
- Modify: `src/state/appStore.ts:4` (extend `SettingsTab`)
- Modify: `src/components/SettingsView.tsx`

**Interfaces:**
- Consumes: `KeymapSettingsView` default export (Task 11, created next — this task can be done first since it only adds the tab plumbing; the import will be unresolved until Task 11 lands, so do Task 10 and Task 11 as one combined commit if working through this plan sequentially, or stub `KeymapSettingsView.tsx` with a one-line placeholder component here and let Task 11 replace it).

- [ ] **Step 1: Extend `SettingsTab`**

In `src/state/appStore.ts`, replace:

```ts
export type SettingsTab = "general" | "backup" | "plugins";
```

with:

```ts
export type SettingsTab = "general" | "backup" | "plugins" | "keymap";
```

- [ ] **Step 2: Add the tab to `SettingsView.tsx`**

Add the import:

```ts
import KeymapSettingsView from "./KeymapSettingsView";
```

Add a new `TabButton` after the existing "Plugins" one:

```tsx
        <TabButton active={settingsTab === "keymap"} onClick={() => setSettingsTab("keymap")}>
          Tastaturbelegung
        </TabButton>
```

Add a new conditional render after the existing `{settingsTab === "plugins" && <PluginsView />}`:

```tsx
      {settingsTab === "keymap" && <KeymapSettingsView />}
```

- [ ] **Step 3: Create a temporary stub so the build passes**

Create `src/components/KeymapSettingsView.tsx` with:

```tsx
export default function KeymapSettingsView() {
  return <div>Tastaturbelegung (folgt in Task 11)</div>;
}
```

(Task 11 replaces this file's entire contents with the real implementation — this stub only exists so Tasks 10 and 11 can be reviewed/committed independently per this plan's task boundaries.)

- [ ] **Step 4: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed.

- [ ] **Step 5: Manual verification in the Browser tool**

Start the dev server, navigate to Einstellungen, click "Tastaturbelegung", confirm the stub text renders. Stop the preview server after.

- [ ] **Step 6: Commit**

```bash
git add src/state/appStore.ts src/components/SettingsView.tsx src/components/KeymapSettingsView.tsx
git commit -m "feat: add Tastaturbelegung tab to Settings (stub content)"
```

---

### Task 11: `KeyRecorder` component

**Files:**
- Create: `src/components/KeyRecorder.tsx`

**Interfaces:**
- Consumes: `formatBinding` from `src/lib/keymap.ts` (Task 3).
- Produces: `export default function KeyRecorder({ value, mode, onChange }: { value: string; mode: "combo" | "sequence"; onChange: (value: string) => void })`. Task 12 mounts one `KeyRecorder` per row.

- [ ] **Step 1: Create the file**

```tsx
import { useEffect, useState } from "react";
import { formatBinding } from "../lib/keymap";

const IGNORED_KEYS = ["Control", "Shift", "Alt", "Meta"];

interface KeyRecorderProps {
  value: string;
  // "combo": records one keydown (modifiers + key) via formatBinding, e.g.
  // "Ctrl+Shift+K". "sequence": records two consecutive keydowns with no
  // modifiers held on either, joined by a space, e.g. "g c" — mirrors
  // useGlobalHotkeys.ts's actual "prefix key, then follow-up key" mechanism
  // for goto_customers/goto_systems/goto_journal.
  mode: "combo" | "sequence";
  onChange: (value: string) => void;
}

export default function KeyRecorder({ value, mode, onChange }: KeyRecorderProps) {
  const [recording, setRecording] = useState(false);
  const [pendingFirst, setPendingFirst] = useState<string | null>(null);

  useEffect(() => {
    if (!recording) return;

    function onKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
        setRecording(false);
        setPendingFirst(null);
        return;
      }
      if (IGNORED_KEYS.includes(e.key)) return;

      if (mode === "combo") {
        onChange(formatBinding(e));
        setRecording(false);
        return;
      }

      if (pendingFirst === null) {
        setPendingFirst(e.key.toLowerCase());
        return;
      }
      onChange(`${pendingFirst} ${e.key.toLowerCase()}`);
      setRecording(false);
      setPendingFirst(null);
    }

    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [recording, mode, pendingFirst, onChange]);

  return (
    <button
      type="button"
      onClick={() => {
        setRecording(true);
        setPendingFirst(null);
      }}
      style={{
        fontFamily: "var(--font-mono)",
        minWidth: "9rem",
        borderColor: recording ? "var(--accent)" : undefined,
      }}
    >
      {recording ? (pendingFirst ? `${pendingFirst} …` : "Taste(n) drücken…") : value}
    </button>
  );
}
```

- [ ] **Step 2: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed (this file isn't imported anywhere yet, so a successful build only confirms it type-checks in isolation — Task 12 exercises it for real).

- [ ] **Step 3: Commit**

```bash
git add src/components/KeyRecorder.tsx
git commit -m "feat: add KeyRecorder, the shortcut-capture control for Settings"
```

---

### Task 12: `KeymapSettingsView.tsx` — the real implementation

**Files:**
- Modify: `src/components/KeymapSettingsView.tsx` (replaces Task 10's stub entirely)

**Interfaces:**
- Consumes: `KeyRecorder` (Task 11); `Keymap`, `defaultKeymap`, `formatBindingForDisplay` from `src/lib/keymap.ts` (Task 3); `formatInvokeError` from `src/lib/errors.ts` (existing, same as `GeneralSettingsView.tsx` uses); backend commands `get_keymap`/`set_keymap`/`get_hotkeys`/`set_hotkeys` (Task 2).

- [ ] **Step 1: Replace the stub with the real component**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";
import { defaultKeymap, type Keymap } from "../lib/keymap";
import KeyRecorder from "./KeyRecorder";

// Kept in sync with src-tauri/src/config.rs::HotkeyConfig field-for-field.
interface HotkeyConfig {
  quick_capture: string;
  search: string;
  clipboard_screenshot: string;
}

const DEFAULT_HOTKEYS: HotkeyConfig = {
  quick_capture: "Ctrl+Alt+Space",
  search: "Ctrl+Alt+F",
  clipboard_screenshot: "Ctrl+Alt+S",
};

interface KeymapRow {
  key: keyof Keymap;
  label: string;
  mode: "combo" | "sequence";
}

const KEYMAP_ROWS: KeymapRow[] = [
  { key: "command_palette", label: "Command Palette öffnen", mode: "combo" },
  { key: "quick_capture", label: "Schnellerfassung öffnen", mode: "combo" },
  { key: "save", label: "Speichern", mode: "combo" },
  { key: "goto_customers", label: "Zu Kundenliste", mode: "sequence" },
  { key: "goto_systems", label: "Zu Systemliste", mode: "sequence" },
  { key: "goto_journal", label: "Zum Journal", mode: "sequence" },
  { key: "list_next", label: "Liste: nächster Eintrag", mode: "combo" },
  { key: "list_prev", label: "Liste: vorheriger Eintrag", mode: "combo" },
  { key: "edit_selected", label: "Ausgewähltes bearbeiten", mode: "combo" },
];

interface HotkeyRow {
  key: keyof HotkeyConfig;
  label: string;
}

const HOTKEY_ROWS: HotkeyRow[] = [
  { key: "quick_capture", label: "Schnellerfassung öffnen" },
  { key: "search", label: "Fenster anzeigen / Suche" },
  { key: "clipboard_screenshot", label: "Zwischenablage-Screenshot" },
];

export default function KeymapSettingsView() {
  const [keymap, setKeymap] = useState<Keymap | null>(null);
  const [hotkeys, setHotkeysState] = useState<HotkeyConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([invoke<Keymap>("get_keymap"), invoke<HotkeyConfig>("get_hotkeys")])
      .then(([loadedKeymap, loadedHotkeys]) => {
        if (cancelled) return;
        setKeymap(loadedKeymap);
        setHotkeysState(loadedHotkeys);
      })
      .catch((e) => {
        if (!cancelled) setError(formatInvokeError(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function saveKeymap() {
    if (keymap === null) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      await invoke("set_keymap", { keymap });
      setStatus("Gespeichert.");
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  async function saveHotkeys() {
    if (hotkeys === null) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      await invoke("set_hotkeys", { hotkeys });
      setStatus("Gespeichert. Wirkt erst nach einem Neustart der Anwendung.");
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  function resetToDefaults() {
    setKeymap(defaultKeymap());
    setHotkeysState(DEFAULT_HOTKEYS);
    setStatus(null);
    setError(null);
  }

  if (keymap === null || hotkeys === null) {
    return (
      <div>
        <h1 style={{ fontSize: "1.1rem" }}>Tastaturbelegung</h1>
        {error ? (
          <p style={{ color: "var(--danger)", fontSize: "0.85rem" }}>Fehler: {error}</p>
        ) : (
          <p style={{ color: "var(--text-muted)" }}>Lädt…</p>
        )}
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "36rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Tastaturbelegung</h1>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Global (systemweit)</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Wirkt auch, wenn Wartungsdoku keinen Fokus hat. Eine Änderung hier wirkt erst nach einem Neustart der Anwendung.
        </p>
        <table style={{ borderCollapse: "collapse", marginTop: "0.25rem" }}>
          <tbody>
            {HOTKEY_ROWS.map((row) => (
              <tr key={row.key}>
                <td style={{ padding: "0.3rem 0.75rem 0.3rem 0", fontSize: "0.85rem" }}>{row.label}</td>
                <td style={{ padding: "0.3rem 0" }}>
                  <KeyRecorder
                    value={hotkeys[row.key]}
                    mode="combo"
                    onChange={(newValue) => setHotkeysState({ ...hotkeys, [row.key]: newValue })}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div>
          <button type="button" className="btn-primary" disabled={busy} onClick={() => void saveHotkeys()}>
            Speichern
          </button>
        </div>
      </section>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>App</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Wirkt nur, während Wartungsdoku im Vordergrund ist. Änderungen hier wirken sofort.
        </p>
        <table style={{ borderCollapse: "collapse", marginTop: "0.25rem" }}>
          <tbody>
            {KEYMAP_ROWS.map((row) => (
              <tr key={row.key}>
                <td style={{ padding: "0.3rem 0.75rem 0.3rem 0", fontSize: "0.85rem" }}>{row.label}</td>
                <td style={{ padding: "0.3rem 0" }}>
                  <KeyRecorder
                    value={keymap[row.key]}
                    mode={row.mode}
                    onChange={(newValue) => setKeymap({ ...keymap, [row.key]: newValue })}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div>
          <button type="button" className="btn-primary" disabled={busy} onClick={() => void saveKeymap()}>
            Speichern
          </button>
        </div>
      </section>

      <div>
        <button type="button" disabled={busy} onClick={resetToDefaults}>
          Auf Standard zurücksetzen
        </button>
        <p style={{ margin: "0.3rem 0 0", fontSize: "0.78rem", color: "var(--text-muted)" }}>
          Setzt beide Tabellen oben auf ihre Standardwerte zurück — erst mit „Speichern“ wird das übernommen.
        </p>
      </div>

      {status && <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{status}</p>}
      {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
    </div>
  );
}
```

- [ ] **Step 2: Verify types and lint**

Run: `npm run build 2>&1 | tail -20 && npm run lint 2>&1 | tail -20`
Expected: both succeed.

- [ ] **Step 3: Manual verification in the Browser tool**

Start the dev server, navigate to Einstellungen → Tastaturbelegung. Confirm via `read_page` both tables render with the current bindings (e.g. "Ctrl+K" for Command Palette on this Windows dev machine, "g c" for Zu Kundenliste). Click the `list_next` row's recorder button, press `n` (`computer` key action), confirm the button now shows "n". Click "Speichern" under the App section, confirm the success message appears. Reload the page and confirm "n" persisted (re-fetches from the backend). Then click "Auf Standard zurücksetzen", confirm the App table's `list_next` row shows "j" again, click "Speichern" again to restore the real default (leave the dev config in its default state when done — don't leave `list_next` as `n` for later tasks' manual checks). Also test the conflict path: set two rows to the same value (e.g. type into two recorders the same key) and confirm "Speichern" shows the backend's conflict error message. Stop the preview server after.

**Caveat, not a step to attempt:** the spec's Testing section also calls for confirming a rebound `save` binding fires live in an already-open quick capture window without a restart (the `"keymap-changed"` event, Task 2/3). That cannot be checked through this Browser-tool dev preview — `invoke`/`listen` need the real Tauri IPC bridge, unavailable in a plain `vite dev` page (every prior manual-verification step in this plan that touches `invoke` has the same ceiling, e.g. Task 6 only confirms the call fires, not its real effect). Verify this specific cross-window behavior once, manually, in a real `cargo tauri dev`/built app: open quick capture, rebind `save` in the main window's Settings, confirm quick capture's Save button label updates and Ctrl/Cmd+&lt;newkey&gt; saves there without restarting either window.

- [ ] **Step 4: Commit**

```bash
git add src/components/KeymapSettingsView.tsx
git commit -m "feat: implement the Tastaturbelegung settings view"
```

---

### Task 13: Documentation

**Files:**
- Modify: `SHORTCUTS.md`

- [ ] **Step 1: Add a customization note near the top**

Right after the existing intro paragraph (the one ending "...that is called out explicitly below."), add a new paragraph:

```markdown
As of the "Tastaturbelegung" Settings tab, every binding in the "Main
window", "Command Palette", and the `j`/`k`/`e` rows in the per-view
tables below is user-customizable (Settings → Tastaturbelegung); the 3
global hotkeys are too, though a change there needs an app restart to
take effect (`src-tauri/src/commands/keymap.rs::set_hotkeys`). What
follows documents the **defaults** each ships with, not a fixed contract —
check Settings → Tastaturbelegung for what's actually bound on a given
installation. `Ctrl+V`/`Cmd+V`, `Enter`, and `Esc` are the three
exceptions: they stay fixed (see the design spec's Scope section,
`docs/superpowers/specs/2026-09-10-keyboard-shortcut-settings-design.md`,
for why).
```

- [ ] **Step 2: Update the macOS platform note**

Replace the existing macOS bullet (added in commit `01e2e6b`):

```markdown
- **macOS:** every `Ctrl+<letter>` binding in this document (`Ctrl+K`,
  `Ctrl+N`, `Ctrl+S`, `Ctrl+V`) also responds to `Cmd+<letter>` — the
  in-app UI (sidebar footer, Command Palette, quick capture's Save button,
  the `?` shortcut overview) displays `⌘` there instead of `Strg+`
  (`src/lib/platform.ts`). The three *global* hotkeys in the table above
  stay literally `Ctrl+Alt+...` on macOS too, since they are user-editable
  strings in `config.toml` rather than a fixed binding — set them to
  `Cmd+Alt+...` there if you want the Mac-native combination. Window
  context capture is unimplemented on macOS (falls back to
  `context_capture::unsupported`); quick capture itself is unaffected.
```

with:

```markdown
- **macOS:** `Ctrl+K`/`Ctrl+N`/`Ctrl+S` default to `Cmd+K`/`Cmd+N`/`Cmd+S`
  there instead (`KeymapConfig::default()` picks the platform-appropriate
  literal; matching is exact, not "Ctrl also matches Cmd" — see the design
  spec's self-review note). `Ctrl+V` stays a fixed platform convention and
  responds to `Cmd+V` natively (it's the browser's own paste event, not a
  binding this app checks). The three *global* hotkeys stay literally
  `Ctrl+Alt+...` by default on macOS too — rebind them to `Cmd+Alt+...` in
  Settings → Tastaturbelegung (or `config.toml` directly) if you want the
  Mac-native combination; either way a global hotkey change needs an app
  restart. Window context capture is unimplemented on macOS (falls back to
  `context_capture::unsupported`); quick capture itself is unaffected.
```

- [ ] **Step 3: Commit**

```bash
git add SHORTCUTS.md
git commit -m "docs: document the new Tastaturbelegung settings and its defaults-only framing"
```
