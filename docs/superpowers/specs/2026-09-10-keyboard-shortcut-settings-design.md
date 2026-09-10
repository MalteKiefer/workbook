# Keyboard Shortcut Settings Design

## Goal

Add a "Tastaturbelegung" (keyboard bindings) section to Settings where the
user can view and change every keyboard shortcut this app defines, instead
of the current split: the 3 global OS-level hotkeys are already
string-configurable in `config.toml` but have no UI, and every other
shortcut is a literal hardcoded in a keydown handler somewhere in the
frontend.

## Motivation

Came out of a macOS-support session (`docs/superpowers/specs/` sibling work
is unrelated; see recent commits `01e2e6b`, `5344a68`). While fixing
Ctrl-vs-Cmd handling, it became clear there is no user-facing way to
customize any shortcut at all — a real gap for a keyboard-first app whose
own README claims exactly that positioning, and doubly so once different
platforms (Windows/Linux Ctrl vs. macOS Cmd, laptop keyboards missing keys,
personal preference) are in the picture.

## Scope

**In scope — becomes user-editable in Settings → Tastaturbelegung:**

| Section | Action | Current default | Where it lives today |
|---|---|---|---|
| Global (OS-wide) | Quick Capture öffnen | `Ctrl+Alt+Space` | `config.rs::HotkeyConfig.quick_capture` |
| Global (OS-wide) | Fenster anzeigen / Suche | `Ctrl+Alt+F` | `config.rs::HotkeyConfig.search` |
| Global (OS-wide) | Zwischenablage-Screenshot | `Ctrl+Alt+S` | `config.rs::HotkeyConfig.clipboard_screenshot` |
| App | Command Palette öffnen | `Ctrl+K` / `Cmd+K` | `CommandPalette.tsx` (already Cmd-aware) |
| App | Schnellerfassung öffnen | `Ctrl+N` / `Cmd+N` | `useGlobalHotkeys.ts` |
| App | Speichern (Editor, Quick Capture) | `Ctrl+S` / `Cmd+S` | `EntryEditor.tsx`, `QuickCapture.tsx` |
| App | Zu Kundenliste | `g` then `c` | `useGlobalHotkeys.ts` |
| App | Zu Systemliste | `g` then `s` | `useGlobalHotkeys.ts` |
| App | Zum Journal | `g` then `j` | `useGlobalHotkeys.ts` |
| App | Liste: nächster Eintrag | `j` | 11 files, see below |
| App | Liste: vorheriger Eintrag | `k` | 11 files, see below |
| App | Ausgewähltes bearbeiten | `e` | `CustomerListView.tsx`, `SystemListView.tsx`, `JournalView.tsx` |

`list_next`/`list_prev` currently exist as separately hardcoded `j`/`k`
(plus an `ArrowDown`/`ArrowUp` fallback that stays hardcoded, unaffected by
remapping) in: `CustomerListView.tsx`, `SystemListView.tsx`,
`JournalView.tsx`, and all 8 plugin device-list sections (`NinjaPluginSection.tsx`,
`LevelPluginSection.tsx`, `SnipeitPluginSection.tsx`, `IntunePluginSection.tsx`,
`IruPluginSection.tsx`, `JamfPluginSection.tsx`, `AbmPluginSection.tsx`,
`TacticalRmmPluginSection.tsx`). All 11 read from the same shared binding so
a remap is consistent everywhere `j`/`k` means "move selection" — leaving
some of them on the old hardcoded key while others use the new one would be
a worse, inconsistent result than not shipping this at all.

**Out of scope, explicitly excluded, shown read-only (with a short reason)
in the new Settings section:**

- **Strg+V / Cmd+V (Screenshot einfügen).** This is the browser's native
  `paste` event (`onPaste` in `EntryEditor.tsx` / `QuickCapture.tsx`), not a
  keydown comparison — the OS itself decides which physical keys produce a
  paste event, and every platform already does the right thing (Ctrl+V on
  Windows/Linux, Cmd+V on macOS) with zero code on our side. Remapping it to
  an arbitrary combination would mean abandoning the native paste event for
  a manual `navigator.clipboard.read()` call gated behind its own permission
  prompt, with materially less reliable cross-platform image support. Not
  worth it for a binding nobody has actually asked to change.
- **Enter, Esc.** Strong, near-universal platform conventions (confirm/open,
  cancel/back) with meaning that already varies by context (Enter opens a
  customer's system list in one view, opens a read-only detail view in
  another). Remapping either invites more confusion than it removes.
- **Plugin section's `l`/`u`** (compare/take-over in the external-system
  comparison panel) and the ArrowUp/ArrowDown fallback for list navigation:
  unrelated to this feature, untouched.

## Data model (`src-tauri/src/config.rs`)

`HotkeyConfig` (existing, unchanged shape) stays for the 3 global hotkeys.

New:

```rust
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
```

Defaults reproduce today's hardcoded behavior exactly: `"g c"`, `"g s"`,
`"g j"`, `"j"`, `"k"`, `"e"` on every platform, and for the three
Ctrl/Cmd-bearing actions, `KeymapConfig::default()` branches on
`cfg!(target_os = "macos")` to seed `"Cmd+K"`/`"Cmd+N"`/`"Cmd+S"` there and
`"Ctrl+K"`/`"Ctrl+N"`/`"Ctrl+S"` everywhere else. This matters: matching is
**exact** (see `matchesBinding` below, no "Ctrl also matches Cmd"
convenience aliasing) — today's already-shipped Cmd+K/N/S support on macOS
(`01e2e6b`) worked by special-casing "ctrlKey or metaKey" in each of three
separate files, not because the string "Ctrl" has ever meant "Ctrl or Cmd".
Carrying that dual-match into a user-facing, user-editable binding would be
actively confusing (rebind to "Ctrl+Shift+K" on a Mac and have Cmd+Shift+K
silently also work?) — so the platform-appropriate literal is the default
instead, and a rebind means exactly what it says. `goto_*` are stored as
the two-key sequence `"g c"` (space-separated) since that is how the app
already thinks of them (a `g` prefix with an 800ms timeout, then a second
key) — the recorder UI for these three rows records two keypresses instead
of one combination (see below).

Added to `Config` as `pub keymap: KeymapConfig`, `#[serde(default)]`
already covers backward compatibility with every existing `config.toml`.

## Backend

New file `src-tauri/src/commands/keymap.rs`, same shape as
`commands/settings.rs::get_theme_preference`/`set_theme_preference`:

- `get_keymap(state) -> KeymapConfig`
- `set_keymap(state, app, keymap: KeymapConfig) -> Result<(), AppError>` —
  validates every binding string is non-empty and that no two of the 9
  `KeymapConfig` actions share an identical binding (`AppError::Config`
  with a message naming both conflicting actions otherwise; conflicts are
  not checked against `HotkeyConfig`'s 3 global bindings — those fire
  through the OS accelerator layer even when the app isn't focused, a
  materially different dispatch path from the in-page keydown checks here,
  so a literal string collision between the two isn't a real conflict),
  then persists to `config.toml` and emits
  `"keymap-changed"` (mirrors `"theme-changed"`) so an already-open Quick
  Capture window picks up a `save` rebind live without a restart.
- `get_hotkeys(state) -> HotkeyConfig` / `set_hotkeys(state, hotkeys) ->
  Result<(), AppError>` — same persistence, but explicitly documented (both
  in a doc comment and in the Settings UI copy) as **taking effect after
  restarting the app**. Re-registering OS-level global shortcuts at runtime
  means tearing down and rebuilding `tauri_plugin_global_shortcut`'s
  handler closure (`src-tauri/src/hotkeys.rs` currently captures the parsed
  `Shortcut`s by value at plugin-build time in `lib.rs`'s setup hook) —
  doable, but a meaningfully riskier change for a rarely-touched setting.
  Not worth it for v1; revisit if restart-to-apply proves annoying in
  practice.

No change to `hotkeys.rs` itself in this pass beyond what `set_hotkeys`
needs (just persistence).

## Frontend

New `src/lib/keymap.ts`:

```ts
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

// Parses a stored binding like "Ctrl+K", "Cmd+Shift+K", or "e" and reports
// whether `e` matches it. Exact match, no aliasing: "Ctrl" means
// e.ctrlKey, "Cmd"/"Meta" means e.metaKey, "Alt"/"Shift" as expected. The
// default keymap (see KeymapConfig::default() above) already seeds the
// platform-correct literal, so this needs no macOS special-casing itself.
export function matchesBinding(e: KeyboardEvent, binding: string): boolean;

// Formats a KeyboardEvent's modifiers + key back into the same shape,
// e.g. a KeyRecorder capturing Cmd+Shift+K on macOS produces "Cmd+Shift+K".
// Used only by the recorder UI; matchesBinding and this function must stay
// exact inverses of each other for every combination the recorder can
// produce.
export function formatBinding(e: KeyboardEvent): string;
```

New Zustand slice (or a small dedicated store, matching `useAppStore`'s
existing single-store convention) holding the loaded `Keymap`, fetched once
via `get_keymap` on app start (`App.tsx`, alongside other startup fetches)
and updated in place on a `"keymap-changed"` event listener (mirrors
`src/lib/theme.ts::listenForThemeChanges`).

Refactor every hardcoded check listed in the Scope table to call
`matchesBinding(e, keymap.<action>)` instead of a literal comparison. The
`g`-prefix sequences in `useGlobalHotkeys.ts` need the prefix key and
follow-up key split out of the stored `"g c"` string rather than a literal
`"g"`/`"c"` comparison, but otherwise keep today's 800ms pending-prefix
mechanism unchanged.

## New Settings UI (`src/components/KeymapSettingsView.tsx`)

New tab in `SettingsView.tsx` ("Tastaturbelegung", after "Plugins"),
`SettingsTab` extended to `"general" | "backup" | "plugins" | "keymap"` in
`src/state/appStore.ts`.

Layout: two grouped tables (Global / App), one row per action — label,
then a `KeyRecorder` control showing the current binding as text.

`KeyRecorder`: a button; clicking it puts it into "recording" state
(visibly, e.g. border color + "Taste(n) drücken…" placeholder) and attaches
a one-shot keydown listener capturing the pressed combination (modifiers +
key), formatted back into the same `"Ctrl+Shift+K"` string shape the
backend expects. For a `goto_*` row (two-key sequence), the recorder
prompts for two consecutive presses instead of one combination. Esc while
recording cancels without changing the row's value (Esc itself is
intentionally never a recordable target, see Scope).

Below the tables: inline validation message (conflict / empty binding) and
"Speichern" (calls `set_keymap`/`set_hotkeys` as needed, only for the
section that actually changed) / "Auf Standard zurücksetzen" (repopulates
the form from the hardcoded defaults above, still requires Speichern to
persist). A small note under the Global table: "Diese drei Kürzel wirken
erst nach einem Neustart der Anwendung."

The excluded rows (Strg+V, Enter, Esc) are NOT shown in this new section at
all — they stay exactly where they already are today, in the existing "?"
`ShortcutOverview.tsx` compact reference and `SHORTCUTS.md`, both of which
get a one-line pointer added ("Anpassbare Kürzel: Einstellungen →
Tastaturbelegung").

## Testing

- Rust: `config.rs` roundtrip test for `KeymapConfig` (mirrors every
  existing `#[serde(default)]` config-field test in that file — see
  `TacticalRmmConnectionMeta`'s tests for the exact pattern to copy),
  `commands/keymap.rs` conflict-detection unit test.
- Frontend: `matchesBinding`/`formatBinding` unit tests — exact modifier
  matching (a stored "Ctrl+K" must NOT match a Cmd+K event or vice versa),
  multi-modifier combos, case-insensitivity of the letter itself, and that
  the two functions round-trip (`matchesBinding(e, formatBinding(e))` is
  always true).
- Manual: rebind `save` to something unused, confirm it fires in both the
  entry editor and quick capture without a restart; rebind a global hotkey,
  confirm the Settings copy's restart note, confirm restarting picks it up.

## Explicitly not doing

- Live re-registration of the 3 global OS hotkeys without a restart.
- Making Strg+V, Enter, or Esc remappable.
- Per-view-independent `j`/`k`/`e` (e.g. a different `j` in the journal than
  in the customer list) — one binding, applied everywhere the concept
  exists today.
- Import/export of a keymap as a shareable file (nothing asked for this;
  `config.toml` itself is already the persisted, backed-up copy via the
  existing full-config backup).
