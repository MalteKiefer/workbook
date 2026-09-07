# Wartungsdoku — Phase 4c: Command Palette (`Strg+K`) — Implementation Plan

**Goal:** Add the Command Palette, the app's primary keyboard-driven entry point per
spec (`docs/superpowers/specs/2026-09-07-wartungsdoku-design.md`, "Bedienkonzept" /
"Command Palette" section): `Strg+K` opens a centered overlay with a single input that
fuzzy-matches static navigation commands, jumps to a customer/system by name or short
code (`search_directory`), and full-text searches entries (`search_entries`) inline —
all from the same text field, all keyboard-operable, shortcut hints shown per row.

This is **Phase 4c**, building on Phase 4a's store (`src/state/appStore.ts`) and
navigation actions. It does not touch `App.tsx`, `useGlobalHotkeys.ts`, or any Rust
file (see boundaries below) — those are owned by parallel agents in this same working
directory. The component is fully self-contained: it installs its own keydown
listener and manages its own open/closed state, following the same pattern already
used by `src/quick-capture/QuickCapture.tsx` for its own Ctrl+S/Esc handling.

**File:** Create `src/components/CommandPalette.tsx` only.

## Library choice: hand-rolled, not `cmdk`

The spec names `cmdk` as *suggested*, explicitly allowing "an equivalent
implementation of your own". Decision: hand-roll.

Reasons:
- `cmdk`'s main value is local fuzzy-filtering over a static list, but this palette's
  real complexity is merging three heterogeneous sources (static commands filtered
  client-side, `search_directory` results, `search_entries` results with `<mark>`
  HTML snippets) into one keyboard-navigable list — that merge logic has to be
  hand-written regardless of the input library, and the task instructions already
  say to use `shouldFilter={false}` (i.e. disable cmdk's own filtering) if cmdk were
  used, which removes most of its remaining benefit here.
- Installing `cmdk` means `npm install cmdk`, which touches `package.json` and
  `package-lock.json` — both shared files with no isolation (no worktree available;
  several agents editing this same directory concurrently per the task's concurrency
  warning). A plain new `.tsx` file with zero new dependencies avoids that shared-file
  risk entirely.
- Everything else already built in this codebase (`Modal.tsx`, `CustomerForm.tsx`,
  `CustomerListView.tsx`, `QuickCapture.tsx`) uses plain inline-styled React with no
  UI library — a hand-rolled palette matches the codebase's existing conventions
  rather than introducing the first external UI dependency.
- The actual requirements (fuzzy client-side filter over ~3 static commands, a text
  input, a keyboard-navigable results list, shortcut hints, `<mark>`-highlighted
  snippets via `dangerouslySetInnerHTML`) are all straightforward to implement
  directly with `useState`/`useEffect` and one `keydown` listener.

## Design

- **Open/close:** own `open` boolean state. A single `window.addEventListener`
  registered in the **capture phase** (`addEventListener("keydown", handler, true)`)
  handles: `Strg+K` / `Cmd+K` (toggle open, always active — works even while another
  input is focused, matching the spec's "Strg+K bleibt immer aktiv"), and, only while
  `open` is true, `Escape` (close without acting), `ArrowUp`/`ArrowDown` (move
  selection), `Enter` (activate selection). Using the capture phase (rather than the
  default bubble phase `useGlobalHotkeys.ts` and other listeners use) guarantees this
  component's `Escape`/arrow handling runs and calls `stopPropagation()` before any
  other window-level bubble-phase listener sees the event — this avoids double-handling
  (e.g. the global hook's own `Escape` behavior for `formOpen`/`view === "systems"`
  firing at the same time) without needing to coordinate registration order with code
  this phase must not touch. When `open` is false, the listener does not touch
  `Escape`/arrows/`Enter` at all, so it never hijacks those keys globally — only
  `Strg+K` is always live, per the task's explicit instruction.
- **Focus management:** on open, capture `document.activeElement` (so `Strg+K` works
  from inside any text field) and restore it on close — same principle as
  `src/components/Modal.tsx`. Input is autofocused on open.
- **Data flow:** query text drives three sources:
  1. Static commands ("Zu Kundenliste" → `g c` / `goToCustomers()`; "Zu Systemliste"
     → `g s` / `goToSystems()`, only listed when `selectedCustomerId !== null`,
     mirroring the same guard `useGlobalHotkeys.ts` uses for the real `g s` sequence;
     "Zum Journal" → `g j` / `goToJournal()`), filtered client-side by substring match
     on label, shown even for an empty query.
  2. `search_directory(query, 8)` — only fired for a non-empty trimmed query, debounced
     ~180ms.
  3. `search_entries(query, 8)` — same debounce/guard, wrapped in try/catch so a
     rejected FTS5 MATCH (raw punctuation etc.) degrades to zero entry results
     instead of crashing the palette.
     A monotonic request-id ref guards against stale/out-of-order responses
     overwriting newer results.
- **Selection/activation:** all matches (commands, directory hits, entry hits) are
  flattened into one ordered array for arrow-key navigation, grouped visually into
  three labelled sections in the same order. Enter activates the selected row, mouse
  hover/click also work (keyboard remains fully sufficient, mouse is just not
  disabled).
  - Customer hit → `selectCustomer(hit.id)` then `goToCustomers()`.
  - System hit → `selectCustomer(hit.customer_id)` then `goToSystems(hit.customer_id)`
    then `selectSystem(hit.id)`.
  - Entry hit → `selectCustomer(hit.customer_id)`, plus `selectSystem(hit.system_id)`
    if not null, then `goToJournal()`.
  - Any activation closes the palette and restores prior focus.

## Verification

- `npm run build` (`tsc -b && vite build`) must pass with no new dependency installed.
- No changes to `App.tsx`, `useGlobalHotkeys.ts`, or any file under `src-tauri/`.

## Mounting (left to whoever next touches `App.tsx`, out of scope for this phase)

```tsx
import CommandPalette from "./components/CommandPalette";
// ... inside App's returned JSX, rendered once, unconditionally:
<CommandPalette />
```

## Commit plan

1. This plan doc (pathspec-scoped commit).
2. `src/components/CommandPalette.tsx` (pathspec-scoped commit) after `npm run build`
   passes.
