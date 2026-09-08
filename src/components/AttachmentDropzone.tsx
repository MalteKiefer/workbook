import { useCallback, useRef, useState } from "react";

export interface PickedFile {
  bytesBase64: string;
  filename: string;
  mimeType: string;
}

interface AttachmentDropzoneProps {
  onFilesAdded: (files: PickedFile[]) => void;
  children?: React.ReactNode;
}

// Same chunked String.fromCharCode approach as QuickCapture.tsx's existing
// clipboard-paste helper — kept local to this file rather than shared, matching
// this codebase's existing per-file duplication of small helpers.
function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

async function fileToPickedFile(file: File): Promise<PickedFile> {
  const buffer = await file.arrayBuffer();
  return {
    bytesBase64: arrayBufferToBase64(buffer),
    filename: file.name,
    mimeType: file.type || "application/octet-stream",
  };
}

/**
 * Generic, presentational file-picker + drag&drop surface for Anhänge. Turns
 * whatever File objects the user picks or drops into {bytesBase64, filename,
 * mimeType} and hands them to onFilesAdded. Knows nothing about Tauri commands,
 * entry IDs, or the attachment data model — the consumer wires that up.
 *
 * Deliberately does NOT handle clipboard paste: that input method already exists
 * inside the relevant editor's own onPaste handler (see QuickCapture.tsx). This
 * component covers only the other two Anhänge input methods from the spec:
 * Dateiauswahl (tastaturbedienbar) and Drag & Drop.
 */
export default function AttachmentDropzone({ onFilesAdded, children }: AttachmentDropzoneProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  // Counts nested dragenter/dragleave pairs so the drag-over visual state doesn't
  // flicker off when the pointer crosses from the wrapper onto a child element
  // (the button or hint text) — dragleave fires for that too, not just when
  // leaving the whole dropzone.
  const dragCounterRef = useRef(0);
  const [isDragOver, setIsDragOver] = useState(false);

  const handleFiles = useCallback(
    async (fileList: FileList | null) => {
      if (!fileList || fileList.length === 0) return;
      const picked = await Promise.all(Array.from(fileList).map(fileToPickedFile));
      onFilesAdded(picked);
    },
    [onFilesAdded],
  );

  return (
    <div
      role="group"
      aria-label="Anhänge hinzufügen"
      onDragEnter={(e) => {
        e.preventDefault();
        dragCounterRef.current += 1;
        setIsDragOver(true);
      }}
      onDragOver={(e) => {
        // Required unconditionally, or the browser refuses to fire `drop` at all.
        e.preventDefault();
      }}
      onDragLeave={(e) => {
        e.preventDefault();
        dragCounterRef.current = Math.max(0, dragCounterRef.current - 1);
        if (dragCounterRef.current === 0) setIsDragOver(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        dragCounterRef.current = 0;
        setIsDragOver(false);
        void handleFiles(e.dataTransfer.files);
      }}
      style={{
        border: isDragOver ? "2px dashed var(--accent)" : "2px dashed var(--border)",
        borderRadius: "var(--radius-sm)",
        padding: "0.6rem",
        background: isDragOver ? "rgba(109, 139, 250, 0.08)" : "transparent",
        display: "flex",
        flexDirection: "column",
        gap: "0.5rem",
        transition: "border-color 120ms ease, background-color 120ms ease",
      }}
    >
      {children}
      <div style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
        <button type="button" onClick={() => inputRef.current?.click()}>
          Datei auswählen…
        </button>
        <span style={{ fontSize: "0.8rem", color: "var(--text-muted)" }}>
          {isDragOver ? "Dateien hier ablegen" : "oder Dateien hierher ziehen"}
        </span>
      </div>
      <input
        ref={inputRef}
        type="file"
        multiple
        // display:none removes this from the tab order and the accessibility tree
        // entirely — the only reachable, activatable control is the button above,
        // by mouse or by keyboard (Enter/Space on a focused <button> dispatches a
        // trusted click event, which is exactly what inputRef.current.click() needs
        // to be allowed to open the native file picker).
        style={{ display: "none" }}
        onChange={(e) => {
          void handleFiles(e.target.files);
          e.target.value = ""; // allow re-selecting the same file again later
        }}
      />
    </div>
  );
}
