import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../state/appStore";
import { formatInvokeError } from "../lib/errors";
import Modal from "./Modal";

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

// First-pass simplification: datetime-local has no timezone of its own. We
// treat the value as UTC by appending a trailing "Z" — good enough for filter
// bounds, not the spec's relative/absolute parse_temporal_input parser (that
// is for entry timestamps, not filter ranges). Copied verbatim from
// JournalView.tsx — five static things aren't worth a shared module.
function datetimeLocalToIsoUtc(value: string): string | null {
  if (!value) return null;
  return value.length === 16 ? `${value}:00Z` : `${value}Z`;
}

// Characters that can't appear in a Windows/Linux/macOS file name — replaced
// rather than dropped so "Kunde / Standort" doesn't silently collapse into
// "Kunde Standort".
function sanitizeForFilename(name: string): string {
  return name.replace(/[\\/:*?"<>|]/g, "_").trim();
}

// "YYYYMMDD_HHMM__Kunde[__System].pdf" — timestamped at export time (local
// time, same as the OS clock the user reads the save dialog with), customer
// name always included, system name only when a single system was selected
// (not left as the "Alle Systeme" default) since only then does it actually
// narrow down what's in the file.
function buildExportFilename(customerName: string, systemName: string | null): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}`;
  const parts = [stamp, sanitizeForFilename(customerName)];
  if (systemName) parts.push(sanitizeForFilename(systemName));
  return `${parts.join("__")}.pdf`;
}

export default function ExportDialog() {
  const exportDialogOpen = useAppStore((s) => s.exportDialogOpen);
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const selectedSystemId = useAppStore((s) => s.selectedSystemId);
  const formOpen = useAppStore((s) => s.formOpen);
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);
  const closeExportDialog = useAppStore((s) => s.closeExportDialog);
  const pendingAction = useAppStore((s) => s.pendingAction);
  const setPendingAction = useAppStore((s) => s.setPendingAction);

  const [customers, setCustomers] = useState<Customer[]>([]);
  const [systems, setSystems] = useState<System[]>([]);
  const [allCustomers, setAllCustomers] = useState(false);
  const [customerId, setCustomerId] = useState<number | "">("");
  const [systemId, setSystemId] = useState<number | "">("");
  const [fromInput, setFromInput] = useState("");
  const [toInput, setToInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Full customer list, once — same convention as EntryEditor.tsx.
  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  // Fetch-only customer→system cascade. Deliberately never resets systemId
  // itself — the reset lives solely in the customer <select>'s onChange, so
  // it never races the seed-on-open effect below (same rationale documented
  // in EntryEditor.tsx).
  useEffect(() => {
    if (customerId === "") {
      setSystems([]);
      return;
    }
    invoke<System[]>("list_systems", { customerId, includeArchived: false }).then(setSystems);
  }, [customerId]);

  // Seed fields from ambient store selection whenever the dialog opens. Also
  // consumes the Command Palette's "Alle Kunden exportieren" pendingAction
  // (set alongside openExportDialog() in CommandPalette.tsx) by pre-checking
  // "Alle Kunden" here instead of leaving it at its normal false default --
  // kept inside this same effect (rather than a second one) so it can't race
  // the rest of the seeding above.
  useEffect(() => {
    if (!exportDialogOpen) return;
    setCustomerId(selectedCustomerId ?? "");
    setSystemId(selectedSystemId ?? "");
    setFromInput("");
    setToInput("");
    setStatus(null);
    setError(null);
    if (pendingAction === "export-all-customers") {
      setAllCustomers(true);
      setPendingAction(null);
    } else {
      setAllCustomers(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [exportDialogOpen]);

  // Engage the global Esc-closing convention (useGlobalHotkeys watches
  // formOpen) whenever the dialog is open, and disengage on close/unmount.
  useEffect(() => {
    if (!exportDialogOpen) return;
    openForm();
    return () => closeForm();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [exportDialogOpen]);

  // An Esc-triggered close flips formOpen to false via useGlobalHotkeys; mirror
  // that back into exportDialogOpen so this component's own state stays in sync.
  useEffect(() => {
    if (!formOpen && exportDialogOpen) {
      closeExportDialog();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formOpen]);

  const cancel = () => {
    closeForm();
    closeExportDialog();
  };

  async function handleExportMarkdown() {
    if (!allCustomers && customerId === "") {
      setError("Kunde ist erforderlich");
      return;
    }
    setError(null);
    setStatus(null);
    try {
      const destDir = await open({ directory: true, title: "Zielverzeichnis wählen" });
      if (!destDir) return;
      setBusy(true);
      if (allCustomers) {
        await invoke("export_markdown_all_customers", {
          fromUtc: datetimeLocalToIsoUtc(fromInput),
          toUtc: datetimeLocalToIsoUtc(toInput),
          destDir,
        });
        setStatus(`Markdown-Export (alle Kunden) abgeschlossen: ${destDir}`);
      } else {
        await invoke("export_markdown", {
          customerId,
          systemId: systemId === "" ? null : systemId,
          fromUtc: datetimeLocalToIsoUtc(fromInput),
          toUtc: datetimeLocalToIsoUtc(toInput),
          destDir,
        });
        setStatus(`Markdown-Export abgeschlossen: ${destDir}`);
      }
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleExportPdf() {
    if (!allCustomers && customerId === "") {
      setError("Kunde ist erforderlich");
      return;
    }
    setError(null);
    setStatus(null);
    try {
      if (allCustomers) {
        const destDir = await open({ directory: true, title: "Zielverzeichnis wählen" });
        if (!destDir) return;
        setBusy(true);
        await invoke("export_pdf_all_customers", {
          fromUtc: datetimeLocalToIsoUtc(fromInput),
          toUtc: datetimeLocalToIsoUtc(toInput),
          destDir,
        });
        setStatus(`PDF-Export (alle Kunden) abgeschlossen: ${destDir}`);
      } else {
        const customerName = customers.find((c) => c.id === customerId)?.name ?? "Kunde";
        const systemName = systemId === "" ? null : systems.find((s) => s.id === systemId)?.name ?? null;
        const destPath = await save({
          defaultPath: buildExportFilename(customerName, systemName),
          filters: [{ name: "PDF", extensions: ["pdf"] }],
        });
        if (!destPath) return;
        setBusy(true);
        await invoke("export_pdf", {
          customerId,
          systemId: systemId === "" ? null : systemId,
          fromUtc: datetimeLocalToIsoUtc(fromInput),
          toUtc: datetimeLocalToIsoUtc(toInput),
          destPath,
        });
        setStatus(`PDF-Export abgeschlossen: ${destPath}`);
      }
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  if (!exportDialogOpen) return null;

  return (
    <Modal onClose={closeForm}>
      <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem", width: "26rem", maxWidth: "90vw" }}>
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Kunde exportieren</h2>

        <label style={{ display: "flex", alignItems: "center", gap: "0.4rem", fontSize: "0.9rem" }}>
          <input
            type="checkbox"
            checked={allCustomers}
            onChange={(e) => setAllCustomers(e.target.checked)}
          />
          Alle Kunden exportieren
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Kunde
          <select
            value={customerId}
            onChange={(e) => {
              const value = e.target.value === "" ? "" : Number(e.target.value);
              setCustomerId(value);
              setSystemId("");
            }}
            required={!allCustomers}
            disabled={allCustomers}
            autoFocus
          >
            <option value="">Kunde wählen…</option>
            {customers.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} ({c.short_code})
              </option>
            ))}
          </select>
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          System
          <select
            value={systemId}
            onChange={(e) => setSystemId(e.target.value === "" ? "" : Number(e.target.value))}
            disabled={allCustomers || customerId === ""}
          >
            <option value="">Alle Systeme</option>
            {systems.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        </label>

        <div style={{ display: "flex", gap: "0.5rem" }}>
          <label style={{ display: "flex", alignItems: "center", gap: "0.3rem", fontSize: "0.85rem" }}>
            Von
            <input type="datetime-local" value={fromInput} onChange={(e) => setFromInput(e.target.value)} />
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: "0.3rem", fontSize: "0.85rem" }}>
            Bis
            <input type="datetime-local" value={toInput} onChange={(e) => setToInput(e.target.value)} />
          </label>
        </div>

        {status && <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{status}</p>}
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}

        <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end", marginTop: "0.25rem" }}>
          <button type="button" onClick={cancel}>
            Abbrechen
          </button>
          <button
            type="button"
            disabled={(!allCustomers && customerId === "") || busy}
            onClick={() => void handleExportMarkdown()}
          >
            Als Markdown exportieren
          </button>
          <button
            type="button"
            disabled={(!allCustomers && customerId === "") || busy}
            onClick={() => void handleExportPdf()}
          >
            Als PDF exportieren
          </button>
        </div>
      </div>
    </Modal>
  );
}
