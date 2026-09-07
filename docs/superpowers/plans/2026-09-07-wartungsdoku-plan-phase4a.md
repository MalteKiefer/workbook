# Wartungsdoku — Phase 4a: Tastatur-Grundgerüst + Kundenverwaltung — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the placeholder main-window `App.tsx` with a real, keyboard-first app
shell: a Zustand store holding navigation state, a global-hotkey hook handling
vim-style `g`-prefix sequences and `Esc`, and a fully working Kundenverwaltung (list,
create, edit, archive) as the first real view. This is **Phase 4a** of "Command Palette
und Navigation" — split because the palette itself (4c) only makes sense once there are
real views to jump *to*.

**Phase 4b** (Systemliste + CRUD) and **4c** (Command Palette, `Strg+K`) follow once this
is reviewed; **4d** (Journal, Filter, `Strg+N`, `?`-Übersicht) after that.

**Architecture:** Pure frontend — every backend command this phase needs
(`list_customers`, `create_customer`, `update_customer`, `archive_customer`) already
exists from Phase 2. No Rust changes, no new Tauri commands. List-local keys (`j`/`k`/
`Enter`/`e`) are handled by each list view's own keydown listener (only active while
that view is mounted); only genuinely cross-cutting bindings (`g`-sequences, `Esc`) live
in the one global hook — this keeps list views self-contained instead of routing every
keystroke through a giant central switch. Text-field suppression is one shared
`isTypingTarget()` predicate, used by both the global hook and every list view.

**Tech Stack:** `zustand` (state), no other new dependencies.

**Spec:** [docs/superpowers/specs/2026-09-07-wartungsdoku-design.md](../specs/2026-09-07-wartungsdoku-design.md)
**Phase 3b-3 plan (done):** [docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase3b3.md](2026-09-07-wartungsdoku-plan-phase3b3.md)

## Global Constraints

- Jede Funktion ohne Maus erreichbar — jede Aktion, die ein Button anbietet, muss auch
  über Tastatur/Fokus erreichbar sein (native `<button>`/`<input>`-Elemente erfüllen das
  bereits über die native Tab-Reihenfolge).
- Textfelder unterdrücken Einzeltasten-Bindings (`j`, `k`, `e`, `g`-Sequenzen);
  Modifier-Shortcuts bleiben aktiv (in 4a betrifft das nur `Esc`, das immer feuert).
- Fokus ist jederzeit sichtbar (Browser-Default-Outline bleibt unangetastet, wird nicht
  per CSS entfernt). Dialoge fangen den Fokus ein und geben ihn beim Schließen an das
  auslösende Element zurück.
- IPC-Feldnamen: `NewCustomer`/`UpdateCustomer` behalten snake_case-Feldnamen 1:1 im
  JSON (siehe Phase 3b-2-Notiz) — nur die Top-Level-Parameternamen des Commands selbst
  sind camelCased.

---

## Task 1: Store + globaler Hotkey-Hook

**Files:**
- Create: `src/state/appStore.ts`
- Create: `src/hooks/useGlobalHotkeys.ts`
- Modify: `src/App.tsx`

**Interfaces:**
- Produces: `useAppStore` (Zustand, Felder: `view`, `selectedCustomerId`,
  `selectedSystemId`, `formOpen`; Aktionen: `goToCustomers`, `goToSystems`, `goToJournal`,
  `selectCustomer`, `selectSystem`, `openForm`, `closeForm`),
  `pub function isTypingTarget(el: Element | null): boolean`, `useGlobalHotkeys()`

- [ ] **Step 1: Zustand installieren**

```bash
npm install zustand
```

- [ ] **Step 2: `appStore.ts` schreiben**

```typescript
// src/state/appStore.ts
import { create } from "zustand";

export type View = "customers" | "systems" | "journal";

interface AppState {
  view: View;
  selectedCustomerId: number | null;
  selectedSystemId: number | null;
  formOpen: boolean;
  goToCustomers: () => void;
  goToSystems: (customerId?: number) => void;
  goToJournal: () => void;
  selectCustomer: (id: number | null) => void;
  selectSystem: (id: number | null) => void;
  openForm: () => void;
  closeForm: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  view: "customers",
  selectedCustomerId: null,
  selectedSystemId: null,
  formOpen: false,
  goToCustomers: () => set({ view: "customers" }),
  goToSystems: (customerId) =>
    set((state) => ({
      view: "systems",
      selectedCustomerId: customerId ?? state.selectedCustomerId,
    })),
  goToJournal: () => set({ view: "journal" }),
  selectCustomer: (id) => set({ selectedCustomerId: id }),
  selectSystem: (id) => set({ selectedSystemId: id }),
  openForm: () => set({ formOpen: true }),
  closeForm: () => set({ formOpen: false }),
}));
```

- [ ] **Step 3: `useGlobalHotkeys.ts` schreiben**

```typescript
// src/hooks/useGlobalHotkeys.ts
import { useEffect, useRef } from "react";
import { useAppStore } from "../state/appStore";

export function isTypingTarget(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || (el as HTMLElement).isContentEditable;
}

export function useGlobalHotkeys() {
  const pendingPrefixRef = useRef<string | null>(null);
  const pendingTimeoutRef = useRef<number | null>(null);

  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const goToSystems = useAppStore((s) => s.goToSystems);
  const goToJournal = useAppStore((s) => s.goToJournal);
  const formOpen = useAppStore((s) => s.formOpen);
  const closeForm = useAppStore((s) => s.closeForm);
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const view = useAppStore((s) => s.view);

  useEffect(() => {
    function clearPrefix() {
      pendingPrefixRef.current = null;
      if (pendingTimeoutRef.current !== null) {
        window.clearTimeout(pendingTimeoutRef.current);
        pendingTimeoutRef.current = null;
      }
    }

    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (formOpen) {
          e.preventDefault();
          closeForm();
        } else if (view === "systems") {
          goToCustomers();
        }
        clearPrefix();
        return;
      }

      if (isTypingTarget(document.activeElement) || e.ctrlKey || e.metaKey || e.altKey) {
        return;
      }

      if (pendingPrefixRef.current === "g") {
        clearPrefix();
        if (e.key === "c") {
          e.preventDefault();
          goToCustomers();
        } else if (e.key === "s" && selectedCustomerId !== null) {
          e.preventDefault();
          goToSystems();
        } else if (e.key === "j") {
          e.preventDefault();
          goToJournal();
        }
        return;
      }

      if (e.key === "g") {
        pendingPrefixRef.current = "g";
        pendingTimeoutRef.current = window.setTimeout(clearPrefix, 800);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      clearPrefix();
    };
  }, [formOpen, view, selectedCustomerId, goToCustomers, goToSystems, goToJournal, closeForm]);
}
```

- [ ] **Step 4: In `App.tsx` einhängen (Platzhalter-Ansichten, echte Views folgen in Task 4)**

```tsx
// src/App.tsx — komplett ersetzen
import { useGlobalHotkeys } from "./hooks/useGlobalHotkeys";
import { useAppStore } from "./state/appStore";

export default function App() {
  useGlobalHotkeys();
  const view = useAppStore((s) => s.view);

  return (
    <main style={{ fontFamily: "sans-serif", padding: "1rem" }}>
      {view === "customers" && <p>Kundenliste folgt (Task 4 dieser Phase).</p>}
      {view === "systems" && <p>Systemliste folgt (Phase 4b).</p>}
      {view === "journal" && <p>Journal folgt (Phase 4d).</p>}
    </main>
  );
}
```

- [ ] **Step 5: Build verifizieren**

```bash
npm run build
```

Expected: kein TypeScript-Fehler, `dist/` aktuell.

- [ ] **Step 6: Commit**

```bash
git add package.json package-lock.json src/state/appStore.ts src/hooks/useGlobalHotkeys.ts src/App.tsx
git commit -m "feat: add navigation store and global g-prefix/Esc hotkey handling"
```

---

## Task 2: Wiederverwendbarer Dialog (Fokusfalle + Fokus-Rückgabe)

**Files:**
- Create: `src/components/Modal.tsx`

**Interfaces:**
- Produces: `<Modal onClose={...}>{children}</Modal>` — fängt Tab-Fokus innerhalb ein,
  fokussiert beim Öffnen das erste Formularelement, gibt beim Schließen den Fokus an
  das zuvor aktive Element zurück.

- [ ] **Step 1: `Modal.tsx` schreiben**

```tsx
// src/components/Modal.tsx
import { useEffect, useRef } from "react";

interface ModalProps {
  onClose: () => void;
  children: React.ReactNode;
}

export default function Modal({ onClose, children }: ModalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const previouslyFocusedRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    previouslyFocusedRef.current = document.activeElement as HTMLElement | null;
    const firstField = containerRef.current?.querySelector<HTMLElement>("input, textarea, select, button");
    firstField?.focus();
    return () => {
      previouslyFocusedRef.current?.focus();
    };
  }, []);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key !== "Tab" || !containerRef.current) return;
      const focusable = containerRef.current.querySelectorAll<HTMLElement>(
        "input, textarea, select, button, [tabindex]:not([tabindex='-1'])",
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, []);

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.4)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 100,
      }}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div ref={containerRef} style={{ background: "#1e1e1e", color: "#e2e8f0", padding: "1rem", borderRadius: "6px" }}>
        {children}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Build verifizieren**

```bash
npm run build
```

- [ ] **Step 3: Commit**

```bash
git add src/components/Modal.tsx
git commit -m "feat: add reusable modal with focus trap and focus-return"
```

---

## Task 3: Kunden-Formular (Anlegen/Bearbeiten)

**Files:**
- Create: `src/components/CustomerForm.tsx`

**Interfaces:**
- Consumes: `create_customer`, `update_customer` (Phase 2), `Modal` (Task 2), `useAppStore.openForm/closeForm` (Task 1)
- Produces: `<CustomerForm customer={Customer | null} onDone={() => void} />`

- [ ] **Step 1: `CustomerForm.tsx` schreiben**

```tsx
// src/components/CustomerForm.tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import Modal from "./Modal";

interface Customer {
  id: number;
  name: string;
  short_code: string;
  notes: string;
}

interface CustomerFormProps {
  customer: Customer | null;
  onDone: () => void;
}

export default function CustomerForm({ customer, onDone }: CustomerFormProps) {
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);
  const formOpenInStore = useAppStore((s) => s.formOpen);

  const [name, setName] = useState(customer?.name ?? "");
  const [shortCode, setShortCode] = useState(customer?.short_code ?? "");
  const [notes, setNotes] = useState(customer?.notes ?? "");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    openForm();
    return () => closeForm();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!formOpenInStore) {
      onDone();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formOpenInStore]);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    try {
      if (customer) {
        await invoke("update_customer", { id: customer.id, input: { name, short_code: shortCode, notes } });
      } else {
        await invoke("create_customer", { input: { name, short_code: shortCode, notes } });
      }
      closeForm();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <Modal onClose={closeForm}>
      <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "20rem" }}>
        <h2 style={{ margin: 0, fontSize: "1rem" }}>{customer ? "Kunde bearbeiten" : "Neuer Kunde"}</h2>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} required autoFocus />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Kürzel
          <input value={shortCode} onChange={(e) => setShortCode(e.target.value)} required />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Notizen
          <textarea value={notes} onChange={(e) => setNotes(e.target.value)} rows={3} />
        </label>
        {error && <p style={{ color: "crimson" }}>Fehler: {error}</p>}
        <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end" }}>
          <button type="button" onClick={closeForm}>
            Abbrechen
          </button>
          <button type="submit">Speichern</button>
        </div>
      </form>
    </Modal>
  );
}
```

- [ ] **Step 2: Build verifizieren**

```bash
npm run build
```

- [ ] **Step 3: Commit**

```bash
git add src/components/CustomerForm.tsx
git commit -m "feat: add customer create/edit form"
```

---

## Task 4: Kundenliste-Ansicht

**Files:**
- Create: `src/components/CustomerListView.tsx`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `list_customers`, `archive_customer` (Phase 2), `CustomerForm` (Task 3),
  `isTypingTarget` (Task 1)

- [ ] **Step 1: `CustomerListView.tsx` schreiben**

```tsx
// src/components/CustomerListView.tsx
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import CustomerForm from "./CustomerForm";

interface Customer {
  id: number;
  name: string;
  short_code: string;
  notes: string;
  archived_at_utc: string | null;
}

export default function CustomerListView() {
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [editing, setEditing] = useState<Customer | "new" | null>(null);
  const selectCustomer = useAppStore((s) => s.selectCustomer);
  const goToSystems = useAppStore((s) => s.goToSystems);
  const formOpen = useAppStore((s) => s.formOpen);

  const reload = useCallback(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      if (e.key === "j") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, customers.length - 1));
      } else if (e.key === "k") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          selectCustomer(customer.id);
          goToSystems(customer.id);
        }
      } else if (e.key === "e") {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          setEditing(customer);
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [customers, selectedIndex, formOpen, selectCustomer, goToSystems]);

  async function archive(id: number) {
    await invoke("archive_customer", { id });
    reload();
  }

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <h1 style={{ fontSize: "1.1rem" }}>Kunden</h1>
        <button onClick={() => setEditing("new")}>+ Neuer Kunde</button>
      </div>
      <ul style={{ listStyle: "none", padding: 0, fontFamily: "monospace" }}>
        {customers.map((c, i) => (
          <li
            key={c.id}
            style={{
              padding: "0.3rem 0.5rem",
              background: i === selectedIndex ? "#2a2a2a" : "transparent",
              display: "flex",
              justifyContent: "space-between",
            }}
          >
            <span>
              {c.name} ({c.short_code})
            </span>
            <span style={{ display: "flex", gap: "0.4rem" }}>
              <button onClick={() => setEditing(c)}>Bearbeiten</button>
              <button onClick={() => archive(c.id)}>Archivieren</button>
            </span>
          </li>
        ))}
      </ul>
      {editing !== null && (
        <CustomerForm
          customer={editing === "new" ? null : editing}
          onDone={() => {
            setEditing(null);
            reload();
          }}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 2: In `App.tsx` einhängen**

```tsx
// src/App.tsx — komplett ersetzen
import { useGlobalHotkeys } from "./hooks/useGlobalHotkeys";
import { useAppStore } from "./state/appStore";
import CustomerListView from "./components/CustomerListView";

export default function App() {
  useGlobalHotkeys();
  const view = useAppStore((s) => s.view);

  return (
    <main style={{ fontFamily: "sans-serif", padding: "1rem" }}>
      {view === "customers" && <CustomerListView />}
      {view === "systems" && <p>Systemliste folgt (Phase 4b).</p>}
      {view === "journal" && <p>Journal folgt (Phase 4d).</p>}
    </main>
  );
}
```

- [ ] **Step 3: Build verifizieren**

```bash
npm run build
```

Expected: kein TypeScript-Fehler.

- [ ] **Step 4: Commit**

```bash
git add src/components/CustomerListView.tsx src/App.tsx
git commit -m "feat: add customer list view with keyboard navigation and CRUD"
```

---

## Task 5: End-to-End-Verifikation

- [ ] **Step 1: Rust-Testsuite unverändert grün (Kontrolle, dass diese Phase backend-frei blieb)**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo test 2>&1 | grep "test result" && cd ..
```

Expected: weiterhin 59 Tests grün.

- [ ] **Step 2: App real starten, Boot-Log prüfen**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export WARTUNGSDOKU_DATA_DIR="$(pwd)/.smoke-test-data"
cd src-tauri
timeout 8 cargo run --bin wartungsdoku > ../smoke-test.log 2>&1
cd ..
cat smoke-test.log
```

Expected: kein Absturz.

- [ ] **Step 3: Aufräumen, Plan committen**

```bash
rm -rf .smoke-test-data smoke-test.log
git add docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase4a.md
git commit -m "docs: mark Phase 4a plan tasks complete"
```

**Bekannte Grenze:** Tastaturverhalten in der echten Anwendung (g c, j/k, e, Esc,
Fokusfalle im Formular) kann ich hier nicht interaktiv nachstellen — TypeScript-Build
und Code-Review sind die Absicherung dieser Phase, kein automatisierter UI-Test (die
Spec verlangt Tests nur für Datenschicht/Migrationen/Suche, nicht fürs Frontend).

---

## Self-Review (durchgeführt vor Ausführung)

- **Spec-Abdeckung**: `g c` Navigation ✓, `j`/`k` Listennavigation ✓, `Enter`
  auswählen/öffnen ✓, `e` bearbeiten ✓, `Esc` Ebene zurück/Dialog abbrechen ✓,
  Textfelder unterdrücken Einzeltasten-Bindings ✓, Fokus sichtbar (kein CSS-Override
  auf `:focus`) ✓, Dialog fängt Fokus + gibt ihn zurück ✓, jede Funktion ohne Maus
  erreichbar (native Buttons/Inputs) ✓.
- **Bewusst verschoben**: Systemliste (4b), Command Palette (4c), Journal + `Strg+N` +
  `?`-Übersicht (4d). Die Platzhalter-Texte in `App.tsx` für `systems`/`journal` sind
  kein unfertiger Code, sondern ein ehrlicher Zwischenzustand — die Navigation dorthin
  funktioniert bereits (`g s`, `g j` schalten `view` um), nur die Inhalte fehlen noch.
- **Platzhalter-Scan**: keine TBD/TODO in Funktionscode.
- **Typkonsistenz**: `Customer`-Interface in `CustomerForm.tsx` und
  `CustomerListView.tsx` verwendet dieselben Feldnamen wie `db::customers::Customer`
  (Phase 2); `formOpen`/`openForm`/`closeForm` aus dem Store (Task 1) werden in
  `CustomerForm` (Task 3) und im globalen Hotkey-Hook (Task 1) konsistent verwendet.
