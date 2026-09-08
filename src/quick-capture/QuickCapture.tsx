import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import MarkdownEditor, { type MarkdownEditorHandle } from "../components/MarkdownEditor";
import { formatInvokeError } from "../lib/errors";

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

interface System {
  id: number;
  customer_id: number;
  name: string;
  hostname: string;
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

// Plugin-sourced device known from a connected RMM/asset-management plugin
// (NinjaOne, Level.io, Snipe-IT) for the selected customer that has no local
// System linked to it yet. Backend reads already-cached plugin data (no
// network call), so it's cheap to fetch alongside the local systems list.
interface UnlinkedExternalSystemDto {
  plugin: "ninja" | "level" | "snipeit";
  connection_id: string;
  external_id: string;
  name: string;
  hostname: string | null;
  ip_address: string | null;
}

const PLUGIN_LABEL: Record<UnlinkedExternalSystemDto["plugin"], string> = {
  ninja: "Ninja",
  level: "Level",
  snipeit: "Snipe-IT",
};

// System typeahead: a filterable dropdown over the already-fetched systems
// list (filtering by name AND hostname, case-insensitive) instead of a plain
// <select>, since a customer can have many systems and hostname is the
// user-recognizable key in practice. A synthetic "clear" row is always first
// so the field can always be reset back to "Kein System" from the keyboard
// or a click, same as the old <select>'s empty option. Copied from
// EntryEditor.tsx's own copy of the same pattern — not shared, same
// convention as CATEGORIES above.
//
// A third row kind surfaces devices known from a connected plugin but not
// yet linked to any local System (see UnlinkedExternalSystemDto above) —
// selecting one transparently creates the local System and links it, same
// as picking "Als neues System anlegen und verknüpfen" in the plugin
// sections (NinjaPluginSection.tsx etc.), just inline in this typeahead.
type SystemRow = { kind: "clear" } | { kind: "system"; system: System } | { kind: "external"; device: UnlinkedExternalSystemDto };

function systemLabel(system: System): string {
  return system.hostname ? `${system.name} — ${system.hostname}` : system.name;
}

export default function QuickCapture() {
  const titleRef = useRef<HTMLInputElement>(null);
  const editorRef = useRef<MarkdownEditorHandle>(null);
  const draftIsEmptyRef = useRef(true);

  const [customers, setCustomers] = useState<Customer[]>([]);
  const [systems, setSystems] = useState<System[]>([]);
  const [unlinkedExternalSystems, setUnlinkedExternalSystems] = useState<UnlinkedExternalSystemDto[]>([]);
  const [customerId, setCustomerId] = useState<number | "">("");
  const [systemId, setSystemId] = useState<number | "">("");
  const [systemQuery, setSystemQuery] = useState("");
  const [systemSuggestionsOpen, setSystemSuggestionsOpen] = useState(false);
  const [systemHighlightIndex, setSystemHighlightIndex] = useState(0);
  const [systemCreateLinkBusy, setSystemCreateLinkBusy] = useState(false);
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

  const loadCustomers = useCallback(async () => {
    const list = await invoke<Customer[]>("list_customers", { includeArchived: false });
    setCustomers(list);
    return list;
  }, []);

  const loadSystems = useCallback(async (forCustomerId: number | "") => {
    if (forCustomerId === "") {
      setSystems([]);
      setUnlinkedExternalSystems([]);
      return;
    }
    const list = await invoke<System[]>("list_systems", { customerId: forCustomerId, includeArchived: false });
    setSystems(list);
    // Plugin-sourced suggestions are a secondary/enhancement data source on
    // top of the local systems list above — a failure here (e.g. no plugin
    // connected, cache not warm yet) must not break the rest of the form.
    try {
      const external = await invoke<UnlinkedExternalSystemDto[]>("list_unlinked_external_systems_for_customer", {
        customerId: forCustomerId,
      });
      setUnlinkedExternalSystems(external);
    } catch {
      setUnlinkedExternalSystems([]);
    }
  }, []);

  // Initial load — this window is created once at app startup and only
  // shown/hidden afterward (never remounted), so this alone is NOT enough to
  // stay current; the activation handler below re-loads on every show too.
  useEffect(() => {
    loadCustomers();
  }, [loadCustomers]);

  useEffect(() => {
    loadSystems(customerId);
  }, [customerId, loadSystems]);

  // Filtered dropdown rows for the System typeahead — recomputed as the user
  // types. The "clear" row is unfiltered/always present so "Kein System" stays
  // reachable regardless of the current query text.
  const filteredSystemRows = useMemo<SystemRow[]>(() => {
    const q = systemQuery.trim().toLowerCase();
    const matches =
      q === ""
        ? systems
        : systems.filter((s) => s.name.toLowerCase().includes(q) || s.hostname.toLowerCase().includes(q));
    const externalMatches =
      q === ""
        ? unlinkedExternalSystems
        : unlinkedExternalSystems.filter(
            (d) => d.name.toLowerCase().includes(q) || (d.hostname ?? "").toLowerCase().includes(q),
          );
    return [
      { kind: "clear" },
      ...matches.map((system) => ({ kind: "system" as const, system })),
      ...externalMatches.map((device) => ({ kind: "external" as const, device })),
    ];
  }, [systems, unlinkedExternalSystems, systemQuery]);

  // Keep the input's displayed text in sync with the committed systemId
  // (activation seeding from last-used selection/override, or a customer
  // change). Deliberately depends only on systemId/systems, NOT on
  // systemSuggestionsOpen — closing the dropdown via Escape/blur must never
  // re-run this and stomp text the user typed but didn't commit
  // (Escape/click-outside are only supposed to close the list, never discard
  // what's typed).
  useEffect(() => {
    if (systemId === "") {
      setSystemQuery("");
      return;
    }
    const match = systems.find((s) => s.id === systemId);
    setSystemQuery(match ? systemLabel(match) : "");
  }, [systemId, systems]);

  useEffect(() => {
    setSystemHighlightIndex(0);
  }, [systemQuery, systemSuggestionsOpen]);

  // Picking a plugin-sourced suggestion has no local System to point at yet —
  // transparently create one (prefilled from the plugin device) and link it,
  // then commit exactly as if an ordinary existing System had been picked.
  // Mirrors NinjaPluginSection.tsx's (and the Level/Snipe-IT equivalents')
  // "Als neues System anlegen und verknüpfen" create+link sequence.
  async function selectExternalSystemRow(device: UnlinkedExternalSystemDto) {
    if (customerId === "") return;
    const custId = customerId;
    const previousQuery = systemQuery;
    setSystemSuggestionsOpen(false);
    setSystemCreateLinkBusy(true);
    setSystemQuery(`${device.name} — wird angelegt…`);
    setError(null);
    try {
      const created = await invoke<System>("create_system", {
        input: {
          customer_id: custId,
          name: device.name,
          system_type: "",
          hostname: device.hostname ?? "",
          ip_address: device.ip_address ?? "",
          notes: "",
        },
      });
      const linkCommand =
        device.plugin === "ninja" ? "link_system_to_ninja" : device.plugin === "level" ? "link_system_to_level" : "link_system_to_snipeit";
      await invoke(linkCommand, {
        systemId: created.id,
        connectionId: device.connection_id,
        externalId: device.external_id,
      });
      // Reflect the new System locally right away so the query-sync effect
      // below can show it immediately, without depending on the best-effort
      // refetch that follows succeeding.
      setSystems((prev) => [...prev, created]);
      setSystemId(created.id);
      // Nice-to-have, not required for correctness above: refreshes both
      // lists so a reopened dropdown shows this as a normal local System
      // (not still listed as an unlinked plugin suggestion).
      void loadSystems(custId);
    } catch (e) {
      setError(formatInvokeError(e));
      setSystemQuery(previousQuery);
    } finally {
      setSystemCreateLinkBusy(false);
    }
  }

  function selectSystemRow(row: SystemRow) {
    if (row.kind === "clear") {
      setSystemId("");
      setSystemQuery("");
      setSystemSuggestionsOpen(false);
    } else if (row.kind === "system") {
      setSystemId(row.system.id);
      setSystemQuery(systemLabel(row.system));
      setSystemSuggestionsOpen(false);
    } else {
      void selectExternalSystemRow(row.device);
    }
  }

  function handleSystemKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSystemSuggestionsOpen(true);
      setSystemHighlightIndex((i) => Math.min(i + 1, filteredSystemRows.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSystemSuggestionsOpen(true);
      setSystemHighlightIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      if (systemSuggestionsOpen) {
        e.preventDefault();
        const row = filteredSystemRows[systemHighlightIndex];
        if (row) selectSystemRow(row);
      }
    } else if (e.key === "Escape") {
      if (systemSuggestionsOpen) {
        e.preventDefault();
        e.stopPropagation();
        setSystemSuggestionsOpen(false);
      }
    }
  }

  useEffect(() => {
    const unlisten = listen<{
      performed_at_utc: string;
      performed_at_tz: string;
      context_note: string | null;
      override_customer_id: number | null;
      override_system_id: number | null;
    }>(
      "quick-capture-activated",
      async (event) => {
        // Re-fetch on every activation, not just once at window creation —
        // this window is shown/hidden, never remounted, so a customer/system
        // created in the main window afterward would otherwise never appear.
        await loadCustomers();

        if (draftIsEmptyRef.current) {
          const { performed_at_utc, performed_at_tz, context_note, override_customer_id, override_system_id } = event.payload;
          setPerformedAtUtc(performed_at_utc);
          setPerformedAtTz(performed_at_tz);
          setPerformedAtInput("");
          await refreshPreview(performed_at_utc, performed_at_tz);
          if (context_note) {
            setBodyMd(`_Kontext: ${context_note}_\n\n`);
          }

          let resolvedCustomerId: number | "" = "";
          let resolvedSystemId: number | "" = "";

          const last = await invoke<{ customer: Customer | null; system: System | null }>("get_last_selection");
          if (last.customer) resolvedCustomerId = last.customer.id;
          if (last.system) resolvedSystemId = last.system.id;

          // Explicit navigation context (from the main window's Strg+N) takes
          // precedence over the last-used selection above.
          if (override_customer_id !== null) resolvedCustomerId = override_customer_id;
          if (override_system_id !== null) resolvedSystemId = override_system_id;

          setCustomerId(resolvedCustomerId);
          setSystemId(resolvedSystemId);
          setSystemSuggestionsOpen(false);
          // The customerId-keyed effect above only re-fetches systems when
          // customerId actually CHANGES — if it's the same customer as last
          // time but a new system was added under them since, that effect
          // alone would miss it. Force a fresh fetch here too.
          if (resolvedCustomerId !== "") {
            await loadSystems(resolvedCustomerId);
          }
        }
        titleRef.current?.focus();
      },
    );
    return () => {
      unlisten.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshPreview, loadCustomers, loadSystems]);

  useEffect(() => {
    const unlisten = listen<{ bytes_base64: string; mime_type: string }>("quick-capture-paste-image", (event) => {
      const { bytes_base64, mime_type } = event.payload;
      const token = `pending:${crypto.randomUUID()}`;
      const markdown = `![Screenshot](${token})`;
      setBodyMd((prev) => (prev ? `${prev}\n${markdown}` : markdown));
      setPendingAttachments((prev) => [...prev, { token, bytesBase64: bytes_base64, filename: "Screenshot.png", mimeType: mime_type }]);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    const unlisten = currentWindow.onFocusChanged(({ payload: focused }) => {
      // Only steal focus to the title field on a genuinely fresh activation
      // (nothing meaningfully focused yet — i.e. focus sits on <body> or is
      // unset). WebView2 on Windows can fire focus-changed(true) spuriously
      // while the window is already focused and the user is interacting with
      // it (e.g. while typing in the Markdown editor), and unconditionally
      // refocusing the title field here was yanking focus away mid-keystroke,
      // making the editor appear to swallow all input.
      if (focused && (document.activeElement === document.body || document.activeElement === null)) {
        titleRef.current?.focus();
      }
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
      setError(formatInvokeError(e));
    }
  }

  function insertAtCursor(text: string) {
    const editor = editorRef.current;
    if (!editor) {
      setBodyMd((prev) => `${prev}\n${text}`);
      return;
    }
    editor.insertAtCursor(text);
  }

  async function handlePaste(e: ClipboardEvent) {
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
      await invoke("quick_capture_close");
    } catch (e) {
      setError(formatInvokeError(e));
    }
  }, [customerId, systemId, title, bodyMd, category, performedAtUtc, performedAtTz, tagNames, pendingAttachments, resetDraft]);

  const discard = useCallback(async () => {
    resetDraft();
    await invoke("quick_capture_close");
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

  const fieldLabelStyle: CSSProperties = { fontSize: "0.78rem", color: "var(--text-secondary)", fontWeight: 500 };

  return (
    <main
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "0.55rem",
        height: "100vh",
        padding: "0.75rem",
        background: "var(--bg-base)",
      }}
    >
      <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
        <span style={fieldLabelStyle}>Titel</span>
        <input
          ref={titleRef}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="Kurzer Titel des Eintrags"
          style={{ fontSize: "0.95rem" }}
          autoFocus
        />
      </label>
      <div style={{ display: "flex", gap: "0.4rem" }}>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 2, minWidth: 0 }}>
          <span style={fieldLabelStyle}>Kunde</span>
          <select
            value={customerId}
            onChange={(e) => setCustomerId(e.target.value === "" ? "" : Number(e.target.value))}
          >
            <option value="">Kunde wählen…</option>
            {customers.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} ({c.short_code})
              </option>
            ))}
          </select>
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 2, minWidth: 0, position: "relative" }}>
          <span style={fieldLabelStyle}>System</span>
          <input
            value={systemQuery}
            disabled={systemCreateLinkBusy}
            onChange={(e) => {
              setSystemQuery(e.target.value);
              setSystemSuggestionsOpen(true);
            }}
            onFocus={(e) => {
              setSystemSuggestionsOpen(true);
              e.target.select();
            }}
            onBlur={() => setSystemSuggestionsOpen(false)}
            onKeyDown={handleSystemKeyDown}
            placeholder="Kein System"
            autoComplete="off"
          />
          {systemSuggestionsOpen && (
            <div
              style={{
                position: "absolute",
                top: "100%",
                left: 0,
                right: 0,
                marginTop: "0.15rem",
                background: "var(--bg-elevated)",
                border: "1px solid var(--border)",
                borderRadius: "var(--radius-sm)",
                boxShadow: "var(--shadow-modal)",
                maxHeight: "12rem",
                overflowY: "auto",
                zIndex: 10,
              }}
            >
              {filteredSystemRows.map((row, idx) => {
                const key =
                  row.kind === "clear"
                    ? "clear"
                    : row.kind === "system"
                      ? `system-${row.system.id}`
                      : `external-${row.device.plugin}-${row.device.connection_id}-${row.device.external_id}`;
                return (
                  <div
                    key={key}
                    onMouseEnter={() => setSystemHighlightIndex(idx)}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      selectSystemRow(row);
                    }}
                    style={{
                      padding: "0.35rem 0.5rem",
                      cursor: "pointer",
                      fontSize: "0.85rem",
                      background: idx === systemHighlightIndex ? "var(--bg-selected)" : "transparent",
                      color: row.kind === "clear" ? "var(--text-muted)" : "var(--text-primary)",
                    }}
                  >
                    {row.kind === "clear" ? (
                      "Kein System"
                    ) : row.kind === "system" ? (
                      <>
                        {row.system.name}
                        {row.system.hostname && (
                          <span style={{ color: "var(--text-secondary)", fontFamily: "var(--font-mono)", fontSize: "0.85em" }}>
                            {" "}— {row.system.hostname}
                          </span>
                        )}
                      </>
                    ) : (
                      <>
                        {row.device.name}
                        {row.device.hostname && (
                          <span style={{ color: "var(--text-secondary)", fontFamily: "var(--font-mono)", fontSize: "0.85em" }}>
                            {" "}— {row.device.hostname}
                          </span>
                        )}
                        <span style={{ color: "var(--accent)", fontFamily: "var(--font-mono)", fontSize: "0.75em", marginLeft: "0.4rem" }}>
                          · {PLUGIN_LABEL[row.device.plugin]}
                        </span>
                      </>
                    )}
                  </div>
                );
              })}
              {filteredSystemRows.length === 1 && (
                <div style={{ padding: "0.35rem 0.5rem", fontSize: "0.8rem", color: "var(--text-muted)" }}>Keine Treffer</div>
              )}
            </div>
          )}
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1, minWidth: 0 }}>
          <span style={fieldLabelStyle}>Kategorie</span>
          <select value={category} onChange={(e) => setCategory(e.target.value)}>
            {CATEGORIES.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div style={{ display: "flex", gap: "0.5rem", alignItems: "flex-end" }}>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          <span style={fieldLabelStyle}>Zeitpunkt</span>
          <input
            value={performedAtInput}
            onChange={(e) => setPerformedAtInput(e.target.value)}
            onBlur={handlePerformedAtBlur}
            placeholder="jetzt"
            style={{ width: "9rem" }}
          />
        </label>
        <span style={{ fontFamily: "var(--font-mono)", fontSize: "0.78rem", color: "var(--text-secondary)", paddingBottom: "0.45rem" }}>
          {performedAtPreview}
        </span>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1, minHeight: 0 }}>
        <span style={fieldLabelStyle}>Beschreibung (Markdown)</span>
        <MarkdownEditor
          ref={editorRef}
          value={bodyMd}
          onChange={setBodyMd}
          onPaste={handlePaste}
          placeholder="Markdown…"
          minHeight="4rem"
        />
      </div>
      <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
        <span style={fieldLabelStyle}>Tags</span>
        <input value={tagNames} onChange={(e) => setTagNames(e.target.value)} placeholder="Tags, durch Komma getrennt" />
      </label>
      {error && <p style={{ color: "var(--danger)", margin: 0, fontSize: "0.82rem" }}>Fehler: {error}</p>}
      <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end" }}>
        <button type="button" onClick={() => void discard()}>
          Verwerfen (Esc)
        </button>
        <button type="button" className="btn-primary" onClick={() => void save()}>
          Speichern (Strg+S)
        </button>
      </div>
    </main>
  );
}
