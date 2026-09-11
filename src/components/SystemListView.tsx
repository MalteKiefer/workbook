import { useCallback, useEffect, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { getKeymap, matchesBinding } from "../lib/keymap";
import { pickCsvFile, type ImportSummary } from "../lib/csvImport";
import { formatInvokeError } from "../lib/errors";
import ImportSummaryPanel from "./ImportSummaryPanel";
import NetworkScanModal from "./NetworkScanModal";

interface System {
  id: number;
  customer_id: number;
  name: string;
  system_type: string;
  hostname: string;
  ip_address: string;
  notes: string;
  archived_at_utc: string | null;
  maintenance_interval_days: number | null;
  // Computed by list_systems_with_maintenance_status (maintenance::is_overdue),
  // not stored columns -- see SystemWithMaintenanceStatus on the Rust side.
  overdue: boolean;
  last_performed_at_utc: string | null;
}

// Kept in sync with src-tauri/src/commands/systems.rs::BulkArchiveSummary.
interface BulkArchiveSummary {
  archived: number;
  errors: string[];
}

// Kept in sync with src-tauri/src/commands/systems.rs::BulkUpdateSummary.
interface BulkUpdateSummary {
  updated: number;
  errors: string[];
}

// Same synchronous de-DE formatting already used for other system-generated
// timestamps shown as plain text (BackupView.tsx, UpdateSettingsView.tsx) --
// simpler than the async format_timestamp_for_display command, which exists
// for temporal-input-parsing round-trips rather than one-off badge tooltips.
function formatOverdueSince(s: System): string {
  if (s.last_performed_at_utc === null) return "Anlage des Systems";
  return new Date(s.last_performed_at_utc).toLocaleString("de-DE");
}

export default function SystemListView() {
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const formOpen = useAppStore((s) => s.formOpen);
  const systemEditorTarget = useAppStore((s) => s.systemEditorTarget);
  const openSystemEditor = useAppStore((s) => s.openSystemEditor);
  const pendingAction = useAppStore((s) => s.pendingAction);
  const setPendingAction = useAppStore((s) => s.setPendingAction);

  const [systems, setSystems] = useState<System[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);

  const [importBusy, setImportBusy] = useState(false);
  const [importSummary, setImportSummary] = useState<ImportSummary | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [bulkArchiveBusy, setBulkArchiveBusy] = useState(false);
  const [bulkArchiveErrors, setBulkArchiveErrors] = useState<string[] | null>(null);
  const [bulkIntervalBusy, setBulkIntervalBusy] = useState(false);
  const [bulkIntervalErrors, setBulkIntervalErrors] = useState<string[] | null>(null);

  const [scanModalOpen, setScanModalOpen] = useState(false);

  const reload = useCallback(() => {
    if (selectedCustomerId === null) return;
    invoke<System[]>("list_systems_with_maintenance_status", {
      customerId: selectedCustomerId,
      includeArchived: false,
    }).then(setSystems);
  }, [selectedCustomerId]);

  // Memoized (like `reload` above) so it has a stable identity to depend on
  // from the pendingAction effect below without re-running on every render.
  const handleImportCsv = useCallback(async () => {
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
  }, [selectedCustomerId, reload]);

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

  // Consumes the Command Palette's "CSV-Import: Systeme" pendingAction (set in
  // CommandPalette.tsx alongside goToCustomerDetail()) by running this view's own
  // existing CSV-import handler once it mounts. Clearing it immediately is
  // essential -- otherwise it would silently re-fire on every unrelated
  // re-mount of this view.
  useEffect(() => {
    if (pendingAction === "import-systems-csv") {
      setPendingAction(null);
      void handleImportCsv();
    }
  }, [pendingAction, setPendingAction, handleImportCsv]);

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

  async function handleBulkSetInterval() {
    const input = window.prompt(
      `Wartungsintervall in Tagen für ${selectedIds.size} System(e) (leer lassen, um das Intervall zu entfernen):`,
    );
    if (input === null) return; // cancelled
    const trimmed = input.trim();
    let days: number | null;
    if (trimmed === "") {
      days = null;
    } else {
      const parsed = Number(trimmed);
      if (!Number.isInteger(parsed) || parsed <= 0) {
        setBulkIntervalErrors([`"${input}" ist keine gültige Anzahl Tage (positive ganze Zahl, oder leer für "kein Intervall").`]);
        return;
      }
      days = parsed;
    }

    setBulkIntervalErrors(null);
    setBulkIntervalBusy(true);
    try {
      const summary = await invoke<BulkUpdateSummary>("bulk_set_maintenance_interval", {
        ids: Array.from(selectedIds),
        maintenanceIntervalDays: days,
      });
      setSelectedIds(new Set());
      reload();
      if (summary.errors.length > 0) setBulkIntervalErrors(summary.errors);
    } finally {
      setBulkIntervalBusy(false);
    }
  }

  if (selectedCustomerId === null) {
    return null;
  }

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "flex-end", alignItems: "flex-start", marginBottom: "0.75rem" }}>
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
          <button onClick={() => setScanModalOpen(true)}>
            Netzwerk scannen…
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
          <span style={{ display: "flex", gap: "0.4rem" }}>
            <button disabled={bulkArchiveBusy} onClick={() => void handleBulkArchive()}>
              Archivieren
            </button>
            <button disabled={bulkIntervalBusy} onClick={() => void handleBulkSetInterval()}>
              Wartungsintervall setzen…
            </button>
          </span>
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
      {bulkIntervalErrors && bulkIntervalErrors.length > 0 && (
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
              <strong style={{ color: "var(--danger)" }}>{bulkIntervalErrors.length}</strong> Fehler beim Setzen des Wartungsintervalls
            </p>
            <button onClick={() => setBulkIntervalErrors(null)}>Schließen</button>
          </div>
          <ul style={{ margin: 0, padding: "0 0 0 1.1rem", fontSize: "0.8rem", color: "var(--text-secondary)", maxHeight: "8rem", overflow: "auto" }}>
            {bulkIntervalErrors.map((message, i) => (
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
              {s.name}
              {s.overdue && (
                <span
                  title={`Überfällig seit ${formatOverdueSince(s)}`}
                  style={{
                    display: "inline-block",
                    width: "6px",
                    height: "6px",
                    borderRadius: "50%",
                    background: "var(--danger)",
                    marginLeft: "0.4rem",
                    verticalAlign: "middle",
                  }}
                />
              )}{" "}
              <span style={{ color: "var(--text-muted)" }}>({s.system_type})</span>{" "}
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
      {scanModalOpen && (
        <NetworkScanModal
          customerId={selectedCustomerId}
          onClose={() => setScanModalOpen(false)}
          onSystemCreated={reload}
        />
      )}
    </div>
  );
}
