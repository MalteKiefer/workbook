import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import Modal from "./Modal";

interface System {
  id: number;
  customer_id: number;
  name: string;
  system_type: string;
  hostname: string;
  ip_address: string;
  notes: string;
}

interface SystemFormProps {
  system: System | null;
  customerId: number;
  onDone: () => void;
}

export default function SystemForm({ system, customerId, onDone }: SystemFormProps) {
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);
  const formOpenInStore = useAppStore((s) => s.formOpen);

  const [name, setName] = useState(system?.name ?? "");
  const [systemType, setSystemType] = useState(system?.system_type ?? "");
  const [hostname, setHostname] = useState(system?.hostname ?? "");
  const [ipAddress, setIpAddress] = useState(system?.ip_address ?? "");
  const [notes, setNotes] = useState(system?.notes ?? "");
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
      if (system) {
        await invoke("update_system", {
          id: system.id,
          input: { name, system_type: systemType, hostname, ip_address: ipAddress, notes },
        });
      } else {
        await invoke("create_system", {
          input: { customer_id: customerId, name, system_type: systemType, hostname, ip_address: ipAddress, notes },
        });
      }
      closeForm();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <Modal onClose={closeForm}>
      <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "20rem" }}>
        <h2 style={{ margin: 0, fontSize: "1rem" }}>{system ? "System bearbeiten" : "Neues System"}</h2>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} required autoFocus />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Typ
          <input value={systemType} onChange={(e) => setSystemType(e.target.value)} required />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Hostname
          <input value={hostname} onChange={(e) => setHostname(e.target.value)} style={{ fontFamily: "monospace" }} />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          IP-Adresse
          <input value={ipAddress} onChange={(e) => setIpAddress(e.target.value)} style={{ fontFamily: "monospace" }} />
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
