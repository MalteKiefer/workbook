import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../state/appStore";
import Modal from "./Modal";

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
  tags: string[];
}

interface Attachment {
  id: number;
  original_filename: string;
  mime_type: string;
  size_bytes: number;
}

interface Customer {
  id: number;
  name: string;
}

interface System {
  id: number;
  name: string;
}

const CATEGORY_LABELS: Record<string, string> = {
  wartung: "Wartung",
  stoerung: "Störung",
  aenderung: "Änderung",
  installation: "Installation",
  sonstiges: "Sonstiges",
};

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// Read-only view of an entry, reachable by double-clicking a row in
// JournalView.tsx. Deliberately separate from EntryEditor.tsx rather than
// just opening the edit form directly — the user asked for a view step
// first, edit only via an explicit "Bearbeiten" button, so a double-click
// can never accidentally put them in edit mode.
export default function EntryDetailModal() {
  const viewingEntryId = useAppStore((s) => s.viewingEntryId);
  const closeEntryDetail = useAppStore((s) => s.closeEntryDetail);
  const openEntryEditor = useAppStore((s) => s.openEntryEditor);
  const formOpenInStore = useAppStore((s) => s.formOpen);
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);

  const [entry, setEntry] = useState<Entry | null>(null);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [customerName, setCustomerName] = useState<string | null>(null);
  const [systemName, setSystemName] = useState<string | null>(null);
  const [performedAtDisplay, setPerformedAtDisplay] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (viewingEntryId === null) return;
    setError(null);
    setEntry(null);
    invoke<Entry>("get_entry", { id: viewingEntryId })
      .then(async (loaded) => {
        setEntry(loaded);
        const [customers, display] = await Promise.all([
          invoke<Customer[]>("list_customers", { includeArchived: true }),
          invoke<string>("format_timestamp_for_display", {
            utc: loaded.performed_at_utc,
            tz: loaded.performed_at_tz,
          }),
        ]);
        setCustomerName(customers.find((c) => c.id === loaded.customer_id)?.name ?? null);
        setPerformedAtDisplay(display);
        if (loaded.system_id !== null) {
          const systems = await invoke<System[]>("list_systems", { customerId: loaded.customer_id, includeArchived: true });
          setSystemName(systems.find((s) => s.id === loaded.system_id)?.name ?? null);
        } else {
          setSystemName(null);
        }
      })
      .catch((e) => setError(String(e)));
    invoke<Attachment[]>("list_attachments_for_entry", { entryId: viewingEntryId })
      .then(setAttachments)
      .catch(() => setAttachments([]));
  }, [viewingEntryId]);

  useEffect(() => {
    if (viewingEntryId === null) return;
    openForm();
    return () => closeForm();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewingEntryId]);

  useEffect(() => {
    if (!formOpenInStore && viewingEntryId !== null) {
      closeEntryDetail();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formOpenInStore]);

  function handleEdit() {
    if (entry === null) return;
    closeForm();
    closeEntryDetail();
    openEntryEditor(entry.id);
  }

  async function handleOpenAttachment(attachmentId: number) {
    try {
      await invoke("open_attachment", { attachmentId });
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleExportAttachment(attachmentId: number, filename: string) {
    try {
      const destPath = await saveFileDialog({ defaultPath: filename });
      if (destPath) {
        await invoke("copy_attachment_to", { attachmentId, destPath });
      }
    } catch (e) {
      setError(String(e));
    }
  }

  if (viewingEntryId === null) return null;

  return (
    <Modal onClose={closeEntryDetail}>
      <div style={{ display: "flex", flexDirection: "column", gap: "0.6rem", width: "36rem", maxWidth: "90vw", maxHeight: "85vh", overflowY: "auto" }}>
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
        {entry === null && !error && <p style={{ color: "var(--text-muted)", margin: 0 }}>Lädt…</p>}
        {entry && (
          <>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: "0.5rem" }}>
              <h2 style={{ margin: 0, fontSize: "1.05rem" }}>{entry.title}</h2>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: "0.8rem", color: "var(--text-secondary)", whiteSpace: "nowrap" }}>
                {performedAtDisplay}
              </span>
            </div>
            <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap", fontSize: "0.8rem", color: "var(--text-muted)" }}>
              <span>{customerName ?? `Kunde #${entry.customer_id}`}</span>
              {systemName && <span>· {systemName}</span>}
              <span>· {CATEGORY_LABELS[entry.category] ?? entry.category}</span>
              {entry.tags.length > 0 && <span>· {entry.tags.join(", ")}</span>}
            </div>
            <pre
              style={{
                whiteSpace: "pre-wrap",
                fontFamily: "var(--font-mono)",
                fontSize: "0.85rem",
                background: "var(--bg-surface)",
                border: "1px solid var(--border-subtle)",
                borderRadius: "var(--radius-sm)",
                padding: "0.6rem",
                margin: 0,
              }}
            >
              {entry.body_md}
            </pre>
            {attachments.length > 0 && (
              <div>
                <span style={{ fontSize: "0.78rem", color: "var(--text-secondary)", fontWeight: 500 }}>Anhänge</span>
                <ul style={{ listStyle: "none", padding: 0, margin: "0.3rem 0 0" }}>
                  {attachments.map((a) => (
                    <li
                      key={a.id}
                      style={{
                        display: "flex",
                        justifyContent: "space-between",
                        alignItems: "center",
                        gap: "0.5rem",
                        padding: "0.3rem 0",
                        borderTop: "1px solid var(--border-subtle)",
                      }}
                    >
                      <span style={{ fontFamily: "var(--font-mono)", fontSize: "0.82rem", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {a.original_filename} <span style={{ color: "var(--text-muted)" }}>({formatSize(a.size_bytes)})</span>
                      </span>
                      <span style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
                        <button type="button" onClick={() => handleOpenAttachment(a.id)}>
                          Öffnen
                        </button>
                        <button type="button" onClick={() => handleExportAttachment(a.id, a.original_filename)}>
                          Exportieren
                        </button>
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end", marginTop: "0.25rem" }}>
              <button type="button" onClick={closeEntryDetail}>
                Schließen
              </button>
              <button type="button" className="btn-primary" onClick={handleEdit}>
                Bearbeiten
              </button>
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
