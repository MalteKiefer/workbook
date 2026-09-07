# Wartungsdoku — Phase 4b: Systemliste + CRUD — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the System-list-scoped-to-a-customer view (list, create, edit, archive),
the second real view in the "Kunde → System → Eintrag" hierarchy, following directly on
Phase 4a's Kundenverwaltung. This is **Phase 4b** of "Command Palette und Navigation" —
the Command Palette itself (4c) and Journal (4d) still follow later.

**Architecture:** Pure frontend, same shape as Phase 4a's customer view. Every backend
command this phase needs (`list_systems`, `create_system`, `update_system`,
`archive_system`) already exists (see `src-tauri/src/commands/systems.rs`) — no Rust
changes, no new Tauri commands. `list_customers` (already existing, no per-id getter) is
reused client-side to resolve the customer's display name for the view header. List-local
keys (`j`/`k`/`e`) are handled by this view's own keydown listener, exactly like
`CustomerListView`; cross-cutting bindings (`g`-sequences, `Esc`) stay in the existing
global hook untouched.

**Tech Stack:** No new dependencies.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
**Phase 4a plan (done, closest pattern reference):** [docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase4a.md](2026-09-07-wartungsdoku-plan-phase4a.md)

## Global Constraints (carried over from 4a, still apply)

- Jede Funktion ohne Maus erreichbar.
- Textfelder unterdrücken Einzeltasten-Bindings (`j`, `k`, `e`); Modifier-Shortcuts
  bleiben aktiv. Reuse `isTypingTarget()` from `src/hooks/useGlobalHotkeys.ts` verbatim —
  do not reimplement it.
- Fokus ist jederzeit sichtbar; `Modal` (already built) fängt Fokus ein und gibt ihn
  zurück — reuse it verbatim for `SystemForm`, do not build a new dialog primitive.
- IPC-Feldnamen: `NewSystem`/`UpdateSystem` behalten snake_case-Feldnamen 1:1 im JSON
  payload (`system_type`, `ip_address`, `customer_id`) — nur der Top-Level-Parametername
  des Commands selbst (`input`, `customerId`, `includeArchived`) ist camelCased.
- Monospace für Hostnames/IPs/Code (spec line 272) — apply `fontFamily: "monospace"` to
  the `hostname`/`ip_address` spans specifically in the list row, not the whole row.

**Boundary:** Do not touch `src/App.tsx`, `src-tauri/src/lib.rs`, or any Rust file — other
agents are working on those concurrently in this same directory. This phase's two new
files must build standalone; a human wires `SystemListView` into `App.tsx` afterward.

---

## Task 1: System-Formular (Anlegen/Bearbeiten)

**Files:**
- Create: `src/components/SystemForm.tsx`

**Interfaces:**
- Consumes: `create_system`, `update_system` (existing Rust commands), `Modal`
  (`src/components/Modal.tsx`), `useAppStore.openForm/closeForm`
  (`src/state/appStore.ts`)
- Produces: `<SystemForm system={System | null} customerId={number} onDone={() => void} />`

**Reuse decision:** Copy `CustomerForm.tsx`'s structure (local `System` interface,
`openForm`/`closeForm` wiring via two `useEffect`s, controlled inputs, submit try/catch
with an inline error `<p>`, Modal wrapper, Abbrechen/Speichern button row) field-for-field.
The only structural difference from `CustomerForm` is the extra required `customerId`
prop (a System's `NewSystem` payload needs `customer_id`, which isn't on the entity being
edited in create mode) and five text fields instead of two-plus-notes:

| Field | Label (German) | Maps to |
|---|---|---|
| `name` | Name | `name` |
| `system_type` | Typ | `system_type` (free text, e.g. "Server", "Firewall", "NAS" — not a dropdown, per spec) |
| `hostname` | Hostname | `hostname` |
| `ip_address` | IP-Adresse | `ip_address` |
| `notes` | Notizen | `notes` (textarea, like Customer's notes) |

- [ ] **Step 1: `SystemForm.tsx` schreiben**, mirroring `CustomerForm.tsx`:
  - Local `interface System { id, customer_id, name, system_type, hostname, ip_address, notes }`.
  - `interface SystemFormProps { system: System | null; customerId: number; onDone: () => void }`.
  - `useState` per field, seeded from `system?.<field> ?? ""`.
  - Same `openForm()`/`closeForm()` mount-effect and `formOpenInStore` watch-effect as
    `CustomerForm` (so the global `Esc` handler, which calls `closeForm()`, closes this
    form too).
  - `handleSubmit`: on edit, `invoke("update_system", { id: system.id, input: { name, system_type: systemType, hostname, ip_address: ipAddress, notes } })`; on create,
    `invoke("create_system", { input: { customer_id: customerId, name, system_type: systemType, hostname, ip_address: ipAddress, notes } })`.
    Note the required snake_case keys inside `input` (`system_type`, `ip_address`,
    `customer_id`) even though the local state variables are camelCase — this is the
    easy-to-get-wrong IPC rule called out above.
  - Title: "System bearbeiten" / "Neues System".
- [ ] **Step 2: Build verifizieren** (`npm run build`).
- [ ] **Step 3: Commit** (pathspec-scoped, per concurrency warning):
  ```
  git add src/components/SystemForm.tsx
  git commit -m "feat: add system create/edit form"
  ```

---

## Task 2: Systemliste-Ansicht

**Files:**
- Create: `src/components/SystemListView.tsx`

**Interfaces:**
- Consumes: `list_systems`, `archive_system` (existing Rust commands), `list_customers`
  (existing, used only to resolve the customer's display name — no `get_customer`
  command exists), `SystemForm` (Task 1), `isTypingTarget`
  (`src/hooks/useGlobalHotkeys.ts`), `useAppStore` fields `selectedCustomerId`,
  `formOpen`, action `goToCustomers`.
- Produces: `<SystemListView />` (no props — reads `selectedCustomerId` from the store,
  same as `CustomerListView` reads no props and drives itself off the store/its own
  local state).

**Reuse decision:** Copy `CustomerListView.tsx`'s structure (local `System` interface,
`reload` callback + mount effect, list-local keydown effect guarded by
`formOpen`/`isTypingTarget`, `editing: System | "new" | null` state, per-row
Bearbeiten/Archivieren buttons, `+ Neu...` button, conditional `SystemForm` mount) with
these deltas:

- **No-customer guard**: if `selectedCustomerId` is `null`, render a fallback ("Kein
  Kunde ausgewählt — zurück zur Kundenliste" + a button calling `goToCustomers()`)
  instead of the list. This shouldn't normally happen (the global hotkey hook only
  allows `g s` when a customer is selected, and `CustomerListView`'s `Enter` handler
  already calls `selectCustomer` before `goToSystems`), but the view must not crash on
  a direct/unexpected mount with no selection. All hooks are still called unconditionally
  on every render (React rule) — the `null` check only gates what JSX is *returned*, not
  which hooks run.
- **Customer name header**: a second effect fetches
  `invoke<Customer[]>("list_customers", { includeArchived: true })` whenever
  `selectedCustomerId` changes, finds the matching customer client-side, and stores its
  `name` in local state for the "Systeme von {customerName}" heading. If not found
  (defensive — e.g. stale id), fall back to a heading without the name rather than
  crashing.
- **Back button**: an explicit "← Zurück zu Kunden" button calling `goToCustomers()`,
  additional to (not replacing) the existing global `Esc` behavior already wired in
  `useGlobalHotkeys.ts`.
- **Fetch scoping**: `invoke("list_systems", { customerId: selectedCustomerId, includeArchived: false })` — reload on mount and whenever `selectedCustomerId` changes (dependency
  array, not mount-only like `CustomerListView`'s, since this view's data is scoped to a
  value that can change under it).
- **Row rendering**: name, system_type, hostname — with `style={{ fontFamily: "monospace" }}`
  applied to the `hostname` and `ip_address` spans specifically (not the whole `<li>`),
  per the spec's monospace-for-hostnames/IPs guidance.
- **Keydown**: `j`/`k` move `selectedIndex` exactly like `CustomerListView`. `e` opens
  `SystemForm` in edit mode for the selected row. `Enter` is intentionally **not bound** —
  there is no system-detail view yet (later phase); do not invent a fake navigation
  target for it.
- **Archive**: `invoke("archive_system", { id })` then `reload()`, per row.
- **Create**: "+ Neues System" button sets `editing = "new"`; `SystemForm` receives
  `customerId={selectedCustomerId}` in create mode (safe because the button/list only
  render once the no-customer guard has passed).

- [ ] **Step 1: `SystemListView.tsx` schreiben** per the deltas above, using
  `CustomerListView.tsx` as the line-by-line template for everything not called out as
  different.
- [ ] **Step 2: Build verifizieren** (`npm run build`).
- [ ] **Step 3: Commit** (pathspec-scoped):
  ```
  git add src/components/SystemListView.tsx
  git commit -m "feat: add system list view scoped to selected customer"
  ```

---

## Task 3: Verifikation

- [ ] **Step 1:** `npm run build` (runs `tsc -b && vite build`) — zero TypeScript errors,
  `dist/` current. This is the phase's compile-check; no Rust changes were made and the
  full Tauri app is not launched (frontend-only phase, same rationale as 4a's Task 5
  used for `App.tsx`-level changes — here there isn't even an `App.tsx` change to
  smoke-test since wiring is left to a human/later step).
- [ ] **Step 2:** Confirm neither new file imports or modifies `src/App.tsx`,
  `src-tauri/src/lib.rs`, or any file under `src-tauri/` (boundary check).
- [ ] **Step 3: Commit this plan doc** (pathspec-scoped, before implementation per the
  task instructions — see commit log for ordering):
  ```
  git add docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase4b.md
  git commit -m "docs: add Phase 4b implementation plan (system list view)"
  ```

**Bekannte Grenze:** Wie in 4a — Tastaturverhalten in der echten Anwendung (`j`/`k`, `e`,
Fokusfalle im Formular, `Esc`) kann hier nicht interaktiv nachgestellt werden;
TypeScript-Build und Code-Review (Pattern-Treue zu `CustomerListView`/`CustomerForm`)
sind die Absicherung. Zusätzlich: da `App.tsx` bewusst nicht angefasst wird (Boundary),
ist auch keine End-to-End-Sichtprüfung der neuen View im laufenden Fenster möglich — die
Anbindung übernimmt ein Mensch oder eine spätere Phase.

---

## Self-Review (durchgeführt vor Ausführung)

- **Spec-Abdeckung**: Systemliste je Kunde ✓, `j`/`k` Listennavigation ✓, `e` bearbeiten
  ✓, Archivieren ✓, Anlegen ✓, Monospace für Hostname/IP ✓, Zurück-Navigation (Button +
  bestehendes globales `Esc`) ✓. Bewusst ausgelassen: System-Detailansicht/Historie (Enter
  auf eine Zeile) — existiert laut Aufgabenstellung noch nicht, folgt in einer späteren
  Phase; kein Platzhalter-Navigationsziel wird erfunden.
  Kundenverschieben zwischen Kunden ist außerhalb dieser Phase — `update_system` bietet
  ohnehin kein `customer_id`-Feld.
- **Bewusst verschoben**: Command Palette (4c), Journal (4d), Systemdetail/-historie.
- **Typkonsistenz**: Lokales `System`-Interface in `SystemForm.tsx` und
  `SystemListView.tsx` verwendet dieselben Feldnamen wie die Rust-Typen in
  `src-tauri/src/commands/systems.rs` (`id`, `customer_id`, `name`, `system_type`,
  `hostname`, `ip_address`, `notes`, `created_at_utc`, `created_at_tz`,
  `updated_at_utc`, `updated_at_tz`, `archived_at_utc`, `archived_at_tz` — list view only
  needs a subset, form needs none of the timestamp fields).
- **IPC-Namenskonvention**: `create_system`/`update_system`-Aufrufe verschachteln
  snake_case-Feldnamen (`system_type`, `ip_address`, `customer_id`) innerhalb von
  `input`, während Top-Level-Parameter (`customerId`, `includeArchived`, `input`)
  camelCased sind — konsistent mit der bestehenden `create_customer`/`list_customers`-
  Konvention aus Phase 4a.
