import { useCallback, useEffect, useMemo, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { getKeymap, matchesBinding } from "../lib/keymap";

interface Customer {
  id: number;
  name: string;
}

// Kept in sync with src-tauri/src/commands/systems.rs::OverdueSystemDto.
interface OverdueSystemDto {
  system_id: number;
  system_name: string;
  customer_id: number;
  customer_name: string;
  last_performed_at_utc: string | null;
}

// Local subset of the full Entry shape (src-tauri/src/db/entries.rs) --
// same "only the fields this view actually renders" convention JournalView.tsx's
// own local System interface already uses.
interface Entry {
  id: number;
  customer_id: number;
  title: string;
  performed_at_utc: string;
}

// Mirrors JournalView.tsx's own reload() filter shape exactly -- every
// field null selects every entry, newest first (see db::entries::list's
// ORDER BY performed_at_utc DESC), of which we keep only the first 8.
interface EntryFilter {
  customer_id: number | null;
  system_id: number | null;
  category: string | null;
  tag: string | null;
  from_utc: string | null;
  to_utc: string | null;
}

const RECENT_ENTRY_COUNT = 8;

// Same synchronous de-DE formatting SystemListView.tsx's own
// formatOverdueSince() uses for this exact concept, adapted to take the
// timestamp directly rather than a whole System object -- OverdueSystemDto
// already carries last_performed_at_utc on its own.
function formatOverdueSince(lastPerformedAtUtc: string | null): string {
  if (lastPerformedAtUtc === null) return "Anlage des Systems";
  return new Date(lastPerformedAtUtc).toLocaleString("de-DE");
}

const listRowStyle: CSSProperties = {
  padding: "0.5rem 0.6rem",
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
  gap: "0.5rem",
  cursor: "pointer",
};

// Mirrors CommandPalette.tsx's FlatItem pattern: each row (regardless of
// which section it belongs to) is reduced to a render() function and an
// activate() callback, and every section's items get concatenated into one
// flat, keyboard-navigable sequence walked by a single selectedIndex. Here
// there are only two sections (overdue systems, recent entries) instead of
// CommandPalette's several, but the shape is identical.
type DashboardSection = "overdue" | "entries";

interface FlatItem {
  key: string;
  section: DashboardSection;
  // Precomputed position in the FLATTENED items array (not a per-section
  // index) so the two sections' independent <ul> renders can still share
  // one running index for highlighting -- entryItems continues numbering
  // where overdueItems left off (see its useMemo below).
  globalIndex: number;
  render: () => ReactNode;
  activate: () => void;
}

export default function DashboardView() {
  const selectCustomer = useAppStore((s) => s.selectCustomer);
  const goToSystems = useAppStore((s) => s.goToSystems);
  // EntryEditor/CustomerForm/SystemForm are globally mounted (see App.tsx)
  // and can be open while Dashboard sits underneath -- guard on formOpen
  // exactly like CustomerListView.tsx/SystemListView.tsx do, so this view's
  // own j/k/Enter don't fire while an unrelated modal has focus.
  const formOpen = useAppStore((s) => s.formOpen);

  const [customers, setCustomers] = useState<Customer[] | null>(null);
  const [overdueSystems, setOverdueSystems] = useState<OverdueSystemDto[] | null>(null);
  const [recentEntries, setRecentEntries] = useState<Entry[] | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);

  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  useEffect(() => {
    invoke<OverdueSystemDto[]>("list_overdue_systems").then(setOverdueSystems);
  }, []);

  useEffect(() => {
    // There's no dedicated "recent N globally" backend command -- reuse
    // list_entries with every filter field null (every entry, newest
    // first) and keep only the first RECENT_ENTRY_COUNT client-side.
    const filter: EntryFilter = {
      customer_id: null,
      system_id: null,
      category: null,
      tag: null,
      from_utc: null,
      to_utc: null,
    };
    invoke<Entry[]>("list_entries", { filter }).then(setRecentEntries);
  }, []);

  const openCustomerSystems = useCallback(
    (customerId: number) => {
      selectCustomer(customerId);
      goToSystems(customerId);
    },
    [selectCustomer, goToSystems],
  );

  const customerNameById = useMemo(
    () => new Map((customers ?? []).map((c) => [c.id, c.name])),
    [customers],
  );
  const recentEntriesToShow = useMemo(
    () => (recentEntries ?? []).slice(0, RECENT_ENTRY_COUNT),
    [recentEntries],
  );

  const overdueItems = useMemo<FlatItem[]>(
    () =>
      (overdueSystems ?? []).map((s, i) => ({
        key: `overdue-${s.system_id}`,
        section: "overdue",
        globalIndex: i,
        render: () => (
          <>
            <span>
              {s.system_name}{" "}
              <span style={{ color: "var(--text-muted)" }}>({s.customer_name})</span>
            </span>
            <span style={{ fontSize: "0.8rem", color: "var(--danger)", whiteSpace: "nowrap" }}>
              seit {formatOverdueSince(s.last_performed_at_utc)}
            </span>
          </>
        ),
        activate: () => openCustomerSystems(s.customer_id),
      })),
    [overdueSystems, openCustomerSystems],
  );

  const entryItems = useMemo<FlatItem[]>(
    () =>
      recentEntriesToShow.map((entry, i) => ({
        key: `entry-${entry.id}`,
        section: "entries",
        // Continues numbering right after overdueItems so both sections
        // form one continuous j/k sequence instead of two independent
        // per-section selections.
        globalIndex: overdueItems.length + i,
        render: () => (
          <>
            <span>
              {entry.title}{" "}
              <span style={{ color: "var(--text-muted)" }}>
                ({customerNameById.get(entry.customer_id) ?? `Kunde #${entry.customer_id}`})
              </span>
            </span>
            <span style={{ fontSize: "0.8rem", color: "var(--text-secondary)", whiteSpace: "nowrap" }}>
              {new Date(entry.performed_at_utc).toLocaleString("de-DE")}
            </span>
          </>
        ),
        activate: () => openCustomerSystems(entry.customer_id),
      })),
    [recentEntriesToShow, overdueItems.length, customerNameById, openCustomerSystems],
  );

  const items = useMemo<FlatItem[]>(() => [...overdueItems, ...entryItems], [overdueItems, entryItems]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, items.length - 1));
      } else if (matchesBinding(e, keymap.list_prev)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        // Dashboard has no per-row "edit" action (unlike CustomerListView/
        // SystemListView), so unlike those views' handlers there's no
        // edit_selected branch here -- just navigation. Guard against an
        // empty items array (freshly seeded database, nothing to select
        // yet) the same way CustomerListView.tsx's `if (customer)` does.
        const item = items[selectedIndex];
        if (item) {
          e.preventDefault();
          item.activate();
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [items, selectedIndex, formOpen]);

  return (
    <div>
      <h1 style={{ fontSize: "1.1rem" }}>Dashboard</h1>
      <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem", marginTop: "0.25rem" }}>
        {customers === null || overdueSystems === null
          ? "Lade…"
          : `${customers.length} aktive Kunden · ${overdueSystems.length} Systeme überfällig`}
      </p>

      <section style={{ marginTop: "1.5rem" }}>
        <h2 style={{ fontSize: "1rem", marginBottom: "0.5rem" }}>Überfällige Wartungen</h2>
        {overdueSystems === null ? (
          <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade…</p>
        ) : overdueItems.length === 0 ? (
          <p style={{ color: "var(--success)", fontSize: "0.85rem" }}>Keine überfälligen Wartungen.</p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {overdueItems.map((item) => (
              <li
                key={item.key}
                className="list-row"
                onClick={item.activate}
                title="Systeme dieses Kunden öffnen"
                style={{
                  ...listRowStyle,
                  background: item.globalIndex === selectedIndex ? "var(--bg-selected)" : "transparent",
                }}
              >
                {item.render()}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section style={{ marginTop: "1.5rem" }}>
        <h2 style={{ fontSize: "1rem", marginBottom: "0.5rem" }}>Letzte Einträge</h2>
        {recentEntries === null || customers === null ? (
          <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade…</p>
        ) : entryItems.length === 0 ? (
          <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>Keine Einträge vorhanden.</p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {entryItems.map((item) => (
              <li
                key={item.key}
                className="list-row"
                onClick={item.activate}
                title="Systeme dieses Kunden öffnen"
                style={{
                  ...listRowStyle,
                  background: item.globalIndex === selectedIndex ? "var(--bg-selected)" : "transparent",
                }}
              >
                {item.render()}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
