# Wartungsdoku — Phase 5: `AttachmentList.tsx` — Implementation Plan

**Goal:** A generic, presentational `src/components/AttachmentList.tsx` that renders
the attachments belonging to one entry — filename, size, and Öffnen/Exportieren/
Entfernen actions per spec ("Anhänge" under Kernworkflows: "Öffnen/Exportieren/
Entfernen per Tastatur") — with zero knowledge of how those actions are implemented.
It receives an already-fetched `Attachment[]` and four callback/resolver props; a
human wires the real `list_attachments_for_entry` / `open_attachment` /
`copy_attachment_to` / `remove_attachment` / `read_attachment_data_url` Tauri commands
(built by a parallel agent, see
`docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase5-attachments-backend.md`)
into those props later, inside whatever editor component ends up owning an entry's
attachment section. This keeps the component buildable and independently reviewable
with zero backend dependency.

**File:** Create `src/components/AttachmentList.tsx` only.

## Scope decision: one unified list, images included

Spec text (Kernworkflows → Anhänge): *"Bilder werden im Markdown-Vorschaubereich
gerendert. Nicht-Bilder (Config-Exporte, Logs, ZIPs) erscheinen als Liste am
Eintrag."*

Read literally, this describes **two different UI surfaces**: images render inline
inside the Markdown body preview, at the point in the text where their Markdown
reference (`![...](...)`) appears; non-images get a separate flat list attached to
the entry. That body-preview rendering is a different component's job entirely — it
needs to parse/render `entries.body_md` as HTML and splice in image data per
reference position, which requires knowing *where in the text* each attachment is
referenced. This component receives none of that — only a flat `attachments: Attachment[]`
with no positional/textual information — so it cannot implement that half of the
spec sentence regardless of how it's structured internally.

Given that, the choice is what *this* component should do with the images it's
handed:

1. Silently drop images from its rendering (`others` only), trusting the body-preview
   component to be the sole place they ever appear.
2. Render everything it's given — images included — as a complete, flat reference
   list for the entry.

Decision: **(2), a single unified list, images included**, in the order the
`attachments` prop arrives in (assumed caller-sorted, e.g. by `created_at_utc` —
this component does no client-side re-sorting or grouping-by-type). Reasoning:

- The prop is literally named `attachments` and documented as receiving "already-
  fetched data" for one entry — nothing scopes it to "non-image attachments only".
  Silently filtering out images would make this component's output depend on an
  unstated, easy-to-violate contract with whatever populates the `attachments` prop.
- A body-preview-with-inline-images feature does not exist yet in this codebase
  (`MarkdownEditor.tsx` — the only Markdown-rendering component built so far — is a
  plain CodeMirror text editor, not a rendered preview). Dropping images here would
  leave zero UI to open/export/remove an image attachment at all until that separate
  feature lands. Keeping images here means the "Öffnen/Exportieren/Entfernen per
  Tastatur" requirement is satisfied for every attachment unconditionally, today.
- Even once inline body-preview rendering exists, having *both* an inline preview and
  a complete attachment list is not contradictory — many editors (Linear, GitHub
  issues — the spec's own stated visual reference) show attached images both inline
  in rendered body text *and* in a flat "Attachments" list/footer. This list is the
  always-present, position-independent fallback/complete view; inline rendering
  (elsewhere) is an enhancement layered on top of the same data.
- A single `<ul>` in received order (rather than the starting-point sketch's
  images-block-then-others-block split) was chosen over grouping-by-type: grouping
  reorders items away from their `created_at` sequence for no stated benefit, and
  "a list of an entry's attachments" is one concept, not two — the per-row rendering
  (thumbnail vs. plain filename row) already carries the images/non-images
  distinction visually without a structural split. Each `<li>` decides its own
  layout (`mime_type.startsWith("image/")` → thumbnail above the filename/action
  row; otherwise a single filename/action row), all inside one list.
- Images render as a bounded-height thumbnail (`max-height`), not full width — this
  is a list row, not the body-preview surface, so "dicht aber lesbar" (Gestaltung)
  applies: a full-width image here would dominate the list for one attachment among
  potentially many.

## Non-goal: `resolveImageUrl` caching/dedup

Each image attachment resolves its displayable URL once, in a `useEffect` on mount
(and re-resolves only if `attachment.id`/`attachment.sha256` actually change — not on
every parent re-render that happens to pass a new object reference for the same
attachment, which is a one-line, effectively-free improvement over depending on the
whole `attachment` object). What this component deliberately does **not** do: cache
resolved URLs across re-mounts, dedupe concurrent/repeated calls for the same
`sha256` across multiple `AttachmentList` instances (e.g. the same image attached to
two entries, or the list unmounting/remounting when a user navigates away and back),
or hold any cache eviction policy.

Reasoning: this component has no visibility into what `resolveImageUrl` actually
costs (the real implementation is a parallel agent's `read_attachment_data_url`
Tauri command — an IPC round-trip plus a full file read into a base64 data URL; its
cost profile, and whether the integration layer above this component already caches
by `sha256`, are unknown from here). Building a cache into a purely presentational
list component, sight unseen of the real cost and without a signal that repeated
resolution is actually a problem, is speculative complexity. If it turns out to
matter, the natural place for a `sha256`-keyed cache is the integration layer that
implements `resolveImageUrl` (it can memoize there once, shared across every
`AttachmentList` instance and remount) — not duplicated inside this component.

## Task: `AttachmentList.tsx` component

**Props** (stable — this is the integration contract a human wires up later):

```ts
interface Attachment {
  id: number;
  entry_id: number;
  sha256: string;
  original_filename: string;
  mime_type: string;
  size_bytes: number;
  created_at_utc: string;
  created_at_tz: string;
}

interface AttachmentListProps {
  attachments: Attachment[];
  onOpen: (attachmentId: number) => void;
  onExport: (attachmentId: number) => void;
  onRemove: (attachmentId: number) => void;
  resolveImageUrl: (attachment: Attachment) => Promise<string>;
}
```

- Empty `attachments` → render nothing (`null`), matching this codebase's existing
  pattern of the parent component owning any section header/empty-state copy (the
  integrator likely already renders an "Anhänge" heading conditionally around this
  component).
- One `<ul>`, one `<li>` per attachment, in the order given. Each row: filename
  (`monospace`, matching the codebase-wide convention for technical values) + size
  (`formatSize` helper: B/KB/MB) + `Öffnen`/`Exportieren`/`Entfernen` `<button>`s
  wired to the matching callback with the attachment's `id`. Image rows additionally
  render a bounded-height `<img>` thumbnail above the filename row, resolved via
  `resolveImageUrl` in a small `ImageThumbnail` subcomponent with loading/error
  states (`Lade …`, `Bild konnte nicht geladen werden: …`) so a slow or failing
  resolve never blocks the rest of the list or removes the row's action buttons.
- Every action is a native `<button type="button">` — per the task brief, standard
  HTML gives Tab-reachability and Enter/Space activation for free; no custom
  `tabIndex`/`onKeyDown` needed, confirmed against how every other list view in this
  codebase (`CustomerListView.tsx`, `SystemListView.tsx`) already does the same for
  its row actions.
- Dark inline-style palette matching the rest of the app (`#1e1e1e`/`#181818`/`#2a2a2a`
  tones, `#a0a0a0` secondary text, `1px solid #333` row separators) — no CSS
  framework, consistent with every other component in `src/components/`.

- [ ] Write `src/components/AttachmentList.tsx` per the above.
- [ ] `npm run build` (`tsc -b && vite build`) compiles with no new TypeScript errors.
- [ ] Commit (pathspec: `docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase5-attachment-list.md`
      for this plan, then `src/components/AttachmentList.tsx` for the implementation —
      kept as two separate commits per the task's plan-first workflow).

## Boundaries — do not touch

`src/App.tsx`, `src/state/appStore.ts`, `src/hooks/useGlobalHotkeys.ts`, any other
existing `src/components/*.tsx` file, any `src-tauri/` Rust file,
`package.json`/`package-lock.json` (no new dependencies needed for this component).
Several other agents are working in this same directory concurrently on those files;
every commit here uses an explicit pathspec covering only the files this phase
touches.
