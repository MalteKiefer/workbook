# Customer Detail Page

## Problem

`CustomerForm.tsx` started as a small edit modal (name, short code, notes)
and grew, over several features added this session, into a stack of five
things inside one modal: the core fields, `LocationsPanel`,
`ExpiringItemsPanel`, `VaultPanel`, and `AuditLogPanel` (the last as a
collapsed `<details>`). The modal is now cramped and hard to scan — too
much unrelated content competing for the same narrow vertical space.

## Goal

Replace the modal with a dedicated, full-page customer view with tabs,
the same pattern `SettingsView.tsx` already uses successfully
(`settingsTab` state + a row of `TabButton`s + conditional rendering).
Each of the five things above gets its own tab and its own full-width
space instead of being stacked. `SystemListView.tsx` (currently a
separate top-level nav destination) becomes one of these tabs too, since
in practice an admin always navigates to a customer's systems by way of
selecting that customer first — folding it in removes a navigation hop
and puts everything about one customer in one place.

## Non-goals

- No change to any of the five panel components' internal logic
  (`LocationsPanel.tsx`, `ExpiringItemsPanel.tsx`, `VaultPanel.tsx`,
  `AuditLogPanel.tsx`, `SystemListView.tsx`'s own content) — this is a
  re-hosting, not a rewrite. Each keeps its existing props contract
  (`customerId`, or `entityType`/`entityId` for the audit panel) and
  internal `invoke` calls untouched.
- No change to `SystemForm.tsx`, `EntryEditor.tsx`, or any other
  globally-mounted modal — they stay exactly as they are.
- No change to the Dashboard, Journal, or Settings views beyond updating
  the handful of navigation call sites listed below.

## Architecture

### New state (`src/state/appStore.ts`)

```ts
export type CustomerDetailTab = "uebersicht" | "systeme" | "standorte" | "ablauf" | "zugangsdaten" | "verlauf";
```

Add to `AppState`:
- `customerDetailTab: CustomerDetailTab` (default `"systeme"` — see
  "Default tab" below)
- `setCustomerDetailTab: (tab: CustomerDetailTab) => void`

Add `"customer-detail"` to the existing `View` union, replacing
`"systems"` (searched the codebase: `"systems"` as a `View` value is
only ever read in `App.tsx`'s nav-highlight/render lines and set by
`goToSystems` — both change together, see below).

`goToSystems(customerId?: number)` is renamed to
`goToCustomerDetail(customerId?: number, tab?: CustomerDetailTab)`,
keeping its existing behavior of only touching `selectedCustomerId` when
a `customerId` is actually passed (so re-navigating to the currently
open customer's detail page from, say, a keyboard shortcut doesn't reset
which customer is selected):

```ts
goToCustomerDetail: (customerId, tab) =>
  set((state) => ({
    view: "customer-detail",
    selectedCustomerId: customerId ?? state.selectedCustomerId,
    customerDetailTab: tab ?? "systeme",
  })),
```

**Default tab is `"systeme"`**, not `"uebersicht"` — every existing call
site of `goToSystems` today lands the admin on the systems list, and
none of them need to change their call signature to keep doing exactly
that (they just call `goToCustomerDetail(id)` with no third argument,
same as they call `goToSystems(id)` today).

### New component: `src/components/CustomerDetailView.tsx`

Structural mirror of `SettingsView.tsx`: a heading, a `TabButton` row
(reusing that same local `TabButton` component's shape — either import
it if `SettingsView.tsx` exports it, or copy the same 15-line
implementation; check at implementation time which is cleaner given
React fast-refresh/component-per-file conventions this codebase seems to
lean on), and six conditionally-rendered tab bodies:

| Tab | Renders |
|---|---|
| Übersicht | Name/short_code/notes fields (inline-editable, not a modal — see below), plus small stat tiles: system count, overdue count, expiring-soon count, location count |
| Systeme | `<SystemListView />` unchanged |
| Standorte | `<LocationsPanel customerId={id} />` unchanged, full width instead of embedded |
| Ablauf | `<ExpiringItemsPanel customerId={id} />` unchanged, full width |
| Zugangsdaten | `<VaultPanel customerId={id} />` unchanged, full width |
| Verlauf | `<AuditLogPanel entityType="customer" entityId={id} />` unchanged — note this panel currently renders nothing when there are zero audit rows (`if (entries.length === 0) return null;` in `AuditLogPanel.tsx`); as a whole tab body that would show a blank tab rather than a collapsed row, so this tab needs a small "noch keine Einträge" fallback added around it, a one-line change to `AuditLogPanel.tsx` (or a wrapper in the new tab body) rather than to the panel's data-fetching logic |

`SystemListView.tsx` itself needs zero internal changes for this move —
it already reads `selectedCustomerId` from the store rather than taking
a prop, so simply rendering `<SystemListView />` inside the "Systeme" tab
body works exactly like it does today as a full view.

### `CustomerForm.tsx` narrows to create-only

Every one of its current callers passes either `"new"` or a numeric
customer id (edit mode). Grepping the whole codebase for
`openCustomerEditor(`:

- **17 plugin section components** (`NinjaPluginSection.tsx`,
  `AbmPluginSection.tsx`, `AcronisPluginSection.tsx`, etc.) call it only
  with `"new"`, from their own "create-and-link" flow when a
  plugin-discovered device doesn't match an existing local customer.
  **These do not change at all.**
- **`CustomerListView.tsx`** calls it with a numeric id in exactly two
  places: the row `Enter`-key handler (line ~101) and the row's
  "Bearbeiten" button (line ~268). **Both change** to
  `goToCustomerDetail(customer.id, "uebersicht")` instead.
- **`CommandPalette.tsx`**'s `"edit-customer"` command (line ~223) calls
  `openCustomerEditor(selectedCustomerId)`. **Changes** to
  `goToCustomerDetail(selectedCustomerId, "uebersicht")`.

After this, `customerEditorTarget` in the store is only ever `"new"` or
`null` in practice — its type stays `"new" | number | null` (no need to
narrow the type itself, that's more churn than value), but
`CustomerForm.tsx`'s own body can drop its `isEditMode`
branch/edit-mode data-loading `useEffect` entirely, since it will only
ever be mounted in create mode going forward.

### Other navigation call sites to retarget

All found via the grep for `goToSystems`/`view === "systems"`:

- **`App.tsx`**: the nav `<NavLink>` highlight condition
  (`view === "customers" || view === "systems"`) becomes
  `view === "customers" || view === "customer-detail"`; the render line
  `{view === "systems" && <SystemListView />}` becomes
  `{view === "customer-detail" && <CustomerDetailView />}`.
- **`DashboardView.tsx`**'s `openCustomerSystems` helper (calls
  `selectCustomer` + `goToSystems`) renames its inner call to
  `goToCustomerDetail` — used by both the "Überfällige Wartungen" and
  "Letzte Einträge" row activations, both should keep landing on the
  Systeme tab (no behavior change, just the renamed function).
- **`CustomerListView.tsx`**'s row click/Enter (the "view this
  customer's systems" path, separate from the "Bearbeiten" edit path
  above) keeps going to the Systeme tab: `goToCustomerDetail(c.id)`.
- **`CommandPalette.tsx`**: `"goto-systems"` command and the
  `"import-systems-csv"` command's `goToSystems()` call both become
  `goToCustomerDetail()` (no tab arg — defaults to Systeme, unchanged
  behavior). The directory-search result handler
  (`goToSystems(hit.customer_id)` at line ~348) likewise renames with no
  behavior change.
- **`useGlobalHotkeys.ts`**: the `Escape`-from-systems-view check
  (`view === "systems"`) becomes `view === "customer-detail"`; the
  `goToSystems()` call (line ~71, a direct keyboard shortcut to jump to
  the current customer's systems) renames to `goToCustomerDetail()`.

None of these need a NEW tab argument — they all preserve today's
"lands on the systems list" behavior for free via the `"systeme"`
default described above. The only NEW call sites passing an explicit
`"uebersicht"` tab are the three edit-mode migrations
(`CustomerListView.tsx` ×2, `CommandPalette.tsx` ×1) replacing
`openCustomerEditor(numericId)`.

## Data flow

No backend/Rust changes. Every panel already fetches its own data via
`invoke` keyed on a `customerId` prop it's given — moving where that
prop comes from (a modal vs. a tab body, both ultimately reading
`selectedCustomerId` from the store) doesn't change any of that.

## Testing

No frontend test harness exists in this codebase (confirmed multiple
times this session) — verification is `npm run build` + `npm run lint`
+ manual click-through, same as every frontend-only feature built this
session.

## Open questions for the implementation plan

- Exact visual treatment of the Übersicht tab's stat tiles (styling only,
  not a design blocker — can follow this codebase's existing
  bordered-`<section>` convention used throughout `BackupView.tsx`/
  `SettingsView.tsx`).
- Whether `TabButton` should be extracted into its own shared component
  file now that two views (`SettingsView.tsx`, `CustomerDetailView.tsx`)
  need it, or duplicated once more and extracted later if a third
  consumer appears — YAGNI leans toward duplicating once more here, matching
  how small this component is (~15 lines) and this codebase's general
  preference for avoiding premature abstraction.
