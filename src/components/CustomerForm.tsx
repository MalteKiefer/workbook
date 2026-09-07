import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import Modal from "./Modal";

interface Customer {
  id: number;
  name: string;
  short_code: string;
  notes: string;
}

interface CustomerFormProps {
  customer: Customer | null;
  onDone: () => void;
}

export default function CustomerForm({ customer, onDone }: CustomerFormProps) {
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);
  const formOpenInStore = useAppStore((s) => s.formOpen);

  const [name, setName] = useState(customer?.name ?? "");
  const [shortCode, setShortCode] = useState(customer?.short_code ?? "");
  const [notes, setNotes] = useState(customer?.notes ?? "");
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
      if (customer) {
        await invoke("update_customer", { id: customer.id, input: { name, short_code: shortCode, notes } });
      } else {
        await invoke("create_customer", { input: { name, short_code: shortCode, notes } });
      }
      closeForm();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <Modal onClose={closeForm}>
      <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "20rem" }}>
        <h2 style={{ margin: 0, fontSize: "1rem" }}>{customer ? "Kunde bearbeiten" : "Neuer Kunde"}</h2>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} required autoFocus />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Kürzel
          <input value={shortCode} onChange={(e) => setShortCode(e.target.value)} required />
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
