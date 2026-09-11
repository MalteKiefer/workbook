import { useEffect, useState } from "react";

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

interface AttachmentListProps {
  attachments: Attachment[];
  onOpen: (attachmentId: number) => void;
  onExport: (attachmentId: number) => void;
  onRemove?: (attachmentId: number) => void;
  resolveImageUrl: (attachment: Attachment) => Promise<string>;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function isImage(attachment: Attachment): boolean {
  return attachment.mime_type.startsWith("image/");
}

function isPdf(attachment: Attachment): boolean {
  return attachment.mime_type === "application/pdf";
}

// Resolves and renders one image attachment as a bounded-height thumbnail. Only
// entry_id/sha256 (not the whole `attachment` object) are effect dependencies, so a
// parent re-render that passes a new-but-equivalent Attachment object for the same
// underlying file doesn't trigger a redundant resolve. No cross-render/cross-instance
// caching beyond that — see the plan doc's "Non-goal: resolveImageUrl caching/dedup".
function ImageThumbnail({
  attachment,
  resolveImageUrl,
}: {
  attachment: Attachment;
  resolveImageUrl: AttachmentListProps["resolveImageUrl"];
}) {
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setUrl(null);
    setError(false);
    resolveImageUrl(attachment)
      .then((resolved) => {
        if (!cancelled) setUrl(resolved);
      })
      .catch(() => {
        if (!cancelled) setError(true);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [attachment.id, attachment.sha256, resolveImageUrl]);

  if (error) {
    return (
      <p style={{ color: "var(--danger)", fontSize: "0.85rem", margin: "0 0 0.3rem" }}>
        Bild konnte nicht geladen werden: {attachment.original_filename}
      </p>
    );
  }
  if (!url) {
    return (
      <p style={{ fontSize: "0.85rem", color: "var(--text-muted)", margin: "0 0 0.3rem" }}>
        Lade {attachment.original_filename}…
      </p>
    );
  }
  return (
    <img
      src={url}
      alt={attachment.original_filename}
      style={{
        display: "block",
        maxWidth: "100%",
        maxHeight: "12rem",
        borderRadius: "4px",
        marginBottom: "0.3rem",
      }}
    />
  );
}

// Resolves and renders one PDF attachment as an inline preview, click-to-toggle
// rather than eager — a PDF can be much larger than a typical screenshot, and
// this list can show several attachments at once, so the full base64 data URL
// is only resolved once the admin explicitly asks to see it. Collapsing again
// drops the loaded data URL, freeing the memory.
function PdfPreview({
  attachment,
  resolveImageUrl,
}: {
  attachment: Attachment;
  resolveImageUrl: AttachmentListProps["resolveImageUrl"];
}) {
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(false);

  function toggle() {
    if (open) {
      setOpen(false);
      setUrl(null);
      return;
    }
    setOpen(true);
    setError(false);
    setLoading(true);
    resolveImageUrl(attachment)
      .then((resolved) => setUrl(resolved))
      .catch(() => setError(true))
      .finally(() => setLoading(false));
  }

  return (
    <div style={{ marginBottom: "0.3rem" }}>
      <button type="button" onClick={toggle} style={{ fontSize: "0.8rem" }}>
        {open ? "Vorschau schließen" : "Vorschau anzeigen"}
      </button>
      {open && loading && (
        <p style={{ fontSize: "0.85rem", color: "var(--text-muted)", margin: "0.3rem 0 0" }}>
          Lade {attachment.original_filename}…
        </p>
      )}
      {open && error && (
        <p style={{ color: "var(--danger)", fontSize: "0.85rem", margin: "0.3rem 0 0" }}>
          Vorschau konnte nicht geladen werden: {attachment.original_filename}
        </p>
      )}
      {open && url && (
        <iframe
          src={url}
          title={attachment.original_filename}
          style={{
            display: "block",
            width: "100%",
            height: "24rem",
            border: "1px solid var(--border-subtle)",
            borderRadius: "4px",
            marginTop: "0.3rem",
          }}
        />
      )}
    </div>
  );
}

// Generic, presentational attachment list for one entry. Receives already-fetched
// data and callbacks only — no Tauri `invoke` calls here, see the plan doc
// (docs/superpowers/plans/2026-09-07-wartungsdoku-plan-phase5-attachment-list.md)
// for the images-vs-non-images scope decision (single unified list, images
// included, in the order the `attachments` prop arrives in).
export default function AttachmentList({ attachments, onOpen, onExport, onRemove, resolveImageUrl }: AttachmentListProps) {
  if (attachments.length === 0) {
    return null;
  }

  return (
    <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
      {attachments.map((a) => (
        <li
          key={a.id}
          style={{
            padding: "0.4rem 0.5rem",
            borderBottom: "1px solid var(--border-subtle)",
          }}
        >
          {isImage(a) && <ImageThumbnail attachment={a} resolveImageUrl={resolveImageUrl} />}
          {isPdf(a) && <PdfPreview attachment={a} resolveImageUrl={resolveImageUrl} />}
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem" }}>
            <span
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: "0.85rem",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {a.original_filename} <span style={{ color: "var(--text-muted)" }}>({formatSize(a.size_bytes)})</span>
            </span>
            <span style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
              <button type="button" onClick={() => onOpen(a.id)}>
                Öffnen
              </button>
              <button type="button" onClick={() => onExport(a.id)}>
                Exportieren
              </button>
              {onRemove && (
                <button type="button" onClick={() => onRemove(a.id)}>
                  Entfernen
                </button>
              )}
            </span>
          </div>
        </li>
      ))}
    </ul>
  );
}
