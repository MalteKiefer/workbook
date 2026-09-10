import { useEffect, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";

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

export default function DashboardView() {
  const selectCustomer = useAppStore((s) => s.selectCustomer);
  const goToSystems = useAppStore((s) => s.goToSystems);

  const [customers, setCustomers] = useState<Customer[] | null>(null);
  const [overdueSystems, setOverdueSystems] = useState<OverdueSystemDto[] | null>(null);
  const [recentEntries, setRecentEntries] = useState<Entry[] | null>(null);

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

  function openCustomerSystems(customerId: number) {
    selectCustomer(customerId);
    goToSystems(customerId);
  }

  const customerNameById = new Map((customers ?? []).map((c) => [c.id, c.name]));
  const recentEntriesToShow = (recentEntries ?? []).slice(0, RECENT_ENTRY_COUNT);

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
        ) : overdueSystems.length === 0 ? (
          <p style={{ color: "var(--success)", fontSize: "0.85rem" }}>Keine überfälligen Wartungen.</p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {overdueSystems.map((s) => (
              <li
                key={s.system_id}
                className="list-row"
                onClick={() => openCustomerSystems(s.customer_id)}
                title="Systeme dieses Kunden öffnen"
                style={listRowStyle}
              >
                <span>
                  {s.system_name}{" "}
                  <span style={{ color: "var(--text-muted)" }}>({s.customer_name})</span>
                </span>
                <span style={{ fontSize: "0.8rem", color: "var(--danger)", whiteSpace: "nowrap" }}>
                  seit {formatOverdueSince(s.last_performed_at_utc)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section style={{ marginTop: "1.5rem" }}>
        <h2 style={{ fontSize: "1rem", marginBottom: "0.5rem" }}>Letzte Einträge</h2>
        {recentEntries === null || customers === null ? (
          <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade…</p>
        ) : recentEntriesToShow.length === 0 ? (
          <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>Keine Einträge vorhanden.</p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {recentEntriesToShow.map((entry) => (
              <li
                key={entry.id}
                className="list-row"
                onClick={() => openCustomerSystems(entry.customer_id)}
                title="Systeme dieses Kunden öffnen"
                style={listRowStyle}
              >
                <span>
                  {entry.title}{" "}
                  <span style={{ color: "var(--text-muted)" }}>
                    ({customerNameById.get(entry.customer_id) ?? `Kunde #${entry.customer_id}`})
                  </span>
                </span>
                <span style={{ fontSize: "0.8rem", color: "var(--text-secondary)", whiteSpace: "nowrap" }}>
                  {new Date(entry.performed_at_utc).toLocaleString("de-DE")}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
