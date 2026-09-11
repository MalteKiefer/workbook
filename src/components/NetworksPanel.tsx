import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";

interface Network {
  id: number;
  customer_id: number;
  name: string;
  cidr: string;
  notes: string;
}

interface FormState {
  name: string;
  cidr: string;
  notes: string;
}

const EMPTY_FORM: FormState = {
  name: "",
  cidr: "",
  notes: "",
};

export default function NetworksPanel({ customerId }: { customerId: number }) {
  const [networks, setNetworks] = useState<Network[]>([]);
  const [editingId, setEditingId] = useState<number | "new" | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    invoke<Network[]>("list_networks_for_customer", { customerId })
      .then(setNetworks)
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerId]);

  useEffect(() => {
    reload();
  }, [reload]);

  function startCreate() {
    setEditingId("new");
    setForm(EMPTY_FORM);
    setError(null);
  }

  function startEdit(n: Network) {
    setEditingId(n.id);
    setForm({ name: n.name, cidr: n.cidr, notes: n.notes });
    setError(null);
  }

  function cancelEdit() {
    setEditingId(null);
    setForm(EMPTY_FORM);
    setError(null);
  }

  async function handleSave() {
    setBusy(true);
    setError(null);
    try {
      if (editingId === "new") {
        await invoke("create_network", { input: { customer_id: customerId, ...form } });
      } else if (editingId !== null) {
        await invoke("update_network", { id: editingId, input: form });
      }
      setEditingId(null);
      setForm(EMPTY_FORM);
      reload();
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete(id: number) {
    if (!window.confirm("Netzwerk wirklich löschen?")) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("delete_network", { id });
      reload();
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
      {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}

      {editingId === null && (
        <button type="button" onClick={startCreate} style={{ alignSelf: "flex-start" }}>
          + Neues Netzwerk
        </button>
      )}

      {editingId !== null && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "0.5rem",
            padding: "0.75rem",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            background: "var(--bg-surface)",
          }}
        >
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Name
            <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required autoFocus />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            CIDR-Bereich
            <input
              value={form.cidr}
              onChange={(e) => setForm({ ...form, cidr: e.target.value })}
              placeholder="192.168.1.0/24"
              style={{ fontFamily: "var(--font-mono)" }}
              required
            />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Notizen
            <textarea value={form.notes} onChange={(e) => setForm({ ...form, notes: e.target.value })} rows={3} />
          </label>
          <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end" }}>
            <button type="button" onClick={cancelEdit} disabled={busy}>
              Abbrechen
            </button>
            <button
              type="button"
              className="btn-primary"
              onClick={() => void handleSave()}
              disabled={busy || form.name.trim() === "" || form.cidr.trim() === ""}
            >
              Speichern
            </button>
          </div>
        </div>
      )}

      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {networks.map((n) => (
          <li
            key={n.id}
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "0.5rem",
              padding: "0.4rem 0",
              borderTop: "1px solid var(--border-subtle)",
            }}
          >
            <span style={{ fontSize: "0.85rem" }}>
              {n.name} <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>{n.cidr}</span>
            </span>
            <span style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
              <button type="button" onClick={() => startEdit(n)} disabled={busy}>
                Bearbeiten
              </button>
              <button type="button" onClick={() => void handleDelete(n.id)} disabled={busy}>
                Löschen
              </button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
