import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import SystemForm from "./SystemForm";

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

  const [systems, setSystems] = useState<System[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [editing, setEditing] = useState<System | "new" | null>(null);
  const [customerName, setCustomerName] = useState<string | null>(null);

  const reload = useCallback(() => {
    if (selectedCustomerId === null) return;
    invoke<System[]>("list_systems", { customerId: selectedCustomerId, includeArchived: false }).then(setSystems);
  }, [selectedCustomerId]);

  useEffect(() => {
    reload();
  }, [reload]);

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
      if (e.key === "j") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, systems.length - 1));
      } else if (e.key === "k") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "e") {
        const system = systems[selectedIndex];
        if (system) {
          e.preventDefault();
          setEditing(system);
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [systems, selectedIndex, formOpen]);

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
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
        <div>
          <button onClick={goToCustomers} style={{ marginBottom: "0.4rem" }}>
            ← Zurück zu Kunden
          </button>
          <h1 style={{ fontSize: "1.1rem", margin: 0 }}>
            Systeme von {customerName ?? `Kunde #${selectedCustomerId}`}
          </h1>
        </div>
        <button onClick={() => setEditing("new")}>+ Neues System</button>
      </div>
      <ul style={{ listStyle: "none", padding: 0 }}>
        {systems.map((s, i) => (
          <li
            key={s.id}
            style={{
              padding: "0.3rem 0.5rem",
              background: i === selectedIndex ? "#2a2a2a" : "transparent",
              display: "flex",
              justifyContent: "space-between",
            }}
          >
            <span>
              {s.name} ({s.system_type}){" "}
              <span style={{ fontFamily: "monospace" }}>{s.hostname}</span>
              {s.ip_address && <span style={{ fontFamily: "monospace" }}> — {s.ip_address}</span>}
            </span>
            <span style={{ display: "flex", gap: "0.4rem" }}>
              <button onClick={() => setEditing(s)}>Bearbeiten</button>
              <button onClick={() => archive(s.id)}>Archivieren</button>
            </span>
          </li>
        ))}
      </ul>
      {editing !== null && (
        <SystemForm
          system={editing === "new" ? null : editing}
          customerId={selectedCustomerId}
          onDone={() => {
            setEditing(null);
            reload();
          }}
        />
      )}
    </div>
  );
}
