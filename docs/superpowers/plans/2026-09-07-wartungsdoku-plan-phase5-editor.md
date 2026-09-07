# Wartungsdoku — Phase 5: CodeMirror-Markdown-Editor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the plain `<textarea>` used for an Entry's Markdown body with a real
CodeMirror 6 editor (per the spec's "Editor" choice under Technischer Rahmen), packaged
as a reusable `src/components/MarkdownEditor.tsx` component with a small imperative
handle (`insertAtCursor`, `focus`) so call sites that currently splice text into a
`<textarea>` via `selectionStart`/`selectionEnd` can do the CodeMirror-native equivalent.
First integration point: `src/quick-capture/QuickCapture.tsx`'s body field. A second
integration point (a main-window entry editor) is being built by a parallel agent and is
out of scope here — this component's public interface is designed as a drop-in
replacement so that later wiring needs no changes to `MarkdownEditor.tsx` itself.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
— "Editor" under Technischer Rahmen (CodeMirror 6 + `@codemirror/lang-markdown`),
"Anhänge" under Kernworkflows (paste-image-to-attachment flow that this phase must not
regress), and "Fokus ist jederzeit sichtbar" (focus-visibility requirement).

**Closest pattern reference:** `src/quick-capture/QuickCapture.tsx`'s existing
`insertAtCursor`/`handlePaste`/`bodyRef` textarea wiring (Phase 3b), which this phase
replaces surgically — the paste-to-attachment flow, the `quick-capture-paste-image`
event's direct `setBodyMd` string-append path, and all other behaviors in that file stay
unchanged.

## Concurrency

Several agents work in this same directory in parallel on different files. Per the
session's concurrency rule: every commit in this phase uses an explicit pathspec
covering only the files this phase touches — never a bare `git commit` or `git add -A`.

**Boundary — do not touch:** `src/App.tsx`, `src/state/appStore.ts`,
`src/hooks/useGlobalHotkeys.ts`, any other `src/components/*.tsx` file besides the new
`MarkdownEditor.tsx`, and any `src-tauri/` Rust file.

## Task 1: `MarkdownEditor.tsx` component

**Files:**
- Create: `src/components/MarkdownEditor.tsx`
- Modify: `package.json` / `package-lock.json` (new dependencies)

**Dependencies:**

```bash
npm install codemirror @codemirror/lang-markdown @codemirror/view @codemirror/state @codemirror/commands @codemirror/language
```

**Interface** (must stay stable — a parallel agent's main-window editor will consume this
later without touching this file):

```ts
export interface MarkdownEditorHandle {
  insertAtCursor: (text: string) => void;
  focus: () => void;
}
interface MarkdownEditorProps {
  value: string;
  onChange: (value: string) => void;
  onPaste?: (event: ClipboardEvent) => void;
  placeholder?: string;
}
```

`forwardRef<MarkdownEditorHandle, MarkdownEditorProps>` component. Internally:

- One `EditorView` created once (empty deps `useEffect`), torn down on unmount. Extensions:
  `basicSetup` (from the top-level `codemirror` package) + `history()` +
  `keymap.of([...defaultKeymap, ...historyKeymap])` + `markdown()` +
  `EditorView.lineWrapping` (prose wraps, no horizontal scroll) + an
  `EditorView.updateListener` that calls the *latest* `onChange` (via a ref, so the
  editor extensions don't need to be recreated when the parent re-renders with a new
  callback identity) on `docChanged` + a `domEventHandlers({ paste })` that forwards the
  native `ClipboardEvent` to the latest `onPaste`.
- A second `useEffect` keyed on `value` syncs external changes (e.g. `resetDraft()`
  clearing the field, or `QuickCapture.tsx`'s `quick-capture-paste-image` listener doing
  a direct `setBodyMd` string-append outside any editor interaction) into the CodeMirror
  document — but only when `value` actually differs from the doc's current text, so the
  user's own typing (which already updated `value` via `onChange`) doesn't cause a
  redundant dispatch that would reset cursor position.
- `useImperativeHandle` exposes `insertAtCursor` (replace the current selection with
  `text`, place the cursor right after it, refocus — the CodeMirror-native equivalent of
  the textarea's manual `selectionStart`/`selectionEnd` splice) and `focus`.
- Styling: plain inline `style={{...}}` on the container `<div>` (no CSS framework in
  this codebase), dark/dense/monospace to match the rest of the app's overlay components
  (`Modal.tsx`, `CommandPalette.tsx` use `#1e1e1e` background / `#e2e8f0` text); a
  `EditorView.theme(...)` extension sets the editor's own background/text/font-family
  and caret/selection colors to match, since CodeMirror does not inherit page styles.
  CodeMirror's default focus ring is left in place (not overridden) to satisfy the
  spec's "Fokus ist jederzeit sichtbar".

- [ ] Install the six CodeMirror packages listed above.
- [ ] Write `src/components/MarkdownEditor.tsx` per the interface above.
- [ ] `npm run build` compiles with no new TypeScript errors.
- [ ] Commit (pathspec: `package.json package-lock.json src/components/MarkdownEditor.tsx`).

## Task 2: Wire into `QuickCapture.tsx`

**Files:**
- Modify: `src/quick-capture/QuickCapture.tsx`

**Changes** (surgical swap of the body input mechanism only — every other behavior in
this file, including the `quick-capture-activated`/`quick-capture-paste-image` listeners,
save/discard, context-note prefill, and window-focus-regained handling, is left exactly
as-is):

- Replace `const bodyRef = useRef<HTMLTextAreaElement>(null)` with
  `const editorRef = useRef<MarkdownEditorHandle>(null)`.
- `insertAtCursor(text: string)` calls `editorRef.current?.insertAtCursor(text)` instead
  of the manual DOM `selectionStart`/`selectionEnd` splice (CodeMirror owns cursor state
  now; the old fallback-to-string-append branch is no longer needed since the imperative
  handle either exists or the ref hasn't mounted yet — mirror the same "no-op fallback"
  shape defensively if the handle isn't ready).
- `handlePaste`'s parameter type changes from
  `React.ClipboardEvent<HTMLTextAreaElement>` to the native `ClipboardEvent` (CodeMirror's
  `domEventHandlers` forwards a raw DOM event, not a React synthetic one). Body is
  unchanged — `e.preventDefault()` / `e.clipboardData` have the same shape on both.
- Replace the `<textarea ref={bodyRef} .../>` JSX with
  `<MarkdownEditor ref={editorRef} value={bodyMd} onChange={setBodyMd} onPaste={handlePaste} placeholder="Markdown…" />`.
- Leave `titleRef`-focused effects (window-focus-regained, `quick-capture-activated`)
  untouched — they focus the title field, not the body, by design (cursor starts in the
  title on a fresh capture).

- [ ] Apply the changes above.
- [ ] `npm run build` compiles with no new TypeScript errors.
- [ ] Manually re-read the full file top to bottom to confirm no existing behavior
      (paste-image-to-`pendingAttachments`, both Tauri event listeners, save/discard,
      context-note prefill) was silently dropped.
- [ ] Commit (pathspec: `src/quick-capture/QuickCapture.tsx`).

## Verification

- `npm run build` (`tsc -b && vite build`) after each task.
- Behavioral checklist carried over from the file's existing tests-by-inspection (no
  automated frontend test suite exists yet in this repo): typing in the body field still
  updates `bodyMd`; pasting an image still calls `create_entry` with a matching
  `pending_attachments` entry and a `pending:<uuid>` token in `body_md`; the
  `Strg+Alt+S` global-hotkey path (`quick-capture-paste-image` event, no focused webview)
  still appends its own placeholder markdown to `bodyMd` and is reflected in the editor.
