# Wartungsdoku — Phase 5: Main-Window Entry Editor — Implementation Plan

**Goal:** Give the main window a full entry editor for both creating a new entry and
editing an existing one — the "Eintrag erfassen" workflow (spec, Kernworkflows) outside
the compact quick-capture window, plus the ability to correct a previously logged entry.
Quick-capture (`src/quick-capture/QuickCapture.tsx`) only ever creates; this component is
the create+edit counterpart that lives in the main window and is driven by the
`appStore` navigation store rather than a Tauri event payload.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
— "Eintrag erfassen" under Kernworkflows, and the `entries` table under Datenmodell
(the `performed_at` vs `created_at` distinction: `performed_at` is the
nachweisrelevant, user-editable activity timestamp; `created_at` is backend-set and
never user-editable — this editor exposes no `created_at` field at all).

**Closest pattern references:**
- `src/quick-capture/QuickCapture.tsx` — cascading customer→system selects, the
  `performedAtInput` → `parse_temporal_input` (on blur) → `format_timestamp_for_display`
  preview pattern, the local `CATEGORIES` constant, the `Strg+S` window-level keydown
  listener, and the create-entry `invoke` call shape (snake_case fields inside `input`,
  camelCase top-level param names).
- `src/components/CustomerForm.tsx` — the `openForm()`/`closeForm()` mount/unmount
  effect pair and the second effect that watches `formOpen` flipping to `false` (Esc via
  `useGlobalHotkeys`) and reacts to it — adapted here to call `closeEntryEditor()`
  instead of an `onDone` prop, since this component has no such prop.

## Concurrency

Several agents work in this same directory in parallel, each scoped to different files.
Every commit in this phase uses an explicit pathspec covering only the files this phase
touches — never a bare `git commit`, never `git add -A`/`git add .`.

**Boundary — do not touch:** `src/App.tsx`, `src/hooks/useGlobalHotkeys.ts`,
`src/components/JournalView.tsx`, `src/components/MarkdownEditor.tsx`,
`src/components/AttachmentList.tsx`, `src/components/AttachmentDropzone.tsx` (the latter
three are being built concurrently by other agents — do not import from or depend on
them). No Rust changes.

## Scope boundary (explicit, for the record)

This editor's own save path sends `pending_attachments: []` on every `create_entry`/
`update_entry` call. Attaching files to an *already-saved* entry is a separate component
another agent is building; it will be wired in during a later integration pass. New
entries that need a fresh screenshot attached at creation time are quick-capture's job
(`Strg+V` there), not this editor's. `src/App.tsx` is not touched here, so mounting
`<EntryEditor />` into the app tree is also part of that later integration pass — this
phase delivers a self-contained, working component driven entirely by the store.

## Task 1: `appStore.ts` — editor-open state

**File:** `src/state/appStore.ts` (additive only — every existing field/action stays
exactly as-is)

Add:
```typescript
editorTarget: "new" | number | null; // null = closed, "new" = create mode, number = editing that entry's id
openEntryEditor: (target: "new" | number) => void;
closeEntryEditor: () => void;
```
with `editorTarget: null` as the initial value; both actions do the obvious `set({...})`.

## Task 2: `src/components/EntryEditor.tsx`

**Modal choice:** reuse `Modal.tsx` as the overlay/focus-trap wrapper rather than
building a custom one. `Modal` only centers its children and applies no fixed size, so
the inner form container gets its own wide/tall inline size (roughly `40rem` wide,
`85vh` max-height, internal scroll) — enough room for a real editing surface without
duplicating `Modal`'s already-correct focus-trap-in/focus-return-out logic (the spec's
"Dialoge fangen den Fokus ein…" requirement). Passing `closeForm` as `Modal`'s `onClose`
means an outside click also flows through the same `formOpen`-watching effect as an
Esc-triggered close, so there is exactly one close path to reason about.

**State:** title, customerId/systemId (`number | ""`), category, tagsInput (comma
string — joined from `tags: string[]` on load, split back to `tag_names` on save),
bodyMd, performedAtInput/Utc/Tz/Preview (exact QuickCapture shape), loadingEntry (edit
mode only, disables Speichern until `get_entry` resolves), error.

**Load/init effect** (keyed on `editorTarget`):
- `"new"`: blank fields, `customerId`/`systemId` seeded from
  `selectedCustomerId`/`selectedSystemId` in the store (ambient context, same idea as
  `SystemListView`/`JournalView` reading those), and — since there is no activation
  event here to hand us a resolved "now" like quick-capture gets — resolve the initial
  `performed_at` preview ourselves via `parse_temporal_input("")` (documented as
  accepting empty string = now) so the preview is populated immediately, not just after
  first blur.
- a number: `get_entry({ id })`, then populate every field, `tagsInput = tags.join(", ")`,
  and resolve the preview immediately via `format_timestamp_for_display` using the
  loaded `performed_at_utc`/`_tz` — the raw `performedAtInput` text box stays empty (the
  user hasn't typed a new value), but the preview must show the *current* stored
  timestamp right away, not a blank string, so the user can see what they're changing.

**Customer→system cascade — deliberate deviation from a literal reading of the task
brief:** the brief describes the established pattern as "refetch systems on customerId
change, resetting systemId when it does" (this is `JournalView`'s literal code;
`QuickCapture`'s actual cascade effect does *not* reset `systemId` itself — the reset
only makes sense for a *user-driven* customer change). If the fetch-on-`customerId`
effect unconditionally reset `systemId` to `""`, it would clobber the value this
component just set programmatically two lines earlier (edit-mode load setting
`customerId` then `systemId` from the loaded entry, or new-mode seeding both from the
store) — both of those go through the same `customerId` state and would race the
effect's reset. Resolution: the fetch-on-`customerId` effect only fetches the system
list and never touches `systemId`; the customer `<select>`'s `onChange` handler is the
one place that resets `systemId` to `""`, since that is the only place a *user-driven*
customer change actually happens. Net effect is identical from the user's perspective
(pick a different customer manually → system resets) without breaking programmatic
load. Noting this explicitly so a reviewer doesn't mistake it for a missed requirement.

**Customer select in edit mode:** `update_entry` has no `customer_id` field — the
backend command cannot move an entry to a different customer. Rather than silently
dropping an in-UI customer change on save, the customer `<select>` is `disabled` when
`editorTarget` is a number (still shows the correct current customer, and the system
select stays live so `system_id` remains editable).

**Fields:** title (text input), customer (`<select>`), system (`<select>`, "kein
System" → `system_id: null`), category (`<select>`, the fixed 5 values, same local
`CATEGORIES` constant as the other views — kept local per this codebase's existing
"five static entries aren't worth a shared module" convention, see `JournalView.tsx`),
tags (comma text input), `performed_at` (text input + live preview, QuickCapture's exact
blur-parse-preview flow), body (`<textarea rows={15}>`, flex-grow, generous height — no
`MarkdownEditor` import; a human swaps this for the shared CodeMirror component in a
later integration pass once both pieces exist).

**Save:** `create_entry` (mode `"new"`) or `update_entry` (mode = numeric id), both with
`pending_attachments: []`. Success → `closeForm()` then `closeEntryEditor()`. Failure →
inline `{error && <p style={{color:"crimson"}}>Fehler: {error}</p>}`, same as every other
form in the codebase.

**Cancel:** `closeForm()` + `closeEntryEditor()`, no save.

**`openForm`/`closeForm` wiring:** a `useEffect` keyed on `editorTarget` calls
`openForm()` whenever it is non-null and returns a cleanup calling `closeForm()`
(mirrors `CustomerForm`'s mount/unmount pair, adapted to react to the target changing
rather than firing once on mount, since this component is expected to stay mounted
across open/close cycles once wired into `App.tsx` later). A second effect watches
`formOpen`; when it flips to `false` while `editorTarget` is still non-null (the Esc
path, handled by the existing `useGlobalHotkeys` global handler), it calls
`closeEntryEditor()` — so an Esc close and a Save/Cancel-button close both end up in the
same terminal state (`editorTarget: null`, `formOpen: false`).

**`Strg+S`:** component-local `window.addEventListener("keydown", ...)` exactly like
QuickCapture's, guarded with `if (editorTarget === null) return;` inside the handler so
it's inert whenever the editor isn't open; effect re-runs on `[editorTarget, save]` so
the closure always has the current save callback.

## Task 3: Verify

`npm run build` (`tsc -b && vite build`). Fix any TypeScript error before considering
this phase done.
