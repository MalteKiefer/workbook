import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";

interface EntryTemplate {
  id: number;
  name: string;
  system_type: string;
  title: string;
  body_md: string;
  category: string;
  tags_csv: string;
}

const CATEGORY_OPTIONS: { value: string; label: string }[] = [
  { value: "wartung", label: "Wartung" },
  { value: "stoerung", label: "Störung" },
  { value: "aenderung", label: "Änderung" },
  { value: "installation", label: "Installation" },
  { value: "sonstiges", label: "Sonstiges" },
];

interface FormState {
  name: string;
  system_type: string;
  title: string;
  body_md: string;
  category: string;
  tags_csv: string;
}

const EMPTY_FORM: FormState = {
  name: "",
  system_type: "",
  title: "",
  body_md: "",
  category: "wartung",
  tags_csv: "",
};

export default function TemplatesSettingsView() {
  const [templates, setTemplates] = useState<EntryTemplate[]>([]);
  const [editingId, setEditingId] = useState<number | "new" | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    invoke<EntryTemplate[]>("list_entry_templates")
      .then(setTemplates)
      .catch((e) => setError(formatInvokeError(e)));
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  function startCreate() {
    setEditingId("new");
    setForm(EMPTY_FORM);
    setError(null);
  }

  function startEdit(t: EntryTemplate) {
    setEditingId(t.id);
    setForm({
      name: t.name,
      system_type: t.system_type,
      title: t.title,
      body_md: t.body_md,
      category: t.category,
      tags_csv: t.tags_csv,
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
        await invoke("create_entry_template", { input: form });
      } else if (editingId !== null) {
        await invoke("update_entry_template", { id: editingId, input: form });
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
    if (!window.confirm("Vorlage wirklich löschen?")) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("delete_entry_template", { id });
      reload();
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem", maxWidth: "36rem" }}>
      <p style={{ fontSize: "0.85rem", color: "var(--text-secondary)", margin: 0 }}>
        Vorlagen füllen Titel, Beschreibung, Kategorie und Tags eines neuen Journal-Eintrags vor.
        "Systemtyp" ist nur eine freie Beschriftung zur Einordnung in dieser Liste.
      </p>
      {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}

      {editingId === null && (
        <button type="button" className="btn-primary" onClick={startCreate} style={{ alignSelf: "flex-start" }}>
          + Neue Vorlage
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
            <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Systemtyp (optional)
            <input value={form.system_type} onChange={(e) => setForm({ ...form, system_type: e.target.value })} />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Titel
            <input value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Kategorie
            <select value={form.category} onChange={(e) => setForm({ ...form, category: e.target.value })}>
              {CATEGORY_OPTIONS.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Tags (kommagetrennt)
            <input value={form.tags_csv} onChange={(e) => setForm({ ...form, tags_csv: e.target.value })} />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Beschreibung
            <textarea
              value={form.body_md}
              onChange={(e) => setForm({ ...form, body_md: e.target.value })}
              rows={5}
              style={{ fontFamily: "var(--font-mono)", fontSize: "0.85rem" }}
            />
          </label>
          <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end" }}>
            <button type="button" onClick={cancelEdit} disabled={busy}>
              Abbrechen
            </button>
            <button type="button" className="btn-primary" onClick={() => void handleSave()} disabled={busy || form.name.trim() === ""}>
              Speichern
            </button>
          </div>
        </div>
      )}

      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {templates.map((t) => (
          <li
            key={t.id}
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "0.5rem",
              padding: "0.5rem 0",
              borderTop: "1px solid var(--border-subtle)",
            }}
          >
            <span style={{ fontSize: "0.85rem" }}>
              {t.name}
              {t.system_type && <span style={{ color: "var(--text-muted)" }}> · {t.system_type}</span>}
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
