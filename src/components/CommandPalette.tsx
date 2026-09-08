import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { formatInvokeError } from "../lib/errors";

interface DirectoryHit {
  kind: "customer" | "system";
  id: number;
  customer_id: number;
  label: string;
}

interface EntryHit {
  entry_id: number;
  customer_id: number;
  system_id: number | null;
  title: string;
  snippet: string;
  performed_at_utc: string;
  performed_at_tz: string;
}

interface StaticCommand {
  id: string;
  label: string;
  shortcut: string;
  run: () => void;
  keepOpen?: boolean;
}

type Section = "commands" | "directory" | "entries";

interface FlatItem {
  key: string;
  section: Section;
  render: () => ReactNode;
  activate: () => void;
}

const SECTION_LABEL: Record<Section, string> = {
  commands: "Befehle",
  directory: "Kunden & Systeme",
  entries: "Einträge",
};

const DEBOUNCE_MS = 180;

export default function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [directoryHits, setDirectoryHits] = useState<DirectoryHit[]>([]);
  const [entryHits, setEntryHits] = useState<EntryHit[]>([]);
  const [notice, setNotice] = useState<string | null>(null);

  const inputRef = useRef<HTMLInputElement>(null);
  const previouslyFocusedRef = useRef<HTMLElement | null>(null);
  const debounceTimerRef = useRef<number | null>(null);
  const requestIdRef = useRef(0);

  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const selectedSystemId = useAppStore((s) => s.selectedSystemId);
  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const goToSystems = useAppStore((s) => s.goToSystems);
  const goToJournal = useAppStore((s) => s.goToJournal);
  const goToSettings = useAppStore((s) => s.goToSettings);
  const selectCustomer = useAppStore((s) => s.selectCustomer);
  const selectSystem = useAppStore((s) => s.selectSystem);
  const openEntryEditor = useAppStore((s) => s.openEntryEditor);
  const openExportDialog = useAppStore((s) => s.openExportDialog);
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const openSystemEditor = useAppStore((s) => s.openSystemEditor);
  const openShortcutOverview = useAppStore((s) => s.openShortcutOverview);

  const close = useCallback(() => {
    setOpen(false);
    setQuery("");
    setDirectoryHits([]);
    setEntryHits([]);
    setSelectedIndex(0);
    setNotice(null);
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    requestIdRef.current += 1; // invalidate any in-flight responses
    previouslyFocusedRef.current?.focus();
    previouslyFocusedRef.current = null;
  }, []);

  const openPalette = useCallback(() => {
    previouslyFocusedRef.current = document.activeElement as HTMLElement | null;
    setOpen(true);
  }, []);

  // Autofocus the input whenever the palette opens.
  useEffect(() => {
    if (!open) return;
    const id = window.setTimeout(() => inputRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, [open]);

  // Static, always-available commands (context-dependent per current view/selection).
  const staticCommands = useMemo<StaticCommand[]>(() => {
    const cmds: StaticCommand[] = [
      {
        id: "quick-capture",
        label: "Schnellerfassung öffnen",
        shortcut: "Strg+N",
        run: () => {
          void invoke("open_quick_capture_with_context", {
            customerId: selectedCustomerId,
            systemId: selectedSystemId,
          });
        },
      },
      { id: "new-entry", label: "Neuer Eintrag (im Hauptfenster)", shortcut: "", run: () => openEntryEditor("new") },
      { id: "new-customer", label: "Neuer Kunde", shortcut: "", run: () => openCustomerEditor("new") },
      { id: "goto-customers", label: "Zu Kundenliste", shortcut: "g c", run: goToCustomers },
      {
        id: "new-system",
        label: "Neues System",
        shortcut: "",
        // Always listed (not just once a customer is already selected) — a
        // system always belongs to a customer, so without one selected this
        // just tells the user to pick one first instead of silently vanishing.
        keepOpen: selectedCustomerId === null,
        run: () => {
          if (selectedCustomerId === null) {
            setNotice("Bitte zuerst einen Kunden auswählen (Kundenliste oder Suche oben).");
            return;
          }
          openSystemEditor("new", selectedCustomerId);
        },
      },
    ];
    if (selectedCustomerId !== null) {
      cmds.push({ id: "goto-systems", label: "Zu Systemliste", shortcut: "g s", run: () => goToSystems() });
      cmds.push({
        id: "export-customer",
        label: "Kunde exportieren",
        shortcut: "",
        // This command has no filtered view of its own to sync from (unlike
        // JournalView's "Exportieren" button) — "Kunde exportieren" means
        // exporting the whole customer, unrestricted. ExportDialog seeds its
        // System field from the global selectedSystemId though, so without
        // clearing it here a system merely left selected from an earlier,
        // unrelated palette action (e.g. jumping to a specific system) would
        // silently scope the export to that system, dropping every entry
        // not assigned to it.
        run: () => {
          selectSystem(null);
          openExportDialog();
        },
      });
    }
    cmds.push({ id: "goto-journal", label: "Zum Journal", shortcut: "g j", run: goToJournal });
    cmds.push({ id: "goto-backup", label: "Zu Einstellungen → Backup", shortcut: "", run: () => goToSettings("backup") });
    cmds.push({ id: "goto-plugins", label: "Zu Einstellungen → Plugins", shortcut: "", run: () => goToSettings("plugins") });
    cmds.push({ id: "shortcuts", label: "Tastaturkürzel anzeigen", shortcut: "?", run: openShortcutOverview });
    cmds.push({
      id: "cleanup-orphans",
      label: "Anhänge bereinigen",
      shortcut: "",
      keepOpen: true,
      run: () => {
        setNotice("Bereinige…");
        invoke<{ removed_count: number; removed_bytes: number }>("cleanup_orphans")
          .then((result) => {
            const kb = (result.removed_bytes / 1024).toFixed(1);
            setNotice(`${result.removed_count} verwaiste Datei(en) entfernt (${kb} KB freigegeben).`);
          })
          .catch((e) => setNotice(`Fehler: ${formatInvokeError(e)}`));
      },
    });
    return cmds;
  }, [
    selectedCustomerId,
    selectedSystemId,
    goToCustomers,
    goToSystems,
    goToJournal,
    goToSettings,
    openEntryEditor,
    openExportDialog,
    openCustomerEditor,
    openSystemEditor,
    openShortcutOverview,
    selectSystem,
  ]);

  const filteredCommands = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q === "") return staticCommands;
    return staticCommands.filter((c) => c.label.toLowerCase().includes(q));
  }, [staticCommands, query]);

  // Debounced backend search — directory (customers/systems) and full-text entries.
  useEffect(() => {
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    const trimmed = query.trim();
    if (trimmed === "") {
      setDirectoryHits([]);
      setEntryHits([]);
      return;
    }
    debounceTimerRef.current = window.setTimeout(() => {
      const requestId = ++requestIdRef.current;
      invoke<DirectoryHit[]>("search_directory", { query: trimmed, limit: 8 })
        .then((hits) => {
          if (requestIdRef.current === requestId) setDirectoryHits(hits);
        })
        .catch(() => {
          if (requestIdRef.current === requestId) setDirectoryHits([]);
        });
      invoke<EntryHit[]>("search_entries", { query: trimmed, limit: 8 })
        .then((hits) => {
          if (requestIdRef.current === requestId) setEntryHits(hits);
        })
        .catch(() => {
          // FTS5 can reject raw punctuation etc. — degrade to no entry results.
          if (requestIdRef.current === requestId) setEntryHits([]);
        });
    }, DEBOUNCE_MS);
    return () => {
      if (debounceTimerRef.current !== null) window.clearTimeout(debounceTimerRef.current);
    };
  }, [query]);

  // Reset selection whenever the query (and therefore result set) changes.
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  const commandItems = useMemo<FlatItem[]>(
    () =>
      filteredCommands.map((cmd) => ({
        key: `cmd-${cmd.id}`,
        section: "commands",
        render: () => (
          <Row label={cmd.label} hint={cmd.shortcut} />
        ),
        activate: () => {
          cmd.run();
          if (!cmd.keepOpen) close();
        },
      })),
    [filteredCommands, close],
  );

  const directoryItems = useMemo<FlatItem[]>(
    () =>
      directoryHits.map((hit) => ({
        key: `dir-${hit.kind}-${hit.id}`,
        section: "directory",
        render: () => (
          <Row label={hit.label} hint={hit.kind === "customer" ? "Kunde" : "System"} />
        ),
        activate: () => {
          if (hit.kind === "customer") {
            selectCustomer(hit.id);
            goToCustomers();
          } else {
            selectCustomer(hit.customer_id);
            goToSystems(hit.customer_id);
            selectSystem(hit.id);
          }
          close();
        },
      })),
    [directoryHits, selectCustomer, goToCustomers, goToSystems, selectSystem, close],
  );

  const entryItems = useMemo<FlatItem[]>(
    () =>
      entryHits.map((hit) => ({
        key: `entry-${hit.entry_id}`,
        section: "entries",
        render: () => (
          <div>
            <div style={{ color: "var(--text-primary)" }}>{hit.title}</div>
            <div
              className="cp-snippet"
              style={{ fontSize: "0.8em", color: "var(--text-secondary)", marginTop: "0.15rem" }}
              // Own local SQLite full-text data, not untrusted web content — safe to render.
              dangerouslySetInnerHTML={{ __html: hit.snippet }}
            />
          </div>
        ),
        activate: () => {
          selectCustomer(hit.customer_id);
          if (hit.system_id !== null) selectSystem(hit.system_id);
          goToJournal();
          close();
        },
      })),
    [entryHits, selectCustomer, selectSystem, goToJournal, close],
  );

  const items = useMemo<FlatItem[]>(
    () => [...commandItems, ...directoryItems, ...entryItems],
    [commandItems, directoryItems, entryItems],
  );

  // Single capture-phase listener: Strg+K/Cmd+K always live (even in a focused text
  // field), Escape/arrows/Enter only owned while the palette is open. Capture phase
  // (not bubble, like useGlobalHotkeys.ts and other listeners) plus stopPropagation
  // guarantees this runs first and prevents double-handling (e.g. the global hook's
  // own Escape behavior) without depending on component mount/registration order.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      const isModK = (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k";
      if (isModK) {
        e.preventDefault();
        e.stopPropagation();
        if (open) {
          close();
        } else {
          openPalette();
        }
        return;
      }

      if (!open) return;

      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close();
        return;
      }

      if (e.key === "ArrowDown") {
        e.preventDefault();
        e.stopPropagation();
        setSelectedIndex((i) => (items.length === 0 ? 0 : Math.min(i + 1, items.length - 1)));
        return;
      }

      if (e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        setSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }

      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        const item = items[selectedIndex];
        item?.activate();
        return;
      }
    }

    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [open, items, selectedIndex, close, openPalette]);

  if (!open) return null;

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(8, 9, 12, 0.55)",
        backdropFilter: "blur(2px)",
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "center",
        paddingTop: "10vh",
        zIndex: 200,
      }}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <style>{".cp-snippet mark { background: var(--accent); color: var(--accent-text); border-radius: 2px; padding: 0 2px; }"}</style>
      <div
        style={{
          background: "var(--bg-elevated)",
          color: "var(--text-primary)",
          borderRadius: "var(--radius-lg)",
          border: "1px solid var(--border)",
          width: "36rem",
          maxWidth: "90vw",
          maxHeight: "70vh",
          display: "flex",
          flexDirection: "column",
          boxShadow: "var(--shadow-modal)",
          overflow: "hidden",
        }}
      >
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Befehl, Kunde, System oder Volltext…"
          style={{
            padding: "0.75rem",
            fontSize: "1rem",
            background: "transparent",
            border: "none",
            borderRadius: 0,
            borderBottom: "1px solid var(--border)",
            color: "inherit",
            outline: "none",
          }}
        />
        {notice && (
          <div
            style={{
              padding: "0.5rem 0.75rem",
              fontSize: "0.85rem",
              color: "var(--text-secondary)",
              borderBottom: "1px solid var(--border)",
            }}
          >
            {notice}
          </div>
        )}
        <div style={{ overflowY: "auto", padding: "0.25rem 0" }}>
          {items.length === 0 && (
            <div style={{ padding: "0.75rem", color: "var(--text-muted)" }}>Keine Treffer</div>
          )}
          {items.map((item, idx) => {
            const showHeader = idx === 0 || items[idx - 1].section !== item.section;
            return (
              <div key={item.key}>
                {showHeader && <SectionLabel text={SECTION_LABEL[item.section]} />}
                <div
                  role="option"
                  aria-selected={idx === selectedIndex}
                  onMouseEnter={() => setSelectedIndex(idx)}
                  onClick={() => item.activate()}
                  style={{
                    padding: "0.4rem 0.75rem",
                    cursor: "pointer",
                    background: idx === selectedIndex ? "var(--bg-selected)" : "transparent",
                  }}
                >
                  {item.render()}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function Row({ label, hint }: { label: string; hint: string }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", gap: "0.75rem" }}>
      <span>{label}</span>
      <span style={{ color: "var(--text-muted)", fontSize: "0.85em", fontFamily: "var(--font-mono)" }}>{hint}</span>
    </div>
  );
}

function SectionLabel({ text }: { text: string }) {
  return (
    <div
      style={{
        padding: "0.3rem 0.75rem",
        fontSize: "0.7rem",
        textTransform: "uppercase",
        color: "var(--text-muted)",
        letterSpacing: "0.05em",
        fontWeight: 600,
      }}
    >
      {text}
    </div>
  );
}
