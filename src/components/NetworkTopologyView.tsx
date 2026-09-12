import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";
import { cidrContains } from "../lib/cidr";

// Mirrors src-tauri/src/db/networks.rs::Network.
interface Network {
  id: number;
  customer_id: number;
  name: string;
  cidr: string;
  location_id: number | null;
  notes: string;
}

// Local subset of src-tauri/src/db/locations.rs::Location -- only the
// fields this view needs, same "local minimal interface" convention
// CustomerDetailView.tsx's own SystemStat/ExpiringItemStat interfaces use
// for the same kind of partial backend shape.
interface Location {
  id: number;
  name: string;
}

// Local subset of src-tauri/src/db/systems.rs::System -- see EntryEditor.tsx's
// own System interface for the same `ip_address` field/casing on this DTO.
interface SystemSummary {
  id: number;
  name: string;
  ip_address: string;
}

const boxStyle = {
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-sm)",
  padding: "0.6rem 0.75rem",
  background: "var(--bg-surface)",
} as const;

export default function NetworkTopologyView({ customerId }: { customerId: number }) {
  const [networks, setNetworks] = useState<Network[] | null>(null);
  const [locations, setLocations] = useState<Location[] | null>(null);
  const [systems, setSystems] = useState<SystemSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setError(null);
    Promise.all([
      invoke<Network[]>("list_networks_for_customer", { customerId }),
      invoke<Location[]>("list_locations_for_customer", { customerId }),
      invoke<SystemSummary[]>("list_systems", { customerId, includeArchived: false }),
    ])
      .then(([n, l, s]) => {
        setNetworks(n);
        setLocations(l);
        setSystems(s);
      })
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerId]);

  if (error) {
    return <p style={{ color: "var(--danger)", fontSize: "0.85rem" }}>Fehler: {error}</p>;
  }
  if (networks === null || locations === null || systems === null) {
    return <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade…</p>;
  }
  const allSystems = systems;

  function systemsInNetwork(network: Network): SystemSummary[] {
    return allSystems.filter((s) => s.ip_address.trim() !== "" && cidrContains(network.cidr, s.ip_address));
  }

  const systemsByNetworkId = new Map(networks.map((n) => [n.id, systemsInNetwork(n)]));

  const matchedSystemIds = new Set(networks.flatMap((n) => (systemsByNetworkId.get(n.id) ?? []).map((s) => s.id)));
  const unassignedSystems = systems.filter((s) => !matchedSystemIds.has(s.id));

  const knownLocationIds = new Set(locations.map((l) => l.id));
  const networksByLocation = new Map<number, Network[]>();
  const unassignedNetworks: Network[] = [];
  for (const network of networks) {
    if (network.location_id === null || !knownLocationIds.has(network.location_id)) {
      unassignedNetworks.push(network);
    } else {
      const list = networksByLocation.get(network.location_id) ?? [];
      list.push(network);
      networksByLocation.set(network.location_id, list);
    }
  }

  function renderNetwork(network: Network) {
    const matched = systemsByNetworkId.get(network.id) ?? [];
    return (
      <div key={network.id} style={{ ...boxStyle, marginTop: "0.5rem", marginLeft: "1rem" }}>
        <div style={{ fontSize: "0.9rem", fontWeight: 600 }}>
          {network.name} <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)", fontWeight: 400 }}>{network.cidr}</span>
        </div>
        {matched.length === 0 ? (
          <p style={{ margin: "0.3rem 0 0", fontSize: "0.8rem", color: "var(--text-muted)" }}>
            Keine Systeme in diesem Bereich.
          </p>
        ) : (
          <ul style={{ listStyle: "none", margin: "0.3rem 0 0", padding: 0 }}>
            {matched.map((s) => (
              <li key={s.id} style={{ fontSize: "0.85rem", padding: "0.15rem 0" }}>
                {s.name} <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>{s.ip_address}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1rem" }}>
      {locations.length === 0 && networks.length === 0 && (
        <p style={{ color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Noch keine Standorte oder Netzwerke angelegt.
        </p>
      )}

      {locations.map((location) => {
        const locationNetworks = networksByLocation.get(location.id) ?? [];
        return (
          <div key={location.id} style={boxStyle}>
            <div style={{ fontSize: "1rem", fontWeight: 700 }}>{location.name}</div>
            {locationNetworks.length === 0 ? (
              <p style={{ margin: "0.3rem 0 0", fontSize: "0.8rem", color: "var(--text-muted)" }}>
                Keine Netzwerke an diesem Standort hinterlegt.
              </p>
            ) : (
              locationNetworks.map(renderNetwork)
            )}
          </div>
        );
      })}

      {unassignedNetworks.length > 0 && (
        <div style={boxStyle}>
          <div style={{ fontSize: "1rem", fontWeight: 700, color: "var(--text-muted)" }}>Ohne Standort</div>
          {unassignedNetworks.map(renderNetwork)}
        </div>
      )}

      {unassignedSystems.length > 0 && (
        <div style={boxStyle}>
          <div style={{ fontSize: "1rem", fontWeight: 700, color: "var(--text-muted)" }}>
            Systeme ohne zugeordnetes Netzwerk
          </div>
          <p style={{ margin: "0.3rem 0 0.5rem", fontSize: "0.78rem", color: "var(--text-muted)" }}>
            Keine IP-Adresse hinterlegt, oder die IP liegt in keinem der oben aufgeführten Netzwerke.
          </p>
          <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {unassignedSystems.map((s) => (
              <li key={s.id} style={{ fontSize: "0.85rem", padding: "0.15rem 0" }}>
                {s.name}
                {s.ip_address && (
                  <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}> {s.ip_address}</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
