import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";

// Mirrors src-tauri/src/network_scan.rs::HostScanResult.
interface HostScanResult {
  ip: string;
  open_ports: number[];
  device_type: string | null;
  mac: string | null;
  vendor: string | null;
  hostname: string | null;
}

// Mirrors src-tauri/src/snmp_probe.rs::SnmpProbeResult.
interface SnmpProbeResult {
  sys_descr: string | null;
  sys_name: string | null;
  sys_location: string | null;
  sys_up_time: string | null;
}

interface Network {
  id: number;
  customer_id: number;
  name: string;
  cidr: string;
}

interface NetworkDetailPanelProps {
  network: Network;
  onBack: () => void;
}

// Reached by clicking "Scannen" on a network row in NetworksPanel.tsx
// (the "Netzwerke" tab of CustomerDetailView.tsx) -- replaces that tab's
// list view while open, same list<->detail toggle CustomerListView.tsx
// uses for CustomerDetailView.tsx one level up. Combines three
// independent, unprivileged discovery mechanisms against ONE shared
// target field (pre-filled from the network's own CIDR, still editable
// -- e.g. to scan a narrower sub-range without editing the saved
// network): a blocking TCP-connect sweep (scan_network), a per-host
// SNMPv1/v2c probe (probe_snmp), and an optional shell-out to a
// system-installed nmap binary (run_nmap_scan), offered only when
// is_nmap_available() returns true.
export default function NetworkDetailPanel({ network, onBack }: NetworkDetailPanelProps) {
  const [target, setTarget] = useState(network.cidr);
  const [scanBusy, setScanBusy] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [results, setResults] = useState<HostScanResult[]>([]);
  const [filterText, setFilterText] = useState("");

  const [creatingIp, setCreatingIp] = useState<string | null>(null);
  const [createdIps, setCreatedIps] = useState<Set<string>>(new Set());
  const [createErrors, setCreateErrors] = useState<Record<string, string>>({});

  const [expandedIp, setExpandedIp] = useState<string | null>(null);
  const [communityByIp, setCommunityByIp] = useState<Record<string, string>>({});
  const [snmpBusyIp, setSnmpBusyIp] = useState<string | null>(null);
  const [snmpResultByIp, setSnmpResultByIp] = useState<Record<string, SnmpProbeResult>>({});
  const [snmpErrorByIp, setSnmpErrorByIp] = useState<Record<string, string>>({});
  const [quickConnectErrorByIp, setQuickConnectErrorByIp] = useState<Record<string, string>>({});

  const [nmapAvailable, setNmapAvailable] = useState<boolean | null>(null);
  const [nmapBusy, setNmapBusy] = useState(false);
  const [nmapOutput, setNmapOutput] = useState<string | null>(null);
  const [nmapError, setNmapError] = useState<string | null>(null);
  const [nmapCopied, setNmapCopied] = useState(false);
  const nmapCopyResetTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (nmapCopyResetTimeoutRef.current !== null) {
        window.clearTimeout(nmapCopyResetTimeoutRef.current);
      }
    };
  }, []);

  useEffect(() => {
    invoke<boolean>("is_nmap_available")
      .then(setNmapAvailable)
      .catch(() => setNmapAvailable(false));
  }, []);

  async function handleScan() {
    setScanError(null);
    setScanBusy(true);
    try {
      const found = await invoke<HostScanResult[]>("scan_network", { cidr: target });
      setResults(found);
      setCreatedIps(new Set());
      setCreateErrors({});
      setQuickConnectErrorByIp({});
      setExpandedIp(null);
      setFilterText("");
    } catch (e) {
      setScanError(formatInvokeError(e));
    } finally {
      setScanBusy(false);
    }
  }

  async function handleCreateSystem(result: HostScanResult) {
    setCreatingIp(result.ip);
    setCreateErrors((prev) => {
      const next = { ...prev };
      delete next[result.ip];
      return next;
    });
    try {
      await invoke("create_system", {
        input: {
          customer_id: network.customer_id,
          name: result.ip,
          system_type: "",
          hostname: "",
          ip_address: result.ip,
          notes: "",
          maintenance_interval_days: null,
        },
      });
      setCreatedIps((prev) => new Set(prev).add(result.ip));
    } catch (e) {
      setCreateErrors((prev) => ({ ...prev, [result.ip]: formatInvokeError(e) }));
    } finally {
      setCreatingIp(null);
    }
  }

  function toggleSnmpSection(ip: string) {
    setExpandedIp((prev) => (prev === ip ? null : ip));
    setCommunityByIp((prev) => (ip in prev ? prev : { ...prev, [ip]: "public" }));
  }

  async function handleProbeSnmp(ip: string) {
    setSnmpBusyIp(ip);
    setSnmpErrorByIp((prev) => {
      const next = { ...prev };
      delete next[ip];
      return next;
    });
    try {
      const result = await invoke<SnmpProbeResult>("probe_snmp", {
        ip,
        community: communityByIp[ip] ?? "public",
      });
      setSnmpResultByIp((prev) => ({ ...prev, [ip]: result }));
    } catch (e) {
      setSnmpErrorByIp((prev) => ({ ...prev, [ip]: formatInvokeError(e) }));
    } finally {
      setSnmpBusyIp(null);
    }
  }

  async function handleOpenUrl(ip: string, url: string) {
    setQuickConnectErrorByIp((prev) => {
      const next = { ...prev };
      delete next[ip];
      return next;
    });
    try {
      await invoke("open_url", { url });
    } catch (e) {
      setQuickConnectErrorByIp((prev) => ({ ...prev, [ip]: formatInvokeError(e) }));
    }
  }

  async function handleOpenRdp(ip: string) {
    setQuickConnectErrorByIp((prev) => {
      const next = { ...prev };
      delete next[ip];
      return next;
    });
    try {
      await invoke("open_rdp", { ip });
    } catch (e) {
      setQuickConnectErrorByIp((prev) => ({ ...prev, [ip]: formatInvokeError(e) }));
    }
  }

  async function handleOpenShare(ip: string) {
    setQuickConnectErrorByIp((prev) => {
      const next = { ...prev };
      delete next[ip];
      return next;
    });
    try {
      await invoke("open_share", { ip });
    } catch (e) {
      setQuickConnectErrorByIp((prev) => ({ ...prev, [ip]: formatInvokeError(e) }));
    }
  }

  async function handleNmapScan() {
    setNmapBusy(true);
    setNmapError(null);
    setNmapOutput(null);
    try {
      const output = await invoke<string>("run_nmap_scan", { target });
      setNmapOutput(output);
    } catch (e) {
      setNmapError(formatInvokeError(e));
    } finally {
      setNmapBusy(false);
    }
  }

  async function handleCopyNmapOutput() {
    if (nmapOutput === null) return;
    try {
      await navigator.clipboard.writeText(nmapOutput);
      setNmapCopied(true);
      if (nmapCopyResetTimeoutRef.current !== null) {
        window.clearTimeout(nmapCopyResetTimeoutRef.current);
      }
      nmapCopyResetTimeoutRef.current = window.setTimeout(() => {
        setNmapCopied(false);
        nmapCopyResetTimeoutRef.current = null;
      }, 2000);
    } catch (e) {
      setNmapError(formatInvokeError(e));
    }
  }

  const filteredResults = results.filter((result) => {
    const needle = filterText.trim().toLowerCase();
    if (needle === "") return true;
    return (
      result.ip.includes(needle) ||
      (result.hostname?.toLowerCase().includes(needle) ?? false) ||
      (result.vendor?.toLowerCase().includes(needle) ?? false) ||
      (result.device_type?.toLowerCase().includes(needle) ?? false) ||
      result.open_ports.some((port) => String(port).includes(needle))
    );
  });

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem" }}>
      <div>
        <button onClick={onBack} style={{ marginBottom: "0.5rem" }}>
          ← Zurück zu Netzwerken
        </button>
        <h2 style={{ margin: 0, fontSize: "1.05rem" }}>
          {network.name}{" "}
          <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)", fontSize: "0.85em" }}>
            {network.cidr}
          </span>
        </h2>
      </div>

      <div style={{ display: "flex", gap: "0.4rem", alignItems: "flex-end" }}>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
          Ziel (IP oder CIDR)
          <input
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            placeholder="192.168.1.0/24"
            disabled={scanBusy || nmapBusy}
            style={{ fontFamily: "var(--font-mono)" }}
          />
        </label>
        <button className="btn-primary" onClick={() => void handleScan()} disabled={scanBusy || target.trim() === ""}>
          Scannen
        </button>
        {nmapAvailable === true && (
          <button onClick={() => void handleNmapScan()} disabled={nmapBusy || target.trim() === ""}>
            Mit nmap scannen
          </button>
        )}
      </div>
      {nmapAvailable === false && (
        <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-muted)" }}>
          nmap nicht gefunden — für erweiterte Scans lokal installieren, dieses Feature erkennt eine vorhandene
          Installation automatisch.
        </p>
      )}
      {scanBusy && (
        <p style={{ margin: 0, fontSize: "0.82rem", color: "var(--text-secondary)" }}>
          Scanne… (kann bis zu einer Minute dauern)
        </p>
      )}
      {scanError && <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.82rem" }}>Fehler: {scanError}</p>}

      {results.length > 0 && (
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Suchen (IP, Hostname, Hersteller, Typ, Port)
          <input
            value={filterText}
            onChange={(e) => setFilterText(e.target.value)}
            placeholder="z.B. 192.168.1, druck, 3389…"
            style={{ fontFamily: "var(--font-mono)" }}
          />
        </label>
      )}

      {results.length > 0 && (
        <>
          {filteredResults.length === 0 ? (
            <p style={{ margin: 0, fontSize: "0.82rem", color: "var(--text-muted)" }}>
              Keine Treffer für "{filterText}".
            </p>
          ) : (
            <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
              {filteredResults.map((result) => (
                <li
                  key={result.ip}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "0.4rem",
                    padding: "0.5rem 0.6rem",
                    borderTop: "1px solid var(--border-subtle)",
                  }}
                >
                  <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem" }}>
                    <span style={{ fontSize: "0.85rem" }}>
                      <span style={{ fontFamily: "var(--font-mono)" }}>{result.ip}</span>{" "}
                      <span style={{ color: "var(--text-muted)" }}>
                        {result.open_ports.length > 0 ? result.open_ports.join(", ") : "keine bekannten Ports offen"}
                      </span>
                    </span>
                    <HostActionsMenu
                      result={result}
                      isCreated={createdIps.has(result.ip)}
                      isCreating={creatingIp === result.ip}
                      onToggleSnmp={() => toggleSnmpSection(result.ip)}
                      onCreateSystem={() => void handleCreateSystem(result)}
                      onOpenUrl={(url) => void handleOpenUrl(result.ip, url)}
                      onOpenRdp={() => void handleOpenRdp(result.ip)}
                      onOpenShare={() => void handleOpenShare(result.ip)}
                    />
                  </div>
                  {(result.hostname || result.mac || result.vendor || result.device_type) && (
                    <span style={{ fontSize: "0.78rem", color: "var(--text-muted)" }}>
                      {[
                        result.hostname,
                        result.mac && result.vendor ? `${result.mac} (${result.vendor})` : result.mac ?? result.vendor,
                        result.device_type,
                      ]
                        .filter((part): part is string => Boolean(part))
                        .join(" · ")}
                    </span>
                  )}
                  {createErrors[result.ip] && (
                    <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.8rem" }}>
                      Fehler: {createErrors[result.ip]}
                    </p>
                  )}
                  {quickConnectErrorByIp[result.ip] && (
                    <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.8rem" }}>
                      Fehler: {quickConnectErrorByIp[result.ip]}
                    </p>
                  )}
                  {expandedIp === result.ip && (
                    <div
                      style={{
                        display: "flex",
                        flexDirection: "column",
                        gap: "0.4rem",
                        padding: "0.5rem",
                        border: "1px solid var(--border)",
                        borderRadius: "var(--radius-sm)",
                        background: "var(--bg-surface)",
                      }}
                    >
                      <div style={{ display: "flex", gap: "0.4rem", alignItems: "flex-end" }}>
                        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
                          Community-String
                          <input
                            value={communityByIp[result.ip] ?? "public"}
                            onChange={(e) =>
                              setCommunityByIp((prev) => ({ ...prev, [result.ip]: e.target.value }))
                            }
                            disabled={snmpBusyIp === result.ip}
                          />
                        </label>
                        <button
                          type="button"
                          onClick={() => void handleProbeSnmp(result.ip)}
                          disabled={snmpBusyIp === result.ip}
                        >
                          Abfragen
                        </button>
                      </div>
                      {snmpBusyIp === result.ip && (
                        <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Fragt ab…</p>
                      )}
                      {snmpErrorByIp[result.ip] && (
                        <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.8rem" }}>
                          Fehler: {snmpErrorByIp[result.ip]}
                        </p>
                      )}
                      {snmpResultByIp[result.ip] && (
                        <dl style={{ margin: 0, fontSize: "0.8rem", display: "grid", gridTemplateColumns: "auto 1fr", gap: "0.2rem 0.5rem" }}>
                          {snmpResultByIp[result.ip].sys_descr !== null && (
                            <>
                              <dt style={{ color: "var(--text-muted)" }}>sysDescr</dt>
                              <dd style={{ margin: 0 }}>{snmpResultByIp[result.ip].sys_descr}</dd>
                            </>
                          )}
                          {snmpResultByIp[result.ip].sys_name !== null && (
                            <>
                              <dt style={{ color: "var(--text-muted)" }}>sysName</dt>
                              <dd style={{ margin: 0 }}>{snmpResultByIp[result.ip].sys_name}</dd>
                            </>
                          )}
                          {snmpResultByIp[result.ip].sys_location !== null && (
                            <>
                              <dt style={{ color: "var(--text-muted)" }}>sysLocation</dt>
                              <dd style={{ margin: 0 }}>{snmpResultByIp[result.ip].sys_location}</dd>
                            </>
                          )}
                          {snmpResultByIp[result.ip].sys_up_time !== null && (
                            <>
                              <dt style={{ color: "var(--text-muted)" }}>sysUpTime</dt>
                              <dd style={{ margin: 0 }}>{snmpResultByIp[result.ip].sys_up_time}</dd>
                            </>
                          )}
                        </dl>
                      )}
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
        </>
      )}

      {nmapBusy && <p style={{ margin: 0, fontSize: "0.82rem", color: "var(--text-secondary)" }}>Scanne mit nmap…</p>}
      {nmapError && <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.82rem" }}>Fehler: {nmapError}</p>}
      {nmapOutput !== null && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "0.5rem",
            padding: "0.75rem",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            background: "var(--bg-surface)",
          }}
        >
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <h3 style={{ margin: 0, fontSize: "0.9rem" }}>nmap-Ausgabe</h3>
            <button type="button" onClick={() => void handleCopyNmapOutput()}>
              {nmapCopied ? "Kopiert!" : "Kopieren"}
            </button>
          </div>
          <pre
            style={{
              margin: 0,
              padding: "0.75rem",
              fontFamily: "var(--font-mono)",
              fontSize: "0.78rem",
              lineHeight: 1.5,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              overflowY: "auto",
              maxHeight: "60vh",
              background: "var(--bg-elevated)",
              border: "1px solid var(--border-subtle)",
              borderRadius: "var(--radius-sm)",
            }}
          >
            {nmapOutput}
          </pre>
        </div>
      )}
    </div>
  );
}

const WEB_PORT_SCHEMES: Record<number, "http" | "https"> = {
  80: "http",
  443: "https",
  8080: "http",
  8443: "https",
};

interface HostActionsMenuProps {
  result: HostScanResult;
  isCreated: boolean;
  isCreating: boolean;
  onToggleSnmp: () => void;
  onCreateSystem: () => void;
  onOpenUrl: (url: string) => void;
  onOpenRdp: () => void;
  onOpenShare: () => void;
}

function HostActionsMenu({
  result,
  isCreated,
  isCreating,
  onToggleSnmp,
  onCreateSystem,
  onOpenUrl,
  onOpenRdp,
  onOpenShare,
}: HostActionsMenuProps) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function handlePointerDown(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  const webPortItems = result.open_ports
    .filter((port) => port in WEB_PORT_SCHEMES)
    .map((port) => {
      const scheme = WEB_PORT_SCHEMES[port];
      const url = `${scheme}://${result.ip}:${port}`;
      return { key: `web-${port}`, label: `Im Browser öffnen (${url})`, action: () => onOpenUrl(url) };
    });
  const sshItem = result.open_ports.includes(22)
    ? { key: "ssh", label: "Per SSH verbinden", action: () => onOpenUrl(`ssh://${result.ip}`) }
    : null;
  const rdpItem = result.open_ports.includes(3389)
    ? { key: "rdp", label: "Remotedesktop öffnen (RDP)", action: onOpenRdp }
    : null;
  const shareItem = result.open_ports.includes(445)
    ? { key: "share", label: "Netzwerkfreigabe öffnen (SMB)", action: onOpenShare }
    : null;
  const connectItems = [...webPortItems, sshItem, rdpItem, shareItem].filter(
    (item): item is { key: string; label: string; action: () => void } => item !== null
  );

  function runAndClose(action: () => void) {
    action();
    setOpen(false);
  }

  const menuItemStyle: CSSProperties = {
    display: "block",
    width: "100%",
    textAlign: "left",
    background: "transparent",
    border: "none",
    padding: "0.4rem 0.6rem",
    fontSize: "0.85rem",
    borderRadius: "var(--radius-sm)",
    cursor: "pointer",
  };

  return (
    <div ref={containerRef} style={{ position: "relative", flexShrink: 0 }}>
      <button
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Aktionen für ${result.ip}`}
        style={{ fontSize: "1rem", lineHeight: 1, padding: "0.3rem 0.6rem" }}
      >
        ⋮
      </button>
      {open && (
        <div
          role="menu"
          style={{
            position: "absolute",
            right: 0,
            top: "100%",
            marginTop: "0.25rem",
            zIndex: 10,
            minWidth: "15rem",
            display: "flex",
            flexDirection: "column",
            padding: "0.25rem",
            background: "var(--bg-surface)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            boxShadow: "var(--shadow-sm)",
          }}
        >
          <button type="button" role="menuitem" style={menuItemStyle} onClick={() => runAndClose(onToggleSnmp)}>
            SNMP abfragen
          </button>
          <button
            type="button"
            role="menuitem"
            style={menuItemStyle}
            disabled={isCreated || isCreating}
            onClick={() => runAndClose(onCreateSystem)}
          >
            {isCreated ? "Angelegt ✓" : "Als System anlegen"}
          </button>
          {connectItems.length > 0 && (
            <>
              <div style={{ borderTop: "1px solid var(--border-subtle)", margin: "0.25rem 0" }} />
              {connectItems.map((item) => (
                <button
                  key={item.key}
                  type="button"
                  role="menuitem"
                  style={menuItemStyle}
                  onClick={() => runAndClose(item.action)}
                >
                  {item.label}
                </button>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}
