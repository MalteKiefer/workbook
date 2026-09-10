import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { getKeymap, matchesBinding } from "../lib/keymap";

interface System {
  id: number;
  customer_id: number;
  name: string;
  system_type: string;
  hostname: string;
  ip_address: string;
  notes: string;
  archived_at_utc: string | null;
}

interface Customer {
  id: number;
  name: string;
}

export default function SystemListView() {
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const formOpen = useAppStore((s) => s.formOpen);
  const systemEditorTarget = useAppStore((s) => s.systemEditorTarget);
  const openSystemEditor = useAppStore((s) => s.openSystemEditor);

  const [systems, setSystems] = useState<System[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [customerName, setCustomerName] = useState<string | null>(null);

  const reload = useCallback(() => {
    if (selectedCustomerId === null) return;
    invoke<System[]>("list_systems", { customerId: selectedCustomerId, includeArchived: false }).then(setSystems);
  }, [selectedCustomerId]);

  useEffect(() => {
    reload();
  }, [reload]);

  // SystemForm is globally mounted and driven by the store, so this view
  // doesn't get an onDone callback — reload whenever its editor closes instead.
  const prevEditorTargetRef = useRef(systemEditorTarget);
  useEffect(() => {
    if (prevEditorTargetRef.current !== null && systemEditorTarget === null) {
      reload();
    }
    prevEditorTargetRef.current = systemEditorTarget;
  }, [systemEditorTarget, reload]);

  useEffect(() => {
    if (selectedCustomerId === null) {
      setCustomerName(null);
      return;
    }
    invoke<Customer[]>("list_customers", { includeArchived: true }).then((customers) => {
      const match = customers.find((c) => c.id === selectedCustomerId);
      setCustomerName(match ? match.name : null);
    });
  }, [selectedCustomerId]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, systems.length - 1));
      } else if (matchesBinding(e, keymap.list_prev)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const system = systems[selectedIndex];
        if (system && selectedCustomerId !== null) {
          e.preventDefault();
          openSystemEditor(system.id, selectedCustomerId);
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [systems, selectedIndex, formOpen, selectedCustomerId, openSystemEditor]);

  async function archive(id: number) {
    await invoke("archive_system", { id });
    reload();
  }

  if (selectedCustomerId === null) {
    return (
      <div>
        <p>Kein Kunde ausgewählt — zurück zur Kundenliste</p>
        <button onClick={goToCustomers}>Zurück zu Kunden</button>
      </div>
    );
  }

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", marginBottom: "0.75rem" }}>
        <div>
          <button onClick={goToCustomers} style={{ marginBottom: "0.5rem" }}>
            ← Zurück zu Kunden
          </button>
          <h1 style={{ fontSize: "1.1rem" }}>
            Systeme von {customerName ?? `Kunde #${selectedCustomerId}`}
          </h1>
        </div>
        <button className="btn-primary" onClick={() => openSystemEditor("new", selectedCustomerId)}>
          + Neues System
        </button>
      </div>
      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {systems.map((s, i) => (
          <li
            key={s.id}
            className="list-row"
            style={{
              padding: "0.5rem 0.6rem",
              background: i === selectedIndex ? "var(--bg-selected)" : "transparent",
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
            }}
          >
            <span>
              {s.name} <span style={{ color: "var(--text-muted)" }}>({s.system_type})</span>{" "}
              <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "0.85em" }}>
                {s.hostname}
              </span>
              {s.ip_address && (
                <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "0.85em" }}>
                  {" "}
                  — {s.ip_address}
                </span>
              )}
            </span>
            <span style={{ display: "flex", gap: "0.4rem" }}>
              <button onClick={() => openSystemEditor(s.id, selectedCustomerId)}>Bearbeiten</button>
              <button onClick={() => archive(s.id)}>Archivieren</button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
