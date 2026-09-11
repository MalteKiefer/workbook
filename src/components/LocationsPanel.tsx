import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";

interface Location {
  id: number;
  customer_id: number;
  name: string;
  address_line1: string;
  address_line2: string;
  postal_code: string;
  city: string;
  country: string;
  phone: string;
  notes: string;
}

interface FormState {
  name: string;
  address_line1: string;
  address_line2: string;
  postal_code: string;
  city: string;
  country: string;
  phone: string;
  notes: string;
}

const EMPTY_FORM: FormState = {
  name: "",
  address_line1: "",
  address_line2: "",
  postal_code: "",
  city: "",
  country: "",
  phone: "",
  notes: "",
};

export default function LocationsPanel({ customerId }: { customerId: number }) {
  const [locations, setLocations] = useState<Location[]>([]);
  const [editingId, setEditingId] = useState<number | "new" | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    invoke<Location[]>("list_locations_for_customer", { customerId })
      .then(setLocations)
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

  function startEdit(l: Location) {
    setEditingId(l.id);
    setForm({
      name: l.name,
      address_line1: l.address_line1,
      address_line2: l.address_line2,
      postal_code: l.postal_code,
      city: l.city,
      country: l.country,
      phone: l.phone,
      notes: l.notes,
    });
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
        await invoke("create_location", { input: { customer_id: customerId, ...form } });
      } else if (editingId !== null) {
        await invoke("update_location", { id: editingId, input: form });
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
    if (!window.confirm("Standort wirklich löschen?")) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("delete_location", { id });
      reload();
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <details style={{ marginTop: "0.5rem" }}>
      <summary style={{ cursor: "pointer", fontSize: "0.85rem", color: "var(--text-secondary)" }}>
        Standorte ({locations.length})
      </summary>
      <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem", marginTop: "0.4rem" }}>
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}

        {editingId === null && (
          <button type="button" onClick={startCreate} style={{ alignSelf: "flex-start" }}>
            + Neuer Standort
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
              Adresse Zeile 1
              <input
                value={form.address_line1}
                onChange={(e) => setForm({ ...form, address_line1: e.target.value })}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Adresse Zeile 2
              <input
                value={form.address_line2}
                onChange={(e) => setForm({ ...form, address_line2: e.target.value })}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              PLZ
              <input value={form.postal_code} onChange={(e) => setForm({ ...form, postal_code: e.target.value })} />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Ort
              <input value={form.city} onChange={(e) => setForm({ ...form, city: e.target.value })} />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Land
              <input value={form.country} onChange={(e) => setForm({ ...form, country: e.target.value })} />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Telefon
              <input value={form.phone} onChange={(e) => setForm({ ...form, phone: e.target.value })} />
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
                disabled={busy || form.name.trim() === ""}
              >
                Speichern
              </button>
            </div>
          </div>
        )}

        <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
          {locations.map((l) => (
            <li
              key={l.id}
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
                {l.name}
                {(l.postal_code || l.city) && (
                  <span style={{ color: "var(--text-muted)" }}> · {[l.postal_code, l.city].filter(Boolean).join(" ")}</span>
                )}
              </span>
              <span style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
                <button type="button" onClick={() => startEdit(l)} disabled={busy}>
                  Bearbeiten
                </button>
                <button type="button" onClick={() => void handleDelete(l.id)} disabled={busy}>
                  Löschen
                </button>
              </span>
            </li>
          ))}
        </ul>
      </div>
    </details>
  );
}
