import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";

interface NetworkScanSettingsDto {
  ports: number[];
}

// Parses the comma-separated ports input, returning null if any piece isn't
// an integer between 1 and 65535, or the resulting list would be empty.
function parsePorts(input: string): number[] | null {
  const pieces = input
    .split(",")
    .map((piece) => piece.trim())
    .filter((piece) => piece !== "");
  if (pieces.length === 0) return null;
  const ports: number[] = [];
  for (const piece of pieces) {
    if (!/^\d+$/.test(piece)) return null;
    const port = Number(piece);
    if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
    ports.push(port);
  }
  return ports;
}

export default function NetworkSettingsView() {
  const [portsInput, setPortsInput] = useState("");
  const [portsLoaded, setPortsLoaded] = useState(false);
  const [portsBusy, setPortsBusy] = useState(false);
  const [portsError, setPortsError] = useState<string | null>(null);
  const [portsStatus, setPortsStatus] = useState<string | null>(null);

  const [nmapAvailable, setNmapAvailable] = useState<boolean | null>(null);
  const [installHint, setInstallHint] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<NetworkScanSettingsDto>("get_network_scan_settings")
      .then((settings) => {
        if (cancelled) return;
        setPortsInput(settings.ports.join(", "));
        setPortsLoaded(true);
      })
      .catch((e) => {
        if (cancelled) return;
        setPortsError(formatInvokeError(e));
        setPortsLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    invoke<boolean>("is_nmap_available")
      .then((available) => {
        if (cancelled) return;
        setNmapAvailable(available);
        if (!available) {
          invoke<string>("nmap_install_hint")
            .then((hint) => {
              if (!cancelled) setInstallHint(hint);
            })
            .catch(() => {
              // No install hint block shown if this fails -- the
              // "nmap nicht gefunden" line above still communicates the
              // missing dependency on its own.
            });
        }
      })
      .catch(() => {
        if (!cancelled) setNmapAvailable(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleSavePorts() {
    if (portsBusy) return;
    setPortsError(null);
    setPortsStatus(null);
    const parsed = parsePorts(portsInput);
    if (parsed === null) {
      setPortsError("Ports müssen durch Komma getrennte Zahlen zwischen 1 und 65535 sein.");
      return;
    }
    setPortsBusy(true);
    try {
      await invoke("set_network_scan_settings", { ports: parsed });
      setPortsStatus("Gespeichert.");
    } catch (e) {
      setPortsError(formatInvokeError(e));
    } finally {
      setPortsBusy(false);
    }
  }

  async function handleCopyHint() {
    if (installHint === null) return;
    try {
      await navigator.clipboard.writeText(installHint);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (e) {
      setPortsError(formatInvokeError(e));
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "36rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Netzwerk</h1>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Scan-Ports</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          TCP-Ports, die bei jedem Netzwerk-Scan geprüft werden, durch Komma getrennt.
        </p>
        {!portsLoaded ? (
          <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade Einstellungen…</p>
        ) : (
          <>
            <input
              value={portsInput}
              onChange={(e) => setPortsInput(e.target.value)}
              disabled={portsBusy}
              placeholder="22, 80, 443, 445, 3389, 8080"
              style={{ fontFamily: "var(--font-mono)" }}
            />
            <div>
              <button type="button" className="btn-primary" disabled={portsBusy} onClick={() => void handleSavePorts()}>
                Speichern
              </button>
            </div>
          </>
        )}
        {portsStatus && <p style={{ margin: 0, color: "var(--success)", fontSize: "0.82rem" }}>{portsStatus}</p>}
        {portsError && <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.82rem" }}>Fehler: {portsError}</p>}
      </section>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>nmap</h2>
        {nmapAvailable === null ? (
          <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>Prüfe…</p>
        ) : nmapAvailable ? (
          <p style={{ margin: 0, color: "var(--success)", fontSize: "0.85rem" }}>
            nmap gefunden — erweiterte Scans verfügbar.
          </p>
        ) : (
          <>
            <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
              nmap wurde nicht gefunden. Für erweiterte Scans lokal installieren:
            </p>
            {installHint !== null && (
              <div style={{ display: "flex", flexDirection: "column", gap: "0.4rem" }}>
                <pre
                  style={{
                    margin: 0,
                    padding: "0.5rem",
                    fontFamily: "var(--font-mono)",
                    fontSize: "0.78rem",
                    overflow: "auto",
                    background: "var(--bg-surface)",
                    border: "1px solid var(--border)",
                    borderRadius: "var(--radius-sm)",
                  }}
                >
                  {installHint}
                </pre>
                <div>
                  <button type="button" onClick={() => void handleCopyHint()}>
                    {copied ? "Kopiert!" : "Kopieren"}
                  </button>
                </div>
              </div>
            )}
          </>
        )}
      </section>
    </div>
  );
}
