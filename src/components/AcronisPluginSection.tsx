import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { formatInvokeError } from "../lib/errors";
import { getKeymap, matchesBinding } from "../lib/keymap";
import Modal from "./Modal";

// Acronis Cyber Protect Cloud (backup / cyber-protection platform,
// developer.acronis.com) plugin settings screen, ninth integration after
// NinjaOne, Level.io, Snipe-IT, Microsoft Intune, Iru (Kandji), Jamf Pro,
// Apple Business Manager, and Tactical RMM (TacticalRmmPluginSection.tsx,
// the structural template for this file), and DIFFERENT IN KIND from
// every one of those: Acronis is not an RMM/MDM, it surfaces BACKUP
// HEALTH per device, never device inventory management. See
// docs/PLUGIN_ARCHITECTURE.md "Acronis-Plugin" and plugin::acronis's own
// module docs for the full, verified API reasoning.
//
// Structurally closest to TacticalRmmPluginSection.tsx: a connection (one
// Acronis API client) can see multiple tenants, each individually mapped
// to a local Kunde, resources shown/grouped per mapped tenant. TWO
// deliberate differences, both forced by Acronis's own API shape (see
// commands::acronis module docs):
//
// 1. Tenant discovery is NOT cache-first here, unlike every other plugin
//    section's client/organization/company list. `sync_acronis_connection`
//    only ever fetches resources for tenants that are ALREADY mapped (the
//    resources API is tenant-scoped by design, see plugin::acronis module
//    docs), so there is no cached record of an UNMAPPED tenant to show.
//    Opening a connection's overlay therefore fires a live
//    `list_acronis_tenants` call every time (in addition to the usual
//    cache-first `get_cached_acronis_sync`), purely to populate the
//    Kunde-mapping candidate list. That live tenant list (with its own
//    `mapped_customer_id`) is the source of truth for the mapping
//    `<select>` here, NOT `group.customer_id` from the cached sync
//    (unlike Tactical RMM, which only ever reads mapping state off its
//    cached device groups).
// 2. There is no field-level "Vergleichen/Übernehmen" table like Tactical
//    RMM's (Name/Hostname/IP-Adresse rows): Acronis has no verified
//    single-resource detail endpoint, only a tenant-wide, UNFILTERED
//    `resource_statuses` payload (`get_acronis_system_details` returns
//    the WHOLE mapped tenant's raw response, see commands::acronis module
//    docs), and no field names inside it (beyond the already-used
//    `id`/`name`/`severity`) were ever confirmed against real docs. Rather
//    than guess at unverified field names for a compare/adopt table, the
//    details panel below just shows this resource's own raw JSON entry
//    (found by matching `id` against `device.external_id` inside the
//    returned `items` array), read-only, same "caller/UI interprets it"
//    contract the backend already documents.
//
// A Kunde-mapping select's option list, backup-status badge colors,
// keyboard j/k/Enter/l/u navigation, filtered/paginated device list, and
// the "+ Neuen Kunden anlegen…" flow otherwise follow EXACTLY the same
// conventions as TacticalRmmPluginSection.tsx/SnipeitPluginSection.tsx.
// Acronis resources have no hostname/ip_address/platform field at all
// (see plugin::acronis module docs: this plugin's only per-resource
// payload beyond identity is `backup_status`), so, unlike every RMM/MDM
// section here, the "link to existing system" match heuristic below
// compares `device.name` against the local System's own `name`, not
// `hostname` (there is no hostname to compare).

interface AcronisConnectionDto {
  id: string;
  label: string;
  datacenter_url: string;
}

interface AcronisTenantDto {
  id: string;
  name: string;
  mapped_customer_id: number | null;
}

interface AcronisResourceDto {
  external_id: string;
  name: string;
  // Acronis Alert Manager "severity", passed through verbatim, kept as a
  // free string here too (not a union type), so an unrecognized future
  // value still renders instead of failing a type check. `null` means "no
  // alert-manager entry for this resource at all" (never backed up / not
  // protected), not an error; see plugin::acronis module docs.
  backup_status: string | null;
  linked_system_id: number | null;
}

interface AcronisTenantResourceGroupDto {
  tenant_id: string;
  tenant_name: string;
  customer_id: number | null;
  devices: AcronisResourceDto[];
}

interface CachedAcronisSyncDto {
  synced_at_utc: string;
  groups: AcronisTenantResourceGroupDto[];
}

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

// Mirrors SystemForm.tsx's local System shape exactly, needed here both
// to render the "existing system" picker and to build the
// update_system/create_system input objects the same way SystemForm.tsx
// does, analogous to every other plugin section.
interface System {
  id: number;
  customer_id: number;
  name: string;
  system_type: string;
  hostname: string;
  ip_address: string;
  notes: string;
}

// Synthetic <option> value for "+ Neuen Kunden anlegen…" inside a tenant's
// Kunde-mapping <select>.
const CREATE_NEW_CUSTOMER_VALUE = "__create_new__";

// German labels + colors for Acronis's own free-form Alert Manager
// "severity" strings (see module docs on why they're not a Rust/TS enum).
// "ok"/"information" both read as "backup is fine" per the brief this
// plugin was built from; "warning" gets its own color: no --warning CSS
// custom property exists in theme.css (checked: only
// --success/--danger/--text-muted are defined), so a literal amber is used
// here, the same way TacticalRmmPluginSection.tsx's STATUS_COLORS uses
// --text-muted/--danger/--success literals for its own three-state badge.
const BACKUP_STATUS_LABELS: Record<string, string> = {
  ok: "In Ordnung",
  information: "In Ordnung",
  warning: "Warnung",
  error: "Fehler",
  critical: "Fehler",
};
const BACKUP_STATUS_COLORS: Record<string, string> = {
  ok: "var(--success)",
  information: "var(--success)",
  warning: "#d97706",
  error: "var(--danger)",
  critical: "var(--danger)",
};
const NO_STATUS_LABEL = "Kein Status";
const NO_STATUS_COLOR = "var(--text-muted)";

function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return "UTC";
  }
}

// Resources come back from the Acronis API in whatever order the API
// returns them in, so sort alphabetically by name (German collation) for
// display, independent of the text filter, analogous to
// TacticalRmmPluginSection.tsx's sortByName.
function sortByName<T extends { name: string }>(items: T[]): T[] {
  return items.slice().sort((a, b) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
}

// How many resources an expanded tenant shows per page.
const DEVICE_PAGE_SIZE = 10;

function paginateDevices<T>(items: T[], page: number): { pageItems: T[]; totalPages: number; clampedPage: number } {
  const totalPages = Math.max(1, Math.ceil(items.length / DEVICE_PAGE_SIZE));
  const clampedPage = Math.min(Math.max(page, 0), totalPages - 1);
  const start = clampedPage * DEVICE_PAGE_SIZE;
  return { pageItems: items.slice(start, start + DEVICE_PAGE_SIZE), totalPages, clampedPage };
}

const cardStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "0.6rem",
  padding: "1rem",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-sm)",
  background: "var(--bg-surface)",
};

const connectionRowStyle: CSSProperties = {
  padding: "1rem",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-sm)",
  background: "var(--bg-surface)",
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
  gap: "0.5rem",
  flexWrap: "wrap",
};

const sectionLabelStyle: CSSProperties = {
  fontSize: "0.72rem",
  textTransform: "uppercase",
  color: "var(--text-muted)",
  letterSpacing: "0.03em",
};

const mutedStyle: CSSProperties = { margin: "0.2rem 0", fontSize: "0.8rem", color: "var(--text-muted)" };

function ErrorText({ children }: { children: React.ReactNode }) {
  return <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {children}</p>;
}

function SuccessText({ children }: { children: React.ReactNode }) {
  return <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{children}</p>;
}

function KeyHint({ label }: { label: string }) {
  return (
    <span
      style={{
        fontFamily: "var(--font-mono)",
        color: "var(--text-muted)",
        fontSize: "0.72rem",
        border: "1px solid var(--border-subtle)",
        borderRadius: "3px",
        padding: "0 0.3em",
        lineHeight: 1.5,
      }}
    >
      {label}
    </span>
  );
}

// Small colored dot + label for a resource's backup-health severity.
function BackupStatusBadge({ status }: { status: string | null }) {
  const color = status ? (BACKUP_STATUS_COLORS[status] ?? "var(--text-muted)") : NO_STATUS_COLOR;
  const label = status ? (BACKUP_STATUS_LABELS[status] ?? status) : NO_STATUS_LABEL;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "0.3rem", fontSize: "0.78rem", color }}>
      <span style={{ width: "0.5em", height: "0.5em", borderRadius: "50%", background: color, display: "inline-block" }} />
      {label}
    </span>
  );
}

function PaginationBar({
  page,
  totalPages,
  onPrev,
  onNext,
}: {
  page: number;
  totalPages: number;
  onPrev: () => void;
  onNext: () => void;
}) {
  if (totalPages <= 1) return null;
  return (
    <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", justifyContent: "flex-end", padding: "0.2rem 0 0" }}>
      <button type="button" disabled={page <= 0} onClick={onPrev} style={{ fontSize: "0.78rem", padding: "0.15rem 0.5rem" }}>
        ◀ Zurück
      </button>
      <span style={{ fontSize: "0.78rem", color: "var(--text-secondary)" }}>
        Seite {page + 1} von {totalPages}
      </span>
      <button type="button" disabled={page >= totalPages - 1} onClick={onNext} style={{ fontSize: "0.78rem", padding: "0.15rem 0.5rem" }}>
        Weiter ▶
      </button>
    </div>
  );
}

function DeviceSummaryLine({ device }: { device: AcronisResourceDto }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
      <span>{device.name}</span>
      <BackupStatusBadge status={device.backup_status} />
    </span>
  );
}

// Acronis resources have no hostname field at all (see module docs), so
// the suggested match key for "Mit bestehendem System verknüpfen" is
// the resource's own display name, compared against a local System's own
// `name` field (unlike every RMM/MDM section here, which compares against
// `hostname`).
function matchKeyForDevice(device: AcronisResourceDto): string {
  return device.name.trim().toLowerCase();
}

// Finds this resource's own entry inside a get_acronis_system_details
// payload (`{"items": [...]}`, the WHOLE mapped tenant's raw
// resource_statuses response; see module docs on why this can't be
// scoped down to one resource server-side). `null` if the payload has no
// usable `items` array, or no entry with a matching `id`.
function findOwnResourceEntry(data: Record<string, unknown>, externalId: string): Record<string, unknown> | null {
  const items = data["items"];
  if (!Array.isArray(items)) return null;
  const match = items.find((item) => item && typeof item === "object" && (item as Record<string, unknown>)["id"] === externalId);
  return (match as Record<string, unknown> | undefined) ?? null;
}

export default function AcronisPluginSection() {
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);

  const [connections, setConnections] = useState<AcronisConnectionDto[]>([]);
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [connectionsError, setConnectionsError] = useState<string | null>(null);

  const [pendingCustomerCreationGroupKey, setPendingCustomerCreationGroupKey] = useState<string | null>(null);

  // Add-connection modal: four fields, unlike every other plugin section
  // here: Label, Datacenter-URL, Client ID, AND Client Secret (see module
  // docs: a genuine three-value credential, Datacenter-URL itself
  // non-secret).
  const [addFormOpen, setAddFormOpen] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [newDatacenterUrl, setNewDatacenterUrl] = useState("");
  const [newClientId, setNewClientId] = useState("");
  const [newClientSecret, setNewClientSecret] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addStatus, setAddStatus] = useState<string | null>(null);
  const [addError, setAddError] = useState<string | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  const [openConnectionId, setOpenConnectionId] = useState<string | null>(null);

  // Live tenant list per connection (see module docs: NOT cache-first,
  // unlike every other plugin section's client/organization/company list),
  // the source of truth for the Kunde-mapping <select>s below.
  const [tenantsByConnection, setTenantsByConnection] = useState<Record<string, AcronisTenantDto[] | null>>({});
  const [tenantsLoaded, setTenantsLoaded] = useState<Record<string, boolean>>({});
  const [tenantsBusy, setTenantsBusy] = useState<Record<string, boolean>>({});
  const [tenantsError, setTenantsError] = useState<Record<string, string | null>>({});

  const [tenantMapBusy, setTenantMapBusy] = useState<Record<string, boolean>>({});
  const [tenantMapError, setTenantMapError] = useState<Record<string, string | null>>({});

  // Cached (offline) resource sync per connection, only ever containing
  // groups for tenants that were mapped AT THE TIME of the last sync (see
  // module docs).
  const [cachedSyncByConnection, setCachedSyncByConnection] = useState<Record<string, CachedAcronisSyncDto | null>>({});
  const [cacheBusy, setCacheBusy] = useState<Record<string, boolean>>({});
  const [cacheError, setCacheError] = useState<Record<string, string | null>>({});
  const [syncedAtDisplay, setSyncedAtDisplay] = useState<Record<string, string>>({});

  const [syncBusy, setSyncBusy] = useState<Record<string, boolean>>({});
  const [syncStatus, setSyncStatus] = useState<Record<string, string | null>>({});
  const [syncError, setSyncError] = useState<Record<string, string | null>>({});

  const [removeBusy, setRemoveBusy] = useState<Record<string, boolean>>({});

  const [deviceFilter, setDeviceFilter] = useState<Record<string, string>>({});

  // Which tenants are expanded, collapsed by default, keyed by
  // `${connectionId}:${tenantId}`, same convention as
  // TacticalRmmPluginSection.tsx's client groups.
  const [expandedGroupKeys, setExpandedGroupKeys] = useState<Set<string>>(new Set());

  const [pageByGroup, setPageByGroup] = useState<Record<string, number>>({});

  const [activeGroupKey, setActiveGroupKey] = useState<string | null>(null);
  const [selectedIndexByGroup, setSelectedIndexByGroup] = useState<Record<string, number>>({});

  const firstFilterInputRef = useRef<HTMLInputElement | null>(null);
  const autoFocusedConnectionRef = useRef<string | null>(null);

  const [localSystemsByCustomer, setLocalSystemsByCustomer] = useState<Record<number, System[]>>({});

  const [linkPickerKey, setLinkPickerKey] = useState<string | null>(null);
  const [linkPickerSelection, setLinkPickerSelection] = useState<number | "">("");
  const [linkBusy, setLinkBusy] = useState<Record<string, boolean>>({});
  const [createLinkBusy, setCreateLinkBusy] = useState<Record<string, boolean>>({});
  const [unlinkBusy, setUnlinkBusy] = useState<Record<string, boolean>>({});
  const [deviceError, setDeviceError] = useState<Record<string, string | null>>({});
  // Group-scoped, keyed by `${connectionId}:${tenantId}` — disables the
  // "Alle anlegen" bulk button for one tenant while it works through that
  // tenant's unlinked resources, independent of the per-device
  // `createLinkBusy` map (both are set during a bulk run, so a resource's
  // own row also shows busy).
  const [bulkCreateBusy, setBulkCreateBusy] = useState<Record<string, boolean>>({});

  const [detailsOpenKey, setDetailsOpenKey] = useState<string | null>(null);
  const [detailsBusy, setDetailsBusy] = useState<Record<string, boolean>>({});
  const [detailsError, setDetailsError] = useState<Record<string, string | null>>({});
  // The found own-resource entry (or null if not present in the returned
  // payload), NOT the raw compare-table data TacticalRmmPluginSection.tsx
  // keeps, see module docs on why there's no field-level compare/adopt
  // table here.
  const [detailsData, setDetailsData] = useState<Record<string, Record<string, unknown> | null>>({});

  const reloadConnections = useCallback(() => {
    invoke<AcronisConnectionDto[]>("list_acronis_connections")
      .then(setConnections)
      .catch((e) => setConnectionsError(formatInvokeError(e)));
  }, []);

  const refreshCustomers = useCallback(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false })
      .then(setCustomers)
      .catch(() => {});
  }, []);

  useEffect(() => {
    reloadConnections();
    refreshCustomers();
  }, [reloadConnections, refreshCustomers]);

  const prevCustomerEditorTargetRef = useRef(customerEditorTarget);
  useEffect(() => {
    if (prevCustomerEditorTargetRef.current !== null && customerEditorTarget === null && pendingCustomerCreationGroupKey) {
      const groupKey = pendingCustomerCreationGroupKey;
      const previousCustomers = customers;
      void (async () => {
        try {
          const freshCustomers = await invoke<Customer[]>("list_customers", { includeArchived: false });
          setCustomers(freshCustomers);
          const previousIds = new Set(previousCustomers.map((c) => c.id));
          const newlyCreated = freshCustomers.filter((c) => !previousIds.has(c.id));
          if (newlyCreated.length === 1) {
            const separatorIdx = groupKey.indexOf(":");
            const connectionId = groupKey.slice(0, separatorIdx);
            const tenantId = groupKey.slice(separatorIdx + 1);
            const connection = connections.find((c) => c.id === connectionId);
            const tenant = tenantsByConnection[connectionId]?.find((t) => t.id === tenantId);
            if (connection && tenant) {
              await handleTenantMappingChange(connection, tenant, String(newlyCreated[0].id));
            }
          }
        } catch {
          // Best-effort auto-select; the manual 🔄 refresh button next to
          // each select remains as a fallback.
        } finally {
          setPendingCustomerCreationGroupKey(null);
        }
      })();
    }
    prevCustomerEditorTargetRef.current = customerEditorTarget;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [customerEditorTarget, pendingCustomerCreationGroupKey, customers, connections, tenantsByConnection]);

  async function refreshLocalSystems(customerId: number): Promise<System[]> {
    const list = await invoke<System[]>("list_systems", { customerId, includeArchived: false });
    setLocalSystemsByCustomer((prev) => ({ ...prev, [customerId]: list }));
    return list;
  }

  async function updateSyncedAtDisplay(connectionId: string, utc: string) {
    try {
      const display = await invoke<string>("format_timestamp_for_display", { utc, tz: localTimeZone() });
      setSyncedAtDisplay((prev) => ({ ...prev, [connectionId]: display }));
    } catch {
      setSyncedAtDisplay((prev) => ({ ...prev, [connectionId]: utc }));
    }
  }

  // Live fetch of this connection's customer-kind tenants (see module
  // docs: NOT cache-first). Also seeds localSystemsByCustomer for every
  // already-mapped tenant, so the "link to existing system" picker/match
  // heuristic works immediately.
  const loadTenants = useCallback(async (connection: AcronisConnectionDto) => {
    const id = connection.id;
    setTenantsBusy((prev) => ({ ...prev, [id]: true }));
    setTenantsError((prev) => ({ ...prev, [id]: null }));
    try {
      const tenants = await invoke<AcronisTenantDto[]>("list_acronis_tenants", { connectionId: id });
      setTenantsByConnection((prev) => ({ ...prev, [id]: tenants }));
      setTenantsLoaded((prev) => ({ ...prev, [id]: true }));
      const customerIds = Array.from(
        new Set(tenants.map((t) => t.mapped_customer_id).filter((c): c is number => c !== null)),
      );
      await Promise.all(customerIds.map((cid) => refreshLocalSystems(cid).catch(() => {})));
    } catch (err) {
      setTenantsError((prev) => ({ ...prev, [id]: formatInvokeError(err) }));
    } finally {
      setTenantsBusy((prev) => ({ ...prev, [id]: false }));
    }
  }, []);

  const loadCachedSync = useCallback(async (connection: AcronisConnectionDto) => {
    const id = connection.id;
    setCacheBusy((prev) => ({ ...prev, [id]: true }));
    setCacheError((prev) => ({ ...prev, [id]: null }));
    try {
      const cached = await invoke<CachedAcronisSyncDto | null>("get_cached_acronis_sync", { connectionId: id });
      setCachedSyncByConnection((prev) => ({ ...prev, [id]: cached }));
      if (cached) {
        await updateSyncedAtDisplay(id, cached.synced_at_utc);
      }
    } catch (err) {
      setCacheError((prev) => ({ ...prev, [id]: formatInvokeError(err) }));
    } finally {
      setCacheBusy((prev) => ({ ...prev, [id]: false }));
    }
  }, []);

  function openTenantDeviceModal(connection: AcronisConnectionDto) {
    setOpenConnectionId(connection.id);
    setActiveGroupKey(null);
    setSelectedIndexByGroup({});
    autoFocusedConnectionRef.current = null;
    void loadTenants(connection);
    void loadCachedSync(connection);
  }

  function closeTenantDeviceModal() {
    setOpenConnectionId(null);
  }

  async function handleTestConnection() {
    setTestBusy(true);
    setTestResult(null);
    try {
      await invoke("test_acronis_connection", {
        datacenterUrl: newDatacenterUrl,
        clientId: newClientId,
        clientSecret: newClientSecret,
      });
      setTestResult({ ok: true, message: "✓ Verbindung erfolgreich" });
    } catch (err) {
      setTestResult({ ok: false, message: formatInvokeError(err) });
    } finally {
      setTestBusy(false);
    }
  }

  async function handleAddConnection(e: React.FormEvent) {
    e.preventDefault();
    setAddError(null);
    setAddStatus(null);
    setAddBusy(true);
    try {
      await invoke("add_acronis_connection", {
        label: newLabel,
        datacenterUrl: newDatacenterUrl,
        clientId: newClientId,
        clientSecret: newClientSecret,
      });
      setAddStatus(`Verbindung "${newLabel}" hinzugefügt.`);
      setNewLabel("");
      setNewDatacenterUrl("");
      setNewClientId("");
      setNewClientSecret("");
      setTestResult(null);
      reloadConnections();
      setAddFormOpen(false);
    } catch (err) {
      setAddError(formatInvokeError(err));
    } finally {
      setAddBusy(false);
    }
  }

  async function handleRemoveConnection(connection: AcronisConnectionDto) {
    const confirmed = window.confirm(`Acronis-Verbindung "${connection.label}" wirklich entfernen?`);
    if (!confirmed) return;
    const id = connection.id;
    setRemoveBusy((prev) => ({ ...prev, [id]: true }));
    setConnectionsError(null);
    try {
      await invoke("remove_acronis_connection", { id });
      if (openConnectionId === id) setOpenConnectionId(null);
      setCachedSyncByConnection((prev) => {
        const next = { ...prev };
        delete next[id];
        return next;
      });
      setTenantsByConnection((prev) => {
        const next = { ...prev };
        delete next[id];
        return next;
      });
      reloadConnections();
    } catch (err) {
      setConnectionsError(formatInvokeError(err));
    } finally {
      setRemoveBusy((prev) => ({ ...prev, [id]: false }));
    }
  }

  async function handleSync(connection: AcronisConnectionDto) {
    const id = connection.id;
    setSyncBusy((prev) => ({ ...prev, [id]: true }));
    setSyncStatus((prev) => ({ ...prev, [id]: null }));
    setSyncError((prev) => ({ ...prev, [id]: null }));
    try {
      const groups = await invoke<AcronisTenantResourceGroupDto[]>("sync_acronis_connection", { connectionId: id });
      const deviceCount = groups.reduce((sum, g) => sum + g.devices.length, 0);
      setSyncStatus((prev) => ({
        ...prev,
        [id]: `${groups.length} Mandant(en) synchronisiert, ${deviceCount} Ressource(n) gefunden.`,
      }));
      await loadCachedSync(connection);
      setPageByGroup((prev) => {
        const next = { ...prev };
        for (const key of Object.keys(next)) {
          if (key.startsWith(`${id}:`)) next[key] = 0;
        }
        return next;
      });
    } catch (err) {
      setSyncError((prev) => ({ ...prev, [id]: formatInvokeError(err) }));
    } finally {
      setSyncBusy((prev) => ({ ...prev, [id]: false }));
    }
  }

  async function handleTenantMappingChange(connection: AcronisConnectionDto, tenant: AcronisTenantDto, rawValue: string) {
    const mapKey = `${connection.id}:${tenant.id}`;
    setTenantMapBusy((prev) => ({ ...prev, [mapKey]: true }));
    setTenantMapError((prev) => ({ ...prev, [mapKey]: null }));
    try {
      if (rawValue === "") {
        await invoke("unmap_acronis_tenant", { connectionId: connection.id, tenantId: tenant.id });
      } else {
        const customerId = Number(rawValue);
        await invoke("map_acronis_tenant", {
          connectionId: connection.id,
          tenantId: tenant.id,
          tenantName: tenant.name,
          customerId,
        });
        void refreshLocalSystems(customerId);
      }
      // Optimistic local update so the select reflects the new mapping
      // immediately; this IS the authoritative state here (see module
      // docs), no cache re-read needed for the mapping status itself.
      setTenantsByConnection((prev) => {
        const current = prev[connection.id];
        if (!current) return prev;
        return {
          ...prev,
          [connection.id]: current.map((t) =>
            t.id === tenant.id ? { ...t, mapped_customer_id: rawValue === "" ? null : Number(rawValue) } : t,
          ),
        };
      });
      // Re-read the cache too (no network) so an already-cached group
      // picks up an unmapped tenant's customer_id becoming None right away.
      await loadCachedSync(connection);
    } catch (err) {
      setTenantMapError((prev) => ({ ...prev, [mapKey]: formatInvokeError(err) }));
    } finally {
      setTenantMapBusy((prev) => ({ ...prev, [mapKey]: false }));
    }
  }

  function openLinkPicker(connection: AcronisConnectionDto, tenant: AcronisTenantDto, device: AcronisResourceDto) {
    if (tenant.mapped_customer_id === null) return;
    const customerId = tenant.mapped_customer_id;
    const key = `${connection.id}:${device.external_id}`;
    setLinkPickerKey(key);
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    const localSystems = localSystemsByCustomer[customerId] ?? [];
    const deviceKey = matchKeyForDevice(device);
    const match = deviceKey !== "" ? localSystems.find((s) => s.name.trim().toLowerCase() === deviceKey) : undefined;
    setLinkPickerSelection(match ? match.id : "");
    if (!localSystemsByCustomer[customerId]) {
      void refreshLocalSystems(customerId);
    }
  }

  async function confirmLink(connection: AcronisConnectionDto, tenant: AcronisTenantDto, device: AcronisResourceDto) {
    if (linkPickerSelection === "") return;
    const key = `${connection.id}:${device.external_id}`;
    setLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("link_system_to_acronis", {
        systemId: linkPickerSelection,
        connectionId: connection.id,
        tenantId: tenant.id,
        externalId: device.external_id,
      });
      setLinkPickerKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setLinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function createAndLink(connection: AcronisConnectionDto, tenant: AcronisTenantDto, device: AcronisResourceDto) {
    if (tenant.mapped_customer_id === null) return;
    const customerId = tenant.mapped_customer_id;
    const key = `${connection.id}:${device.external_id}`;
    setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      // A local System has no backup-status field of its own, so rather
      // than silently losing that Acronis-native context on creation, the
      // status at creation time is seeded into "Notizen" once, up front,
      // the same "one-time default, never an automatic overwrite later"
      // convention as TacticalRmmPluginSection.tsx's Site/Plattform notes
      // (see docs/PLUGIN_ARCHITECTURE.md's "external data never
      // auto-overwrites self-maintained fields" rule).
      const statusLabel = device.backup_status
        ? (BACKUP_STATUS_LABELS[device.backup_status] ?? device.backup_status)
        : NO_STATUS_LABEL;
      const created = await invoke<System>("create_system", {
        input: {
          customer_id: customerId,
          name: device.name,
          system_type: "",
          hostname: "",
          ip_address: "",
          notes: `Acronis-Sicherungsstatus bei Anlage: ${statusLabel}`,
        },
      });
      await invoke("link_system_to_acronis", {
        systemId: created.id,
        connectionId: connection.id,
        tenantId: tenant.id,
        externalId: device.external_id,
      });
      await refreshLocalSystems(customerId);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setCreateLinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  // Bulk version of `createAndLink`: works through every unlinked resource
  // in one tenant sequentially. A failure on one resource does not abort
  // the rest — it is recorded in the same `deviceError` map that already
  // surfaces per-resource errors, so a partial run still leaves the row's
  // own "Neu anlegen" button as the retry path. `refreshLocalSystems`/
  // `loadCachedSync` run once at the end, not per resource, so a tenant
  // with many resources doesn't refetch the whole customer's system list N
  // times.
  async function createAndLinkAll(connection: AcronisConnectionDto, tenant: AcronisTenantDto, devices: AcronisResourceDto[]) {
    if (tenant.mapped_customer_id === null || devices.length === 0) return;
    const customerId = tenant.mapped_customer_id;
    const groupKey = `${connection.id}:${tenant.id}`;
    setBulkCreateBusy((prev) => ({ ...prev, [groupKey]: true }));
    try {
      for (const device of devices) {
        const key = `${connection.id}:${device.external_id}`;
        setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
        setDeviceError((prev) => ({ ...prev, [key]: null }));
        try {
          const statusLabel = device.backup_status
            ? (BACKUP_STATUS_LABELS[device.backup_status] ?? device.backup_status)
            : NO_STATUS_LABEL;
          const created = await invoke<System>("create_system", {
            input: {
              customer_id: customerId,
              name: device.name,
              system_type: "",
              hostname: "",
              ip_address: "",
              notes: `Acronis-Sicherungsstatus bei Anlage: ${statusLabel}`,
            },
          });
          await invoke("link_system_to_acronis", {
            systemId: created.id,
            connectionId: connection.id,
            tenantId: tenant.id,
            externalId: device.external_id,
          });
        } catch (err) {
          setDeviceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
        } finally {
          setCreateLinkBusy((prev) => ({ ...prev, [key]: false }));
        }
      }
      await refreshLocalSystems(customerId);
      await loadCachedSync(connection);
    } finally {
      setBulkCreateBusy((prev) => ({ ...prev, [groupKey]: false }));
    }
  }

  async function handleUnlink(connection: AcronisConnectionDto, device: AcronisResourceDto) {
    if (device.linked_system_id === null) return;
    const key = `${connection.id}:${device.external_id}`;
    setUnlinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("unlink_system_from_acronis", { systemId: device.linked_system_id, connectionId: connection.id });
      if (detailsOpenKey === key) setDetailsOpenKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setUnlinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function toggleDetails(connection: AcronisConnectionDto, tenant: AcronisTenantDto, device: AcronisResourceDto) {
    const key = `${connection.id}:${device.external_id}`;
    if (detailsOpenKey === key) {
      setDetailsOpenKey(null);
      return;
    }
    setDetailsOpenKey(key);
    setDetailsError((prev) => ({ ...prev, [key]: null }));
    setDetailsBusy((prev) => ({ ...prev, [key]: true }));
    try {
      const data = await invoke<Record<string, unknown>>("get_acronis_system_details", {
        connectionId: connection.id,
        tenantId: tenant.id,
      });
      setDetailsData((prev) => ({ ...prev, [key]: findOwnResourceEntry(data, device.external_id) }));
    } catch (err) {
      setDetailsError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setDetailsBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  // Combines a tenant's filtered "unlinked" and "linked" devices into one
  // flat, keyboard-navigable list, shared by renderDeviceGroup (for
  // rendering) and the global keydown handler below, analogous to
  // TacticalRmmPluginSection.tsx's computeGroupDevices. Devices come from
  // the CACHED sync group for this tenant (if any); see module docs on
  // why an unsynced/unmapped tenant simply has none.
  function computeGroupDevices(connectionId: string, tenant: AcronisTenantDto, cachedGroup: AcronisTenantResourceGroupDto | undefined) {
    const groupKey = `${connectionId}:${tenant.id}`;
    const filterText = deviceFilter[groupKey] ?? "";
    const q = filterText.trim().toLowerCase();
    const allDevices = cachedGroup?.devices ?? [];
    const sortedDevices = sortByName(allDevices);
    const filteredDevices =
      q === "" ? sortedDevices : sortedDevices.filter((d) => d.name.toLowerCase().includes(q));
    const isMapped = tenant.mapped_customer_id !== null;
    const linked = isMapped ? filteredDevices.filter((d) => d.linked_system_id !== null) : [];
    const unlinked = isMapped ? filteredDevices.filter((d) => d.linked_system_id === null) : [];
    const navItems: { device: AcronisResourceDto; kind: "unlinked" | "linked" }[] = [
      ...unlinked.map((device) => ({ device, kind: "unlinked" as const })),
      ...linked.map((device) => ({ device, kind: "linked" as const })),
    ];
    return { groupKey, filterText, filteredDevices, isMapped, linked, unlinked, navItems };
  }

  // Global j/k (+ ArrowDown/ArrowUp) / Enter / l / u handling for the
  // currently keyboard-active tenant's device list, the same
  // isTypingTarget-gated pattern as TacticalRmmPluginSection.tsx.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (!openConnectionId || addFormOpen) return;
      if (isTypingTarget(document.activeElement)) return;
      const connection = connections.find((c) => c.id === openConnectionId);
      const tenants = tenantsByConnection[openConnectionId];
      const cached = cachedSyncByConnection[openConnectionId];
      if (!connection || !tenants) return;
      const activeTenant = activeGroupKey
        ? tenants.find((t) => `${openConnectionId}:${t.id}` === activeGroupKey)
        : undefined;
      if (!activeTenant) return;
      const cachedGroup = cached?.groups.find((g) => g.tenant_id === activeTenant.id);
      const { groupKey, navItems } = computeGroupDevices(connection.id, activeTenant, cachedGroup);
      if (!expandedGroupKeys.has(groupKey)) return;
      if (navItems.length === 0) return;
      const rawPage = pageByGroup[groupKey] ?? 0;
      const { pageItems, totalPages, clampedPage } = paginateDevices(navItems, rawPage);
      if (pageItems.length === 0) return;
      const index = Math.min(selectedIndexByGroup[groupKey] ?? 0, pageItems.length - 1);

      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next) || e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndexByGroup((prev) => ({ ...prev, [groupKey]: Math.min(index + 1, pageItems.length - 1) }));
      } else if (matchesBinding(e, keymap.list_prev) || e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndexByGroup((prev) => ({ ...prev, [groupKey]: Math.max(index - 1, 0) }));
      } else if (e.key === "Enter") {
        const item = pageItems[index];
        if (item) {
          e.preventDefault();
          if (item.kind === "unlinked") {
            void createAndLink(connection, activeTenant, item.device);
          } else {
            void toggleDetails(connection, activeTenant, item.device);
          }
        }
      } else if (e.key === "l") {
        const item = pageItems[index];
        if (item && item.kind === "unlinked") {
          e.preventDefault();
          openLinkPicker(connection, activeTenant, item.device);
        }
      } else if (e.key === "u") {
        const item = pageItems[index];
        if (item && item.kind === "linked") {
          e.preventDefault();
          void handleUnlink(connection, item.device);
        }
      } else if (e.key === "ArrowRight") {
        if (clampedPage < totalPages - 1) {
          e.preventDefault();
          setPageByGroup((prev) => ({ ...prev, [groupKey]: clampedPage + 1 }));
          setSelectedIndexByGroup((prev) => ({ ...prev, [groupKey]: 0 }));
        }
      } else if (e.key === "ArrowLeft") {
        if (clampedPage > 0) {
          e.preventDefault();
          setPageByGroup((prev) => ({ ...prev, [groupKey]: clampedPage - 1 }));
          setSelectedIndexByGroup((prev) => ({ ...prev, [groupKey]: 0 }));
        }
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    openConnectionId,
    addFormOpen,
    connections,
    tenantsByConnection,
    cachedSyncByConnection,
    activeGroupKey,
    selectedIndexByGroup,
    deviceFilter,
    expandedGroupKeys,
    pageByGroup,
  ]);

  // Default/track which tenant is keyboard-active: keep the current one if
  // it's still present, otherwise fall back to the first mapped tenant
  // that has any cached devices at all.
  useEffect(() => {
    if (!openConnectionId) return;
    const tenants = tenantsByConnection[openConnectionId];
    const cached = cachedSyncByConnection[openConnectionId];
    if (!tenants) return;
    setActiveGroupKey((prev) => {
      if (prev && tenants.some((t) => `${openConnectionId}:${t.id}` === prev)) return prev;
      const firstWithDevices = tenants.find((t) => (cached?.groups.find((g) => g.tenant_id === t.id)?.devices.length ?? 0) > 0);
      return firstWithDevices ? `${openConnectionId}:${firstWithDevices.id}` : null;
    });
  }, [openConnectionId, tenantsByConnection, cachedSyncByConnection]);

  useEffect(() => {
    if (!openConnectionId) return;
    if (autoFocusedConnectionRef.current === openConnectionId) return;
    const tenants = tenantsByConnection[openConnectionId];
    const cached = cachedSyncByConnection[openConnectionId];
    if (!tenants || !tenants.some((t) => (cached?.groups.find((g) => g.tenant_id === t.id)?.devices.length ?? 0) > 0)) return;
    if (firstFilterInputRef.current) {
      firstFilterInputRef.current.focus();
      autoFocusedConnectionRef.current = openConnectionId;
    }
  }, [openConnectionId, tenantsByConnection, cachedSyncByConnection]);

  function renderDetailsPanel(device: AcronisResourceDto, key: string) {
    const busy = detailsBusy[key] ?? false;
    const err = detailsError[key] ?? null;
    const ownEntry = detailsData[key];

    return (
      <div
        style={{
          marginTop: "0.3rem",
          padding: "0.6rem",
          background: "var(--bg-elevated)",
          border: "1px solid var(--border-subtle)",
          borderRadius: "var(--radius-sm)",
          display: "flex",
          flexDirection: "column",
          gap: "0.4rem",
        }}
      >
        <div style={{ display: "flex", gap: "0.75rem", alignItems: "center", flexWrap: "wrap", fontSize: "0.8rem", color: "var(--text-secondary)" }}>
          <BackupStatusBadge status={device.backup_status} />
        </div>
        {busy && <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Lade Acronis-Details…</p>}
        {err && <ErrorText>{err}</ErrorText>}
        {!busy && !err && ownEntry === null && (
          <p style={mutedStyle}>Keine Rohdaten für diese Ressource in der Acronis-Antwort gefunden.</p>
        )}
        {!busy && !err && ownEntry !== null && ownEntry !== undefined && (
          <pre
            style={{
              fontSize: "0.75rem",
              background: "var(--bg-base)",
              padding: "0.5rem",
              borderRadius: "var(--radius-sm)",
              overflowX: "auto",
              maxHeight: "16rem",
              margin: 0,
            }}
          >
            {JSON.stringify(ownEntry, null, 2)}
          </pre>
        )}
      </div>
    );
  }

  function renderDeviceGroup(connection: AcronisConnectionDto, tenant: AcronisTenantDto, cachedGroup: AcronisTenantResourceGroupDto | undefined) {
    const { groupKey, filterText, filteredDevices, isMapped, linked, unlinked, navItems } = computeGroupDevices(
      connection.id,
      tenant,
      cachedGroup,
    );
    const localSystems = tenant.mapped_customer_id !== null ? localSystemsByCustomer[tenant.mapped_customer_id] ?? [] : [];
    const mapKey = groupKey;
    const mapBusyFlag = tenantMapBusy[mapKey] ?? false;
    const mapErr = tenantMapError[mapKey] ?? null;
    const isGroupActive = activeGroupKey === groupKey;
    const isExpanded = expandedGroupKeys.has(groupKey);
    const totalDeviceCount = cachedGroup?.devices.length ?? 0;
    const linkedDeviceCount = cachedGroup?.devices.filter((d) => d.linked_system_id !== null).length ?? 0;
    const hasSyncedThisTenant = cachedGroup !== undefined;

    const rawPage = pageByGroup[groupKey] ?? 0;
    let pageItems: { device: AcronisResourceDto; kind: "unlinked" | "linked" }[] = [];
    let totalPages = 1;
    let clampedPage = 0;
    if (isMapped) {
      const paged = paginateDevices(navItems, rawPage);
      pageItems = paged.pageItems;
      totalPages = paged.totalPages;
      clampedPage = paged.clampedPage;
    }
    const rawSelectedIndex = selectedIndexByGroup[groupKey] ?? 0;
    const effectiveSelectedIndex = pageItems.length === 0 ? -1 : Math.min(rawSelectedIndex, pageItems.length - 1);

    const pageUnlinked: { device: AcronisResourceDto; navIdx: number }[] = [];
    const pageLinked: { device: AcronisResourceDto; navIdx: number }[] = [];
    pageItems.forEach((item, navIdx) => {
      if (item.kind === "unlinked") pageUnlinked.push({ device: item.device, navIdx });
      else pageLinked.push({ device: item.device, navIdx });
    });

    function selectRow(navIdx: number) {
      setActiveGroupKey(groupKey);
      setSelectedIndexByGroup((prev) => ({ ...prev, [groupKey]: navIdx }));
    }
    function toggleExpanded() {
      setExpandedGroupKeys((prev) => {
        const next = new Set(prev);
        if (next.has(groupKey)) {
          next.delete(groupKey);
        } else {
          next.add(groupKey);
        }
        return next;
      });
      if (!isExpanded) {
        setPageByGroup((prev) => ({ ...prev, [groupKey]: 0 }));
        setActiveGroupKey(groupKey);
      }
    }
    function goToPage(nextPage: number) {
      setPageByGroup((prev) => ({ ...prev, [groupKey]: nextPage }));
      setSelectedIndexByGroup((prev) => ({ ...prev, [groupKey]: 0 }));
      setActiveGroupKey(groupKey);
    }

    return (
      <div
        key={tenant.id}
        style={{
          border: "1px solid var(--border-subtle)",
          borderRadius: "var(--radius-sm)",
          padding: "0.6rem",
          display: "flex",
          flexDirection: "column",
          gap: "0.4rem",
        }}
      >
        <div
          role="button"
          tabIndex={0}
          aria-expanded={isExpanded}
          onClick={toggleExpanded}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              toggleExpanded();
            }
          }}
          style={{ display: "flex", alignItems: "center", gap: "0.5rem", cursor: "pointer", userSelect: "none" }}
        >
          <span
            aria-hidden="true"
            style={{ fontSize: "0.75rem", color: "var(--text-secondary)", width: "0.9em", display: "inline-block", textAlign: "center" }}
          >
            {isExpanded ? "▾" : "▸"}
          </span>
          <span style={{ fontWeight: 600, fontSize: "0.9rem" }}>{tenant.name}</span>
          {hasSyncedThisTenant && (
            <span style={{ fontSize: "0.78rem", color: "var(--text-secondary)" }}>
              {totalDeviceCount} Ressource{totalDeviceCount === 1 ? "" : "n"}
              {isMapped && `, ${linkedDeviceCount} verknüpft`}
            </span>
          )}
        </div>

        <div style={{ display: "flex", gap: "0.4rem", alignItems: "center", flexWrap: "wrap" }}>
          <span style={sectionLabelStyle}>Kunde</span>
          <select
            value={tenant.mapped_customer_id ?? ""}
            disabled={mapBusyFlag}
            onChange={(e) => {
              const raw = e.target.value;
              if (raw === CREATE_NEW_CUSTOMER_VALUE) {
                e.target.value = tenant.mapped_customer_id === null ? "" : String(tenant.mapped_customer_id);
                setPendingCustomerCreationGroupKey(groupKey);
                openCustomerEditor("new");
                return;
              }
              void handleTenantMappingChange(connection, tenant, raw);
            }}
          >
            <option value="">— nicht zugeordnet —</option>
            {customers.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} ({c.short_code})
              </option>
            ))}
            <option value={CREATE_NEW_CUSTOMER_VALUE}>+ Neuen Kunden anlegen…</option>
          </select>
          {mapBusyFlag && <span style={{ fontSize: "0.75rem", color: "var(--text-muted)" }}>speichere…</span>}
        </div>
        {mapErr && <ErrorText>{mapErr}</ErrorText>}

        {isExpanded && (
          <>
            {!isMapped && (
              <p style={mutedStyle}>Dieser Mandant ist noch keinem Kunden zugeordnet — oben zuordnen, um Ressourcen zu verknüpfen.</p>
            )}
            {isMapped && !hasSyncedThisTenant && (
              <p style={mutedStyle}>
                Noch nicht synchronisiert. Auf "Aktualisieren" oben klicken, um Ressourcen für diesen Mandanten zu laden.
              </p>
            )}
            {isMapped && hasSyncedThisTenant && totalDeviceCount === 0 && (
              <p style={mutedStyle}>Keine Ressourcen bei diesem Mandanten.</p>
            )}

            {isMapped && hasSyncedThisTenant && totalDeviceCount > 0 && (
              <input
                ref={groupKey === activeGroupKey ? firstFilterInputRef : undefined}
                value={filterText}
                onChange={(e) => {
                  const value = e.target.value;
                  setDeviceFilter((prev) => ({ ...prev, [groupKey]: value }));
                  setPageByGroup((prev) => ({ ...prev, [groupKey]: 0 }));
                }}
                onFocus={() => setActiveGroupKey(groupKey)}
                placeholder={`Ressourcen filtern (${totalDeviceCount})…`}
                style={{ maxWidth: "20rem" }}
              />
            )}

            {isMapped && hasSyncedThisTenant && (
              <>
                <div>
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "0.5rem" }}>
                    <div style={sectionLabelStyle}>Nicht verknüpft ({unlinked.length})</div>
                    {unlinked.length > 0 && (
                      <button
                        type="button"
                        disabled={bulkCreateBusy[groupKey] ?? false}
                        onClick={() => void createAndLinkAll(connection, tenant, unlinked)}
                      >
                        {bulkCreateBusy[groupKey] ? "Lege an…" : `Alle anlegen (${unlinked.length})`}
                      </button>
                    )}
                  </div>
                  {filteredDevices.length === 0 && totalDeviceCount > 0 && (
                    <p style={mutedStyle}>Keine Ressourcen entsprechen dem Filter.</p>
                  )}
                  {unlinked.length === 0 && filteredDevices.length > 0 && <p style={mutedStyle}>Keine offenen Ressourcen.</p>}
                  {unlinked.length > 0 && pageUnlinked.length === 0 && (
                    <p style={mutedStyle}>Keine offenen Ressourcen auf dieser Seite.</p>
                  )}
                  <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
                    {pageUnlinked.map(({ device, navIdx }, i) => {
                      const key = `${connection.id}:${device.external_id}`;
                      const isSelected = isGroupActive && navIdx === effectiveSelectedIndex;
                      const pickerOpen = linkPickerKey === key;
                      const busyLink = linkBusy[key] ?? false;
                      const busyCreate = createLinkBusy[key] ?? false;
                      const rowErr = deviceError[key] ?? null;
                      return (
                        <li key={device.external_id} style={{ borderTop: i === 0 ? "none" : "1px solid var(--border-subtle)" }}>
                          <div
                            className="list-row"
                            role="option"
                            aria-selected={isSelected}
                            onMouseEnter={() => selectRow(navIdx)}
                            onClick={() => selectRow(navIdx)}
                            style={{
                              display: "flex",
                              justifyContent: "space-between",
                              alignItems: "center",
                              gap: "0.5rem",
                              flexWrap: "wrap",
                              padding: "0.4rem 0.6rem",
                              background: isSelected ? "var(--bg-selected)" : "transparent",
                            }}
                          >
                            <DeviceSummaryLine device={device} />
                            <span style={{ display: "flex", alignItems: "center", gap: "0.4rem", flexShrink: 0 }}>
                              <button type="button" disabled={busyCreate} onClick={() => void createAndLink(connection, tenant, device)}>
                                {busyCreate ? "Lege an…" : "Neu anlegen"}
                              </button>
                              <KeyHint label="Enter" />
                              <button type="button" disabled={busyCreate} onClick={() => openLinkPicker(connection, tenant, device)}>
                                Verknüpfen…
                              </button>
                              <KeyHint label="l" />
                            </span>
                          </div>
                          {pickerOpen && (
                            <div style={{ display: "flex", gap: "0.4rem", alignItems: "center", padding: "0.3rem 0.6rem 0" }}>
                              <select
                                value={linkPickerSelection}
                                onChange={(e) => setLinkPickerSelection(e.target.value === "" ? "" : Number(e.target.value))}
                                autoFocus
                              >
                                <option value="">System wählen…</option>
                                {sortByName(localSystems).map((s) => (
                                  <option key={s.id} value={s.id}>
                                    {s.name}
                                    {s.hostname ? ` — ${s.hostname}` : ""}
                                  </option>
                                ))}
                              </select>
                              <button
                                type="button"
                                disabled={linkPickerSelection === "" || busyLink}
                                onClick={() => void confirmLink(connection, tenant, device)}
                              >
                                {busyLink ? "Verknüpfe…" : "Verknüpfen"}
                              </button>
                              <button type="button" onClick={() => setLinkPickerKey(null)}>
                                Abbrechen
                              </button>
                            </div>
                          )}
                          {rowErr && (
                            <div style={{ padding: "0 0.6rem" }}>
                              <ErrorText>{rowErr}</ErrorText>
                            </div>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                </div>

                <div>
                  <div style={sectionLabelStyle}>Bereits verknüpft ({linked.length})</div>
                  {linked.length === 0 && <p style={mutedStyle}>Keine verknüpften Ressourcen.</p>}
                  {linked.length > 0 && pageLinked.length === 0 && (
                    <p style={mutedStyle}>Keine verknüpften Ressourcen auf dieser Seite.</p>
                  )}
                  <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
                    {pageLinked.map(({ device, navIdx }, i) => {
                      const key = `${connection.id}:${device.external_id}`;
                      const isSelected = isGroupActive && navIdx === effectiveSelectedIndex;
                      const detailsIsOpen = detailsOpenKey === key;
                      const busyUnlink = unlinkBusy[key] ?? false;
                      const rowErr = deviceError[key] ?? null;
                      const localSystem = localSystems.find((s) => s.id === device.linked_system_id);
                      return (
                        <li key={device.external_id} style={{ borderTop: i === 0 ? "none" : "1px solid var(--border-subtle)" }}>
                          <div
                            className="list-row"
                            role="option"
                            aria-selected={isSelected}
                            onMouseEnter={() => selectRow(navIdx)}
                            onClick={() => selectRow(navIdx)}
                            style={{
                              display: "flex",
                              justifyContent: "space-between",
                              alignItems: "center",
                              gap: "0.5rem",
                              flexWrap: "wrap",
                              padding: "0.4rem 0.6rem",
                              background: isSelected ? "var(--bg-selected)" : "transparent",
                            }}
                          >
                            <span style={{ display: "inline-flex", alignItems: "center", gap: "0.4rem", flexWrap: "wrap" }}>
                              <DeviceSummaryLine device={device} />
                              {localSystem && (
                                <span style={{ color: "var(--text-muted)", fontSize: "0.8em" }}>· lokal: {localSystem.name}</span>
                              )}
                            </span>
                            <span style={{ display: "flex", alignItems: "center", gap: "0.4rem", flexShrink: 0 }}>
                              <button type="button" onClick={() => void toggleDetails(connection, tenant, device)}>
                                {detailsIsOpen ? "Details ausblenden" : "Details"}
                              </button>
                              <KeyHint label="Enter" />
                              <button type="button" disabled={busyUnlink} onClick={() => void handleUnlink(connection, device)}>
                                {busyUnlink ? "Entferne…" : "Entfernen"}
                              </button>
                              <KeyHint label="u" />
                            </span>
                          </div>
                          {rowErr && (
                            <div style={{ padding: "0 0.6rem" }}>
                              <ErrorText>{rowErr}</ErrorText>
                            </div>
                          )}
                          {detailsIsOpen && renderDetailsPanel(device, key)}
                        </li>
                      );
                    })}
                  </ul>
                </div>

                <PaginationBar
                  page={clampedPage}
                  totalPages={totalPages}
                  onPrev={() => goToPage(Math.max(0, clampedPage - 1))}
                  onNext={() => goToPage(Math.min(totalPages - 1, clampedPage + 1))}
                />
              </>
            )}
          </>
        )}
      </div>
    );
  }

  function renderConnectionModal(connection: AcronisConnectionDto) {
    const id = connection.id;
    const busy = syncBusy[id] ?? false;
    const status = syncStatus[id] ?? null;
    const err = syncError[id] ?? null;
    const cached = cachedSyncByConnection[id] ?? null;
    const cacheBusyFlag = cacheBusy[id] ?? false;
    const cacheErr = cacheError[id] ?? null;
    const syncedDisplay = syncedAtDisplay[id];
    const tenants = tenantsByConnection[id] ?? null;
    const tenantsHaveLoaded = tenantsLoaded[id] ?? false;
    const tenantsBusyFlag = tenantsBusy[id] ?? false;
    const tenantsErr = tenantsError[id] ?? null;

    return (
      <Modal onClose={closeTenantDeviceModal}>
        <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem", minWidth: "32rem", maxWidth: "60rem", maxHeight: "80vh", overflowY: "auto" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: "0.5rem" }}>
            <div>
              <h2 style={{ margin: 0, fontSize: "1rem" }}>{connection.label}</h2>
              <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)", fontFamily: "var(--font-mono)" }}>
                {connection.datacenter_url}
              </div>
            </div>
            <button type="button" onClick={closeTenantDeviceModal}>
              Schließen
            </button>
          </div>

          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "0.5rem",
              flexWrap: "wrap",
              paddingTop: "0.5rem",
              borderTop: "1px solid var(--border-subtle)",
            }}
          >
            <span style={{ fontSize: "0.82rem", color: "var(--text-secondary)" }}>
              {cacheBusyFlag
                ? "Lade…"
                : cached
                  ? `Zuletzt synchronisiert: ${syncedDisplay ?? cached.synced_at_utc}`
                  : "Noch nicht synchronisiert."}
            </span>
            <button type="button" disabled={busy} onClick={() => void handleSync(connection)}>
              {busy ? "Aktualisiere…" : "Aktualisieren"}
            </button>
          </div>
          {status && <SuccessText>{status}</SuccessText>}
          {err && <ErrorText>{err}</ErrorText>}
          {cacheErr && <ErrorText>{cacheErr}</ErrorText>}
          {tenantsErr && <ErrorText>{tenantsErr}</ErrorText>}

          {tenantsBusyFlag && !tenants && <p style={mutedStyle}>Lade Mandanten…</p>}
          {tenantsHaveLoaded && tenants && tenants.length === 0 && (
            <p style={mutedStyle}>Keine Kunden-Mandanten in dieser Acronis-Verbindung gefunden.</p>
          )}
          {tenants &&
            sortByName(tenants).map((tenant) =>
              renderDeviceGroup(connection, tenant, cached?.groups.find((g) => g.tenant_id === tenant.id)),
            )}
        </div>
      </Modal>
    );
  }

  const openConnection = connections.find((c) => c.id === openConnectionId) ?? null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "52rem" }}>
      <section style={cardStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
          <h2 style={{ margin: 0, fontSize: "1rem" }}>Acronis-Verbindungen</h2>
          <button type="button" onClick={() => setAddFormOpen(true)}>
            + Neue Verbindung
          </button>
        </div>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Liest ausschließlich den Sicherungsstatus aus Acronis Cyber Protect Cloud (Backup-Plattform, keine
          Geräteverwaltung) — es werden nie Daten dorthin zurückgeschrieben. Eine Verbindung kann mehrere Mandanten
          sehen; jeder Mandant wird einzeln einem Kunden zugeordnet.
        </p>
        {connectionsError && <ErrorText>{connectionsError}</ErrorText>}
        {connections.length === 0 && (
          <p style={{ margin: 0, color: "var(--text-muted)", fontSize: "0.85rem" }}>Noch keine Verbindung eingerichtet.</p>
        )}
        {connections.map((connection) => {
          const removing = removeBusy[connection.id] ?? false;
          return (
            <div key={connection.id} style={connectionRowStyle}>
              <div>
                <div style={{ fontWeight: 600 }}>{connection.label}</div>
                <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)", fontFamily: "var(--font-mono)" }}>
                  {connection.datacenter_url}
                </div>
              </div>
              <div style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
                <button type="button" onClick={() => openTenantDeviceModal(connection)}>
                  Mandanten &amp; Ressourcen
                </button>
                <button type="button" disabled={removing} onClick={() => void handleRemoveConnection(connection)}>
                  {removing ? "Entferne…" : "Entfernen"}
                </button>
              </div>
            </div>
          );
        })}
      </section>

      {addFormOpen && (
        <Modal onClose={() => setAddFormOpen(false)}>
          <form onSubmit={(e) => void handleAddConnection(e)} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "22rem" }}>
            <h2 style={{ margin: 0, fontSize: "1rem" }}>Neue Acronis-Verbindung</h2>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Label
              <input value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="ACME Acronis" required autoFocus />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Datacenter-URL
              <input
                value={newDatacenterUrl}
                onChange={(e) => {
                  setNewDatacenterUrl(e.target.value);
                  setTestResult(null);
                }}
                placeholder="https://eu2-cloud.acronis.com"
                style={{ fontFamily: "var(--font-mono)" }}
                required
              />
              <span style={{ fontSize: "0.75rem", color: "var(--text-muted)" }}>
                Wird zusammen mit Client-ID/-Secret bei der Erstellung des API-Clients in der Acronis-Verwaltungskonsole
                angezeigt.
              </span>
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Client-ID
              <input
                value={newClientId}
                onChange={(e) => {
                  setNewClientId(e.target.value);
                  setTestResult(null);
                }}
                style={{ fontFamily: "var(--font-mono)" }}
                required
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Client-Secret
              <input
                type="password"
                value={newClientSecret}
                onChange={(e) => {
                  setNewClientSecret(e.target.value);
                  setTestResult(null);
                }}
                required
              />
              <span style={{ fontSize: "0.75rem", color: "var(--text-muted)" }}>
                Erzeugt beim Anlegen eines API-Clients in Acronis Cyber Protect Cloud.
              </span>
            </label>
            {testResult && (testResult.ok ? <SuccessText>{testResult.message}</SuccessText> : <ErrorText>{testResult.message}</ErrorText>)}
            {addStatus && <SuccessText>{addStatus}</SuccessText>}
            {addError && <ErrorText>{addError}</ErrorText>}
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end", marginTop: "0.25rem" }}>
              <button type="button" onClick={() => setAddFormOpen(false)}>
                Abbrechen
              </button>
              <button
                type="button"
                disabled={testBusy || !newDatacenterUrl || !newClientId || !newClientSecret}
                onClick={() => void handleTestConnection()}
              >
                {testBusy ? "Teste…" : "Verbindung testen"}
              </button>
              <button type="submit" disabled={addBusy}>
                {addBusy ? "Wird hinzugefügt…" : "Hinzufügen"}
              </button>
            </div>
          </form>
        </Modal>
      )}

      {openConnection && renderConnectionModal(openConnection)}
    </div>
  );
}
