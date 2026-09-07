import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../state/appStore";
import Modal from "./Modal";
import MarkdownEditor, { type MarkdownEditorHandle } from "./MarkdownEditor";
import AttachmentDropzone, { type PickedFile } from "./AttachmentDropzone";
import AttachmentList from "./AttachmentList";

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

interface Attachment {
  id: number;
  entry_id: number;
  sha256: string;
  original_filename: string;
  mime_type: string;
  size_bytes: number;
  created_at_utc: string;
  created_at_tz: string;
}

interface PendingAttachment {
  token: string;
  bytesBase64: string;
  filename: string;
  mimeType: string;
}

// Mirrors src-tauri/src/attachments/store.rs's relative_path_for(): deterministic,
// content-addressed naming — attachments/{first 2 hex chars}/{sha256}{.ext}. Used to
// build a Markdown image reference immediately after add_attachment_to_entry returns,
// without a second round-trip to ask the backend for the path it just computed.
function relativePathFor(sha256: string, originalFilename: string): string {
  const dotIndex = originalFilename.lastIndexOf(".");
  const ext = dotIndex >= 0 ? originalFilename.slice(dotIndex) : "";
  return `attachments/${sha256.slice(0, 2)}/${sha256}${ext}`;
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
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [pendingAttachments, setPendingAttachments] = useState<PendingAttachment[]>([]);

  const titleRef = useRef<HTMLInputElement>(null);
  const editorRef = useRef<MarkdownEditorHandle>(null);

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
      setAttachments([]);
      setPendingAttachments([]);
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
    setPendingAttachments([]);
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
    invoke<Attachment[]>("list_attachments_for_entry", { entryId: editorTarget })
      .then(setAttachments)
      .catch((e) => setError(String(e)));
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

  // Shared by drag&drop, the file-picker button, and clipboard paste. In edit
  // mode the entry already exists, so a picked/pasted file is attached
  // immediately via the backend (no "pending" state); in create mode the
  // entry doesn't exist yet, so files are held locally and resolved into real
  // attachments server-side when create_entry runs (same pending_attachments
  // mechanism the quick-capture window already uses).
  const handleFilesAdded = useCallback(
    async (files: PickedFile[]) => {
      for (const file of files) {
        if (typeof editorTarget === "number") {
          try {
            const attachment = await invoke<Attachment>("add_attachment_to_entry", {
              entryId: editorTarget,
              bytesBase64: file.bytesBase64,
              originalFilename: file.filename,
              mimeType: file.mimeType,
            });
            const relativePath = relativePathFor(attachment.sha256, attachment.original_filename);
            editorRef.current?.insertAtCursor(`![${file.filename}](${relativePath})`);
            setAttachments((prev) => [...prev, attachment]);
          } catch (e) {
            setError(String(e));
          }
        } else {
          const token = `pending:${crypto.randomUUID()}`;
          editorRef.current?.insertAtCursor(`![${file.filename}](${token})`);
          setPendingAttachments((prev) => [
            ...prev,
            { token, bytesBase64: file.bytesBase64, filename: file.filename, mimeType: file.mimeType },
          ]);
        }
      }
    },
    [editorTarget],
  );

  const handleEditorPaste = useCallback(
    (event: ClipboardEvent) => {
      const items = event.clipboardData?.items;
      if (!items) return;
      for (const item of Array.from(items)) {
        if (item.type.startsWith("image/")) {
          event.preventDefault();
          const file = item.getAsFile();
          if (!file) return;
          void file.arrayBuffer().then((buffer) => {
            const bytes = new Uint8Array(buffer);
            let binary = "";
            for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
            void handleFilesAdded([{ bytesBase64: btoa(binary), filename: "Screenshot.png", mimeType: item.type }]);
          });
          return;
        }
      }
    },
    [handleFilesAdded],
  );

  const handleAttachmentExport = useCallback(
    async (attachmentId: number) => {
      const attachment = attachments.find((a) => a.id === attachmentId);
      if (!attachment) return;
      try {
        const destPath = await saveFileDialog({ defaultPath: attachment.original_filename });
        if (destPath) {
          await invoke("copy_attachment_to", { attachmentId, destPath });
        }
      } catch (e) {
        setError(String(e));
      }
    },
    [attachments],
  );

  const handleAttachmentRemove = useCallback((attachmentId: number) => {
    invoke("remove_attachment", { attachmentId })
      .then(() => setAttachments((prev) => prev.filter((a) => a.id !== attachmentId)))
      .catch((e) => setError(String(e)));
  }, []);

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
            pending_attachments: pendingAttachments.map((p) => ({
              placeholder_token: p.token,
              bytes_base64: p.bytesBase64,
              original_filename: p.filename,
              mime_type: p.mimeType,
            })),
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
    pendingAttachments,
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
          <AttachmentDropzone onFilesAdded={handleFilesAdded}>
            <MarkdownEditor
              ref={editorRef}
              value={bodyMd}
              onChange={setBodyMd}
              onPaste={handleEditorPaste}
              placeholder="Markdown…"
            />
          </AttachmentDropzone>
        </label>

        {isEditMode && (
          <AttachmentList
            attachments={attachments}
            onOpen={(id) => void invoke("open_attachment", { attachmentId: id }).catch((e) => setError(String(e)))}
            onExport={handleAttachmentExport}
            onRemove={handleAttachmentRemove}
            resolveImageUrl={(attachment) => invoke<string>("read_attachment_data_url", { attachmentId: attachment.id })}
          />
        )}

        {pendingAttachments.length > 0 && (
          <p style={{ fontSize: "0.8rem", color: "#a0a0a0" }}>
            {pendingAttachments.length} Anhang/Anhänge werden beim Speichern hinzugefügt.
          </p>
        )}

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
