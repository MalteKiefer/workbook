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
