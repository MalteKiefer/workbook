import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import type { CustomerDetailTab } from "../state/appStore";
import { formatInvokeError } from "../lib/errors";
import { isExpiringSoon } from "../lib/expiry";
import SystemListView from "./SystemListView";
import TicketsPanel from "./TicketsPanel";
import LocationsPanel from "./LocationsPanel";
import NetworksPanel from "./NetworksPanel";
import ExpiringItemsPanel from "./ExpiringItemsPanel";
import VaultPanel from "./VaultPanel";
import AuditLogPanel from "./AuditLogPanel";

interface Customer {
  id: number;
  name: string;
  short_code: string;
  notes: string;
}

// Local subset of SystemWithMaintenanceStatus (src-tauri/src/commands/systems.rs)
// -- only the fields this view's stat tile needs, same "local minimal
// interface" convention SystemListView.tsx's own System interface already
// uses for the same backend shape.
interface SystemStat {
  overdue: boolean;
}

// Local subset of ExpiringItem (src-tauri/src/db/expiring_items.rs) --
// matches DashboardView.tsx's own local interface for the same shape.
interface ExpiringItemStat {
  customer_id: number;
  expires_on: string;
  reminder_days_before: number;
}

// Same TabButton shape as SettingsView.tsx -- duplicated rather than
// extracted into a shared component, see this plan's Global Constraints.
function TabButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      style={{
        background: active ? "var(--bg-hover)" : "transparent",
        border: "1px solid " + (active ? "var(--border)" : "transparent"),
        color: active ? "var(--text-primary)" : "var(--text-secondary)",
        fontWeight: active ? 600 : 500,
      }}
    >
      {children}
    </button>
  );
}

// Same multi-word-initialism / single-word-truncation shortcode deriver as
// CustomerForm.tsx -- duplicated for the same reason TabButton is (small,
// self-contained, not worth a shared-utility file for one extra caller).
function deriveShortCode(rawName: string): string {
  const words = rawName.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "";
  if (words.length > 1) {
    return words
      .slice(0, 5)
      .map((w) => w[0])
      .join("")
      .toUpperCase();
  }
  return words[0]
    .replace(/[^a-zA-ZäöüÄÖÜß0-9]/g, "")
    .slice(0, 4)
    .toUpperCase();
}

function StatTile({ label, value }: { label: string; value: number | null }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "0.2rem",
        padding: "0.75rem 1rem",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-sm)",
        background: "var(--bg-surface)",
        minWidth: "8rem",
      }}
    >
      <span style={{ fontSize: "1.3rem", fontWeight: 600 }}>{value ?? "…"}</span>
      <span style={{ fontSize: "0.78rem", color: "var(--text-secondary)" }}>{label}</span>
    </div>
  );
}

function UebersichtTab({ customerId }: { customerId: number }) {
  const [name, setName] = useState("");
  const [shortCode, setShortCode] = useState("");
  const [notes, setNotes] = useState("");
  const [shortCodeTouched, setShortCodeTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const [systemCount, setSystemCount] = useState<number | null>(null);
  const [overdueCount, setOverdueCount] = useState<number | null>(null);
  const [expiringCount, setExpiringCount] = useState<number | null>(null);
  const [locationCount, setLocationCount] = useState<number | null>(null);

  useEffect(() => {
    setError(null);
    invoke<Customer[]>("list_customers", { includeArchived: true })
      .then((list) => {
        const match = list.find((c) => c.id === customerId);
        if (match) {
          setName(match.name);
          setShortCode(match.short_code);
          setNotes(match.notes);
          setShortCodeTouched(true);
        }
      })
      .catch((e) => setError(formatInvokeError(e)));
  }, [customerId]);

  useEffect(() => {
    invoke<SystemStat[]>("list_systems_with_maintenance_status", { customerId, includeArchived: false })
      .then((systems) => {
        setSystemCount(systems.length);
        setOverdueCount(systems.filter((s) => s.overdue).length);
      })
      .catch(() => {
        setSystemCount(0);
        setOverdueCount(0);
      });
    invoke<ExpiringItemStat[]>("list_expiring_items")
      .then((items) => {
        const forThisCustomer = items.filter((i) => i.customer_id === customerId);
        setExpiringCount(forThisCustomer.filter((i) => isExpiringSoon(i.expires_on, i.reminder_days_before)).length);
      })
      .catch(() => setExpiringCount(0));
    invoke<unknown[]>("list_locations_for_customer", { customerId })
      .then((locations) => setLocationCount(locations.length))
      .catch(() => setLocationCount(0));
  }, [customerId]);

  async function handleSave() {
    setError(null);
    setBusy(true);
    try {
      await invoke("update_customer", { id: customerId, input: { name, short_code: shortCode, notes } });
      setSaved(true);
      window.setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1rem", maxWidth: "36rem" }}>
      <div style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
        <StatTile label="Systeme" value={systemCount} />
        <StatTile label="Überfällig" value={overdueCount} />
        <StatTile label="Bald ablaufend" value={expiringCount} />
        <StatTile label="Standorte" value={locationCount} />
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          void handleSave();
        }}
        style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}
      >
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Name
          <input
            value={name}
            onChange={(e) => {
              const value = e.target.value;
              setName(value);
              if (!shortCodeTouched) setShortCode(deriveShortCode(value));
            }}
            required
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Kürzel
          <input
            value={shortCode}
            onChange={(e) => {
              setShortCode(e.target.value);
              setShortCodeTouched(true);
            }}
            style={{ fontFamily: "var(--font-mono)" }}
            required
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Notizen
          <textarea value={notes} onChange={(e) => setNotes(e.target.value)} rows={3} />
        </label>
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
        <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
          <button type="submit" className="btn-primary" disabled={busy}>
            Speichern
          </button>
          {saved && <span style={{ color: "var(--success)", fontSize: "0.85rem" }}>Gespeichert ✓</span>}
        </div>
      </form>
    </div>
  );
}

export default function CustomerDetailView() {
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const customerDetailTab = useAppStore((s) => s.customerDetailTab);
  const setCustomerDetailTab = useAppStore((s) => s.setCustomerDetailTab);

  const [customerName, setCustomerName] = useState<string | null>(null);

  useEffect(() => {
    if (selectedCustomerId === null) {
      setCustomerName(null);
      return;
    }
    invoke<{ id: number; name: string }[]>("list_customers", { includeArchived: true }).then((customers) => {
      const match = customers.find((c) => c.id === selectedCustomerId);
      setCustomerName(match ? match.name : null);
    });
  }, [selectedCustomerId]);

  if (selectedCustomerId === null) {
    return (
      <div>
        <p>Kein Kunde ausgewählt — zurück zur Kundenliste</p>
        <button onClick={goToCustomers}>Zurück zu Kunden</button>
      </div>
    );
  }

  const tabs: { id: CustomerDetailTab; label: string }[] = [
    { id: "uebersicht", label: "Übersicht" },
    { id: "systeme", label: "Systeme" },
    { id: "tickets", label: "Tickets" },
    { id: "standorte", label: "Standorte" },
    { id: "netzwerke", label: "Netzwerke" },
    { id: "ablauf", label: "Ablauf-Tracking" },
    { id: "zugangsdaten", label: "Zugangsdaten" },
    { id: "verlauf", label: "Verlauf" },
  ];

  return (
    <div>
      <button onClick={goToCustomers} style={{ marginBottom: "0.5rem" }}>
        ← Zurück zu Kunden
      </button>
      <h1 style={{ fontSize: "1.1rem", marginBottom: "0.75rem" }}>{customerName ?? `Kunde #${selectedCustomerId}`}</h1>
      <div
        style={{
          display: "flex",
          gap: "0.4rem",
          marginBottom: "1rem",
          paddingBottom: "0.6rem",
          borderBottom: "1px solid var(--border)",
        }}
      >
        {tabs.map((tab) => (
          <TabButton key={tab.id} active={customerDetailTab === tab.id} onClick={() => setCustomerDetailTab(tab.id)}>
            {tab.label}
          </TabButton>
        ))}
      </div>
      {customerDetailTab === "uebersicht" && <UebersichtTab customerId={selectedCustomerId} />}
      {customerDetailTab === "systeme" && <SystemListView />}
      {customerDetailTab === "tickets" && <TicketsPanel customerId={selectedCustomerId} />}
      {customerDetailTab === "standorte" && <LocationsPanel customerId={selectedCustomerId} />}
      {customerDetailTab === "netzwerke" && <NetworksPanel customerId={selectedCustomerId} />}
      {customerDetailTab === "ablauf" && <ExpiringItemsPanel customerId={selectedCustomerId} />}
      {customerDetailTab === "zugangsdaten" && <VaultPanel customerId={selectedCustomerId} />}
      {customerDetailTab === "verlauf" && <AuditLogPanel entityType="customer" entityId={selectedCustomerId} asTab />}
    </div>
  );
}
