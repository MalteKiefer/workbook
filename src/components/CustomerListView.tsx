import { useCallback, useEffect, useRef, useState } from "react";
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

export default function CustomerListView() {
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const selectCustomer = useAppStore((s) => s.selectCustomer);
  const goToSystems = useAppStore((s) => s.goToSystems);
  const formOpen = useAppStore((s) => s.formOpen);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);

  const [importBusy, setImportBusy] = useState(false);
  const [importSummary, setImportSummary] = useState<ImportSummary | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  const reload = useCallback(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

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
          goToSystems(customer.id);
        }
      } else if (matchesBinding(e, keymap.edit_selected)) {
        const customer = customers[selectedIndex];
        if (customer) {
          e.preventDefault();
          openCustomerEditor(customer.id);
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [customers, selectedIndex, formOpen, selectCustomer, goToSystems, openCustomerEditor]);

  async function archive(id: number) {
    await invoke("archive_customer", { id });
    reload();
  }

  async function handleImportCsv() {
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
  }

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "0.75rem" }}>
        <h1 style={{ fontSize: "1.1rem" }}>Kunden</h1>
        <span style={{ display: "flex", gap: "0.4rem" }}>
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
            <span
              onClick={() => {
                selectCustomer(c.id);
                goToSystems(c.id);
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
                  goToSystems(c.id);
                }}
              >
                Systeme →
              </button>
              <button onClick={() => openCustomerEditor(c.id)}>Bearbeiten</button>
              <button onClick={() => archive(c.id)}>Archivieren</button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
