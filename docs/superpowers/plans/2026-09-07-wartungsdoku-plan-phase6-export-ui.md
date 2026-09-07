# Wartungsdoku — Phase 6: Export UI — Implementation Plan

**Goal:** Give the main window a dialog for the "Export" Kernworkflow (spec §Export):
per customer, optionally filtered by system and/or time range, export either to a
Markdown directory or a single PDF file. This phase is the frontend dialog and its
trigger points (Command Palette entry, store wiring) only — it calls two backend
commands (`export_markdown`, `export_pdf`) that two other agents are building in
parallel in this same directory; this phase does not depend on their Rust landing
first, since TypeScript only checks the shape of the `invoke` call, not that the
command name resolves at runtime.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
— "Export" under Kernworkflows.

**Closest pattern references:**
- `src/components/EntryEditor.tsx` — the `openForm()`/`closeForm()` mount/unmount
  effect pair keyed on a piece of open/closed state, the second effect that watches
  `formOpen` flipping back to `false` (Esc, handled by `useGlobalHotkeys`) and mirrors
  that into local dialog-open state, `Modal`'s `onClose={closeForm}` wiring, and the
  fetch-only (never-resets-the-child-select) customer→system cascade effect — see
  that file's own comment on why the reset must live in the `<select>`'s `onChange`
  rather than in the effect keyed on `customerId` (a reset-on-effect would clobber a
  programmatically seeded child selection set moments earlier in the same render).
- `src/components/JournalView.tsx` — the cascading customer/system `<select>` pair's
  visual shape, the `datetime-local` range filter, and the local
  `datetimeLocalToIsoUtc` helper (copied verbatim per this codebase's "five static
  things aren't worth a shared module" convention, already applied to `CATEGORIES` in
  both `JournalView.tsx` and `EntryEditor.tsx`).
- `src/components/CustomerForm.tsx` — the plain `Modal`-wrapped form shell and inline
  `crimson` error rendering convention.

## Concurrency

Several agents work in this same directory in parallel. Every commit in this phase
uses an explicit pathspec covering only the files this phase touches — never a bare
`git commit`, never `git add -A`/`git add .`.

**Boundary — do not touch:** `src/App.tsx` (a human mounts `<ExportDialog />` there),
`src/hooks/useGlobalHotkeys.ts`, `src/components/EntryEditor.tsx`,
`src/components/JournalView.tsx`, `src/components/Modal.tsx` (read-only references),
`src-tauri/` (no backend changes — this phase only calls commands by name).

## Decision: how `CommandPalette` opens a dialog owned by a different component

`ExportDialog` needs to be opened from `CommandPalette.tsx`, a sibling component with
no parent/child relationship. This codebase already has exactly one precedent for
"open a dialog owned by component A from a trigger in component B": `EntryEditor`'s
`editorTarget` field on `appStore` (set from `JournalView`'s row list, its own "+ Neuer
Eintrag" button, and `Strg+N` in `useGlobalHotkeys`). Rather than inventing a second,
parallel mechanism (a module-level event emitter, a context, etc.) for what is the same
problem, this phase extends the same store with:

```typescript
exportDialogOpen: boolean;      // false = closed
openExportDialog: () => void;
closeExportDialog: () => void;
```

`ExportDialog` reads `exportDialogOpen` and renders `null` when it is `false`.
`CommandPalette`'s new command calls `openExportDialog()`. This keeps the codebase at
one cross-component dialog-triggering idiom instead of two, at the cost of `appStore`
carrying one boolean it otherwise wouldn't need — judged the smaller cost given the
existing `editorTarget` precedent already set that direction.

`editorTarget` is a tri-state (`null | "new" | number`) because `EntryEditor` needs to
distinguish create vs. edit-which-entry; `ExportDialog` has no such distinction (it is
always the same form, seeded from ambient store selection), so a plain boolean is
enough — no need to smuggle a customer id through the open state itself when the
dialog can just read `selectedCustomerId` directly on open, same as every other view.

## Decision: Command Palette entry visibility

The new `"Kunde exportieren"` command is gated on `selectedCustomerId !== null`,
mirroring the existing `"goto-systems"` entry's exact conditional-push pattern in
`staticCommands`. Rationale: the export dialog's customer `<select>` is not optional
(the spec: export runs "pro Kunde") — showing the command with no ambient customer
selected would open a dialog whose primary field is still empty and forces an extra
selection before anything useful can happen, and the command palette already has this
exact "hide the command until there is customer context to act on" precedent for
`goto-systems`. The dialog's own customer `<select>` still lets the user change away
from the ambient customer once open (filters aren't locked to it) — the gating is only
about whether the command is *worth surfacing* with zero context, not a hard
restriction on what the dialog can export.

## Task 1: `appStore.ts` — export-dialog-open state

**File:** `src/state/appStore.ts` (additive only — every existing field/action stays
exactly as-is). Add the three fields from the decision above, `exportDialogOpen: false`
as the initial value, both actions the obvious `set({...})`.

## Task 2: `src/components/ExportDialog.tsx` (new file)

**Modal choice:** reuse `Modal.tsx`, `onClose={closeForm}` — identical convention to
`EntryEditor`/`CustomerForm`.

**Local state:** `customers: Customer[]`, `systems: System[]`,
`customerId: number | ""`, `systemId: number | ""`, `fromInput`/`toInput` (raw
`datetime-local` strings), `busy: boolean` (disables both export buttons while a
command is in flight), `status: string | null` (success message), `error: string | null`.

**Effects:**
1. Fetch the full customer list once, unconditional (`useEffect(..., [])`) — same as
   `EntryEditor`'s "Full customer list, once" effect; cheap enough not to gate behind
   `exportDialogOpen`.
2. Fetch-only customer→system cascade keyed on `[customerId]`: empties `systems` when
   `customerId === ""`, otherwise calls `list_systems`. Deliberately never resets
   `systemId` itself (see the `EntryEditor` reference above) — the reset lives solely
   in the customer `<select>`'s `onChange`, so it never races the seed effect below.
3. Seed-on-open, keyed on `[exportDialogOpen]`: when it flips `true`, reset
   `customerId`/`systemId` from `selectedCustomerId`/`selectedSystemId` in the store
   (ambient context, same convention `SystemListView`/`JournalView`/`EntryEditor` all
   use), clear `fromInput`/`toInput`/`status`/`error`.
4. `openForm`/`closeForm` wiring keyed on `[exportDialogOpen]`: calls `openForm()`
   while open, `closeForm()` on close/unmount — identical shape to `EntryEditor`'s.
5. `formOpen`-watcher keyed on `[formOpen]`: when it flips to `false` while
   `exportDialogOpen` is still `true` (the Esc/backdrop-click path), calls
   `closeExportDialog()` — converges Esc-close and button-close on the same terminal
   state, identical to `EntryEditor`'s.

**Fields:** customer `<select>` (required, "Kunde wählen…" placeholder option, same
shape as `EntryEditor`'s), system `<select>` ("Alle Systeme" null option, disabled
while `customerId === ""`), a Von/Bis `datetime-local` pair converted through the local
`datetimeLocalToIsoUtc` helper (copied from `JournalView.tsx` verbatim, same
"good enough for filter bounds, not the spec's relative/absolute parser" rationale
already documented there).

**Actions — two plain buttons, not a form submit:** since there are two distinct
terminal actions (Markdown vs. PDF) rather than one, this dialog is a plain `<div>`,
not a `<form onSubmit>` — a native form would make Enter ambiguous between the two.
Both buttons are `disabled` when `customerId === ""` or `busy`.

- **"Als Markdown exportieren"**: `open({ directory: true, title: "Zielverzeichnis
  wählen" })` from `@tauri-apps/plugin-dialog`; `null` → do nothing (user cancelled,
  no message). Otherwise set `busy`, `invoke("export_markdown", { customerId, systemId:
  systemId === "" ? null : systemId, fromUtc: datetimeLocalToIsoUtc(fromInput), toUtc:
  datetimeLocalToIsoUtc(toInput), destDir })`, on success set `status` to a short
  confirmation naming the chosen directory, on failure `setError(String(e))` — same
  inline `crimson` rendering as every other form in the codebase. `finally` clears
  `busy`.
- **"Als PDF exportieren"**: same shape, `save({ defaultPath:
  "Wartungsdokumentation.pdf", filters: [{ name: "PDF", extensions: ["pdf"] }] })`,
  `invoke("export_pdf", { ..., destPath })`.

Both dialog functions are imported as `open`/`save` directly (no alias needed here —
unlike `EntryEditor.tsx`, which aliases `save` to `saveFileDialog` only because it
already has its own local `save` callback for persisting the entry; this component's
handlers are named `handleExportMarkdown`/`handleExportPdf`, so no collision exists).

**Cancel button:** calls both `closeForm()` and `closeExportDialog()` directly (mirrors
`EntryEditor`'s `cancel` callback) — `Modal`'s own `onClose` only calls `closeForm()`
and relies on the `formOpen`-watcher effect to converge, exactly as in `EntryEditor`.

## Task 3: `src/components/CommandPalette.tsx`

Add `const openExportDialog = useAppStore((s) => s.openExportDialog);` alongside the
file's other `useAppStore` reads, and one entry in `staticCommands`, pushed
conditionally right after the existing `"goto-systems"` push (same
`if (selectedCustomerId !== null)` guard, see the Decision section above):

```typescript
{ id: "export-customer", label: "Kunde exportieren", shortcut: "", run: () => openExportDialog() },
```

## Task 4: Verify

`npm run build` (`tsc -b && vite build`). Fix any TypeScript error before considering
this phase done. Backend commands (`export_markdown`/`export_pdf`) are not expected to
exist yet at verification time — `tsc` only checks the `invoke` call's own argument
shape against the types declared at the call site, not that the Rust command exists.

## Out of scope (explicit, for the record)

Mounting `<ExportDialog />` into `src/App.tsx` is left to a human integration pass,
same as `EntryEditor` was in its own phase — `App.tsx` is a stated boundary file for
this phase.
