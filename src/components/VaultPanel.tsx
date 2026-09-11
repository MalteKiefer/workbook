import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";

interface VaultEntry {
  id: number;
  customer_id: number;
  system_id: number | null;
  label: string;
  username: string;
  secret: string | null;
  url: string;
  notes: string | null;
  created_at_utc: string;
  created_at_tz: string;
  updated_at_utc: string;
  updated_at_tz: string;
}

interface SystemOption {
  id: number;
  name: string;
}

interface FormState {
  system_id: number | null;
  label: string;
  username: string;
  secret: string;
  url: string;
  notes: string;
}

const EMPTY_FORM: FormState = {
  system_id: null,
  label: "",
  username: "",
  secret: "",
  url: "",
  notes: "",
};

// Embedded in CustomerForm.tsx (edit mode only, mirroring AuditLogPanel).
// Three states, checked in order: vault never set up -> setup form; set up
// but locked this session -> unlock form; unlocked -> entry list + inline
// create/edit form. `secret`/`notes` come back from the backend already
// decrypted (only while unlocked) -- the "Anzeigen" toggle here is purely a
// local show/hide of text already held in memory, not a second decrypt.
export default function VaultPanel({ customerId }: { customerId: number }) {
  const [hasPassphrase, setHasPassphrase] = useState<boolean | null>(null);
  const [unlocked, setUnlocked] = useState(false);
  const [passphrase, setPassphrase] = useState("");
  const [entries, setEntries] = useState<VaultEntry[]>([]);
  const [systems, setSystems] = useState<SystemOption[]>([]);
  const [editingId, setEditingId] = useState<number | "new" | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [revealedIds, setRevealedIds] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshStatus = useCallback(() => {
    invoke<boolean>("has_vault_passphrase")
      .then(setHasPassphrase)
      .catch((e) => setError(formatInvokeError(e)));
    invoke<boolean>("is_vault_unlocked")
      .then(setUnlocked)
      .catch((e) => setError(formatInvokeError(e)));
  }, []);

  useEffect(() => {
    refreshStatus();
  }, [refreshStatus]);

  const reloadEntries = useCallback(() => {
    invoke<VaultEntry[]>("list_vault_entries_for_customer", { customerId })
      .then(setEntries)
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerId]);

  useEffect(() => {
    if (!unlocked) return;
    reloadEntries();
    invoke<SystemOption[]>("list_systems", { customerId, includeArchived: false })
      .then(setSystems)
      .catch((e) => setError(formatInvokeError(e)));
  }, [unlocked, reloadEntries, customerId]);

  async function handleSetup(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await invoke("set_vault_passphrase", { passphrase });
      setPassphrase("");
      refreshStatus();
    } catch (err) {
      setError(formatInvokeError(err));
    } finally {
      setBusy(false);
    }
  }

  async function handleUnlock(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await invoke("unlock_vault", { passphrase });
      setPassphrase("");
      refreshStatus();
    } catch (err) {
      setError(formatInvokeError(err));
    } finally {
      setBusy(false);
    }
  }

  async function handleLock() {
    setBusy(true);
    setError(null);
    try {
      await invoke("lock_vault");
    } catch (err) {
      setError(formatInvokeError(err));
    } finally {
      setBusy(false);
      setUnlocked(false);
      setEntries([]);
      setEditingId(null);
      setForm(EMPTY_FORM);
      setRevealedIds(new Set());
    }
  }

  function startCreate() {
    setEditingId("new");
    setForm(EMPTY_FORM);
    setError(null);
  }

  function startEdit(entry: VaultEntry) {
    setEditingId(entry.id);
    setForm({
      system_id: entry.system_id,
      label: entry.label,
      username: entry.username,
      secret: entry.secret ?? "",
      url: entry.url,
      notes: entry.notes ?? "",
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
      const secret = form.secret.trim() === "" ? null : form.secret;
      const notes = form.notes.trim() === "" ? null : form.notes;
      if (editingId === "new") {
        await invoke("create_vault_entry", {
          input: {
            customer_id: customerId,
            system_id: form.system_id,
            label: form.label,
            username: form.username,
            secret,
            url: form.url,
            notes,
          },
        });
      } else if (editingId !== null) {
        await invoke("update_vault_entry", {
          id: editingId,
          input: {
            system_id: form.system_id,
            label: form.label,
            username: form.username,
            secret,
            url: form.url,
            notes,
          },
        });
      }
      setEditingId(null);
      setForm(EMPTY_FORM);
      reloadEntries();
    } catch (err) {
      setError(formatInvokeError(err));
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete(id: number) {
    if (!window.confirm("Zugangsdaten-Eintrag wirklich löschen?")) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("delete_vault_entry", { id });
      reloadEntries();
    } catch (err) {
      setError(formatInvokeError(err));
    } finally {
      setBusy(false);
    }
  }

  function toggleReveal(id: number) {
    setRevealedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  async function copySecret(secret: string) {
    try {
      await navigator.clipboard.writeText(secret);
    } catch (err) {
      setError(formatInvokeError(err));
    }
  }

  if (hasPassphrase === null) return null;

  return (
    <div style={{ marginTop: "0.5rem", display: "flex", flexDirection: "column", gap: "0.5rem" }}>
      <h3 style={{ margin: 0, fontSize: "0.9rem" }}>Zugangsdaten-Tresor</h3>
      {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}

      {!hasPassphrase && (
        <form onSubmit={(e) => void handleSetup(e)} style={{ display: "flex", flexDirection: "column", gap: "0.4rem" }}>
          <p style={{ fontSize: "0.82rem", color: "var(--text-secondary)", margin: 0 }}>
            Der Tresor speichert Zugangsdaten für diesen Kunden verschlüsselt. Lege ein Master-Passwort fest, um ihn
            einmalig einzurichten.
          </p>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Master-Passwort
            <input type="password" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} required autoFocus />
          </label>
          <button
            type="submit"
            className="btn-primary"
            disabled={busy || passphrase.trim() === ""}
            style={{ alignSelf: "flex-start" }}
          >
            Tresor einrichten
          </button>
        </form>
      )}

      {hasPassphrase && !unlocked && (
        <form onSubmit={(e) => void handleUnlock(e)} style={{ display: "flex", flexDirection: "column", gap: "0.4rem" }}>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Master-Passwort
            <input type="password" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} required autoFocus />
          </label>
          <button
            type="submit"
            className="btn-primary"
            disabled={busy || passphrase.trim() === ""}
            style={{ alignSelf: "flex-start" }}
          >
            Entsperren
          </button>
        </form>
      )}

      {hasPassphrase && unlocked && (
        <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            {editingId === null ? (
              <button type="button" className="btn-primary" onClick={startCreate} disabled={busy}>
                + Neuer Eintrag
              </button>
            ) : (
              <span />
            )}
            <button type="button" onClick={() => void handleLock()} disabled={busy}>
              Tresor sperren
            </button>
          </div>

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
                Bezeichnung
                <input value={form.label} onChange={(e) => setForm({ ...form, label: e.target.value })} required autoFocus />
              </label>
              <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
                System (optional)
                <select
                  value={form.system_id ?? ""}
                  onChange={(e) =>
                    setForm({ ...form, system_id: e.target.value === "" ? null : Number(e.target.value) })
                  }
                >
                  <option value="">-- kein System --</option>
                  {systems.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name}
                    </option>
                  ))}
                </select>
              </label>
              <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
                Benutzername
                <input value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })} />
              </label>
              <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
                Passwort / Secret
                <input
                  type="text"
                  value={form.secret}
                  onChange={(e) => setForm({ ...form, secret: e.target.value })}
                  style={{ fontFamily: "var(--font-mono)" }}
                />
              </label>
              <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
                URL
                <input value={form.url} onChange={(e) => setForm({ ...form, url: e.target.value })} />
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
                  disabled={busy || form.label.trim() === ""}
                >
                  Speichern
                </button>
              </div>
            </div>
          )}

          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {entries.map((entry) => (
              <li
                key={entry.id}
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
                  {entry.label}
                  {entry.username && <span style={{ color: "var(--text-muted)" }}> · {entry.username}</span>}
                  {entry.secret !== null && (
                    <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>
                      {" "}
                      · {revealedIds.has(entry.id) ? entry.secret : "••••••••"}
                    </span>
                  )}
                </span>
                <span style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
                  {entry.secret !== null && (
                    <>
                      <button type="button" onClick={() => toggleReveal(entry.id)} disabled={busy}>
                        {revealedIds.has(entry.id) ? "Verbergen" : "Anzeigen"}
                      </button>
                      <button type="button" onClick={() => void copySecret(entry.secret as string)} disabled={busy}>
                        Kopieren
                      </button>
                    </>
                  )}
                  <button type="button" onClick={() => startEdit(entry)} disabled={busy}>
                    Bearbeiten
                  </button>
                  <button type="button" onClick={() => void handleDelete(entry.id)} disabled={busy}>
                    Löschen
                  </button>
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
