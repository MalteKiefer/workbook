import { useEffect, useState } from "react";
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

  const [creatingIp, setCreatingIp] = useState<string | null>(null);
  const [createdIps, setCreatedIps] = useState<Set<string>>(new Set());
  const [createErrors, setCreateErrors] = useState<Record<string, string>>({});

  const [expandedIp, setExpandedIp] = useState<string | null>(null);
  const [communityByIp, setCommunityByIp] = useState<Record<string, string>>({});
  const [snmpBusyIp, setSnmpBusyIp] = useState<string | null>(null);
  const [snmpResultByIp, setSnmpResultByIp] = useState<Record<string, SnmpProbeResult>>({});
  const [snmpErrorByIp, setSnmpErrorByIp] = useState<Record<string, string>>({});

  const [nmapAvailable, setNmapAvailable] = useState<boolean | null>(null);
  const [nmapBusy, setNmapBusy] = useState(false);
  const [nmapOutput, setNmapOutput] = useState<string | null>(null);
  const [nmapError, setNmapError] = useState<string | null>(null);

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
      setExpandedIp(null);
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
        <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
          {results.map((result) => (
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
                <span style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
                  <button type="button" onClick={() => toggleSnmpSection(result.ip)}>
                    SNMP abfragen
                  </button>
                  {createdIps.has(result.ip) ? (
                    <button type="button" disabled>
                      Angelegt ✓
                    </button>
                  ) : (
                    <button
                      type="button"
                      onClick={() => void handleCreateSystem(result)}
                      disabled={creatingIp === result.ip}
                    >
                      Als System anlegen
                    </button>
                  )}
                </span>
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

      {nmapBusy && <p style={{ margin: 0, fontSize: "0.82rem", color: "var(--text-secondary)" }}>Scanne mit nmap…</p>}
      {nmapError && <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.82rem" }}>Fehler: {nmapError}</p>}
      {nmapOutput !== null && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "0.4rem",
            padding: "0.6rem",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
          }}
        >
          <h3 style={{ margin: 0, fontSize: "0.85rem" }}>nmap-Ausgabe</h3>
          <pre
            style={{
              margin: 0,
              padding: "0.5rem",
              fontFamily: "var(--font-mono)",
              fontSize: "0.78rem",
              overflow: "auto",
              maxHeight: "20rem",
              background: "var(--bg-surface)",
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
