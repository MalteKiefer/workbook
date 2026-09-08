import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

interface System {
  id: number;
  customer_id: number;
  name: string;
}

interface Entry {
  id: number;
  customer_id: number;
  system_id: number | null;
  title: string;
  body_md: string;
  category: string;
  performed_at_utc: string;
  performed_at_tz: string;
  created_at_utc: string;
  created_at_tz: string;
  updated_at_utc: string;
  updated_at_tz: string;
  tags: string[];
}

interface EntryFilter {
  customer_id: number | null;
  system_id: number | null;
  category: string | null;
  tag: string | null;
  from_utc: string | null;
  to_utc: string | null;
}

// Kept local rather than shared with QuickCapture.tsx — five static entries
// aren't worth a shared module.
const CATEGORIES: { value: string; label: string }[] = [
  { value: "wartung", label: "Wartung" },
  { value: "stoerung", label: "Störung" },
  { value: "aenderung", label: "Änderung" },
  { value: "installation", label: "Installation" },
  { value: "sonstiges", label: "Sonstiges" },
];

const CATEGORY_LABELS: Record<string, string> = Object.fromEntries(CATEGORIES.map((c) => [c.value, c.label]));

// First-pass simplification: datetime-local has no timezone of its own. We
// treat the value as UTC by appending a trailing "Z" — good enough for filter
// bounds, not the spec's relative/absolute parse_temporal_input parser (that
// is for entry timestamps, not filter ranges). See phase4d plan doc.
function datetimeLocalToIsoUtc(value: string): string | null {
  if (!value) return null;
  return value.length === 16 ? `${value}:00Z` : `${value}Z`;
}

function bodyPreview(bodyMd: string): string {
  const flat = bodyMd.replace(/\s+/g, " ").trim();
  return flat.length > 100 ? `${flat.slice(0, 100)}…` : flat;
}

export default function JournalView() {
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [systems, setSystems] = useState<System[]>([]);
  const [customerId, setCustomerId] = useState<number | "">("");
  const [systemId, setSystemId] = useState<number | "">("");
  const [category, setCategory] = useState<string | "">("");
  const [tag, setTag] = useState("");
  const [fromInput, setFromInput] = useState("");
  const [toInput, setToInput] = useState("");

  const [entries, setEntries] = useState<Entry[]>([]);
  const [timestamps, setTimestamps] = useState<Record<number, string>>({});
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [expandedId, setExpandedId] = useState<number | null>(null);

  const formOpen = useAppStore((s) => s.formOpen);
  const openEntryEditor = useAppStore((s) => s.openEntryEditor);

  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  useEffect(() => {
    // Reset the system filter whenever the customer filter changes — a
    // previously selected system may not belong to the new customer.
    setSystemId("");
    if (customerId === "") {
      setSystems([]);
      return;
    }
    invoke<System[]>("list_systems", { customerId, includeArchived: false }).then(setSystems);
  }, [customerId]);

  const reload = useCallback(() => {
    const filter: EntryFilter = {
      customer_id: customerId === "" ? null : customerId,
      system_id: systemId === "" ? null : systemId,
      category: category === "" ? null : category,
      tag: tag.trim() === "" ? null : tag.trim(),
      from_utc: datetimeLocalToIsoUtc(fromInput),
      to_utc: datetimeLocalToIsoUtc(toInput),
    };
    invoke<Entry[]>("list_entries", { filter }).then((result) => {
      setEntries(result);
      setSelectedIndex(0);
      setExpandedId(null);
    });
  }, [customerId, systemId, category, tag, fromInput, toInput]);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    let cancelled = false;
    Promise.all(
      entries.map((entry) =>
        invoke<string>("format_timestamp_for_display", {
          utc: entry.performed_at_utc,
          tz: entry.performed_at_tz,
        }).then((display) => [entry.id, display] as const),
      ),
    ).then((pairs) => {
      if (cancelled) return;
      setTimestamps(Object.fromEntries(pairs));
    });
    return () => {
      cancelled = true;
    };
  }, [entries]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      if (e.key === "j") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, entries.length - 1));
      } else if (e.key === "k") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "e") {
        const entry = entries[selectedIndex];
        if (entry) {
          e.preventDefault();
          openEntryEditor(entry.id);
        }
      } else if (e.key === "Enter") {
        const entry = entries[selectedIndex];
        if (entry) {
          e.preventDefault();
          setExpandedId((current) => (current === entry.id ? null : entry.id));
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [entries, selectedIndex, formOpen, openEntryEditor]);

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "0.75rem" }}>
        <h1 style={{ fontSize: "1.1rem" }}>Journal</h1>
        <button className="btn-primary" onClick={() => openEntryEditor("new")}>
          + Neuer Eintrag
        </button>
      </div>
      <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap", marginBottom: "0.75rem" }}>
        <select value={customerId} onChange={(e) => setCustomerId(e.target.value === "" ? "" : Number(e.target.value))}>
          <option value="">Alle Kunden</option>
          {customers.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name} ({c.short_code})
            </option>
          ))}
        </select>
        <select
          value={systemId}
          onChange={(e) => setSystemId(e.target.value === "" ? "" : Number(e.target.value))}
          disabled={customerId === ""}
        >
          <option value="">Alle Systeme</option>
          {systems.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
        <select value={category} onChange={(e) => setCategory(e.target.value)}>
          <option value="">Alle Kategorien</option>
          {CATEGORIES.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>
        <input value={tag} onChange={(e) => setTag(e.target.value)} placeholder="Tag" style={{ width: "8rem" }} />
        <label style={{ display: "flex", alignItems: "center", gap: "0.3rem", fontSize: "0.85rem" }}>
          Von
          <input type="datetime-local" value={fromInput} onChange={(e) => setFromInput(e.target.value)} />
        </label>
        <label style={{ display: "flex", alignItems: "center", gap: "0.3rem", fontSize: "0.85rem" }}>
          Bis
          <input type="datetime-local" value={toInput} onChange={(e) => setToInput(e.target.value)} />
        </label>
      </div>

      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {entries.map((entry, i) => (
          <li
            key={entry.id}
            className="list-row"
            style={{
              padding: "0.5rem 0.6rem",
              background: i === selectedIndex ? "var(--bg-selected)" : "transparent",
              borderBottom: "1px solid var(--border-subtle)",
            }}
          >
            <div style={{ display: "flex", justifyContent: "space-between", gap: "0.5rem" }}>
              <span style={{ fontWeight: 600 }}>{entry.title}</span>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: "0.8rem", color: "var(--text-secondary)" }}>
                {timestamps[entry.id] ?? ""}
              </span>
            </div>
            <div style={{ display: "flex", justifyContent: "space-between", fontSize: "0.8rem", color: "var(--text-muted)" }}>
              <span>{CATEGORY_LABELS[entry.category] ?? entry.category}</span>
              <span>{entry.tags.join(", ")}</span>
            </div>
            {expandedId === entry.id ? (
              <pre
                style={{
                  whiteSpace: "pre-wrap",
                  fontFamily: "var(--font-mono)",
                  fontSize: "0.85rem",
                  background: "var(--bg-surface)",
                  border: "1px solid var(--border-subtle)",
                  borderRadius: "var(--radius-sm)",
                  padding: "0.5rem",
                  marginTop: "0.4rem",
                }}
              >
                {entry.body_md}
              </pre>
            ) : (
              <div style={{ fontSize: "0.85rem", color: "var(--text-secondary)" }}>{bodyPreview(entry.body_md)}</div>
            )}
          </li>
        ))}
        {entries.length === 0 && (
          <li style={{ padding: "0.5rem", color: "var(--text-muted)" }}>Keine Einträge gefunden.</li>
        )}
      </ul>
    </div>
  );
}
