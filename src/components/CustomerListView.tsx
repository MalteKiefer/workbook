import { useCallback, useEffect, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { getKeymap, matchesBinding } from "../lib/keymap";
import { pickCsvFile, type ImportSummary } from "../lib/csvImport";
import { formatInvokeError } from "../lib/errors";
import ImportSummaryPanel from "./ImportSummaryPanel";

interface Customer {
  id: number;
  name: string;
  short_code: string;
  notes: string;
  archived_at_utc: string | null;
}

// Kept in sync with src-tauri/src/commands/customers.rs::BulkArchiveSummary.
interface BulkArchiveSummary {
  archived: number;
  errors: string[];
}

export default function CustomerListView() {
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const selectCustomer = useAppStore((s) => s.selectCustomer);
  const goToCustomerDetail = useAppStore((s) => s.goToCustomerDetail);
  const formOpen = useAppStore((s) => s.formOpen);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const pendingAction = useAppStore((s) => s.pendingAction);
  const setPendingAction = useAppStore((s) => s.setPendingAction);

  const [importBusy, setImportBusy] = useState(false);
  const [importSummary, setImportSummary] = useState<ImportSummary | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [bulkArchiveBusy, setBulkArchiveBusy] = useState(false);
  const [bulkArchiveErrors, setBulkArchiveErrors] = useState<string[] | null>(null);

  const reload = useCallback(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  // Memoized (like `reload` above) so it has a stable identity to depend on
  // from the pendingAction effect below without re-running on every render.
  const handleImportCsv = useCallback(async () => {
    setImportError(null);
    try {
      const csvPath = await pickCsvFile();
      if (csvPath === null) return;
      setImportBusy(true);
      const summary = await invoke<ImportSummary>("import_customers_from_csv", { csvPath });
      setImportSummary(summary);
      reload();
    } catch (e) {
      setImportError(formatInvokeError(e));
    } finally {
      setImportBusy(false);
    }
  }, [reload]);

  useEffect(() => {
    reload();
  }, [reload]);

  // CustomerForm is globally mounted and driven by the store, so this view
  // doesn't get an onDone callback — reload whenever its editor closes instead.
  const prevEditorTargetRef = useRef(customerEditorTarget);
  useEffect(() => {
    if (prevEditorTargetRef.current !== null && customerEditorTarget === null) {
      reload();
    }
    prevEditorTargetRef.current = customerEditorTarget;
  }, [customerEditorTarget, reload]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (formOpen || isTypingTarget(document.activeElement)) return;
      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, customers.length - 1));
      } else if (matchesBinding(e, keymap.list_prev)) {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          selectCustomer(customer.id);
          goToCustomerDetail(customer.id);
        }
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          goToCustomerDetail(customer.id, "uebersicht");
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [customers, selectedIndex, formOpen, selectCustomer, goToCustomerDetail]);

  // Consumes the Command Palette's "CSV-Import: Kunden" pendingAction (set in
  // CommandPalette.tsx alongside goToCustomers()) by running this view's own
  // existing CSV-import handler once it mounts. Clearing it immediately is
  // essential -- otherwise it would silently re-fire on every unrelated
  // re-mount of this view.
  useEffect(() => {
    if (pendingAction === "import-customers-csv") {
      setPendingAction(null);
      void handleImportCsv();
    }
  }, [pendingAction, setPendingAction, handleImportCsv]);

  async function archive(id: number) {
    await invoke("archive_customer", { id });
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
      prev.size === customers.length && customers.length > 0 ? new Set() : new Set(customers.map((c) => c.id)),
    );
  }

  async function handleBulkArchive() {
    if (!window.confirm(`${selectedIds.size} Kunden wirklich archivieren?`)) return;
    setBulkArchiveErrors(null);
    setBulkArchiveBusy(true);
    try {
      const summary = await invoke<BulkArchiveSummary>("archive_customers", { ids: Array.from(selectedIds) });
      setSelectedIds(new Set());
      reload();
      if (summary.errors.length > 0) setBulkArchiveErrors(summary.errors);
    } finally {
      setBulkArchiveBusy(false);
    }
  }

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "0.75rem" }}>
        <h1 style={{ fontSize: "1.1rem" }}>Kunden</h1>
        <span style={{ display: "flex", gap: "0.4rem", alignItems: "center" }}>
          <label style={{ display: "flex", alignItems: "center", gap: "0.3rem", fontSize: "0.85rem" }}>
            <input
              type="checkbox"
              checked={selectedIds.size === customers.length && customers.length > 0}
              onClick={toggleSelectAll}
              onChange={() => {}}
            />
            Alle auswählen
          </label>
          <button disabled={importBusy} onClick={() => void handleImportCsv()}>
            CSV importieren…
          </button>
          <button className="btn-primary" onClick={() => openCustomerEditor("new")}>
            + Neuer Kunde
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
        {customers.map((c, i) => (
          <li
            key={c.id}
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
              checked={selectedIds.has(c.id)}
              onClick={(e) => toggleSelected(c.id, e)}
              onChange={() => {}}
              style={{ marginRight: "0.6rem" }}
            />
            <span
              onClick={() => {
                selectCustomer(c.id);
                goToCustomerDetail(c.id);
              }}
              style={{ cursor: "pointer", flex: 1 }}
              title="Systeme dieses Kunden öffnen"
            >
              {c.name} <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)", fontSize: "0.85em" }}>({c.short_code})</span>
            </span>
            <span style={{ display: "flex", gap: "0.4rem" }}>
              <button
                onClick={() => {
                  selectCustomer(c.id);
                  goToCustomerDetail(c.id);
                }}
              >
                Systeme →
              </button>
              <button onClick={() => goToCustomerDetail(c.id, "uebersicht")}>Bearbeiten</button>
              <button onClick={() => archive(c.id)}>Archivieren</button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
