# Keyboard Shortcuts: Wartungsdoku

Wartungsdoku is designed for full keyboard operation. This file lists the
**actually implemented** shortcuts, grouped by the area where they are
active, verified against the current source code (not just the design
spec, see `docs/superpowers/specs/2026-09-07-wartungsdoku-design.md`).
Where the implementation and the spec draft diverge, that is called out
explicitly below.

## Global hotkeys (system wide)

Active across the whole system, even while Wartungsdoku has no focus or
sits only in the tray. Registered via `tauri-plugin-global-shortcut` in
`src-tauri/src/hotkeys.rs`, defaults from `HotkeyConfig` in
`src-tauri/src/config.rs`. **All three can be freely changed in
`config.toml`** (the `[hotkeys]` section).

| Key (default) | Effect |
|---|---|
| `Ctrl+Alt+Space` | Open the quick capture window |
| `Ctrl+Alt+F` | Show and focus the main window. **Note:** this currently opens no dedicated search UI of its own, it just shows the main window (`window::show_and_focus_main`). Full text search happens there via the Command Palette (`Ctrl+K`). |
| `Ctrl+Alt+S` | Read an image from the clipboard and open quick capture with it already attached as a screenshot |

See also [Platform notes](#platform-notes): these three hotkeys do not work
reliably under Wayland.

## Main window

Active app wide in the main window, registered in
`src/hooks/useGlobalHotkeys.ts`.

| Key | Effect | Also active in text fields? |
|---|---|---|
| `Ctrl+N` | Open the quick capture window with the current customer/system context (`open_quick_capture_with_context`) | Yes |
| `Esc` | Close an open form or overlay; if none is open and the current view is the system list, go back to the customer list | Yes |
| `g` then `c` | Go to customer list | No |
| `g` then `s` | Go to the current customer's system list (only if a customer is selected) | No |
| `g` then `j` | Go to the journal | No |

Per the code, `Esc` and `Ctrl+N` also fire even while an input field is
focused. Every other binding in this table (the `g` sequences) is
suppressed while a text field is focused or Ctrl, Alt or Cmd is held. A
started `g` sequence expires after 800 ms without a second keypress.

**Important nuance:** `Ctrl+N` always opens the separate quick capture
window, not the entry editor embedded in the main window. See
[Entry editor](#entry-editor-main-window-srccomponentsentryeditortsx) below
for the second, separate way to create a new entry.

## Command Palette (`Ctrl+K`)

`Ctrl+K` opens or closes the Command Palette
(`src/components/CommandPalette.tsx`) and is **always active**, even while
a text field is focused (its own capture phase listener with
`stopPropagation`, runs before every other keyboard handler).

| Key | Effect |
|---|---|
| `Ctrl+K` | Open or close the palette |
| `Arrow Down` | Select the next hit |
| `Arrow Up` | Select the previous hit |
| `Enter` | Run the selected item (command, customer/system, or full text hit) |
| `Esc` | Close the palette |

With empty input, the palette shows static commands, including "New
entry", "Go to customer list", "Go to system list", "Go to journal" and
"Clean up attachments"; from the first character on, server side hits from
directory search (customers/systems) and full text search (entries) are
added. The "New entry" command shows the `Ctrl+N` hint in the palette but,
when selected, opens the entry editor in the main window
(`openEntryEditor("new")`), not the same window the global `Ctrl+N` key
opens (see the note above). Selecting a full text entry hit opens that
entry's read only detail view directly (`openEntryDetail`), the same one
reachable by double clicking a row in the journal.

## View specific keys

The following bindings are each only active locally within their own view,
and only while no form is open and no text field is focused.

### Customer list (`src/components/CustomerListView.tsx`)

| Key | Effect |
|---|---|
| `j` | Select the next customer |
| `k` | Select the previous customer |
| `Enter` | Navigate to the selected customer (open their system list) |
| `e` | Edit the selected customer (open the form) |

### System list (`src/components/SystemListView.tsx`)

| Key | Effect |
|---|---|
| `j` | Select the next system |
| `k` | Select the previous system |
| `e` | Edit the selected system (open the form) |
| `Enter` | Not bound. Unlike the customer list, there is no further level to navigate into here; `Enter` currently does nothing |

### Journal (`src/components/JournalView.tsx`)

| Key | Effect |
|---|---|
| `j` | Select the next entry |
| `k` | Select the previous entry |
| `Enter` | Open the selected entry in its read only detail view (`EntryDetailModal`), with an "Edit" button to switch to editing. Double clicking a row does the same. |
| `e` | Open the selected entry directly in the full entry editor |

`Enter` and `e` do **not** do the same thing in this view: `Enter` opens a
read only view of the entry, `e` opens the actual editor directly.

## Entry editor (main window, `src/components/EntryEditor.tsx`)

The editor is a modal in the main window for new or existing entries.
Opened via the Command Palette's "New entry" command, via `e` in the
journal or the entry detail view, or via the "+ New entry" / "Edit"
buttons in the relevant views. Not via the global `Ctrl+N` key (that opens
quick capture instead, see above).

| Key | Effect |
|---|---|
| `Ctrl+S` | Save the entry (creates or updates, depending on context); only active while the editor is open |
| `Esc` | Discard/close the editor. Goes through the global `Esc` handling in `useGlobalHotkeys.ts` (shared `formOpen` state), not a dedicated listener in the editor itself |

## Quick capture window (separate window, `src/quick-capture/QuickCapture.tsx`)

A standalone, compact window, not the main window. Opened via the global
hotkey, via `Ctrl+N` in the main window, or via a CLI flag
(`--quick-capture`). The title field is focused automatically when it
opens.

| Key | Effect |
|---|---|
| `Ctrl+V` | Paste an image from the clipboard; queued as an attachment and inserted as a Markdown reference at the cursor position |
| `Ctrl+S` | Save the entry and close the window |
| `Esc` | Discard the draft and close the window |

## Shortcut overview (`?`)

`?` opens an in app overview of the keyboard shortcuts
(`src/components/ShortcutOverview.tsx`).

| Key | Effect |
|---|---|
| `?` | Open the shortcut overview; only outside text fields, without Ctrl, Alt or Cmd |
| `Esc` | Close the overview |

> **Note:** The table shown in app under `?` is a static copy of the
> original spec table and has not been kept in sync with every
> implementation detail since. For example it still lists `/` as "focus
> search", even though that key is deliberately unbound (see below), and it
> does not distinguish the slightly different `Enter`/`e` behavior per view
> described above. This file (`SHORTCUTS.md`) is the reference, verified
> against the code.

## Not yet bound

| Key | Status |
|---|---|
| `/` | Not yet bound. The spec calls for `/` to focus a search input. For lack of a single, globally unambiguous search field, this binding was deliberately deferred in phase 4d (see `docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase4d.md`). Full text search is reachable today via the Command Palette (`Ctrl+K`). |

## Platform notes

- **Windows and Linux/X11:** The global hotkeys above are registered
  directly with the operating system via `tauri-plugin-global-shortcut`.
- **Linux/Wayland:** Global hotkey registration there is only possible via
  the XDG portal `org.freedesktop.portal.GlobalShortcuts`, which is not
  available on every compositor. Where it is missing, the three global
  hotkeys above do not work reliably. As a fallback, under Wayland as well
  as under X11/Windows, for example to bind your own key combinations via
  third party tools, the CLI flags `wartungsdoku --quick-capture` and
  `wartungsdoku --search` are always available (`src-tauri/src/cli.rs`),
  independent of portal availability. `--search`, like `Ctrl+Alt+F`, only
  shows the main window, without a dedicated search UI.
- **Window context capture** (taking over the title of the previously
  active window as a note, disabled by default) is implemented on Windows
  and Linux/X11 (`src-tauri/src/context_capture/`), unavailable on Wayland
  and deliberately reported there as unsupported rather than silently
  skipped. The X11 path was written on a Windows development machine and
  never compiled there; the file `src-tauri/src/context_capture/x11.rs`
  says so itself (`NOT COMPILED OR TESTED ON THIS HOST`). It should be
  verified for real before production use on Linux.
