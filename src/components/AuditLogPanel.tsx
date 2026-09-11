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
