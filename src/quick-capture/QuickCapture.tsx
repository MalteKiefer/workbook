import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

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

interface PendingAttachment {
  token: string;
  bytesBase64: string;
  filename: string;
  mimeType: string;
}

const CATEGORIES: { value: string; label: string }[] = [
  { value: "wartung", label: "Wartung" },
  { value: "stoerung", label: "Störung" },
  { value: "aenderung", label: "Änderung" },
  { value: "installation", label: "Installation" },
  { value: "sonstiges", label: "Sonstiges" },
];

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

export default function QuickCapture() {
  const titleRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const draftIsEmptyRef = useRef(true);

  const [customers, setCustomers] = useState<Customer[]>([]);
  const [systems, setSystems] = useState<System[]>([]);
  const [customerId, setCustomerId] = useState<number | "">("");
  const [systemId, setSystemId] = useState<number | "">("");
  const [title, setTitle] = useState("");
  const [bodyMd, setBodyMd] = useState("");
  const [category, setCategory] = useState("wartung");
  const [tagNames, setTagNames] = useState("");
  const [performedAtInput, setPerformedAtInput] = useState("");
  const [performedAtUtc, setPerformedAtUtc] = useState("");
  const [performedAtTz, setPerformedAtTz] = useState("");
  const [performedAtPreview, setPerformedAtPreview] = useState("");
  const [pendingAttachments, setPendingAttachments] = useState<PendingAttachment[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    draftIsEmptyRef.current = title === "" && bodyMd === "";
  }, [title, bodyMd]);

  const resetDraft = useCallback(() => {
    setTitle("");
    setBodyMd("");
    setCategory("wartung");
    setTagNames("");
    setPendingAttachments([]);
    setPerformedAtInput("");
    setError(null);
  }, []);

  const refreshPreview = useCallback(async (utc: string, tz: string) => {
    try {
      const preview = await invoke<string>("format_timestamp_for_display", { utc, tz });
      setPerformedAtPreview(preview);
    } catch {
      setPerformedAtPreview("");
    }
  }, []);

  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  useEffect(() => {
    if (customerId === "") {
      setSystems([]);
      return;
    }
    invoke<System[]>("list_systems", { customerId, includeArchived: false }).then(setSystems);
  }, [customerId]);

  useEffect(() => {
    const unlisten = listen<{ performed_at_utc: string; performed_at_tz: string }>(
      "quick-capture-activated",
      async (event) => {
        if (draftIsEmptyRef.current) {
          const { performed_at_utc, performed_at_tz } = event.payload;
          setPerformedAtUtc(performed_at_utc);
          setPerformedAtTz(performed_at_tz);
          setPerformedAtInput("");
          await refreshPreview(performed_at_utc, performed_at_tz);

          const last = await invoke<{ customer: Customer | null; system: System | null }>("get_last_selection");
          if (last.customer) setCustomerId(last.customer.id);
          if (last.system) setSystemId(last.system.id);
        }
        titleRef.current?.focus();
      },
    );
    return () => {
      unlisten.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshPreview]);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    const unlisten = currentWindow.onFocusChanged(({ payload: focused }) => {
      if (focused) titleRef.current?.focus();
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function handlePerformedAtBlur() {
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

  function insertAtCursor(text: string) {
    const el = bodyRef.current;
    if (!el) {
      setBodyMd((prev) => `${prev}\n${text}`);
      return;
    }
    const start = el.selectionStart ?? bodyMd.length;
    const end = el.selectionEnd ?? bodyMd.length;
    const next = `${bodyMd.slice(0, start)}${text}${bodyMd.slice(end)}`;
    setBodyMd(next);
    requestAnimationFrame(() => {
      el.selectionStart = el.selectionEnd = start + text.length;
      el.focus();
    });
  }

  async function handlePaste(e: React.ClipboardEvent<HTMLTextAreaElement>) {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith("image/")) {
        e.preventDefault();
        const file = item.getAsFile();
        if (!file) return;
        const buffer = await file.arrayBuffer();
        const bytesBase64 = arrayBufferToBase64(buffer);
        const token = `pending:${crypto.randomUUID()}`;
        insertAtCursor(`![Screenshot](${token})`);
        setPendingAttachments((prev) => [...prev, { token, bytesBase64, filename: "Screenshot.png", mimeType: item.type }]);
        return;
      }
    }
  }

  const save = useCallback(async () => {
    try {
      await invoke("create_entry", {
        input: {
          customer_id: customerId,
          system_id: systemId === "" ? null : systemId,
          title,
          body_md: bodyMd,
          category,
          performed_at_utc: performedAtUtc,
          performed_at_tz: performedAtTz,
          tag_names: tagNames.split(",").map((s) => s.trim()).filter(Boolean),
          pending_attachments: pendingAttachments.map((p) => ({
            placeholder_token: p.token,
            bytes_base64: p.bytesBase64,
            original_filename: p.filename,
            mime_type: p.mimeType,
          })),
        },
      });
      resetDraft();
      await getCurrentWindow().hide();
    } catch (e) {
      setError(String(e));
    }
  }, [customerId, systemId, title, bodyMd, category, performedAtUtc, performedAtTz, tagNames, pendingAttachments, resetDraft]);

  const discard = useCallback(async () => {
    resetDraft();
    await getCurrentWindow().hide();
  }, [resetDraft]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.ctrlKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      } else if (e.key === "Escape") {
        e.preventDefault();
        void discard();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [save, discard]);

  return (
    <main style={{ fontFamily: "sans-serif", padding: "0.75rem", display: "flex", flexDirection: "column", gap: "0.5rem" }}>
      <input
        ref={titleRef}
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        placeholder="Titel"
        style={{ fontSize: "1rem", padding: "0.4rem" }}
      />
      <div style={{ display: "flex", gap: "0.5rem" }}>
        <select value={customerId} onChange={(e) => setCustomerId(e.target.value === "" ? "" : Number(e.target.value))}>
          <option value="">Kunde wählen…</option>
          {customers.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name} ({c.short_code})
            </option>
          ))}
        </select>
        <select value={systemId} onChange={(e) => setSystemId(e.target.value === "" ? "" : Number(e.target.value))}>
          <option value="">Kein System</option>
          {systems.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
        <select value={category} onChange={(e) => setCategory(e.target.value)}>
          {CATEGORIES.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>
      </div>
      <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
        <input
          value={performedAtInput}
          onChange={(e) => setPerformedAtInput(e.target.value)}
          onBlur={handlePerformedAtBlur}
          placeholder="jetzt"
          style={{ width: "10rem" }}
        />
        <span style={{ fontFamily: "monospace", fontSize: "0.85rem" }}>{performedAtPreview}</span>
      </div>
      <textarea
        ref={bodyRef}
        value={bodyMd}
        onChange={(e) => setBodyMd(e.target.value)}
        onPaste={handlePaste}
        placeholder="Markdown…"
        style={{ flex: 1, minHeight: "10rem", fontFamily: "monospace" }}
      />
      <input value={tagNames} onChange={(e) => setTagNames(e.target.value)} placeholder="Tags, durch Komma getrennt" />
      {error && <p style={{ color: "crimson" }}>Fehler: {error}</p>}
    </main>
  );
}
