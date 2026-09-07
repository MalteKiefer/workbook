import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import Modal from "./Modal";

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

interface System {
  id: number;
  customer_id: number;
  name: string;
}

interface Entry {
  id: number;
  customer_id: number;
  system_id: number | null;
  title: string;
  body_md: string;
  category: string;
  performed_at_utc: string;
  performed_at_tz: string;
  created_at_utc: string;
  created_at_tz: string;
  updated_at_utc: string;
  updated_at_tz: string;
  tags: string[];
}

// Kept local rather than shared — five static entries aren't worth a shared
// module (same convention as JournalView.tsx/QuickCapture.tsx).
const CATEGORIES: { value: string; label: string }[] = [
  { value: "wartung", label: "Wartung" },
  { value: "stoerung", label: "Störung" },
  { value: "aenderung", label: "Änderung" },
  { value: "installation", label: "Installation" },
  { value: "sonstiges", label: "Sonstiges" },
];

export default function EntryEditor() {
  const editorTarget = useAppStore((s) => s.editorTarget);
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const selectedSystemId = useAppStore((s) => s.selectedSystemId);
  const formOpen = useAppStore((s) => s.formOpen);
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);
  const closeEntryEditor = useAppStore((s) => s.closeEntryEditor);

  const [customers, setCustomers] = useState<Customer[]>([]);
  const [systems, setSystems] = useState<System[]>([]);
  const [customerId, setCustomerId] = useState<number | "">("");
  const [systemId, setSystemId] = useState<number | "">("");
  const [title, setTitle] = useState("");
  const [bodyMd, setBodyMd] = useState("");
  const [category, setCategory] = useState("wartung");
  const [tagsInput, setTagsInput] = useState("");
  const [performedAtInput, setPerformedAtInput] = useState("");
  const [performedAtUtc, setPerformedAtUtc] = useState("");
  const [performedAtTz, setPerformedAtTz] = useState("");
  const [performedAtPreview, setPerformedAtPreview] = useState("");
  const [loadingEntry, setLoadingEntry] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const titleRef = useRef<HTMLInputElement>(null);

  // Full customer list, once — used for both the create-mode select and the
  // disabled edit-mode select.
  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  // Refetch systems whenever the customer changes. Deliberately does NOT
  // reset systemId here — this effect also fires when customerId is set
  // programmatically (new-mode default seeding, edit-mode load), and a reset
  // here would clobber a systemId set moments earlier in the same flow. The
  // "reset system when customer changes" UX lives in the customer <select>'s
  // onChange instead, since that is the only place a genuinely user-driven
  // customer change happens. See plan doc for the full rationale.
  useEffect(() => {
    if (customerId === "") {
      setSystems([]);
      return;
    }
    invoke<System[]>("list_systems", { customerId, includeArchived: false }).then(setSystems);
  }, [customerId]);

  const refreshPreview = useCallback(async (utc: string, tz: string) => {
    try {
      const preview = await invoke<string>("format_timestamp_for_display", { utc, tz });
      setPerformedAtPreview(preview);
    } catch {
      setPerformedAtPreview("");
    }
  }, []);

  // Load/init whenever the editor is opened (or switched to a different target).
  useEffect(() => {
    if (editorTarget === null) return;

    setError(null);

    if (editorTarget === "new") {
      setTitle("");
      setBodyMd("");
      setCategory("wartung");
      setTagsInput("");
      setCustomerId(selectedCustomerId ?? "");
      setSystemId(selectedSystemId ?? "");
      setPerformedAtInput("");
      (async () => {
        try {
          const now = await invoke<{ utc: string; tz: string }>("parse_temporal_input", { input: "" });
          setPerformedAtUtc(now.utc);
          setPerformedAtTz(now.tz);
          await refreshPreview(now.utc, now.tz);
        } catch (e) {
          setError(String(e));
        }
      })();
      titleRef.current?.focus();
      return;
    }

    setLoadingEntry(true);
    invoke<Entry>("get_entry", { id: editorTarget })
      .then(async (entry) => {
        setTitle(entry.title);
        setBodyMd(entry.body_md);
        setCategory(entry.category);
        setTagsInput(entry.tags.join(", "));
        setCustomerId(entry.customer_id);
        setSystemId(entry.system_id ?? "");
        setPerformedAtInput("");
        setPerformedAtUtc(entry.performed_at_utc);
        setPerformedAtTz(entry.performed_at_tz);
        await refreshPreview(entry.performed_at_utc, entry.performed_at_tz);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoadingEntry(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorTarget]);

  // Engage the global Esc-closing convention (useGlobalHotkeys watches
  // formOpen) whenever the editor is open, and disengage on close/unmount.
  useEffect(() => {
    if (editorTarget === null) return;
    openForm();
    return () => closeForm();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorTarget]);

  // An Esc-triggered close flips formOpen to false via useGlobalHotkeys; mirror
  // that back into editorTarget so this component's own state stays in sync.
  useEffect(() => {
    if (!formOpen && editorTarget !== null) {
      closeEntryEditor();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formOpen]);

  async function handlePerformedAtBlur() {
    if (performedAtInput.trim() === "") return;
    try {
      const result = await invoke<{ utc: string; tz: string }>("parse_temporal_input", { input: performedAtInput });
      setPerformedAtUtc(result.utc);
      setPerformedAtTz(result.tz);
      await refreshPreview(result.utc, result.tz);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  const save = useCallback(async () => {
    if (editorTarget === null) return;
    if (customerId === "") {
      setError("Kunde ist erforderlich");
      return;
    }
    const tagNames = tagsInput
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    try {
      if (editorTarget === "new") {
        await invoke("create_entry", {
          input: {
            customer_id: customerId,
            system_id: systemId === "" ? null : systemId,
            title,
            body_md: bodyMd,
            category,
            performed_at_utc: performedAtUtc,
            performed_at_tz: performedAtTz,
            tag_names: tagNames,
            pending_attachments: [],
          },
        });
      } else {
        await invoke("update_entry", {
          id: editorTarget,
          input: {
            system_id: systemId === "" ? null : systemId,
            title,
            body_md: bodyMd,
            category,
            performed_at_utc: performedAtUtc,
            performed_at_tz: performedAtTz,
            tag_names: tagNames,
            pending_attachments: [],
          },
        });
      }
      closeForm();
      closeEntryEditor();
    } catch (e) {
      setError(String(e));
    }
  }, [
    editorTarget,
    customerId,
    systemId,
    title,
    bodyMd,
    category,
    performedAtUtc,
    performedAtTz,
    tagsInput,
    closeForm,
    closeEntryEditor,
  ]);

  const cancel = useCallback(() => {
    closeForm();
    closeEntryEditor();
  }, [closeForm, closeEntryEditor]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (editorTarget === null) return;
      if (e.ctrlKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [editorTarget, save]);

  if (editorTarget === null) return null;

  const isEditMode = typeof editorTarget === "number";

  return (
    <Modal onClose={closeForm}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          width: "40rem",
          maxWidth: "90vw",
          maxHeight: "85vh",
          overflowY: "auto",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>{isEditMode ? "Eintrag bearbeiten" : "Neuer Eintrag"}</h2>

        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Titel
          <input ref={titleRef} value={title} onChange={(e) => setTitle(e.target.value)} required autoFocus={!isEditMode} />
        </label>

        <div style={{ display: "flex", gap: "0.5rem" }}>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
            Kunde
            <select
              value={customerId}
              disabled={isEditMode}
              onChange={(e) => {
                const value = e.target.value === "" ? "" : Number(e.target.value);
                setCustomerId(value);
                setSystemId("");
              }}
              required
            >
              <option value="">Kunde wählen…</option>
              {customers.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name} ({c.short_code})
                </option>
              ))}
            </select>
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
            System
            <select value={systemId} onChange={(e) => setSystemId(e.target.value === "" ? "" : Number(e.target.value))}>
              <option value="">Kein System</option>
              {systems.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
            Kategorie
            <select value={category} onChange={(e) => setCategory(e.target.value)}>
              {CATEGORIES.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
          </label>
        </div>

        <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Tätigkeitszeitpunkt
            <input
              value={performedAtInput}
              onChange={(e) => setPerformedAtInput(e.target.value)}
              onBlur={handlePerformedAtBlur}
              placeholder="jetzt, -2h, gestern 9:15, 07.09.2026 14:32"
              style={{ width: "16rem" }}
            />
          </label>
          <span style={{ fontFamily: "monospace", fontSize: "0.85rem" }}>{performedAtPreview}</span>
        </div>

        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
          Beschreibung (Markdown)
          <textarea
            value={bodyMd}
            onChange={(e) => setBodyMd(e.target.value)}
            rows={15}
            style={{ flex: 1, fontFamily: "monospace" }}
          />
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Tags
          <input value={tagsInput} onChange={(e) => setTagsInput(e.target.value)} placeholder="Tags, durch Komma getrennt" />
        </label>

        {error && <p style={{ color: "crimson" }}>Fehler: {error}</p>}

        <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end" }}>
          <button type="button" onClick={cancel}>
            Abbrechen
          </button>
          <button type="submit" disabled={loadingEntry}>
            Speichern
          </button>
        </div>
      </form>
    </Modal>
  );
}
