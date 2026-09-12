import { useCallback, useEffect, useMemo, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { getKeymap, matchesBinding } from "../lib/keymap";
import { EXPIRING_ITEM_KIND_LABELS, isExpiringSoon } from "../lib/expiry";
import { formatInvokeError } from "../lib/errors";

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

// Kept in sync with src-tauri/src/db/expiring_items.rs::ExpiringItem.
interface ExpiringItem {
  id: number;
  customer_id: number;
  system_id: number | null;
  kind: string;
  label: string;
  expires_on: string;
  reminder_days_before: number;
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
// there are three sections (overdue systems, expiring items, recent entries)
// instead of CommandPalette's several, but the shape is identical.
type DashboardSection = "overdue" | "expiring" | "entries";

interface FlatItem {
  key: string;
  section: DashboardSection;
  // Precomputed position in the FLATTENED items array (not a per-section
  // index) so the three sections' independent <ul> renders can still share
  // one running index for highlighting -- expiringFlatItems continues
  // numbering where overdueItems left off, and entryItems continues where
  // expiringFlatItems left off (see their useMemo blocks below).
  globalIndex: number;
  render: () => ReactNode;
  activate: () => void;
}

export default function DashboardView() {
  const selectCustomer = useAppStore((s) => s.selectCustomer);
  const goToCustomerDetail = useAppStore((s) => s.goToCustomerDetail);
  // EntryEditor/CustomerForm/SystemForm are globally mounted (see App.tsx)
  // and can be open while Dashboard sits underneath -- guard on formOpen
  // exactly like CustomerListView.tsx/SystemListView.tsx do, so this view's
  // own j/k/Enter don't fire while an unrelated modal has focus.
  const formOpen = useAppStore((s) => s.formOpen);

  const [customers, setCustomers] = useState<Customer[] | null>(null);
  const [overdueSystems, setOverdueSystems] = useState<OverdueSystemDto[] | null>(null);
  const [expiringItems, setExpiringItems] = useState<ExpiringItem[] | null>(null);
  const [recentEntries, setRecentEntries] = useState<Entry[] | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [icsExportBusy, setIcsExportBusy] = useState(false);
  const [icsExportError, setIcsExportError] = useState<string | null>(null);

  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  useEffect(() => {
    invoke<OverdueSystemDto[]>("list_overdue_systems").then(setOverdueSystems);
  }, []);

  useEffect(() => {
    invoke<ExpiringItem[]>("list_expiring_items").then(setExpiringItems);
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

  async function handleExportIcs() {
    setIcsExportBusy(true);
    setIcsExportError(null);
    try {
      const destPath = await save({
        defaultPath: "wartungsdoku-kalender.ics",
        filters: [{ name: "iCalendar", extensions: ["ics"] }],
      });
      if (destPath === null) return;
      await invoke("export_calendar_ics", { destPath });
    } catch (e) {
      setIcsExportError(formatInvokeError(e));
    } finally {
      setIcsExportBusy(false);
    }
  }

  const openCustomerSystems = useCallback(
    (customerId: number) => {
      selectCustomer(customerId);
      goToCustomerDetail(customerId);
    },
    [selectCustomer, goToCustomerDetail],
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

  // Only the items actually inside their reminder window (maintenance.rs::is_expiring_soon's
  // frontend mirror) -- list_expiring_items itself returns every tracked item
  // regardless of date, same "backend returns everything, frontend narrows"
  // shape as recentEntriesToShow slicing list_entries' full result.
  const dueExpiringItems = useMemo(
    () =>
      (expiringItems ?? []).filter((item) => isExpiringSoon(item.expires_on, item.reminder_days_before)),
    [expiringItems],
  );

  const expiringFlatItems = useMemo<FlatItem[]>(
    () =>
      dueExpiringItems.map((item, i) => ({
        key: `expiring-${item.id}`,
        section: "expiring",
        // Continues numbering right after overdueItems, same convention
        // entryItems's own globalIndex uses below.
        globalIndex: overdueItems.length + i,
        render: () => (
          <>
            <span>
              {item.label}{" "}
              <span style={{ color: "var(--text-muted)" }}>
                ({EXPIRING_ITEM_KIND_LABELS[item.kind] ?? item.kind})
              </span>
            </span>
            <span style={{ fontSize: "0.8rem", color: "var(--danger)", whiteSpace: "nowrap" }}>
              {new Date(item.expires_on).toLocaleDateString("de-DE")}
            </span>
          </>
        ),
        activate: () => openCustomerSystems(item.customer_id),
      })),
    [dueExpiringItems, overdueItems.length, openCustomerSystems],
  );

  const entryItems = useMemo<FlatItem[]>(
    () =>
      recentEntriesToShow.map((entry, i) => ({
        key: `entry-${entry.id}`,
        section: "entries",
        // Continues numbering right after overdueItems AND expiringFlatItems
        // so all three sections form one continuous j/k sequence instead of
        // independent per-section selections.
        globalIndex: overdueItems.length + expiringFlatItems.length + i,
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
    [recentEntriesToShow, overdueItems.length, expiringFlatItems.length, customerNameById, openCustomerSystems],
  );

  const items = useMemo<FlatItem[]>(
    () => [...overdueItems, ...expiringFlatItems, ...entryItems],
    [overdueItems, expiringFlatItems, entryItems],
  );

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
        {customers === null || overdueSystems === null || expiringItems === null
          ? "Lade…"
          : `${customers.length} aktive Kunden · ${overdueSystems.length} Systeme überfällig · ${dueExpiringItems.length} bald ablaufend`}
      </p>

      <div style={{ marginTop: "0.75rem", display: "flex", alignItems: "center", gap: "0.5rem" }}>
        <button type="button" onClick={() => void handleExportIcs()} disabled={icsExportBusy}>
          Kalender exportieren (.ics)
        </button>
        {icsExportError && (
          <span style={{ color: "var(--danger)", fontSize: "0.82rem" }}>Fehler: {icsExportError}</span>
        )}
      </div>

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
        <h2 style={{ fontSize: "1rem", marginBottom: "0.5rem" }}>Bald ablaufend</h2>
        {expiringItems === null ? (
          <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade…</p>
        ) : expiringFlatItems.length === 0 ? (
          <p style={{ color: "var(--success)", fontSize: "0.85rem" }}>Nichts läuft demnächst ab.</p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {expiringFlatItems.map((item) => (
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
