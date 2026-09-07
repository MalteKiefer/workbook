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
