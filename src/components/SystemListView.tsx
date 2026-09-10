import { useCallback, useEffect, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { getKeymap, matchesBinding } from "../lib/keymap";
import { pickCsvFile, type ImportSummary } from "../lib/csvImport";
import { formatInvokeError } from "../lib/errors";
import ImportSummaryPanel from "./ImportSummaryPanel";

interface System {
  id: number;
  customer_id: number;
  name: string;
  system_type: string;
  hostname: string;
  ip_address: string;
  notes: string;
  archived_at_utc: string | null;
}

interface Customer {
  id: number;
  name: string;
}

// Kept in sync with src-tauri/src/commands/systems.rs::BulkArchiveSummary.
interface BulkArchiveSummary {
  archived: number;
  errors: string[];
}

export default function SystemListView() {
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const formOpen = useAppStore((s) => s.formOpen);
  const systemEditorTarget = useAppStore((s) => s.systemEditorTarget);
  const openSystemEditor = useAppStore((s) => s.openSystemEditor);

  const [systems, setSystems] = useState<System[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [customerName, setCustomerName] = useState<string | null>(null);

  const [importBusy, setImportBusy] = useState(false);
  const [importSummary, setImportSummary] = useState<ImportSummary | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [bulkArchiveBusy, setBulkArchiveBusy] = useState(false);
  const [bulkArchiveErrors, setBulkArchiveErrors] = useState<string[] | null>(null);

  const reload = useCallback(() => {
    if (selectedCustomerId === null) return;
    invoke<System[]>("list_systems", { customerId: selectedCustomerId, includeArchived: false }).then(setSystems);
  }, [selectedCustomerId]);

  useEffect(() => {
    reload();
  }, [reload]);

  // SystemForm is globally mounted and driven by the store, so this view
  // doesn't get an onDone callback — reload whenever its editor closes instead.
  const prevEditorTargetRef = useRef(systemEditorTarget);
  useEffect(() => {
    if (prevEditorTargetRef.current !== null && systemEditorTarget === null) {
      reload();
    }
    prevEditorTargetRef.current = systemEditorTarget;
  }, [systemEditorTarget, reload]);

  useEffect(() => {
    if (selectedCustomerId === null) {
      setCustomerName(null);
      return;
    }
    invoke<Customer[]>("list_customers", { includeArchived: true }).then((customers) => {
      const match = customers.find((c) => c.id === selectedCustomerId);
      setCustomerName(match ? match.name : null);
    });
  }, [selectedCustomerId]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, systems.length - 1));
      } else if (matchesBinding(e, keymap.list_prev)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const system = systems[selectedIndex];
        if (system && selectedCustomerId !== null) {
          e.preventDefault();
          openSystemEditor(system.id, selectedCustomerId);
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [systems, selectedIndex, formOpen, selectedCustomerId, openSystemEditor]);

  async function archive(id: number) {
    await invoke("archive_system", { id });
    reload();
  }

  function toggleSelected(id: number, e: MouseEvent) {
    e.stopPropagation();
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  function toggleSelectAll(e: MouseEvent) {
    e.stopPropagation();
    setSelectedIds((prev) =>
      prev.size === systems.length && systems.length > 0 ? new Set() : new Set(systems.map((s) => s.id)),
    );
  }

  async function handleBulkArchive() {
    if (!window.confirm(`${selectedIds.size} Systeme wirklich archivieren?`)) return;
    setBulkArchiveErrors(null);
    setBulkArchiveBusy(true);
    try {
      const summary = await invoke<BulkArchiveSummary>("archive_systems", { ids: Array.from(selectedIds) });
      setSelectedIds(new Set());
      reload();
      if (summary.errors.length > 0) setBulkArchiveErrors(summary.errors);
    } finally {
      setBulkArchiveBusy(false);
    }
  }

  async function handleImportCsv() {
    if (selectedCustomerId === null) return;
    setImportError(null);
    try {
      const csvPath = await pickCsvFile();
      if (csvPath === null) return;
      setImportBusy(true);
      const summary = await invoke<ImportSummary>("import_systems_from_csv", {
        customerId: selectedCustomerId,
        csvPath,
      });
      setImportSummary(summary);
      reload();
    } catch (e) {
      setImportError(formatInvokeError(e));
    } finally {
      setImportBusy(false);
    }
  }

  if (selectedCustomerId === null) {
    return (
      <div>
        <p>Kein Kunde ausgewählt — zurück zur Kundenliste</p>
        <button onClick={goToCustomers}>Zurück zu Kunden</button>
      </div>
    );
  }

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", marginBottom: "0.75rem" }}>
        <div>
          <button onClick={goToCustomers} style={{ marginBottom: "0.5rem" }}>
            ← Zurück zu Kunden
          </button>
          <h1 style={{ fontSize: "1.1rem" }}>
            Systeme von {customerName ?? `Kunde #${selectedCustomerId}`}
          </h1>
        </div>
        <span style={{ display: "flex", gap: "0.4rem", alignItems: "center" }}>
          <label style={{ display: "flex", alignItems: "center", gap: "0.3rem", fontSize: "0.85rem" }}>
            <input
              type="checkbox"
              checked={selectedIds.size === systems.length && systems.length > 0}
              onClick={toggleSelectAll}
              onChange={() => {}}
            />
            Alle auswählen
          </label>
          <button disabled={importBusy} onClick={() => void handleImportCsv()}>
            CSV importieren…
          </button>
          <button className="btn-primary" onClick={() => openSystemEditor("new", selectedCustomerId)}>
            + Neues System
          </button>
        </span>
      </div>
      {importError && <p style={{ color: "var(--danger)", fontSize: "0.82rem", marginBottom: "0.75rem" }}>Fehler: {importError}</p>}
      {importSummary && <ImportSummaryPanel summary={importSummary} onDismiss={() => setImportSummary(null)} />}
      {selectedIds.size > 0 && (
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            padding: "0.5rem 0.6rem",
            marginBottom: "0.75rem",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            background: "var(--bg-surface)",
          }}
        >
          <span style={{ fontSize: "0.85rem" }}>{selectedIds.size} ausgewählt</span>
          <button disabled={bulkArchiveBusy} onClick={() => void handleBulkArchive()}>
            Archivieren
          </button>
        </div>
      )}
      {bulkArchiveErrors && bulkArchiveErrors.length > 0 && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "0.4rem",
            padding: "0.75rem",
            marginBottom: "0.75rem",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            background: "var(--bg-surface)",
          }}
        >
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <p style={{ margin: 0, fontSize: "0.85rem" }}>
              <strong style={{ color: "var(--danger)" }}>{bulkArchiveErrors.length}</strong> Fehler beim Archivieren
            </p>
            <button onClick={() => setBulkArchiveErrors(null)}>Schließen</button>
          </div>
          <ul style={{ margin: 0, padding: "0 0 0 1.1rem", fontSize: "0.8rem", color: "var(--text-secondary)", maxHeight: "8rem", overflow: "auto" }}>
            {bulkArchiveErrors.map((message, i) => (
              <li key={i}>{message}</li>
            ))}
          </ul>
        </div>
      )}
      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {systems.map((s, i) => (
          <li
            key={s.id}
            className="list-row"
            style={{
              padding: "0.5rem 0.6rem",
              background: i === selectedIndex ? "var(--bg-selected)" : "transparent",
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
            }}
          >
            <input
              type="checkbox"
              checked={selectedIds.has(s.id)}
              onClick={(e) => toggleSelected(s.id, e)}
              onChange={() => {}}
              style={{ marginRight: "0.6rem" }}
            />
            <span style={{ flex: 1 }}>
              {s.name} <span style={{ color: "var(--text-muted)" }}>({s.system_type})</span>{" "}
              <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "0.85em" }}>
                {s.hostname}
              </span>
              {s.ip_address && (
                <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "0.85em" }}>
                  {" "}
                  — {s.ip_address}
                </span>
              )}
            </span>
            <span style={{ display: "flex", gap: "0.4rem" }}>
              <button onClick={() => openSystemEditor(s.id, selectedCustomerId)}>Bearbeiten</button>
              <button onClick={() => archive(s.id)}>Archivieren</button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
