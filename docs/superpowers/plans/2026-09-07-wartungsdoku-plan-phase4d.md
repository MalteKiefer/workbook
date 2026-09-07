# Wartungsdoku — Phase 4d: Journal-Ansicht, Strg+N-Kontext, Shortcut-Übersicht — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the "Wiederfinden"/Journal-Ansicht (chronological, filterable entry
list) described in the spec, wire `Strg+N` from the main window to open the existing
quick-capture window pre-filled with the *currently selected* customer/system (instead
of "last used"), and add the `?` Shortcut-Übersicht modal. This is **Phase 4d** of
"Command Palette und Navigation" — the Command Palette itself is Phase 4c, built by a
parallel agent, not touched here.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
— see "Journal-Ansicht" under "Wiederfinden", the "Eintrag erfassen" workflow, and the
"Tastaturbelegung" table.
**Closest pattern references:** Phase 4a plan (customer list + Modal + global hotkeys),
Phase 3b-2/3b-3 plan (quick-capture window, `QuickCaptureActivated` event).

## Concurrency

Several agents work in this same directory in parallel on different files. Per the
session's concurrency rule: every commit in this phase uses an explicit pathspec
covering only the files this phase touches — never a bare `git commit` or `git add -A`.

**Boundary — do not touch:** `src/App.tsx`, `src-tauri/src/lib.rs`,
`src/components/CommandPalette.tsx` (parallel agent's file, if it exists). This phase's
new/changed files must build/compile standalone; a human wires `JournalView`,
`ShortcutOverview`, and the new `invoke_handler!` line into the rest of the app
afterward (exact snippets to hand over are listed at the end of this doc).

## Part 1 — Rust backend: `Strg+N` opens quick-capture with explicit context

**Why:** The quick-capture window today always pre-fills customer/system from
`get_last_selection` (the last customer/system an entry was saved against). The new
`Strg+N` binding in the main window's global hotkey hook should instead pre-fill
whatever customer/system is *currently selected in the main window's navigation state*
— a different, explicit context, not "last used". This needs a new backend entry point
that carries those two ids through the same activation event the frontend already
listens for.

**Files:**
- Edit: `src-tauri/src/quickcapture.rs`
- Edit: `src-tauri/src/commands/quickcapture.rs`

**Changes:**
- `QuickCaptureActivated` gains two new optional fields: `override_customer_id: Option<i64>`,
  `override_system_id: Option<i64>`.
- Shared body of `open()` (foreground capture, context-note capture, timestamp,
  event emit, show+focus) is extracted into a private `open_internal(app, override_customer_id,
  override_system_id)`. `open()` becomes a thin wrapper calling `open_internal(app, None, None)`
  — no behavior change for the existing hotkey/tray triggers. A new public
  `open_with_context(app, customer_id, system_id)` calls `open_internal(app, customer_id, system_id)`.
- New Tauri command `open_quick_capture_with_context(app, customer_id: Option<i64>, system_id: Option<i64>)`
  in `src-tauri/src/commands/quickcapture.rs`, delegating to `crate::quickcapture::open_with_context`.
- **Not registered in `invoke_handler!`** — that edits `src-tauri/src/lib.rs`, reserved
  for the human doing final integration. The exact line to add is reported at the end.
- Verify with `cargo build --bin wartungsdoku && cargo test` (all 59 existing tests
  should pass unchanged — this is a compile-check on an additive change to the data-free
  quickcapture module, not a data-layer change).

## Part 2 — Frontend: quick-capture honors the override context

**File:** `src/quick-capture/QuickCapture.tsx`

- Extend the `"quick-capture-activated"` listener's payload type with
  `override_customer_id: number | null` and `override_system_id: number | null`.
- Inside the existing `draftIsEmptyRef`-guarded fresh-draft branch, after the
  `get_last_selection` call: if `override_customer_id` is not null, set `customerId` to
  it directly (skip/override whatever `get_last_selection` produced); same for
  `override_system_id` → `systemId`. The hotkey/tray-triggered paths (which never send
  overrides — `open()` still passes `None, None`) keep today's last-selection-prefill
  behavior unchanged.

## Part 3 — `JournalView.tsx`

**File:** Create `src/components/JournalView.tsx`, shown by the (human-wired) `App.tsx`
when `view === "journal"` (the store field already exists in `src/state/appStore.ts`).

**Filters** (client state, any change re-invokes `list_entries`):
- Kunde: `<select>` populated from `list_customers({ includeArchived: false })`.
- System: `<select>` populated from `list_systems({ customerId, includeArchived: false })`
  once a customer filter is chosen (same cascading pattern as
  `src/quick-capture/QuickCapture.tsx`'s customer→system effect); cleared/disabled when no
  customer filter is set.
- Kategorie: `<select>` with the same 5 static values/labels as
  `QuickCapture.tsx`'s `CATEGORIES` — duplicated locally as a small constant (not worth
  sharing for 5 static entries).
- Tag: free-text `<input>`, matched against the `tag` filter field verbatim (server does
  substring/exact match — whatever `EntryFilter.tag` already does, unchanged here).
- Zeitraum: two `<input type="datetime-local">` (von/bis). **Simplification** (documented
  here per the task brief): `datetime-local`'s value has no timezone; this phase appends
  `:00Z` naively to build an ISO string for `from_utc`/`to_utc`. This is a first-pass
  filter-bounds convenience, *not* the spec's relative/absolute `parse_temporal_input`
  parser (`-2h`, `gestern 9:15`, …) — that parser is for entry timestamps, not filter
  bounds, and wiring it into filter inputs is left to a later phase if desired.

**IPC field names:** `list_entries` is called as `invoke("list_entries", { filter: {...} })`
— the top-level `filter` param is camelCase, but its fields stay snake_case exactly as
`EntryFilter` in `src-tauri/src/db/entries.rs` declares them: `customer_id`, `system_id`,
`category`, `tag`, `from_utc`, `to_utc` (not `tags`, not `customerId` inside the object).

**Rendering:** list ordered as returned (server already sorts `performed_at_utc` desc).
Each row: `title`, German category label, `format_timestamp_for_display(performed_at_utc,
performed_at_tz)` (invoked per row, never formatted client-side), tags comma-joined,
optional first ~100 chars of `body_md` as a raw-text preview (no markdown rendering).

**Keyboard (local to this view, mirrors `CustomerListView.tsx`'s pattern):**
- Guarded by `formOpen` (store) and `isTypingTarget(document.activeElement)` — no local
  keys fire while a modal/form is open or a text field has focus.
- `j`/`k`: move a local `selectedIndex` down/up, clamped to `[0, entries.length-1]`.
- `Enter` / `e`: **no entry detail/edit view exists yet** (real editor lands in a later
  phase — CodeMirror/markdown editing per the spec's "Reihenfolge der Umsetzung"). As a
  deliberately minimal placeholder, this toggles an inline expanded block under the
  selected row showing its full `body_md` as read-only `<pre>` plain text — not editable,
  not markdown-rendered. This is explicitly a stopgap; a human/later phase should replace
  it with the real detail/edit view.

## Part 4 — `ShortcutOverview.tsx`

**File:** Create `src/components/ShortcutOverview.tsx`, always mounted (human-wired into
`App.tsx` per the snippet at the end of this doc) — manages its own open/closed state.

- Static content: the full "Tastaturbelegung" table from the spec (`Strg+K`, `Strg+N`,
  `Strg+S`, `/`, `g c`, `g s`, `g j`, `j`/`k`, `Enter`, `e`, `Strg+V`, `Esc`, `?`), each
  with a one-line German description, rendered inside the existing `Modal` component
  when open.
- Self-contained listener style (matches the Command Palette pattern a parallel agent is
  building — no coordination needed, just consistency): its own
  `window.addEventListener("keydown", ...)`, opens on bare `?` (i.e. `e.key === "?"`)
  when `!isTypingTarget(document.activeElement)` and no modifier held; closes on `Escape`
  while open. Not wired into `useGlobalHotkeys.ts` — it owns its own key entirely, same
  as `QuickCapture.tsx` owns `Strg+S`/`Esc` for its own window.

## Part 5 — `useGlobalHotkeys.ts`: `Strg+N`

**File:** Edit `src/hooks/useGlobalHotkeys.ts` (confirmed no other parallel agent is
touching this shared file for this task).

- Import `invoke` from `@tauri-apps/api/core`.
- Read `selectedSystemId` from the store alongside the existing `selectedCustomerId`
  (the hook doesn't currently read it — add it).
- In `onKeyDown`, **before** the existing `isTypingTarget || ctrlKey || metaKey || altKey`
  early-return (that guard is for bare-letter/prefix keys; `Strg+N` is a modifier combo
  and must fire regardless of focus, same reasoning as `Esc` already not being gated):
  if `e.ctrlKey && e.key.toLowerCase() === "n"`, call `e.preventDefault()` and
  `invoke("open_quick_capture_with_context", { customerId: selectedCustomerId, systemId: selectedSystemId })`,
  then `return`.

### `/` — deliberately deferred, not bound in this hook

The spec's `/` binding ("Suche fokussieren") has no single global search input to focus
today: the Command Palette (which owns fuzzy/full-text search per the spec) is being
built in parallel by another agent with no visibility into whether it lands before or
after this phase, and Journal's own filter inputs are this phase's *filter* fields, not
a dedicated search box. Binding `/` here to a guessed `document.getElementById("...")`
target would be fragile wiring against a component that may not exist yet or may use a
different id. **Decision:** `/` is not bound in `useGlobalHotkeys.ts` in this phase.
Once the Command Palette (or a Journal search box) exists and can name a stable target
(an id, a ref registered via the store, etc.), a human/later phase adds the binding
deliberately against the real target. This is called out explicitly for whoever wires
the rest of the app together.

## Verification

- `npm run build` — TypeScript compile-check across all changed/new frontend files.
- `export PATH="$HOME/.cargo/bin:$PATH"; cd src-tauri && cargo build --bin wartungsdoku && cargo test && cd ..`
  — expect all 59 existing tests to still pass unchanged (additive, non-data-layer Rust
  change).

## Handover snippets (for the human doing final integration)

`src-tauri/src/lib.rs` `invoke_handler!` list — add:
```
commands::quickcapture::open_quick_capture_with_context,
```

`src/App.tsx` — journal view branch and always-mounted shortcut overview:
```tsx
import JournalView from "./components/JournalView";
import ShortcutOverview from "./components/ShortcutOverview";
// ...
{view === "journal" && <JournalView />}
// ... (mounted once, alongside the rest of the app shell, not inside a view branch)
<ShortcutOverview />
```
