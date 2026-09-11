# Customer Detail Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the cramped `CustomerForm.tsx` modal (core fields + 4 stacked panels) with a dedicated, full-page, tabbed `CustomerDetailView.tsx`, folding in `SystemListView.tsx` as a tab.

**Architecture:** A new `View` value (`"customer-detail"`) and `customerDetailTab` store field drive a new top-level component with 6 tabs, mirroring `SettingsView.tsx`'s existing tab-bar pattern exactly. Five of the six tab bodies are existing, unmodified components (`SystemListView`, `LocationsPanel`, `ExpiringItemsPanel`, `VaultPanel`, `AuditLogPanel`) simply re-hosted; the sixth (Übersicht) is new. `CustomerForm.tsx` narrows to create-only. Every navigation call site that currently sends the admin to the systems list is mechanically renamed; three call sites that currently open the edit modal are redirected to the new page's Übersicht or Verlauf tab instead.

**Tech Stack:** React/TypeScript, Zustand (`useAppStore`), Tauri `invoke`. No backend/Rust changes anywhere in this plan.

**Spec:** `docs/superpowers/specs/2026-09-11-customer-detail-page-design.md`

## Global Constraints

- No Rust/backend changes — every command this plan calls already exists.
- No frontend test harness exists in this codebase (confirmed repeatedly this session) — every task's verification step is `npm run build` + `npm run lint`, run from the repo root, plus a description of what to click through manually. Do not invent a test framework.
- Match this codebase's existing conventions exactly: `useState`/`invoke`/`formatInvokeError` for data fetching, inline `style={{...}}` objects (no CSS files/styled-components), `var(--...)` CSS custom properties for all colors, German-language UI copy.
- `TabButton` is duplicated into the new file rather than extracted into a shared component (YAGNI — it's ~15 lines and this codebase generally avoids premature abstraction; see spec's "Open questions").
- Every step that changes a `.tsx`/`.ts` file must leave `npm run build` and `npm run lint` clean before its commit step.

---

### Task 1: Store — `customerDetailTab` state and the `goToSystems` → `goToCustomerDetail` rename

**Files:**
- Modify: `src/state/appStore.ts`

**Interfaces:**
- Produces: `export type CustomerDetailTab = "uebersicht" | "systeme" | "standorte" | "ablauf" | "zugangsdaten" | "verlauf";`, `customerDetailTab: CustomerDetailTab` (store field), `setCustomerDetailTab: (tab: CustomerDetailTab) => void`, `goToCustomerDetail: (customerId?: number, tab?: CustomerDetailTab) => void` (replaces `goToSystems`), `View` union's `"systems"` value replaced by `"customer-detail"`.
- Consumes: nothing new (this is the foundation task every other task builds on).

This task alone will NOT make the app build cleanly — `goToSystems` and `view === "systems"` are referenced in 5 other files, all fixed in later tasks. That's expected and fine; `npm run build`'s TypeScript errors from those other files are the confirmation this task changed the right things. This task's own step 2 only checks that `appStore.ts` itself has no syntax errors (via a scoped `tsc --noEmit` isn't set up in this project, so instead: read the diff back carefully and confirm the shape matches exactly what's specified here — there is no cheaper isolated check for one file in a project without per-file type-checking, and a full `npm run build` at this point would show many expected, not-yet-fixed errors in other files, which is not useful signal yet).

- [ ] **Step 1: Add the `CustomerDetailTab` type and widen `View`**

In `src/state/appStore.ts`, change:

```ts
export type View = "dashboard" | "customers" | "systems" | "journal" | "settings";
```

to:

```ts
export type View = "dashboard" | "customers" | "customer-detail" | "journal" | "settings";
export type CustomerDetailTab = "uebersicht" | "systeme" | "standorte" | "ablauf" | "zugangsdaten" | "verlauf";
```

- [ ] **Step 2: Add the `customerDetailTab` field and setter to `AppState`**

Add to the `AppState` interface, right after `selectedSystemId: number | null;`:

```ts
  customerDetailTab: CustomerDetailTab;
```

Replace the existing:

```ts
  goToSystems: (customerId?: number) => void;
```

with:

```ts
  goToCustomerDetail: (customerId?: number, tab?: CustomerDetailTab) => void;
  setCustomerDetailTab: (tab: CustomerDetailTab) => void;
```

- [ ] **Step 3: Update the store implementation**

Add `customerDetailTab: "systeme",` to the initial state object, right after `selectedSystemId: null,`.

Replace the existing:

```ts
  goToSystems: (customerId) =>
    set((state) => ({
      view: "systems",
      selectedCustomerId: customerId ?? state.selectedCustomerId,
    })),
```

with:

```ts
  goToCustomerDetail: (customerId, tab) =>
    set((state) => ({
      view: "customer-detail",
      selectedCustomerId: customerId ?? state.selectedCustomerId,
      customerDetailTab: tab ?? "systeme",
    })),
  setCustomerDetailTab: (tab) => set({ customerDetailTab: tab }),
```

- [ ] **Step 4: Read the whole file back and confirm the shape**

Read `src/state/appStore.ts` in full. Confirm: `CustomerDetailTab` is exported; `View` no longer contains `"systems"`; `goToSystems` no longer exists anywhere in the file (only `goToCustomerDetail`); the default `customerDetailTab` is `"systeme"` (not `"uebersicht"` — this preserves every existing caller's current landing behavior for free, per the spec).

- [ ] **Step 5: Commit**

```bash
git add src/state/appStore.ts
git commit -m "refactor: replace goToSystems/systems view with goToCustomerDetail/customer-detail"
```

---

### Task 2: `AuditLogPanel.tsx` — opt-in non-collapsing "as a tab" mode

**Files:**
- Modify: `src/components/AuditLogPanel.tsx`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: a new optional prop `asTab?: boolean` on `AuditLogPanel`. Default (`false`/omitted) behavior is BYTE-IDENTICAL to today (returns `null` when there are zero entries; renders a collapsed `<details>` otherwise) — this is what `SystemForm.tsx` still relies on and this task must not change its behavior there. `asTab={true}` is what Task 3's new Verlauf tab will use: never returns `null`, never collapses, shows a plain "Noch keine Verlaufseinträge." message when empty.

The full current file:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface AuditLogEntry {
  id: number;
  entity_type: string;
  entity_id: number;
  action: string;
  summary: string;
  at_utc: string;
  at_tz: string;
}

export default function AuditLogPanel({ entityType, entityId }: { entityType: "customer" | "system"; entityId: number }) {
  const [entries, setEntries] = useState<AuditLogEntry[]>([]);

  useEffect(() => {
    invoke<AuditLogEntry[]>("list_audit_log_for_entity", { entityType, entityId }).then(setEntries);
  }, [entityType, entityId]);

  if (entries.length === 0) return null;

  return (
    <details style={{ marginTop: "0.5rem" }}>
      <summary style={{ cursor: "pointer", fontSize: "0.85rem", color: "var(--text-secondary)" }}>
        Verlauf ({entries.length})
      </summary>
      <ul style={{ listStyle: "none", padding: 0, margin: "0.4rem 0 0", fontSize: "0.8rem", color: "var(--text-secondary)" }}>
        {entries.map((e) => (
          <li key={e.id} style={{ padding: "0.2rem 0" }}>
            {new Date(e.at_utc).toLocaleString("de-DE")} — {e.summary}
          </li>
        ))}
      </ul>
    </details>
  );
}
```

- [ ] **Step 1: Add the `asTab` prop and its branch**

Replace the whole file with:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface AuditLogEntry {
  id: number;
  entity_type: string;
  entity_id: number;
  action: string;
  summary: string;
  at_utc: string;
  at_tz: string;
}

const listStyle = { listStyle: "none" as const, padding: 0, margin: "0.4rem 0 0", fontSize: "0.8rem", color: "var(--text-secondary)" };

// `asTab`: used by CustomerDetailView.tsx's Verlauf tab, where this panel
// fills a whole tab body rather than sitting collapsed among other content
// (SystemForm.tsx's modal usage, the default) -- a whole tab silently
// rendering nothing (the default `null`-when-empty behavior) would look
// broken, not "nothing to show yet", so this mode always renders a
// container and a plain "no entries" message instead of vanishing.
export default function AuditLogPanel({
  entityType,
  entityId,
  asTab = false,
}: {
  entityType: "customer" | "system";
  entityId: number;
  asTab?: boolean;
}) {
  const [entries, setEntries] = useState<AuditLogEntry[]>([]);

  useEffect(() => {
    invoke<AuditLogEntry[]>("list_audit_log_for_entity", { entityType, entityId }).then(setEntries);
  }, [entityType, entityId]);

  if (asTab) {
    if (entries.length === 0) {
      return <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>Noch keine Verlaufseinträge.</p>;
    }
    return (
      <ul style={listStyle}>
        {entries.map((e) => (
          <li key={e.id} style={{ padding: "0.3rem 0", borderBottom: "1px solid var(--border-subtle)" }}>
            {new Date(e.at_utc).toLocaleString("de-DE")} — {e.summary}
          </li>
        ))}
      </ul>
    );
  }

  if (entries.length === 0) return null;

  return (
    <details style={{ marginTop: "0.5rem" }}>
      <summary style={{ cursor: "pointer", fontSize: "0.85rem", color: "var(--text-secondary)" }}>
        Verlauf ({entries.length})
      </summary>
      <ul style={listStyle}>
        {entries.map((e) => (
          <li key={e.id} style={{ padding: "0.2rem 0" }}>
            {new Date(e.at_utc).toLocaleString("de-DE")} — {e.summary}
          </li>
        ))}
      </ul>
    </details>
  );
}
```

- [ ] **Step 2: Verify**

Run: `npm run build` and `npm run lint` from the repo root.
Expected: both clean. (This file's only current caller, `CustomerForm.tsx`, doesn't pass `asTab`, so it keeps using the default/unchanged branch — no other file needs to change in this task.)

- [ ] **Step 3: Commit**

```bash
git add src/components/AuditLogPanel.tsx
git commit -m "feat: add non-collapsing asTab mode to AuditLogPanel"
```

---

### Task 3: `CustomerDetailView.tsx` — new tabbed page, all 6 tabs

**Files:**
- Create: `src/components/CustomerDetailView.tsx`

**Interfaces:**
- Consumes: `useAppStore`'s `selectedCustomerId`, `customerDetailTab`, `setCustomerDetailTab`, `goToCustomers` (Task 1); `LocationsPanel` (prop: `customerId: number`), `ExpiringItemsPanel` (prop: `customerId: number`), `VaultPanel` (prop: `customerId: number`) — all three unchanged, already exist; `AuditLogPanel` with the new `asTab` prop (Task 2); `SystemListView` (no props, reads `selectedCustomerId` itself) — unchanged, already exists. Backend commands: `list_customers`, `update_customer` (both already used by `CustomerForm.tsx`), `list_systems_with_maintenance_status`, `list_expiring_items`, `list_locations_for_customer` (all already used by their respective existing panels/views).
- Produces: `export default function CustomerDetailView()` — a component taking no props, rendered by `App.tsx` (Task 4) whenever `view === "customer-detail"`.

This component does NOT handle "no customer selected" as a real-world reachable state — every navigation path that sets `view: "customer-detail"` also sets `selectedCustomerId` (either from an argument or from whatever was already selected), exactly like today's `SystemListView.tsx` already assumes. Follow that same existing convention: if `selectedCustomerId` is `null`, show the same "Kein Kunde ausgewählt" fallback `SystemListView.tsx` already has (copy its exact fallback block) rather than crashing.

- [ ] **Step 1: Write the file**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import type { CustomerDetailTab } from "../state/appStore";
import { formatInvokeError } from "../lib/errors";
import { isExpiringSoon } from "../lib/expiry";
import SystemListView from "./SystemListView";
import LocationsPanel from "./LocationsPanel";
import ExpiringItemsPanel from "./ExpiringItemsPanel";
import VaultPanel from "./VaultPanel";
import AuditLogPanel from "./AuditLogPanel";

interface Customer {
  id: number;
  name: string;
  short_code: string;
  notes: string;
}

// Local subset of SystemWithMaintenanceStatus (src-tauri/src/commands/systems.rs)
// -- only the fields this view's stat tile needs, same "local minimal
// interface" convention SystemListView.tsx's own System interface already
// uses for the same backend shape.
interface SystemStat {
  overdue: boolean;
}

// Local subset of ExpiringItem (src-tauri/src/db/expiring_items.rs) --
// matches DashboardView.tsx's own local interface for the same shape.
interface ExpiringItemStat {
  customer_id: number;
  expires_on: string;
  reminder_days_before: number;
}

// Same TabButton shape as SettingsView.tsx -- duplicated rather than
// extracted into a shared component, see this plan's Global Constraints.
function TabButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      style={{
        background: active ? "var(--bg-hover)" : "transparent",
        border: "1px solid " + (active ? "var(--border)" : "transparent"),
        color: active ? "var(--text-primary)" : "var(--text-secondary)",
        fontWeight: active ? 600 : 500,
      }}
    >
      {children}
    </button>
  );
}

// Same multi-word-initialism / single-word-truncation shortcode deriver as
// CustomerForm.tsx -- duplicated for the same reason TabButton is (small,
// self-contained, not worth a shared-utility file for one extra caller).
function deriveShortCode(rawName: string): string {
  const words = rawName.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "";
  if (words.length > 1) {
    return words
      .slice(0, 5)
      .map((w) => w[0])
      .join("")
      .toUpperCase();
  }
  return words[0]
    .replace(/[^a-zA-ZäöüÄÖÜß0-9]/g, "")
    .slice(0, 4)
    .toUpperCase();
}

function StatTile({ label, value }: { label: string; value: number | null }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "0.2rem",
        padding: "0.75rem 1rem",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-sm)",
        background: "var(--bg-surface)",
        minWidth: "8rem",
      }}
    >
      <span style={{ fontSize: "1.3rem", fontWeight: 600 }}>{value ?? "…"}</span>
      <span style={{ fontSize: "0.78rem", color: "var(--text-secondary)" }}>{label}</span>
    </div>
  );
}

function UebersichtTab({ customerId }: { customerId: number }) {
  const [name, setName] = useState("");
  const [shortCode, setShortCode] = useState("");
  const [notes, setNotes] = useState("");
  const [shortCodeTouched, setShortCodeTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const [systemCount, setSystemCount] = useState<number | null>(null);
  const [overdueCount, setOverdueCount] = useState<number | null>(null);
  const [expiringCount, setExpiringCount] = useState<number | null>(null);
  const [locationCount, setLocationCount] = useState<number | null>(null);

  useEffect(() => {
    setError(null);
    invoke<Customer[]>("list_customers", { includeArchived: true })
      .then((list) => {
        const match = list.find((c) => c.id === customerId);
        if (match) {
          setName(match.name);
          setShortCode(match.short_code);
          setNotes(match.notes);
          setShortCodeTouched(true);
        }
      })
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerId]);

  useEffect(() => {
    invoke<SystemStat[]>("list_systems_with_maintenance_status", { customerId, includeArchived: false })
      .then((systems) => {
        setSystemCount(systems.length);
        setOverdueCount(systems.filter((s) => s.overdue).length);
      })
      .catch(() => {
        setSystemCount(0);
        setOverdueCount(0);
      });
    invoke<ExpiringItemStat[]>("list_expiring_items")
      .then((items) => {
        const forThisCustomer = items.filter((i) => i.customer_id === customerId);
        setExpiringCount(forThisCustomer.filter((i) => isExpiringSoon(i.expires_on, i.reminder_days_before)).length);
      })
      .catch(() => setExpiringCount(0));
    invoke<unknown[]>("list_locations_for_customer", { customerId })
      .then((locations) => setLocationCount(locations.length))
      .catch(() => setLocationCount(0));
  }, [customerId]);

  async function handleSave() {
    setError(null);
    setBusy(true);
    try {
      await invoke("update_customer", { id: customerId, input: { name, short_code: shortCode, notes } });
      setSaved(true);
      window.setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1rem", maxWidth: "36rem" }}>
      <div style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
        <StatTile label="Systeme" value={systemCount} />
        <StatTile label="Überfällig" value={overdueCount} />
        <StatTile label="Bald ablaufend" value={expiringCount} />
        <StatTile label="Standorte" value={locationCount} />
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          void handleSave();
        }}
        style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}
      >
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Name
          <input
            value={name}
            onChange={(e) => {
              const value = e.target.value;
              setName(value);
              if (!shortCodeTouched) setShortCode(deriveShortCode(value));
            }}
            required
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Kürzel
          <input
            value={shortCode}
            onChange={(e) => {
              setShortCode(e.target.value);
              setShortCodeTouched(true);
            }}
            style={{ fontFamily: "var(--font-mono)" }}
            required
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Notizen
          <textarea value={notes} onChange={(e) => setNotes(e.target.value)} rows={3} />
        </label>
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
        <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
          <button type="submit" className="btn-primary" disabled={busy}>
            Speichern
          </button>
          {saved && <span style={{ color: "var(--success)", fontSize: "0.85rem" }}>Gespeichert ✓</span>}
        </div>
      </form>
    </div>
  );
}

export default function CustomerDetailView() {
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const customerDetailTab = useAppStore((s) => s.customerDetailTab);
  const setCustomerDetailTab = useAppStore((s) => s.setCustomerDetailTab);

  if (selectedCustomerId === null) {
    return (
      <div>
        <p>Kein Kunde ausgewählt — zurück zur Kundenliste</p>
        <button onClick={goToCustomers}>Zurück zu Kunden</button>
      </div>
    );
  }

  const tabs: { id: CustomerDetailTab; label: string }[] = [
    { id: "uebersicht", label: "Übersicht" },
    { id: "systeme", label: "Systeme" },
    { id: "standorte", label: "Standorte" },
    { id: "ablauf", label: "Ablauf-Tracking" },
    { id: "zugangsdaten", label: "Zugangsdaten" },
    { id: "verlauf", label: "Verlauf" },
  ];

  return (
    <div>
      <button onClick={goToCustomers} style={{ marginBottom: "0.5rem" }}>
        ← Zurück zu Kunden
      </button>
      <div
        style={{
          display: "flex",
          gap: "0.4rem",
          marginBottom: "1rem",
          paddingBottom: "0.6rem",
          borderBottom: "1px solid var(--border)",
        }}
      >
        {tabs.map((tab) => (
          <TabButton key={tab.id} active={customerDetailTab === tab.id} onClick={() => setCustomerDetailTab(tab.id)}>
            {tab.label}
          </TabButton>
        ))}
      </div>
      {customerDetailTab === "uebersicht" && <UebersichtTab customerId={selectedCustomerId} />}
      {customerDetailTab === "systeme" && <SystemListView />}
      {customerDetailTab === "standorte" && <LocationsPanel customerId={selectedCustomerId} />}
      {customerDetailTab === "ablauf" && <ExpiringItemsPanel customerId={selectedCustomerId} />}
      {customerDetailTab === "zugangsdaten" && <VaultPanel customerId={selectedCustomerId} />}
      {customerDetailTab === "verlauf" && <AuditLogPanel entityType="customer" entityId={selectedCustomerId} asTab />}
    </div>
  );
}
```

- [ ] **Step 2: Verify**

Run: `npm run build` and `npm run lint`.
Expected: TypeScript errors remain in `App.tsx`, `CustomerListView.tsx`, `DashboardView.tsx`, `CommandPalette.tsx`, `useGlobalHotkeys.ts`, `CustomerForm.tsx` (all still referencing the now-removed `goToSystems`/`"systems"` — fixed in Tasks 4-7). `CustomerDetailView.tsx` itself must show ZERO errors — read the build output and confirm every reported error's file path is one of those six files, none in `CustomerDetailView.tsx` or `AuditLogPanel.tsx`.

- [ ] **Step 3: Commit**

```bash
git add src/components/CustomerDetailView.tsx
git commit -m "feat: add CustomerDetailView with 6 tabs (Übersicht/Systeme/Standorte/Ablauf/Zugangsdaten/Verlauf)"
```

---

### Task 4: `App.tsx` — mount `CustomerDetailView`, fix nav

**Files:**
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `CustomerDetailView` (Task 3, default export, no props).

- [ ] **Step 1: Import and wire the new view**

Add near the other view imports (after `import CustomerListView from "./components/CustomerListView";`):

```tsx
import CustomerDetailView from "./components/CustomerDetailView";
```

Remove the now-unused:

```tsx
import SystemListView from "./components/SystemListView";
```

(`SystemListView` is still used, just no longer imported directly by `App.tsx` — it's rendered from inside `CustomerDetailView.tsx` now, which imports it itself.)

- [ ] **Step 2: Fix the nav link and the view-render dispatch**

Change:

```tsx
        <NavLink active={view === "customers" || view === "systems"} onClick={goToCustomers}>
```

to:

```tsx
        <NavLink active={view === "customers" || view === "customer-detail"} onClick={goToCustomers}>
```

Change:

```tsx
        {view === "customers" && <CustomerListView />}
        {view === "systems" && <SystemListView />}
```

to:

```tsx
        {view === "customers" && <CustomerListView />}
        {view === "customer-detail" && <CustomerDetailView />}
```

- [ ] **Step 3: Verify**

Run: `npm run build` and `npm run lint`.
Expected: `App.tsx` shows zero errors. Errors should remain only in `CustomerListView.tsx`, `DashboardView.tsx`, `CommandPalette.tsx`, `useGlobalHotkeys.ts`, `CustomerForm.tsx`.

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat: mount CustomerDetailView, retire the systems nav route"
```

---

### Task 5: `CustomerForm.tsx` — narrow to create-only

**Files:**
- Modify: `src/components/CustomerForm.tsx`

**Interfaces:**
- Consumes: nothing new.
- Produces: `CustomerForm` keeps its existing default export and existing store dependency on `customerEditorTarget` (still `"new" | number | null`, type unchanged per the spec — only ever actually `"new"` or `null` after Task 6/7 land, but narrowing the TYPE itself is out of scope, see spec).

After this task, `CustomerForm.tsx` is ONLY ever opened with `customerEditorTarget === "new"` (every numeric-target caller is redirected in Tasks 6-7) — but this task doesn't depend on those; it just removes the modal's now-dead edit-mode machinery and the 4 embedded panels, which were always gated on `typeof customerEditorTarget === "number"` and will now simply never render (since nothing will ever set a numeric target once Tasks 6-7 land) even before those tasks run. Safe to do in either order; placed here to keep the "shrink the old modal" work together as one reviewable unit.

- [ ] **Step 1: Remove the edit-mode data-loading branch, the 4 embedded panels, and their now-unused imports**

Replace the whole file with:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { formatInvokeError } from "../lib/errors";
import Modal from "./Modal";

// Globally mounted (see App.tsx), same pattern as EntryEditor.tsx/ExportDialog.tsx:
// driven entirely by the store's customerEditorTarget rather than local per-view
// state, so it can be opened from anywhere (list row, keyboard shortcut, Command
// Palette) regardless of which view is currently active.
//
// Create-only: editing an existing customer's core fields now happens on the
// Übersicht tab of CustomerDetailView.tsx, and its Standorte/Ablauf/Zugangsdaten/
// Verlauf tabs replaced the panels this modal used to stack underneath the
// fields. `customerEditorTarget` can still technically be a number (the store's
// type is unchanged), but nothing in this codebase sets it to one anymore --
// every call site that used to open this modal in edit mode now calls
// goToCustomerDetail(id, "uebersicht") or goToCustomerDetail(id, "verlauf")
// instead.
export default function CustomerForm() {
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);
  const closeCustomerEditor = useAppStore((s) => s.closeCustomerEditor);
  const formOpenInStore = useAppStore((s) => s.formOpen);
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);

  const [name, setName] = useState("");
  const [shortCode, setShortCode] = useState("");
  const [notes, setNotes] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [shortCodeTouched, setShortCodeTouched] = useState(false);

  // Reset the form whenever the modal opens -- always create mode now, see
  // the module doc comment above.
  useEffect(() => {
    if (customerEditorTarget === null) return;
    setError(null);
    setName("");
    setShortCode("");
    setNotes("");
    setShortCodeTouched(false);
  }, [customerEditorTarget]);

  // Engage the global Esc-closing convention whenever the editor is open, and
  // disengage on close/unmount — same convention as EntryEditor.tsx.
  useEffect(() => {
    if (customerEditorTarget === null) return;
    openForm();
    return () => closeForm();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [customerEditorTarget]);

  // An Esc-triggered close flips formOpen to false via useGlobalHotkeys; mirror
  // that back into customerEditorTarget so this component's own state stays in sync.
  useEffect(() => {
    if (!formOpenInStore && customerEditorTarget !== null) {
      closeCustomerEditor();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formOpenInStore]);

  // Multi-word names become an initialism (e.g. "Musterfirma GmbH" -> "MG");
  // a single word is truncated to its first letters (e.g. "Musterfirma" ->
  // "MUST"). Deliberately simple -- collisions are caught by the backend's
  // uniqueness constraint on save, and the field always stays editable.
  function deriveShortCode(rawName: string): string {
    const words = rawName.trim().split(/\s+/).filter(Boolean);
    if (words.length === 0) return "";
    if (words.length > 1) {
      return words
        .slice(0, 5)
        .map((w) => w[0])
        .join("")
        .toUpperCase();
    }
    return words[0]
      .replace(/[^a-zA-ZäöüÄÖÜß0-9]/g, "")
      .slice(0, 4)
      .toUpperCase();
  }

  function cancel() {
    closeForm();
    closeCustomerEditor();
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    try {
      await invoke("create_customer", { input: { name, short_code: shortCode, notes } });
      closeForm();
      closeCustomerEditor();
    } catch (err) {
      setError(formatInvokeError(err));
    }
  }

  if (customerEditorTarget === null) return null;

  return (
    <Modal onClose={cancel}>
      <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "20rem" }}>
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Neuer Kunde</h2>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Name
          <input
            value={name}
            onChange={(e) => {
              const value = e.target.value;
              setName(value);
              if (!shortCodeTouched) {
                setShortCode(deriveShortCode(value));
              }
            }}
            required
            autoFocus
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Kürzel
          <input
            value={shortCode}
            onChange={(e) => {
              setShortCode(e.target.value);
              setShortCodeTouched(true);
            }}
            style={{ fontFamily: "var(--font-mono)" }}
            required
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Notizen
          <textarea value={notes} onChange={(e) => setNotes(e.target.value)} rows={3} />
        </label>
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
        <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end", marginTop: "0.25rem" }}>
          <button type="button" onClick={cancel}>
            Abbrechen
          </button>
          <button type="submit">Speichern</button>
        </div>
      </form>
    </Modal>
  );
}
```

- [ ] **Step 2: Verify**

Run: `npm run build` and `npm run lint`.
Expected: `CustomerForm.tsx` shows zero errors. Errors should remain only in `CustomerListView.tsx`, `DashboardView.tsx`, `CommandPalette.tsx`, `useGlobalHotkeys.ts`.

- [ ] **Step 3: Commit**

```bash
git add src/components/CustomerForm.tsx
git commit -m "refactor: narrow CustomerForm to create-only"
```

---

### Task 6: `CustomerListView.tsx` — retarget navigation and the edit action

**Files:**
- Modify: `src/components/CustomerListView.tsx`

**Interfaces:**
- Consumes: `goToCustomerDetail` (Task 1, replaces `goToSystems`).

Three call sites in this file, two behaviors:
1. **Pure rename** (navigate to systems list, unchanged behavior): the `Enter`-key handler (currently `selectCustomer(customer.id); goToSystems(customer.id);` around line 94-95), the row's `onClick` (line ~251-252), and the "Systeme →" button's `onClick` (line ~262-263). All three become `goToCustomerDetail(...)` with the exact same arguments — no tab argument needed, since the default (`"systeme"`) already matches today's behavior.
2. **Real behavior change** (used to open the edit modal, now opens the Übersicht tab): the `edit_selected` keybinding handler (currently `openCustomerEditor(customer.id);` around line 101) and the "Bearbeiten" button (currently `onClick={() => openCustomerEditor(c.id)}` around line 268). Both become `goToCustomerDetail(id, "uebersicht")`.

- [ ] **Step 1: Update the store hook import**

Change:

```tsx
  const goToSystems = useAppStore((s) => s.goToSystems);
```

to:

```tsx
  const goToCustomerDetail = useAppStore((s) => s.goToCustomerDetail);
```

- [ ] **Step 2: Fix the `Enter`-key handler (navigate, not edit)**

Change:

```tsx
      } else if (e.key === "Enter") {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          selectCustomer(customer.id);
          goToSystems(customer.id);
        }
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          openCustomerEditor(customer.id);
        }
      }
```

to:

```tsx
      } else if (e.key === "Enter") {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          selectCustomer(customer.id);
          goToCustomerDetail(customer.id);
        }
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          goToCustomerDetail(customer.id, "uebersicht");
        }
      }
```

Update the `useEffect` dependency array on the very next line (currently
`}, [customers, selectedIndex, formOpen, selectCustomer, goToSystems, openCustomerEditor]);`)
to:

```tsx
  }, [customers, selectedIndex, formOpen, selectCustomer, goToCustomerDetail]);
```

(`openCustomerEditor` is dropped from the dependency array since this
`useEffect` no longer calls it — but `openCustomerEditor` itself stays
imported/used elsewhere in this file, see Step 4, so do NOT remove the
`const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);`
line.)

- [ ] **Step 3: Fix the row click and "Systeme →" button (pure renames)**

Change both occurrences of:

```tsx
                selectCustomer(c.id);
                goToSystems(c.id);
```

to:

```tsx
                selectCustomer(c.id);
                goToCustomerDetail(c.id);
```

(There are two occurrences — the row `<span onClick={...}>` and the
"Systeme →" `<button onClick={...}>` right below it, both with this
exact two-line body per the file read earlier in this plan's research.)

- [ ] **Step 4: Fix the "Bearbeiten" button**

Change:

```tsx
              <button onClick={() => openCustomerEditor(c.id)}>Bearbeiten</button>
```

to:

```tsx
              <button onClick={() => goToCustomerDetail(c.id, "uebersicht")}>Bearbeiten</button>
```

`openCustomerEditor` is still used by the "+ Neuer Kunde" button
elsewhere in this file (`onClick={() => openCustomerEditor("new")}`) —
leave that one alone, it's still correct (create mode still goes through
`CustomerForm.tsx`).

- [ ] **Step 5: Verify**

Run: `npm run build` and `npm run lint`.
Expected: `CustomerListView.tsx` shows zero errors. Errors should remain
only in `DashboardView.tsx`, `CommandPalette.tsx`, `useGlobalHotkeys.ts`.

- [ ] **Step 6: Commit**

```bash
git add src/components/CustomerListView.tsx
git commit -m "refactor: retarget CustomerListView navigation to CustomerDetailView"
```

---

### Task 7: Remaining call sites — `DashboardView.tsx`, `CommandPalette.tsx`, `useGlobalHotkeys.ts`

**Files:**
- Modify: `src/components/DashboardView.tsx`
- Modify: `src/components/CommandPalette.tsx`
- Modify: `src/hooks/useGlobalHotkeys.ts`

**Interfaces:**
- Consumes: `goToCustomerDetail` (Task 1).

All changes in this task are mechanical renames EXCEPT one:
`CommandPalette.tsx`'s `"audit-log-customer"` command currently calls
`openCustomerEditor(selectedCustomerId)` to jump straight to a
customer's audit history (the modal's collapsed Verlauf section) — it
must become `goToCustomerDetail(selectedCustomerId, "verlauf")` (the
Verlauf TAB specifically, not the default Übersicht/Systeme tab), since
that command's entire purpose is "show me this customer's history right
now."

- [ ] **Step 1: `DashboardView.tsx`**

Change:

```tsx
  const goToSystems = useAppStore((s) => s.goToSystems);
```

to:

```tsx
  const goToCustomerDetail = useAppStore((s) => s.goToCustomerDetail);
```

Change the `openCustomerSystems` callback's body and dependency array:

```tsx
  const openCustomerSystems = useCallback(
    (customerId: number) => {
      selectCustomer(customerId);
      goToSystems(customerId);
    },
    [selectCustomer, goToSystems],
  );
```

to:

```tsx
  const openCustomerSystems = useCallback(
    (customerId: number) => {
      selectCustomer(customerId);
      goToCustomerDetail(customerId);
    },
    [selectCustomer, goToCustomerDetail],
  );
```

- [ ] **Step 2: `CommandPalette.tsx`**

Change:

```tsx
  const goToSystems = useAppStore((s) => s.goToSystems);
```

to:

```tsx
  const goToCustomerDetail = useAppStore((s) => s.goToCustomerDetail);
```

Change the `"goto-systems"` command:

```tsx
      cmds.push({ id: "goto-systems", label: "Zu Systemliste", shortcut: keymap.goto_systems, run: () => goToSystems() });
```

to:

```tsx
      cmds.push({ id: "goto-systems", label: "Zu Systemliste", shortcut: keymap.goto_systems, run: () => goToCustomerDetail() });
```

Change the `"import-systems-csv"` command's body:

```tsx
        run: () => {
          goToSystems();
          setPendingAction("import-systems-csv");
        },
```

(the one inside the `"import-systems-csv"` command, not the
`"import-customers-csv"` one just above it, which calls `goToCustomers`
and is untouched) to:

```tsx
        run: () => {
          goToCustomerDetail();
          setPendingAction("import-systems-csv");
        },
```

Change the `"audit-log-customer"` command's behavior (not just a rename
— see this task's intro):

```tsx
      cmds.push({
        id: "audit-log-customer",
        label: "Verlauf des ausgewählten Kunden",
        shortcut: "",
        run: () => openCustomerEditor(selectedCustomerId),
      });
```

to:

```tsx
      cmds.push({
        id: "audit-log-customer",
        label: "Verlauf des ausgewählten Kunden",
        shortcut: "",
        run: () => goToCustomerDetail(selectedCustomerId, "verlauf"),
      });
```

(`openCustomerEditor` stays used elsewhere in this file — the
`"new-customer"` command still calls `openCustomerEditor("new")`, and
the `"audit-log-system"` command right below this one still calls
`openSystemEditor(...)`, both unrelated to this change and both untouched.)

Update the `staticCommands` `useMemo`'s dependency array — find:

```tsx
  }, [
    selectedCustomerId,
    selectedSystemId,
    goToDashboard,
    goToCustomers,
    goToSystems,
    goToJournal,
```

and change `goToSystems,` to `goToCustomerDetail,` in place (same
position in the array).

Fix the directory-search result handler:

```tsx
        activate: () => {
          if (hit.kind === "customer") {
            selectCustomer(hit.id);
            goToCustomers();
          } else {
            selectCustomer(hit.customer_id);
            goToSystems(hit.customer_id);
            selectSystem(hit.id);
          }
          close();
        },
      })),
    [directoryHits, selectCustomer, goToCustomers, goToSystems, selectSystem, close],
```

to:

```tsx
        activate: () => {
          if (hit.kind === "customer") {
            selectCustomer(hit.id);
            goToCustomers();
          } else {
            selectCustomer(hit.customer_id);
            goToCustomerDetail(hit.customer_id);
            selectSystem(hit.id);
          }
          close();
        },
      })),
    [directoryHits, selectCustomer, goToCustomers, goToCustomerDetail, selectSystem, close],
```

- [ ] **Step 3: `useGlobalHotkeys.ts`**

Change:

```ts
  const goToSystems = useAppStore((s) => s.goToSystems);
```

to:

```ts
  const goToCustomerDetail = useAppStore((s) => s.goToCustomerDetail);
```

Change:

```ts
        } else if (view === "systems") {
          goToCustomers();
        }
```

to:

```ts
        } else if (view === "customer-detail") {
          goToCustomers();
        }
```

Change:

```ts
        } else if (e.key === systemsFollow && selectedCustomerId !== null) {
          e.preventDefault();
          goToSystems();
        }
```

to:

```ts
        } else if (e.key === systemsFollow && selectedCustomerId !== null) {
          e.preventDefault();
          goToCustomerDetail();
        }
```

Update the outer `useEffect`'s dependency array (the last line of the
hook, currently ending
`}, [formOpen, view, selectedCustomerId, selectedSystemId, goToCustomers, goToSystems, goToJournal, closeForm]);`)
to:

```ts
  }, [formOpen, view, selectedCustomerId, selectedSystemId, goToCustomers, goToCustomerDetail, goToJournal, closeForm]);
```

- [ ] **Step 4: Verify**

Run: `npm run build` and `npm run lint` from the repo root.
Expected: BOTH fully clean now — this is the task after which every
`goToSystems`/`"systems"`-view reference in the codebase has been
replaced. Grep the whole `src/` tree for `goToSystems` and for the
literal string `"systems"` used as a `View` value to double check
nothing was missed:

```bash
grep -rn "goToSystems" src/
grep -rn 'view === "systems"\|view: "systems"' src/
```

Both commands should return no matches.

- [ ] **Step 5: Commit**

```bash
git add src/components/DashboardView.tsx src/components/CommandPalette.tsx src/hooks/useGlobalHotkeys.ts
git commit -m "refactor: retarget remaining navigation call sites to CustomerDetailView"
```

---

## Manual verification (after all 7 tasks)

No automated test harness exists for this — after the final commit, this
needs a real click-through in the running app (not part of any single
task's own step, since it depends on every task having landed):

1. `Kunden` nav → click a customer row → lands on Systeme tab, systems
   list renders exactly as before.
2. Click through all 6 tabs — each renders its own content, no console
   errors, no blank/broken tab.
3. Übersicht tab: edit name, save, confirm the "Gespeichert ✓" flash and
   that `Kürzel` auto-derives from `Name` the same way `CustomerForm.tsx`
   used to (only when the Kürzel field hasn't been manually touched).
4. Stat tiles on Übersicht show real, correct numbers matching what each
   respective tab actually lists.
5. `Kunden` list → "+ Neuer Kunde" still opens the (now-smaller) modal,
   creating a customer still works and still lands back on the list.
6. `Kunden` list → "Bearbeiten" on an existing customer → lands on the
   Übersicht tab of that customer's detail page (not the old modal).
7. Command Palette → "Verlauf des ausgewählten Kunden" (with a customer
   selected) → lands directly on the Verlauf tab.
8. Keyboard: the `g` `s` (or whatever `goto_systems` is bound to)
   two-key shortcut still jumps to the current customer's Systeme tab;
   `Escape` from the customer detail page still returns to the customer
   list.
9. Dashboard → clicking an overdue-system row or a recent-entry row still
   lands on that customer's Systeme tab.
