import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";
import { EXPIRING_ITEM_KIND_LABELS, EXPIRING_ITEM_KIND_OPTIONS } from "../lib/expiry";

// Kept in sync with src-tauri/src/db/expiring_items.rs::ExpiringItem.
interface ExpiringItem {
  id: number;
  customer_id: number;
  system_id: number | null;
  kind: string;
  label: string;
  expires_on: string;
  reminder_days_before: number;
  notes: string;
}

// Local subset of the full System shape (src-tauri/src/db/systems.rs) -- only
// what the optional system <select> needs, same "only the fields this view
// actually renders" convention EntryEditor.tsx's own local System interface
// uses.
interface System {
  id: number;
  name: string;
}

interface FormState {
  system_id: number | null;
  kind: string;
  label: string;
  expires_on: string;
  reminder_days_before: number;
  notes: string;
}

const EMPTY_FORM: FormState = {
  system_id: null,
  kind: "ssl_certificate",
  label: "",
  expires_on: "",
  reminder_days_before: 30,
  notes: "",
};

// Embedded directly in CustomerForm.tsx, only rendered in edit mode (i.e.
// once a customerId exists to scope it to) -- same conditional-embedding
// convention AuditLogPanel.tsx uses there. The create/edit/delete-list shape
// itself mirrors TemplatesSettingsView.tsx exactly (useState/invoke/
// formatInvokeError, window.confirm before delete).
export default function ExpiringItemsPanel({ customerId }: { customerId: number }) {
  const [items, setItems] = useState<ExpiringItem[]>([]);
  const [systems, setSystems] = useState<System[]>([]);
  const [editingId, setEditingId] = useState<number | "new" | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    // No dedicated list_for_customer command exists -- list_expiring_items
    // (db::expiring_items::list_all) is the only listing command, so this
    // filters client-side the same way DashboardView.tsx's cross-customer
    // Dashboard list consumes the very same flat, all-customers response.
    invoke<ExpiringItem[]>("list_expiring_items")
      .then((all) => setItems(all.filter((item) => item.customer_id === customerId)))
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerId]);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    invoke<System[]>("list_systems", { customerId, includeArchived: false })
      .then(setSystems)
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerId]);

  function startCreate() {
    setEditingId("new");
    setForm(EMPTY_FORM);
    setError(null);
  }

  function startEdit(item: ExpiringItem) {
    setEditingId(item.id);
    setForm({
      system_id: item.system_id,
      kind: item.kind,
      label: item.label,
      expires_on: item.expires_on,
      reminder_days_before: item.reminder_days_before,
      notes: item.notes,
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
        await invoke("create_expiring_item", { input: { customer_id: customerId, ...form } });
      } else if (editingId !== null) {
        await invoke("update_expiring_item", { id: editingId, input: form });
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
    if (!window.confirm("Eintrag wirklich löschen?")) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("delete_expiring_item", { id });
      reload();
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem", marginTop: "0.5rem" }}>
      <h3 style={{ margin: 0, fontSize: "0.9rem" }}>Ablaufende Elemente</h3>
      {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}

      {editingId === null && (
        <button type="button" onClick={startCreate} style={{ alignSelf: "flex-start" }}>
          + Neuer Eintrag
        </button>
      )}

      {editingId !== null && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "0.5rem",
            padding: "0.6rem",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            background: "var(--bg-surface)",
          }}
        >
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Art
            <select value={form.kind} onChange={(e) => setForm({ ...form, kind: e.target.value })}>
              {EXPIRING_ITEM_KIND_OPTIONS.map((k) => (
                <option key={k.value} value={k.value}>
                  {k.label}
                </option>
              ))}
            </select>
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Bezeichnung
            <input value={form.label} onChange={(e) => setForm({ ...form, label: e.target.value })} required />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            System (optional)
            <select
              value={form.system_id === null ? "" : String(form.system_id)}
              onChange={(e) =>
                setForm({ ...form, system_id: e.target.value === "" ? null : Number(e.target.value) })
              }
            >
              <option value="">Kein System</option>
              {systems.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Ablaufdatum
            <input
              type="date"
              value={form.expires_on}
              onChange={(e) => setForm({ ...form, expires_on: e.target.value })}
              required
            />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Erinnerung (Tage vorher)
            <input
              type="number"
              value={form.reminder_days_before}
              onChange={(e) => setForm({ ...form, reminder_days_before: Number(e.target.value) })}
              min={0}
            />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Notizen
            <textarea value={form.notes} onChange={(e) => setForm({ ...form, notes: e.target.value })} rows={2} />
          </label>
          <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end" }}>
            <button type="button" onClick={cancelEdit} disabled={busy}>
              Abbrechen
            </button>
            <button
              type="button"
              className="btn-primary"
              onClick={() => void handleSave()}
              disabled={busy || form.label.trim() === "" || form.expires_on.trim() === ""}
            >
              Speichern
            </button>
          </div>
        </div>
      )}

      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {items.map((item) => (
          <li
            key={item.id}
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "0.5rem",
              padding: "0.4rem 0",
              borderTop: "1px solid var(--border-subtle)",
              fontSize: "0.85rem",
            }}
          >
            <span>
              {item.label}{" "}
              <span style={{ color: "var(--text-muted)" }}>
                ({EXPIRING_ITEM_KIND_LABELS[item.kind] ?? item.kind} ·{" "}
                {new Date(item.expires_on).toLocaleDateString("de-DE")})
              </span>
            </span>
            <span style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
              <button type="button" onClick={() => startEdit(item)} disabled={busy}>
                Bearbeiten
              </button>
              <button type="button" onClick={() => void handleDelete(item.id)} disabled={busy}>
                Löschen
              </button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
