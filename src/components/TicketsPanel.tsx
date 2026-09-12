import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";

interface Ticket {
  id: number;
  customer_id: number;
  system_id: number | null;
  title: string;
  description: string;
  status: string;
  time_spent_minutes: number;
}

interface SystemOption {
  id: number;
  name: string;
}

interface FormState {
  system_id: number | null;
  title: string;
  description: string;
  status: string;
  time_spent_minutes: number;
}

const EMPTY_FORM: FormState = {
  system_id: null,
  title: "",
  description: "",
  status: "open",
  time_spent_minutes: 0,
};

const STATUS_LABELS: Record<string, string> = {
  open: "Offen",
  in_progress: "In Bearbeitung",
  closed: "Geschlossen",
};

export default function TicketsPanel({ customerId }: { customerId: number }) {
  const [tickets, setTickets] = useState<Ticket[]>([]);
  const [systems, setSystems] = useState<SystemOption[]>([]);
  const [editingId, setEditingId] = useState<number | "new" | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    invoke<Ticket[]>("list_tickets_for_customer", { customerId })
      .then(setTickets)
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerId]);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    invoke<SystemOption[]>("list_systems", { customerId, includeArchived: false })
      .then(setSystems)
      .catch(() => setSystems([]));
  }, [customerId]);

  function systemName(systemId: number | null): string | null {
    if (systemId === null) return null;
    return systems.find((s) => s.id === systemId)?.name ?? `System #${systemId}`;
  }

  function startCreate() {
    setEditingId("new");
    setForm(EMPTY_FORM);
    setError(null);
  }

  function startEdit(t: Ticket) {
    setEditingId(t.id);
    setForm({
      system_id: t.system_id,
      title: t.title,
      description: t.description,
      status: t.status,
      time_spent_minutes: t.time_spent_minutes,
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
        await invoke("create_ticket", { input: { customer_id: customerId, ...form } });
      } else if (editingId !== null) {
        await invoke("update_ticket", { id: editingId, input: form });
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
    if (!window.confirm("Ticket wirklich löschen?")) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("delete_ticket", { id });
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
          + Neues Ticket
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
            Titel
            <input value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} required autoFocus />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Beschreibung
            <textarea
              value={form.description}
              onChange={(e) => setForm({ ...form, description: e.target.value })}
              rows={3}
            />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Status
            <select value={form.status} onChange={(e) => setForm({ ...form, status: e.target.value })}>
              <option value="open">Offen</option>
              <option value="in_progress">In Bearbeitung</option>
              <option value="closed">Geschlossen</option>
            </select>
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            System (optional)
            <select
              value={form.system_id ?? ""}
              onChange={(e) => setForm({ ...form, system_id: e.target.value === "" ? null : Number(e.target.value) })}
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
            Zeitaufwand (Minuten)
            <input
              type="number"
              min={0}
              step={1}
              value={form.time_spent_minutes}
              onChange={(e) => setForm({ ...form, time_spent_minutes: Number(e.target.value) })}
            />
          </label>
          <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end" }}>
            <button type="button" onClick={cancelEdit} disabled={busy}>
              Abbrechen
            </button>
            <button
              type="button"
              className="btn-primary"
              onClick={() => void handleSave()}
              disabled={busy || form.title.trim() === ""}
            >
              Speichern
            </button>
          </div>
        </div>
      )}

      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {tickets.map((t) => (
          <li
            key={t.id}
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
              {t.title}{" "}
              <span style={{ color: "var(--text-muted)" }}>
                ·{" "}
                {[
                  STATUS_LABELS[t.status] ?? t.status,
                  systemName(t.system_id),
                  t.time_spent_minutes > 0 ? `${t.time_spent_minutes} Min.` : null,
                ]
                  .filter((part): part is string => Boolean(part))
                  .join(" · ")}
              </span>
            </span>
            <span style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
              <button type="button" onClick={() => startEdit(t)} disabled={busy}>
                Bearbeiten
              </button>
              <button type="button" onClick={() => void handleDelete(t.id)} disabled={busy}>
                Löschen
              </button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
