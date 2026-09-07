# Wartungsdoku — Phase 5: Attachment-Dropzone (Datei-Auswahl + Drag&Drop) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide the two remaining input methods for Anhänge — Dateiauswahl
(tastaturbedienbar) and Drag & Drop — as one small, reusable, purely presentational
component: `src/components/AttachmentDropzone.tsx`. It turns whatever `File`/`Blob`
objects the user picked or dropped into `{bytesBase64, filename, mimeType}[]` and hands
them to a parent-supplied `onFilesAdded` callback. It knows nothing about Tauri
commands, entry IDs, or the attachment data model — a later integration pass wires it
into the real entry editor.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
— "Anhänge" under Kernworkflows: „Einfügen per Zwischenablage, per Dateiauswahl
(tastaturbedienbar) und per Drag & Drop." Zwischenablage (clipboard-paste) is already
implemented separately inside `src/quick-capture/QuickCapture.tsx`'s `onPaste` handler
and is explicitly out of scope here — this component covers only file-picker and
drag&drop, to be composed alongside whatever paste handling a consuming editor already
has.

**Closest pattern reference:** `QuickCapture.tsx`'s existing clipboard-paste flow
(`handlePaste` → `item.getAsFile()` → `file.arrayBuffer()` → local
`arrayBufferToBase64` helper). This component reuses the exact same
"browser `File` → base64" approach for the two other input methods, since both a native
`<input type="file">` and the HTML5 `DataTransfer.files` list from a drop event hand
back real `File` objects with an identical `arrayBuffer()` API — no new abstraction
needed.

## Why the plain browser File API, not `@tauri-apps/plugin-dialog`/`plugin-fs`

A parallel agent is adding `tauri-plugin-dialog` (Cargo + `@tauri-apps/plugin-dialog`
npm) for a *different* feature: exporting/saving a file to a user-chosen destination
path, which the browser File API genuinely cannot do (browsers cannot write to an
arbitrary filesystem path chosen via a native "Save As" dialog). That is not this
task. This task is *reading* a file the user already picked or dropped — for that, a
native `<input type="file">` element and the browser's HTML5 drag-and-drop
`DataTransfer.files` API both hand back real `File` objects natively, no plugin
required, no IPC round-trip, no new dependency. Reusing the same plain-browser-API
approach that `QuickCapture.tsx`'s clipboard path already uses keeps all three
Anhänge input methods conceptually identical (`File`/`Blob` in, base64 out) and adds
zero new npm/Tauri dependencies, per the task boundary.

## Hidden-file-input keyboard-accessibility reasoning

Two options were considered for the `<input type="file">` that the visible "Datei
auswählen…" button triggers via `inputRef.current?.click()`:

1. **Absolutely-positioned, `opacity: 0`, `pointerEvents: none`** (as in an earlier
   sketch of this component). Rejected: without `display: none`, `tabindex="-1"`, or
   the `hidden` attribute, a native `<input type="file">` remains a normal focusable
   element and stays in the natural tab order. A keyboard user tabbing through the
   page would land on an *invisible* focused control (no visible focus ring, since
   the element itself has `opacity: 0`) sitting redundantly next to the real button —
   which directly violates this codebase's "Fokus ist jederzeit sichtbar" principle
   (see `MarkdownEditor.tsx`'s theme comment) and is confusing for screen-reader users
   too (an unlabeled second "file input, no file chosen" stop right after the button).

2. **`display: none` on the input; the visible `<button>` calls `inputRef.current.click()`
   in its own `onClick`.** This is the standard, widely-used pattern (Material UI,
   react-dropzone, and countless custom file-upload widgets all do exactly this).
   `display: none` removes the input from the accessibility tree and the tab order
   entirely — there is nothing to trip over. The only focusable, tabbable target is
   the real `<button>`, which is natively Enter/Space-activatable, so activating it by
   mouse or keyboard both go through the identical `onClick` handler.

   The remaining question is whether `.click()` on a `display:none` file input
   actually opens the native OS file-picker dialog. Per documented browser behavior:
   yes, in all current Chromium/Firefox/WebKit engines, as long as the `.click()` call
   happens synchronously inside a handler for a trusted user-gesture event. A
   `<button>`'s `onClick` fires from a real trusted click event both for actual mouse
   clicks *and* for Enter/Space activation of a focused button (the browser itself
   synthesizes a trusted `click` event in the latter case), so "transient activation"
   is present either way and the popup-blocker-style restriction on programmatic
   `.click()` does not apply. This app is a Tauri **desktop** app (Windows, per this
   session's environment — WebView2/Chromium on Windows, or WebKitGTK on Linux/macOS
   builds), not targeting mobile Safari, so the old iOS-Safari-specific quirks around
   hidden file inputs (long since fixed upstream anyway) are not a concern here.

   Chosen: **option 2** — simpler, avoids the stray-tab-stop risk entirely, and is
   the standard pattern for this exact problem.

## Task 1: `AttachmentDropzone.tsx` component

**Files:**
- Create: `src/components/AttachmentDropzone.tsx`

**No new dependencies** — plain browser File/DataTransfer APIs only.

**Interface** (stable for a later integration pass by a human):

```ts
export interface PickedFile {
  bytesBase64: string;
  filename: string;
  mimeType: string;
}

interface AttachmentDropzoneProps {
  onFilesAdded: (files: PickedFile[]) => void;
  children?: React.ReactNode;
}
```

- `arrayBufferToBase64` / `fileToPickedFile`: same chunked-`String.fromCharCode`
  approach as `QuickCapture.tsx`'s existing helper (kept as a local, self-contained
  copy in this file rather than a shared import, matching this codebase's existing
  duplication of small helpers per file — e.g. `QuickCapture.tsx` does not import
  from a shared util module either).
- `handleFiles(fileList: FileList | null)`: guards empty/null, maps every `File` in
  the list through `fileToPickedFile` (`Promise.all`), calls `onFilesAdded` once with
  the full batch.
- File-picker path: a real `<button type="button">` (default, unstyled, consistent
  with every other button in this codebase — see `SystemForm.tsx`/`CustomerForm.tsx`)
  whose `onClick` calls `inputRef.current?.click()`; a `display: none`
  `<input type="file" multiple>` behind it whose `onChange` calls `handleFiles` and
  then resets `e.target.value = ""` so picking the same file twice in a row still
  fires `onChange`.
- Drag&drop path: `onDragEnter`/`onDragOver`/`onDragLeave`/`onDrop` on the wrapping
  `<div>`. A `dragCounter` ref (rather than a plain boolean tied only to
  `onDragOver`/`onDragLeave`) avoids the well-known flicker bug where `dragleave`
  fires when the pointer crosses from the wrapper onto a child element (the button or
  hint text) and would otherwise prematurely clear the drag-over visual state.
  `onDragOver` must still call `e.preventDefault()` unconditionally — browsers refuse
  to fire `drop` at all otherwise.
- Visual affordance: dashed border, default subtle (`#333`, matching
  `MarkdownEditor.tsx`'s container border) turning to an accent blue (`#6ea8fe`,
  matching the sketch's drag-over color) plus a faint tinted background while
  `isDragOver` is true, and a small `"oder Dateien hierher ziehen"` hint next to the
  button so the drop affordance isn't only "a border appears from nothing" — someone
  who has never dragged a file onto the app has a textual hint too. Plain inline
  `style={{...}}`, no CSS framework, consistent with the rest of this codebase.
- `children` renders above the button/hint row so a parent can compose its own
  attachment list / preview area inside the same dropzone surface.

- [ ] Write `src/components/AttachmentDropzone.tsx` per the interface and reasoning
      above.
- [ ] `npm run build` (`tsc -b && vite build`) compiles with no new TypeScript errors.
- [ ] Commit (pathspec: `src/components/AttachmentDropzone.tsx`).

## Verification

- `npm run build` compiles cleanly.
- Manual reasoning check (no live app available in this session): file-picker button
  is a native `<button>`, reachable and activatable by Tab+Enter/Space exactly like
  every other button in the app; hidden input carries zero independent tab stop;
  `.click()` on a `display:none` file input from inside a trusted-click handler is
  documented, standard browser behavior.

## Boundary — do not touch

`src/App.tsx`, `src/state/appStore.ts`, `src/hooks/useGlobalHotkeys.ts`, any other
existing `src/components/*.tsx` file, `src-tauri/` (any Rust file),
`package.json`/`package-lock.json` (zero new dependencies for this task).
