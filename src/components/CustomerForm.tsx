import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { formatInvokeError } from "../lib/errors";
import Modal from "./Modal";
import AuditLogPanel from "./AuditLogPanel";
import ExpiringItemsPanel from "./ExpiringItemsPanel";

interface Customer {
  id: number;
  name: string;
  short_code: string;
  notes: string;
}

// Globally mounted (see App.tsx), same pattern as EntryEditor.tsx/ExportDialog.tsx:
// driven entirely by the store's customerEditorTarget rather than local per-view
// state, so it can be opened from anywhere (list row, keyboard shortcut, Command
// Palette) regardless of which view is currently active.
export default function CustomerForm() {
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);
  const closeCustomerEditor = useAppStore((s) => s.closeCustomerEditor);
  const formOpenInStore = useAppStore((s) => s.formOpen);
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);

  const [name, setName] = useState("");
  const [shortCode, setShortCode] = useState("");
  const [notes, setNotes] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [shortCodeTouched, setShortCodeTouched] = useState(false);

  const isEditMode = typeof customerEditorTarget === "number";

  // Load/init whenever the editor is opened (or switched to a different target) —
  // same convention as EntryEditor.tsx's load/init effect.
  useEffect(() => {
    if (customerEditorTarget === null) return;
    setError(null);

    if (customerEditorTarget === "new") {
      setName("");
      setShortCode("");
      setNotes("");
      setShortCodeTouched(false);
      return;
    }

    invoke<Customer[]>("list_customers", { includeArchived: true })
      .then((list) => {
        const match = list.find((c) => c.id === customerEditorTarget);
        if (match) {
          setName(match.name);
          setShortCode(match.short_code);
          setNotes(match.notes);
          setShortCodeTouched(true);
        }
      })
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerEditorTarget]);

  // Engage the global Esc-closing convention whenever the editor is open, and
  // disengage on close/unmount — same convention as EntryEditor.tsx.
  useEffect(() => {
    if (customerEditorTarget === null) return;
    openForm();
    return () => closeForm();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [customerEditorTarget]);

  // An Esc-triggered close flips formOpen to false via useGlobalHotkeys; mirror
  // that back into customerEditorTarget so this component's own state stays in sync.
  useEffect(() => {
    if (!formOpenInStore && customerEditorTarget !== null) {
      closeCustomerEditor();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formOpenInStore]);

  // Multi-word names become an initialism (e.g. "Musterfirma GmbH" -> "MG");
  // a single word is truncated to its first letters (e.g. "Musterfirma" ->
  // "MUST"). Deliberately simple -- collisions are caught by the backend's
  // uniqueness constraint on save, and the field always stays editable.
  function deriveShortCode(rawName: string): string {
    const words = rawName.trim().split(/\s+/).filter(Boolean);
    if (words.length === 0) return "";
    if (words.length > 1) {
      return words
        .slice(0, 5)
        .map((w) => w[0])
        .join("")
        .toUpperCase();
    }
    return words[0]
      .replace(/[^a-zA-ZäöüÄÖÜß0-9]/g, "")
      .slice(0, 4)
      .toUpperCase();
  }

  function cancel() {
    closeForm();
    closeCustomerEditor();
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    try {
      if (isEditMode) {
        await invoke("update_customer", { id: customerEditorTarget, input: { name, short_code: shortCode, notes } });
      } else {
        await invoke("create_customer", { input: { name, short_code: shortCode, notes } });
      }
      closeForm();
      closeCustomerEditor();
    } catch (err) {
      setError(formatInvokeError(err));
    }
  }

  if (customerEditorTarget === null) return null;

  return (
    <Modal onClose={cancel}>
      <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "20rem" }}>
        <h2 style={{ margin: 0, fontSize: "1rem" }}>{isEditMode ? "Kunde bearbeiten" : "Neuer Kunde"}</h2>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Name
          <input
            value={name}
            onChange={(e) => {
              const value = e.target.value;
              setName(value);
              if (!shortCodeTouched) {
                setShortCode(deriveShortCode(value));
              }
            }}
            required
            autoFocus
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Kürzel
          <input
            value={shortCode}
            onChange={(e) => {
              setShortCode(e.target.value);
              setShortCodeTouched(true);
            }}
            style={{ fontFamily: "var(--font-mono)" }}
            required
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Notizen
          <textarea value={notes} onChange={(e) => setNotes(e.target.value)} rows={3} />
        </label>
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
        {typeof customerEditorTarget === "number" && <ExpiringItemsPanel customerId={customerEditorTarget} />}
        {typeof customerEditorTarget === "number" && <AuditLogPanel entityType="customer" entityId={customerEditorTarget} />}
        <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end", marginTop: "0.25rem" }}>
          <button type="button" onClick={cancel}>
            Abbrechen
          </button>
          <button type="submit">Speichern</button>
        </div>
      </form>
    </Modal>
  );
}
