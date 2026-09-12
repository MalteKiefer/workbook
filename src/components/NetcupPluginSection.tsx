import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { formatInvokeError } from "../lib/errors";
import { getKeymap, matchesBinding } from "../lib/keymap";
import Modal from "./Modal";

// netcup (German hosting provider, vServer/Root-Server VPS products, via its
// "Server Control Panel" REST API) plugin settings screen. Same read-only/
// manual-adopt rule as every other plugin section: external data is always
// shown as supplementary, read-only information next to a System's own
// maintained fields (name/hostname/ip_address/notes), and copying a value
// across is always a deliberate, manual, per-field action, never an
// automatic sync step. This plugin's purpose is server inventory only: no
// device-management-agent concept, no backup-status concept, the simplest
// kind of plugin in this codebase.
//
// Structurally closest to Level.io/Iru: netcup's API has no organization/
// reseller/sub-account concept at all (verified against netcup's own live
// OpenAPI spec: no "list users" endpoint, no account-scoping parameter on
// /servers), so a connection maps 1:1 to exactly one local customer
// (customer_id lives directly on the connection), no org-mapping UI. Like
// Level (fixed API host) and unlike Iru/Snipe-IT/Tactical RMM, netcup has a
// single fixed API host too, so the add-connection form only asks for
// Label + Customer + API-Token, no Base-URL field.
//
// IMPORTANT, verified split (see plugin::netcup/commands::netcup module
// docs): netcup's server LIST endpoint (GET /servers) genuinely has no
// status field and no IP address field at all: ExternalSystemDto.status/
// ip_address below are therefore ALWAYS `null` for every server that came
// from sync_netcup_connection/get_cached_netcup_sync. Real status
// (serverLiveInfo.state) and real IPs (ipv4Addresses[].ip) only ever appear
// once a server's own details are opened (get_netcup_system_details, the
// per-server detail call), never fetched for every server up front, which
// would be an N+1 pattern against an undocumented rate limit. The device
// list below reflects this honestly: no status/IP column, just a note
// pointing at "Details" for that information.
//
// Page layout: the connections list stays inline (identity + two actions
// per row); both the add-connection form and the per-connection device
// list/compare-and-adopt UI live in on-demand <Modal> overlays, exactly like
// LevelPluginSection.tsx/IruPluginSection.tsx.

interface NetcupConnectionDto {
  id: string;
  customer_id: number;
  label: string;
}

interface ExternalSystemDto {
  external_id: string;
  name: string;
  // ALWAYS `null` here: netcup's list endpoint has no status/IP fields at
  // all (see module docs above). Real values only ever come from
  // get_netcup_system_details's raw JSON, scanned by findExternalStatus/
  // findExternalIp below, never from this DTO.
  status: string | null;
  ip_address: string | null;
  linked_system_id: number | null;
}

interface CachedNetcupSyncDto {
  synced_at_utc: string;
  devices: ExternalSystemDto[];
}

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

// Mirrors SystemForm.tsx's local System shape exactly, needed here both to
// render the "existing system" picker and to build the update_system/
// create_system input objects the same way SystemForm.tsx does.
interface System {
  id: number;
  customer_id: number;
  name: string;
  system_type: string;
  hostname: string;
  ip_address: string;
  notes: string;
  maintenance_interval_days: number | null;
  operating_system: string | null;
}

type CompareField = "name" | "hostname" | "ip_address";

// netcup's own server JSON (from get_netcup_system_details) carries the same
// nickname/hostname/name fields the list endpoint does (verified: the detail
// endpoint returns the full server resource, list fields plus the extra
// "live info"), so the same nickname-then-hostname-then-name fallback the
// backend already applies into `name` (see plugin::netcup::map_server) is
// also the candidate order used here for the compare table's "Name" row.
const NAME_KEYS = ["nickname", "hostname", "name"];
const HOSTNAME_KEYS = ["hostname"];

// Sentinel option value for the customer <select>'s "create a new customer"
// convenience entry, never a real customer id, so it can't collide.
const CREATE_NEW_CUSTOMER = "__create_new__";

// Heuristic scan of a get_netcup_system_details() payload, analogous to
// LevelPluginSection.tsx's/IruPluginSection.tsx's own findExternalValue:
// looks for top-level, non-object fields whose key matches one of a handful
// of candidate names.
function findExternalValue(details: Record<string, unknown>, candidateKeys: string[]): string | null {
  const entries = Object.entries(details);
  for (const wanted of candidateKeys) {
    const hit = entries.find(([k]) => k.toLowerCase() === wanted.toLowerCase());
    if (hit && hit[1] !== null && hit[1] !== undefined && typeof hit[1] !== "object") {
      return String(hit[1]);
    }
  }
  return null;
}

// netcup's detail response has no flat top-level IP field: real IPv4
// addresses live under `ipv4Addresses[].ip` (verified, see
// plugin::netcup module docs). The first entry's `ip` is the natural
// display value; ipv6Addresses is deliberately never used here (those are
// prefix/gateway pairs, not host addresses, same reasoning as the Rust
// side, see plugin::netcup module docs).
function findExternalIp(details: Record<string, unknown>): string | null {
  const addresses = details["ipv4Addresses"];
  if (!Array.isArray(addresses)) return null;
  for (const entry of addresses) {
    if (entry && typeof entry === "object" && typeof (entry as Record<string, unknown>).ip === "string") {
      const ip = (entry as Record<string, unknown>).ip as string;
      if (ip.length > 0) return ip;
    }
  }
  return null;
}

// netcup's real server state lives nested under `serverLiveInfo.state`
// (verified, a free-form libvirt-domain-state string like "RUNNING"/
// "SHUTOFF"; no fixed enum on either side). Informational only: a local
// System has no "status" field of its own to adopt this into, exactly like
// TacticalRmmPluginSection.tsx's own status display.
function findExternalStatus(details: Record<string, unknown>): string | null {
  const liveInfo = details["serverLiveInfo"];
  if (liveInfo && typeof liveInfo === "object") {
    const state = (liveInfo as Record<string, unknown>).state;
    if (typeof state === "string" && state.length > 0) return state;
  }
  return null;
}

// No natural "which timezone" answer for a machine-generated synced_at_utc
// timestamp, so we use the viewer's own local zone to render it via
// format_timestamp_for_display, same as every other plugin section.
function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return "UTC";
  }
}

// Servers come back from netcup's API in whatever order the API returns
// them in. Sort alphabetically by name (German collation) for display,
// independent of the free-text filter below.
function sortDevicesByName(devices: ExternalSystemDto[]): ExternalSystemDto[] {
  return devices.slice().sort((a, b) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
}

// netcup's list-derived `name` already folds nickname->hostname->raw-name
// (see plugin::netcup::map_server), so it's the best identifying string
// available for suggesting a "link to existing system" match, compared
// against the only free-text field a local system has for it,
// System.hostname (same convention as LevelPluginSection.tsx, just without
// a separate raw hostname field to prefer over the display name).
function matchKeyForDevice(device: ExternalSystemDto): string {
  return device.name.trim().toLowerCase();
}

// How many devices a connection's device list shows per page.
const DEVICE_PAGE_SIZE = 10;

// Slices `items` (already filtered/sorted) to one page of DEVICE_PAGE_SIZE,
// clamping a possibly-stale stored page number into range, same helper
// shape as IruPluginSection.tsx's own paginateDevices.
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
  display: "flex",
  flexDirection: "column",
  gap: "0.4rem",
  padding: "1rem",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-sm)",
  background: "var(--bg-surface)",
};

const sectionLabelStyle: CSSProperties = {
  fontSize: "0.72rem",
  textTransform: "uppercase",
  color: "var(--text-muted)",
  letterSpacing: "0.03em",
};

const mutedStyle: CSSProperties = { margin: "0.2rem 0", fontSize: "0.8rem", color: "var(--text-muted)" };

const thStyle: CSSProperties = {
  textAlign: "left",
  padding: "0.2rem 0.5rem",
  borderBottom: "1px solid var(--border)",
  color: "var(--text-secondary)",
  fontWeight: 500,
};

const tdStyle: CSSProperties = { padding: "0.2rem 0.5rem", borderBottom: "1px solid var(--border-subtle)" };

const labelStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: "0.2rem" };

function ErrorText({ children }: { children: React.ReactNode }) {
  return <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {children}</p>;
}

function SuccessText({ children }: { children: React.ReactNode }) {
  return <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{children}</p>;
}

// Small inline keybinding hint, mirroring CommandPalette.tsx's Row hint span.
// Makes the row-level keyboard shortcuts (Enter/l/u) discoverable next to
// the mouse-clickable buttons they duplicate, rather than hidden knowledge.
function HintBadge({ label }: { label: string }) {
  return (
    <span
      style={{
        fontFamily: "var(--font-mono)",
        color: "var(--text-muted)",
        fontSize: "0.72rem",
        border: "1px solid var(--border-subtle)",
        borderRadius: "3px",
        padding: "0 0.3rem",
        lineHeight: "1.5",
      }}
    >
      {label}
    </span>
  );
}

function DeviceSummaryLine({ device }: { device: ExternalSystemDto }) {
  // No status/IP column here on purpose. The list this component renders
  // from is always the minimal, list-derived one (status/ip_address are
  // always `null`, see module docs above). "Details" is where that
  // information actually appears, fetched on demand.
  return <span>{device.name}</span>;
}

export default function NetcupPluginSection() {
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);

  const [connections, setConnections] = useState<NetcupConnectionDto[]>([]);
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [connectionsError, setConnectionsError] = useState<string | null>(null);

  // Add-connection modal.
  const [addFormOpen, setAddFormOpen] = useState(false);
  const [newCustomerId, setNewCustomerId] = useState<number | "">("");
  const [newLabel, setNewLabel] = useState("");
  const [newApiToken, setNewApiToken] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addStatus, setAddStatus] = useState<string | null>(null);
  const [addError, setAddError] = useState<string | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [customersRefreshBusy, setCustomersRefreshBusy] = useState(false);
  const kundeSelectRef = useRef<HTMLSelectElement>(null);
  // Set when the user picks "+ Neuen Kunden anlegen…" in the add-connection
  // form's customer select, mirrors LevelPluginSection.tsx's own
  // awaitingNewCustomer flag exactly.
  const [awaitingNewCustomer, setAwaitingNewCustomer] = useState(false);

  // Which connection's device modal is open (page-local UI state).
  const [openConnectionId, setOpenConnectionId] = useState<string | null>(null);

  // Cached (offline) device sync per connection.
  const [cachedSyncByConnection, setCachedSyncByConnection] = useState<Record<string, CachedNetcupSyncDto | null>>({});
  const [cacheLoaded, setCacheLoaded] = useState<Record<string, boolean>>({});
  const [cacheBusy, setCacheBusy] = useState<Record<string, boolean>>({});
  const [cacheError, setCacheError] = useState<Record<string, string | null>>({});
  const [syncedAtDisplay, setSyncedAtDisplay] = useState<Record<string, string>>({});

  // "Aktualisieren" (live sync) per connection.
  const [syncBusy, setSyncBusy] = useState<Record<string, boolean>>({});
  const [syncStatus, setSyncStatus] = useState<Record<string, string | null>>({});
  const [syncError, setSyncError] = useState<Record<string, string | null>>({});

  const [removeBusy, setRemoveBusy] = useState<Record<string, boolean>>({});

  // Device text filter and pagination, both keyed by connection id: netcup
  // has no group/organization concept, so there is exactly one flat device
  // list per connection, not one per group.
  const [deviceFilter, setDeviceFilter] = useState<Record<string, string>>({});
  const deviceFilterInputRef = useRef<HTMLInputElement>(null);
  const [pageByConnection, setPageByConnection] = useState<Record<string, number>>({});
  const [selectedIndexByConnection, setSelectedIndexByConnection] = useState<Record<string, number>>({});

  // Local systems cache, keyed by customer id, used both for the "link to
  // existing system" picker and for the compare/adopt panel.
  const [localSystemsByCustomer, setLocalSystemsByCustomer] = useState<Record<number, System[]>>({});

  // Per-device state, keyed by `${connectionId}:${externalId}`.
  const [linkPickerKey, setLinkPickerKey] = useState<string | null>(null);
  const [linkPickerSelection, setLinkPickerSelection] = useState<number | "">("");
  const [linkBusy, setLinkBusy] = useState<Record<string, boolean>>({});
  const [createLinkBusy, setCreateLinkBusy] = useState<Record<string, boolean>>({});
  const [unlinkBusy, setUnlinkBusy] = useState<Record<string, boolean>>({});
  const [deviceError, setDeviceError] = useState<Record<string, string | null>>({});
  // Keyed by connection id directly (a connection maps 1:1 to one customer,
  // so there's no group/tenant-scoped key here), disables the "Alle
  // anlegen" bulk button for one connection while it works through that
  // connection's unlinked servers, independent of the per-device
  // `createLinkBusy` map (both are set during a bulk run, so a device's own
  // row also shows busy).
  const [bulkCreateBusy, setBulkCreateBusy] = useState<Record<string, boolean>>({});

  const [detailsOpenKey, setDetailsOpenKey] = useState<string | null>(null);
  const [detailsBusy, setDetailsBusy] = useState<Record<string, boolean>>({});
  const [detailsError, setDetailsError] = useState<Record<string, string | null>>({});
  const [detailsData, setDetailsData] = useState<Record<string, Record<string, unknown>>>({});
  const [showRawJson, setShowRawJson] = useState<Record<string, boolean>>({});

  // Adopt-a-field busy/error, keyed by `${localSystemId}:${field}`.
  const [adoptBusy, setAdoptBusy] = useState<Record<string, boolean>>({});
  const [adoptError, setAdoptError] = useState<Record<string, string | null>>({});

  const reloadConnections = useCallback(() => {
    invoke<NetcupConnectionDto[]>("list_netcup_connections")
      .then(setConnections)
      .catch((e) => setConnectionsError(formatInvokeError(e)));
  }, []);

  const reloadCustomers = useCallback(async () => {
    const list = await invoke<Customer[]>("list_customers", { includeArchived: false });
    setCustomers(list);
    return list;
  }, []);

  useEffect(() => {
    reloadConnections();
    void reloadCustomers();
  }, [reloadConnections, reloadCustomers]);

  // CustomerForm is globally mounted and driven by the store, so, exactly
  // like LevelPluginSection.tsx/IruPluginSection.tsx/CustomerListView.tsx,
  // this component learns the editor closed by watching
  // customerEditorTarget transition from non-null back to null, rather than
  // via an onDone callback.
  const prevCustomerEditorTargetRef = useRef(customerEditorTarget);
  useEffect(() => {
    const prevTarget = prevCustomerEditorTargetRef.current;
    prevCustomerEditorTargetRef.current = customerEditorTarget;
    if (prevTarget === null || customerEditorTarget !== null) return;
    if (!awaitingNewCustomer) return;

    const previousCustomers = customers;
    void (async () => {
      try {
        const freshList = await reloadCustomers();
        const previousIds = new Set(previousCustomers.map((c) => c.id));
        const newlyCreated = freshList.filter((c) => !previousIds.has(c.id));
        if (newlyCreated.length === 1) {
          setNewCustomerId(newlyCreated[0].id);
        }
      } catch (err) {
        setAddError(formatInvokeError(err));
      } finally {
        setAwaitingNewCustomer(false);
      }
    })();
  }, [customerEditorTarget, awaitingNewCustomer, customers, reloadCustomers]);

  async function handleRefreshCustomers() {
    setCustomersRefreshBusy(true);
    try {
      await reloadCustomers();
    } catch (err) {
      setAddError(formatInvokeError(err));
    } finally {
      setCustomersRefreshBusy(false);
    }
  }

  async function refreshLocalSystems(customerId: number): Promise<System[]> {
    const list = await invoke<System[]>("list_systems", { customerId, includeArchived: false });
    setLocalSystemsByCustomer((prev) => ({ ...prev, [customerId]: list }));
    return list;
  }

  function getAdoptError(systemId: number): string | null {
    const hit = Object.entries(adoptError).find(([k, v]) => v !== null && k.startsWith(`${systemId}:`));
    return hit ? hit[1] : null;
  }

  function customerLabel(customerId: number): string {
    const customer = customers.find((c) => c.id === customerId);
    return customer ? `${customer.name} (${customer.short_code})` : `Kunde #${customerId}`;
  }

  async function updateSyncedAtDisplay(connectionId: string, utc: string) {
    try {
      const display = await invoke<string>("format_timestamp_for_display", { utc, tz: localTimeZone() });
      setSyncedAtDisplay((prev) => ({ ...prev, [connectionId]: display }));
    } catch {
      setSyncedAtDisplay((prev) => ({ ...prev, [connectionId]: utc }));
    }
  }

  const loadCachedSync = useCallback(async (connection: NetcupConnectionDto) => {
    const id = connection.id;
    setCacheBusy((prev) => ({ ...prev, [id]: true }));
    setCacheError((prev) => ({ ...prev, [id]: null }));
    try {
      const cached = await invoke<CachedNetcupSyncDto | null>("get_cached_netcup_sync", { connectionId: id });
      setCachedSyncByConnection((prev) => ({ ...prev, [id]: cached }));
      setCacheLoaded((prev) => ({ ...prev, [id]: true }));
      if (cached) {
        await updateSyncedAtDisplay(id, cached.synced_at_utc);
      }
      await refreshLocalSystems(connection.customer_id).catch(() => {});
    } catch (err) {
      setCacheError((prev) => ({ ...prev, [id]: formatInvokeError(err) }));
    } finally {
      setCacheBusy((prev) => ({ ...prev, [id]: false }));
    }
  }, []);

  function openDeviceModal(connection: NetcupConnectionDto) {
    setOpenConnectionId(connection.id);
    setLinkPickerKey(null);
    setDetailsOpenKey(null);
    void loadCachedSync(connection);
  }

  function closeDeviceModal() {
    setOpenConnectionId(null);
    setLinkPickerKey(null);
    setDetailsOpenKey(null);
  }

  // The filtered+sorted+unlinked/linked-split device list for one
  // connection, plus the flat keyboard-navigable order (unlinked first, then
  // linked), the single source of truth both renderDeviceModal and the
  // keyboard-navigation effect below walk, so they can never disagree about
  // which devices/rows are currently visible. Analogous to
  // IruPluginSection.tsx's computeConnectionDevices, just without any
  // serial_number/asset_tag fields to also filter on (netcup has none).
  function computeConnectionDevices(connectionId: string) {
    const cached = cachedSyncByConnection[connectionId] ?? null;
    const filterText = deviceFilter[connectionId] ?? "";
    const q = filterText.trim().toLowerCase();
    const allDevices = cached?.devices ?? [];
    const sorted = sortDevicesByName(allDevices);
    const filteredDevices = q === "" ? sorted : sorted.filter((d) => d.name.toLowerCase().includes(q));
    const unlinked = filteredDevices.filter((d) => d.linked_system_id === null);
    const linked = filteredDevices.filter((d) => d.linked_system_id !== null);
    const navItems: { device: ExternalSystemDto; kind: "unlinked" | "linked" }[] = [
      ...unlinked.map((device) => ({ device, kind: "unlinked" as const })),
      ...linked.map((device) => ({ device, kind: "linked" as const })),
    ];
    return { allDevices, filterText, filteredDevices, unlinked, linked, navItems };
  }

  // Resets one connection's device list back to page 1, used whenever the
  // filter text changes (a new filter invalidates whatever page was showing)
  // and after a resync (device set may have changed entirely).
  function resetPage(connectionId: string) {
    setPageByConnection((prev) => ({ ...prev, [connectionId]: 0 }));
    setSelectedIndexByConnection((prev) => ({ ...prev, [connectionId]: 0 }));
  }

  // Auto-focus the device filter input once the modal is open and its cache
  // load has resolved (the input only renders once cached data exists).
  const activeConnectionCacheLoaded = openConnectionId !== null ? (cacheLoaded[openConnectionId] ?? false) : false;
  useEffect(() => {
    if (!activeConnectionCacheLoaded) return;
    const id = window.setTimeout(() => deviceFilterInputRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, [openConnectionId, activeConnectionCacheLoaded]);

  // Keyboard navigation for the open connection's device list: j/k or
  // ArrowDown/Up move the highlight, Enter performs the primary action for
  // the highlighted row (create-and-link for an unlinked device, open/toggle
  // the compare-and-adopt panel for a linked device), l opens the "link to
  // existing system" picker for an unlinked row, u unlinks a linked row.
  // Gated by isTypingTarget exactly like LevelPluginSection.tsx/
  // IruPluginSection.tsx, so typing in the filter input (or the
  // link-picker's <select>, which autofocuses when opened) never triggers
  // row actions.
  useEffect(() => {
    if (openConnectionId === null) return;
    const found = connections.find((c) => c.id === openConnectionId);
    if (!found) return;
    const connection: NetcupConnectionDto = found;
    const connectionId = openConnectionId;

    function onKeyDown(e: KeyboardEvent) {
      if (isTypingTarget(document.activeElement)) return;
      if (linkPickerKey !== null) return; // let the inline link-picker sub-form own its own keys
      const { navItems } = computeConnectionDevices(connectionId);
      if (navItems.length === 0) return;
      const rawPage = pageByConnection[connectionId] ?? 0;
      const { pageItems } = paginateDevices(navItems, rawPage);
      if (pageItems.length === 0) return;
      const idx = Math.min(selectedIndexByConnection[connectionId] ?? 0, pageItems.length - 1);

      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next) || e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndexByConnection((prev) => ({ ...prev, [connectionId]: Math.min(idx + 1, pageItems.length - 1) }));
      } else if (matchesBinding(e, keymap.list_prev) || e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndexByConnection((prev) => ({ ...prev, [connectionId]: Math.max(idx - 1, 0) }));
      } else if (e.key === "Enter") {
        const row = pageItems[idx];
        if (row) {
          e.preventDefault();
          if (row.kind === "unlinked") {
            void createAndLink(connection, row.device);
          } else {
            void toggleDetails(connection, row.device);
          }
        }
      } else if (e.key === "l") {
        const row = pageItems[idx];
        if (row && row.kind === "unlinked") {
          e.preventDefault();
          openLinkPicker(connection, row.device);
        }
      } else if (e.key === "u") {
        const row = pageItems[idx];
        if (row && row.kind === "linked") {
          e.preventDefault();
          void handleUnlink(connection, row.device);
        }
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // createAndLink/handleUnlink/openLinkPicker/toggleDetails are
    // intentionally omitted: they are plain function declarations recreated
    // every render, and listing them would re-subscribe this global keydown
    // listener on every render instead of only when the state above actually
    // changes. Their own bodies read from connection/row (already fresh via
    // the deps above), so a stale reference to the function itself carries
    // no stale-closure risk.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    openConnectionId,
    connections,
    pageByConnection,
    selectedIndexByConnection,
    linkPickerKey,
    cachedSyncByConnection,
    deviceFilter,
    localSystemsByCustomer,
    detailsOpenKey,
  ]);

  function closeAddForm() {
    setAddFormOpen(false);
    setNewCustomerId("");
    setNewLabel("");
    setNewApiToken("");
    setTestResult(null);
    setAddStatus(null);
    setAddError(null);
    setAwaitingNewCustomer(false);
  }

  function handleKundeSelectChange(value: string) {
    if (value === CREATE_NEW_CUSTOMER) {
      setAwaitingNewCustomer(true);
      openCustomerEditor("new");
      // The <select> is controlled by newCustomerId, which we deliberately
      // don't change here, but the browser has already visually flipped
      // the displayed option to the sentinel one, and no state change means
      // no re-render to snap it back. Reset the DOM value imperatively so
      // it doesn't visually stick on "+ Neuen Kunden anlegen…".
      if (kundeSelectRef.current) {
        kundeSelectRef.current.value = newCustomerId === "" ? "" : String(newCustomerId);
      }
      return;
    }
    setNewCustomerId(value === "" ? "" : Number(value));
  }

  async function handleTestConnection() {
    setTestBusy(true);
    setTestResult(null);
    try {
      await invoke("test_netcup_connection", { apiToken: newApiToken });
      setTestResult({ ok: true, message: "✓ Verbindung erfolgreich" });
    } catch (err) {
      setTestResult({ ok: false, message: formatInvokeError(err) });
    } finally {
      setTestBusy(false);
    }
  }

  async function handleAddConnection(e: React.FormEvent) {
    e.preventDefault();
    if (newCustomerId === "") return;
    setAddError(null);
    setAddStatus(null);
    setAddBusy(true);
    try {
      await invoke("add_netcup_connection", {
        customerId: newCustomerId,
        label: newLabel,
        apiToken: newApiToken,
      });
      reloadConnections();
      closeAddForm();
    } catch (err) {
      setAddError(formatInvokeError(err));
    } finally {
      setAddBusy(false);
    }
  }

  async function handleRemoveConnection(connection: NetcupConnectionDto) {
    const confirmed = window.confirm(`netcup-Verbindung "${connection.label}" wirklich entfernen?`);
    if (!confirmed) return;
    const id = connection.id;
    setRemoveBusy((prev) => ({ ...prev, [id]: true }));
    setConnectionsError(null);
    try {
      await invoke("remove_netcup_connection", { id });
      if (openConnectionId === id) closeDeviceModal();
      setCachedSyncByConnection((prev) => {
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

  async function handleSync(connection: NetcupConnectionDto) {
    const id = connection.id;
    setSyncBusy((prev) => ({ ...prev, [id]: true }));
    setSyncStatus((prev) => ({ ...prev, [id]: null }));
    setSyncError((prev) => ({ ...prev, [id]: null }));
    try {
      const devices = await invoke<ExternalSystemDto[]>("sync_netcup_connection", { connectionId: id });
      const count = Array.isArray(devices) ? devices.length : 0;
      setSyncStatus((prev) => ({ ...prev, [id]: `${count} Server gefunden.` }));
      // The device set (and therefore the page count) may have changed
      // entirely, reset back to page 1 rather than risk showing a stale,
      // now out-of-range page.
      resetPage(id);
      // Prefer re-reading the cache afterward (authoritative, server-written
      // synced_at_utc) over building the timestamp from the client clock.
      await loadCachedSync(connection);
    } catch (err) {
      setSyncError((prev) => ({ ...prev, [id]: formatInvokeError(err) }));
    } finally {
      setSyncBusy((prev) => ({ ...prev, [id]: false }));
    }
  }

  function openLinkPicker(connection: NetcupConnectionDto, device: ExternalSystemDto) {
    const customerId = connection.customer_id;
    const key = `${connection.id}:${device.external_id}`;
    setLinkPickerKey(key);
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    const localSystems = localSystemsByCustomer[customerId] ?? [];
    const deviceKey = matchKeyForDevice(device);
    const match = deviceKey !== "" ? localSystems.find((s) => s.hostname.trim().toLowerCase() === deviceKey) : undefined;
    setLinkPickerSelection(match ? match.id : "");
    if (!localSystemsByCustomer[customerId]) {
      void refreshLocalSystems(customerId);
    }
  }

  async function confirmLink(connection: NetcupConnectionDto, device: ExternalSystemDto) {
    if (linkPickerSelection === "") return;
    const key = `${connection.id}:${device.external_id}`;
    setLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("link_system_to_netcup", {
        systemId: linkPickerSelection,
        connectionId: connection.id,
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

  async function createAndLink(connection: NetcupConnectionDto, device: ExternalSystemDto) {
    const customerId = connection.customer_id;
    const key = `${connection.id}:${device.external_id}`;
    setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      const created = await invoke<System>("create_system", {
        input: {
          customer_id: customerId,
          name: device.name,
          system_type: "",
          hostname: "",
          ip_address: "",
          notes: "",
        },
      });
      await invoke("link_system_to_netcup", {
        systemId: created.id,
        connectionId: connection.id,
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

  // Bulk version of `createAndLink`: works through every unlinked server on
  // this connection sequentially. A failure on one server does not abort
  // the rest; it is recorded in the same `deviceError` map that already
  // surfaces per-device errors, so a partial run still leaves the row's own
  // "Neu anlegen" button as the retry path. `refreshLocalSystems`/
  // `loadCachedSync` run once at the end, not per device, so a connection
  // with many servers doesn't refetch the whole customer's system list N
  // times.
  async function createAndLinkAll(connection: NetcupConnectionDto, devices: ExternalSystemDto[]) {
    if (devices.length === 0) return;
    const customerId = connection.customer_id;
    setBulkCreateBusy((prev) => ({ ...prev, [connection.id]: true }));
    try {
      for (const device of devices) {
        const key = `${connection.id}:${device.external_id}`;
        setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
        setDeviceError((prev) => ({ ...prev, [key]: null }));
        try {
          const created = await invoke<System>("create_system", {
            input: {
              customer_id: customerId,
              name: device.name,
              system_type: "",
              hostname: "",
              ip_address: "",
              notes: "",
            },
          });
          await invoke("link_system_to_netcup", {
            systemId: created.id,
            connectionId: connection.id,
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
      setBulkCreateBusy((prev) => ({ ...prev, [connection.id]: false }));
    }
  }

  async function handleUnlink(connection: NetcupConnectionDto, device: ExternalSystemDto) {
    if (device.linked_system_id === null) return;
    const key = `${connection.id}:${device.external_id}`;
    setUnlinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("unlink_system_from_netcup", { systemId: device.linked_system_id, connectionId: connection.id });
      if (detailsOpenKey === key) setDetailsOpenKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setUnlinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function toggleDetails(connection: NetcupConnectionDto, device: ExternalSystemDto) {
    const key = `${connection.id}:${device.external_id}`;
    if (detailsOpenKey === key) {
      setDetailsOpenKey(null);
      return;
    }
    setDetailsOpenKey(key);
    setDetailsError((prev) => ({ ...prev, [key]: null }));
    setDetailsBusy((prev) => ({ ...prev, [key]: true }));
    try {
      const detailsPromise = invoke<Record<string, unknown>>("get_netcup_system_details", {
        connectionId: connection.id,
        externalId: device.external_id,
      });
      const refreshPromise = refreshLocalSystems(connection.customer_id);
      const [data] = await Promise.all([detailsPromise, refreshPromise]);
      setDetailsData((prev) => ({ ...prev, [key]: data }));
    } catch (err) {
      setDetailsError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setDetailsBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function adoptField(localSystem: System, field: CompareField, value: string) {
    const busyKey = `${localSystem.id}:${field}`;
    setAdoptBusy((prev) => ({ ...prev, [busyKey]: true }));
    setAdoptError((prev) => ({ ...prev, [busyKey]: null }));
    try {
      await invoke("update_system", {
        id: localSystem.id,
        input: {
          name: field === "name" ? value : localSystem.name,
          system_type: localSystem.system_type,
          hostname: field === "hostname" ? value : localSystem.hostname,
          ip_address: field === "ip_address" ? value : localSystem.ip_address,
          notes: localSystem.notes,
          maintenance_interval_days: localSystem.maintenance_interval_days,
          operating_system: localSystem.operating_system,
        },
      });
      await refreshLocalSystems(localSystem.customer_id);
    } catch (err) {
      setAdoptError((prev) => ({ ...prev, [busyKey]: formatInvokeError(err) }));
    } finally {
      setAdoptBusy((prev) => ({ ...prev, [busyKey]: false }));
    }
  }

  function renderDetailsPanel(connection: NetcupConnectionDto, device: ExternalSystemDto, key: string) {
    const busy = detailsBusy[key] ?? false;
    const err = detailsError[key] ?? null;
    const data = detailsData[key];
    const localSystems = localSystemsByCustomer[connection.customer_id] ?? [];
    const localSystem =
      device.linked_system_id !== null ? localSystems.find((s) => s.id === device.linked_system_id) : undefined;
    const externalStatus = data ? findExternalStatus(data) : null;

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
        {busy && <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Lade netcup-Details…</p>}
        {err && <ErrorText>{err}</ErrorText>}
        {!busy && data && (
          <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>
            Status (netcup): {externalStatus ?? <span style={{ color: "var(--text-muted)" }}>unbekannt</span>}
          </div>
        )}
        {!busy && data && !localSystem && (
          <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--danger)" }}>
            Lokales System nicht gefunden (evtl. archiviert).
          </p>
        )}
        {!busy && data && localSystem && (
          <>
            <div style={{ overflowX: "auto" }}>
              <table style={{ borderCollapse: "collapse", fontSize: "0.82rem", width: "100%" }}>
                <thead>
                  <tr>
                    <th style={thStyle}>Feld</th>
                    <th style={thStyle}>Lokal</th>
                    <th style={thStyle}>netcup (extern)</th>
                    <th style={thStyle}></th>
                  </tr>
                </thead>
                <tbody>
                  {(
                    [
                      { field: "name" as const, label: "Name", localValue: localSystem.name, externalValue: findExternalValue(data, NAME_KEYS) },
                      {
                        field: "hostname" as const,
                        label: "Hostname",
                        localValue: localSystem.hostname,
                        externalValue: findExternalValue(data, HOSTNAME_KEYS),
                      },
                      {
                        field: "ip_address" as const,
                        label: "IP-Adresse",
                        localValue: localSystem.ip_address,
                        externalValue: findExternalIp(data),
                      },
                    ] satisfies { field: CompareField; label: string; localValue: string; externalValue: string | null }[]
                  ).map((row) => {
                    const adoptKey = `${localSystem.id}:${row.field}`;
                    const adopting = adoptBusy[adoptKey] ?? false;
                    const canAdopt = row.externalValue !== null && row.externalValue !== row.localValue;
                    return (
                      <tr key={row.field}>
                        <td style={tdStyle}>{row.label}</td>
                        <td style={tdStyle}>{row.localValue || <span style={{ color: "var(--text-muted)" }}>—</span>}</td>
                        <td style={{ ...tdStyle, fontFamily: "var(--font-mono)" }}>
                          {row.externalValue ?? <span style={{ color: "var(--text-muted)" }}>—</span>}
                        </td>
                        <td style={tdStyle}>
                          <button
                            type="button"
                            disabled={!canAdopt || adopting}
                            onClick={() => {
                              if (row.externalValue !== null) void adoptField(localSystem, row.field, row.externalValue);
                            }}
                          >
                            {adopting ? "…" : "Übernehmen →"}
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            {getAdoptError(localSystem.id) && <ErrorText>{getAdoptError(localSystem.id)}</ErrorText>}
            <div>
              <button type="button" onClick={() => setShowRawJson((prev) => ({ ...prev, [key]: !prev[key] }))}>
                {showRawJson[key] ? "Rohdaten ausblenden" : "Rohdaten anzeigen"}
              </button>
            </div>
            {showRawJson[key] && (
              <pre
                style={{
                  fontSize: "0.75rem",
                  background: "var(--bg-base)",
                  padding: "0.5rem",
                  borderRadius: "var(--radius-sm)",
                  overflowX: "auto",
                  maxHeight: "12rem",
                  margin: 0,
                }}
              >
                {JSON.stringify(data, null, 2)}
              </pre>
            )}
          </>
        )}
      </div>
    );
  }

  function renderConnectionRow(connection: NetcupConnectionDto) {
    const removing = removeBusy[connection.id] ?? false;

    return (
      <div key={connection.id} style={connectionRowStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
          <div>
            <div style={{ fontWeight: 600 }}>{connection.label}</div>
            <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>{customerLabel(connection.customer_id)}</div>
          </div>
          <div style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
            <button type="button" onClick={() => openDeviceModal(connection)}>
              Server anzeigen
            </button>
            <button type="button" disabled={removing} onClick={() => void handleRemoveConnection(connection)}>
              {removing ? "Entferne…" : "Entfernen"}
            </button>
          </div>
        </div>
      </div>
    );
  }

  function renderDeviceModal(connection: NetcupConnectionDto) {
    const busy = syncBusy[connection.id] ?? false;
    const status = syncStatus[connection.id] ?? null;
    const err = syncError[connection.id] ?? null;
    const cached = cachedSyncByConnection[connection.id] ?? null;
    const hasCacheLoaded = cacheLoaded[connection.id] ?? false;
    const cacheBusyFlag = cacheBusy[connection.id] ?? false;
    const cacheErr = cacheError[connection.id] ?? null;
    const syncedDisplay = syncedAtDisplay[connection.id];

    const { allDevices, filterText, unlinked, linked, navItems } = computeConnectionDevices(connection.id);
    const localSystems = localSystemsByCustomer[connection.customer_id] ?? [];
    const rawPage = pageByConnection[connection.id] ?? 0;
    const { pageItems, totalPages, clampedPage } = paginateDevices(navItems, rawPage);
    const selectedIdx = Math.min(selectedIndexByConnection[connection.id] ?? 0, Math.max(pageItems.length - 1, 0));
    const unlinkedOnPage = pageItems.filter((r) => r.kind === "unlinked").map((r) => r.device);
    const linkedOnPage = pageItems.filter((r) => r.kind === "linked").map((r) => r.device);

    function goToPage(nextPage: number) {
      setPageByConnection((prev) => ({ ...prev, [connection.id]: Math.max(nextPage, 0) }));
      setSelectedIndexByConnection((prev) => ({ ...prev, [connection.id]: 0 }));
    }

    function selectRow(rowIndex: number) {
      setSelectedIndexByConnection((prev) => ({ ...prev, [connection.id]: rowIndex }));
    }

    return (
      <Modal onClose={closeDeviceModal}>
        <div style={{ display: "flex", flexDirection: "column", gap: "0.6rem", minWidth: "34rem", maxWidth: "48rem", maxHeight: "80vh", overflowY: "auto" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: "0.5rem" }}>
            <div>
              <h2 style={{ margin: 0, fontSize: "1rem" }}>{connection.label}</h2>
              <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>{customerLabel(connection.customer_id)}</div>
            </div>
            <button type="button" onClick={closeDeviceModal}>
              Schließen
            </button>
          </div>

          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
            <span style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>
              {cacheBusyFlag
                ? "Lade…"
                : cached
                  ? `Zuletzt synchronisiert: ${syncedDisplay ?? cached.synced_at_utc}`
                  : hasCacheLoaded
                    ? "Noch nicht synchronisiert."
                    : ""}
            </span>
            <button type="button" disabled={busy} onClick={() => void handleSync(connection)}>
              {busy ? "Aktualisiere…" : "Aktualisieren"}
            </button>
          </div>
          {status && <SuccessText>{status}</SuccessText>}
          {err && <ErrorText>{err}</ErrorText>}
          {cacheErr && <ErrorText>{cacheErr}</ErrorText>}

          {!cacheBusyFlag && hasCacheLoaded && !cached && (
            <p style={mutedStyle}>Noch nicht synchronisiert. Auf "Aktualisieren" klicken, um Server von netcup zu holen.</p>
          )}

          {cached && allDevices.length > 0 && (
            <input
              ref={deviceFilterInputRef}
              value={filterText}
              onChange={(e) => {
                setDeviceFilter((prev) => ({ ...prev, [connection.id]: e.target.value }));
                resetPage(connection.id);
              }}
              placeholder={`Server filtern (${allDevices.length})…`}
              style={{ maxWidth: "20rem" }}
            />
          )}
          {cached && allDevices.length === 0 && <p style={mutedStyle}>Keine Server im Cache.</p>}
          {cached && allDevices.length > 0 && navItems.length === 0 && <p style={mutedStyle}>Keine Server gefunden.</p>}

          {cached && navItems.length > 0 && (
            <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
              {(unlinked.length === 0 || unlinkedOnPage.length > 0) && (
                <div>
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "0.5rem" }}>
                    <div style={sectionLabelStyle}>Nicht verknüpft ({unlinked.length})</div>
                    {unlinked.length > 0 && (
                      <button
                        type="button"
                        disabled={bulkCreateBusy[connection.id] ?? false}
                        onClick={() => void createAndLinkAll(connection, unlinked)}
                      >
                        {bulkCreateBusy[connection.id] ? "Lege an…" : `Alle anlegen (${unlinked.length})`}
                      </button>
                    )}
                  </div>
                  {unlinked.length === 0 && <p style={mutedStyle}>Keine offenen Server.</p>}
                  {unlinkedOnPage.map((device, i) => {
                    const rowKey = `${connection.id}:${device.external_id}`;
                    const rowIndex = i;
                    const isSelected = rowIndex === selectedIdx;
                    const pickerOpen = linkPickerKey === rowKey;
                    const busyLink = linkBusy[rowKey] ?? false;
                    const busyCreate = createLinkBusy[rowKey] ?? false;
                    const rowErr = deviceError[rowKey] ?? null;
                    return (
                      <div key={device.external_id}>
                        <div
                          className="list-row"
                          role="option"
                          aria-selected={isSelected}
                          onMouseEnter={() => selectRow(rowIndex)}
                          onClick={() => selectRow(rowIndex)}
                          style={{
                            display: "flex",
                            justifyContent: "space-between",
                            alignItems: "center",
                            gap: "0.5rem",
                            padding: "0.4rem 0.6rem",
                            background: isSelected ? "var(--bg-selected)" : "transparent",
                            flexWrap: "wrap",
                          }}
                        >
                          <DeviceSummaryLine device={device} />
                          <span style={{ display: "flex", alignItems: "center", gap: "0.6rem", flexShrink: 0 }}>
                            <span style={{ display: "inline-flex", alignItems: "center", gap: "0.3rem" }}>
                              <button
                                type="button"
                                disabled={busyCreate}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void createAndLink(connection, device);
                                }}
                                style={{ fontSize: "0.78rem" }}
                              >
                                {busyCreate ? "Lege an…" : "Neu anlegen"}
                              </button>
                              <HintBadge label="Enter" />
                            </span>
                            <span style={{ display: "inline-flex", alignItems: "center", gap: "0.3rem" }}>
                              <button
                                type="button"
                                disabled={busyCreate}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  openLinkPicker(connection, device);
                                }}
                                style={{ fontSize: "0.78rem" }}
                              >
                                Verknüpfen…
                              </button>
                              <HintBadge label="l" />
                            </span>
                          </span>
                        </div>
                        {pickerOpen && (
                          <div style={{ display: "flex", gap: "0.4rem", alignItems: "center", padding: "0.3rem 0.6rem" }}>
                            <select
                              value={linkPickerSelection}
                              onChange={(e) => setLinkPickerSelection(e.target.value === "" ? "" : Number(e.target.value))}
                              autoFocus
                            >
                              <option value="">System wählen…</option>
                              {localSystems.map((s) => (
                                <option key={s.id} value={s.id}>
                                  {s.name}
                                  {s.hostname ? ` (${s.hostname})` : ""}
                                </option>
                              ))}
                            </select>
                            <button
                              type="button"
                              disabled={linkPickerSelection === "" || busyLink}
                              onClick={() => void confirmLink(connection, device)}
                            >
                              {busyLink ? "Verknüpfe…" : "Verknüpfen"}
                            </button>
                            <button type="button" onClick={() => setLinkPickerKey(null)}>
                              Abbrechen
                            </button>
                          </div>
                        )}
                        {rowErr && <div style={{ padding: "0 0.6rem" }}><ErrorText>{rowErr}</ErrorText></div>}
                      </div>
                    );
                  })}
                </div>
              )}

              {(linked.length === 0 || linkedOnPage.length > 0) && (
                <div>
                  <div style={sectionLabelStyle}>Bereits verknüpft ({linked.length})</div>
                  {linked.length === 0 && <p style={mutedStyle}>Keine verknüpften Server.</p>}
                  {linkedOnPage.map((device, i) => {
                    const rowKey = `${connection.id}:${device.external_id}`;
                    const rowIndex = unlinkedOnPage.length + i;
                    const isSelected = rowIndex === selectedIdx;
                    const detailsIsOpen = detailsOpenKey === rowKey;
                    const busyUnlink = unlinkBusy[rowKey] ?? false;
                    const rowErr = deviceError[rowKey] ?? null;
                    const localSystem = localSystems.find((s) => s.id === device.linked_system_id);
                    return (
                      <div key={device.external_id}>
                        <div
                          className="list-row"
                          role="option"
                          aria-selected={isSelected}
                          onMouseEnter={() => selectRow(rowIndex)}
                          onClick={() => selectRow(rowIndex)}
                          style={{
                            display: "flex",
                            justifyContent: "space-between",
                            alignItems: "center",
                            gap: "0.5rem",
                            padding: "0.4rem 0.6rem",
                            background: isSelected ? "var(--bg-selected)" : "transparent",
                            flexWrap: "wrap",
                          }}
                        >
                          <span style={{ display: "inline-flex", alignItems: "center", gap: "0.4rem", flexWrap: "wrap" }}>
                            <DeviceSummaryLine device={device} />
                            {localSystem && (
                              <span style={{ color: "var(--text-muted)", fontSize: "0.8em" }}>· lokal: {localSystem.name}</span>
                            )}
                          </span>
                          <span style={{ display: "flex", alignItems: "center", gap: "0.6rem", flexShrink: 0 }}>
                            <span style={{ display: "inline-flex", alignItems: "center", gap: "0.3rem" }}>
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void toggleDetails(connection, device);
                                }}
                                style={{ fontSize: "0.78rem" }}
                              >
                                {detailsIsOpen ? "Ausblenden" : "Details"}
                              </button>
                              <HintBadge label="Enter" />
                            </span>
                            <span style={{ display: "inline-flex", alignItems: "center", gap: "0.3rem" }}>
                              <button
                                type="button"
                                disabled={busyUnlink}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void handleUnlink(connection, device);
                                }}
                                style={{ fontSize: "0.78rem" }}
                              >
                                {busyUnlink ? "Entferne…" : "Entfernen"}
                              </button>
                              <HintBadge label="u" />
                            </span>
                          </span>
                        </div>
                        {rowErr && <div style={{ padding: "0 0.6rem" }}><ErrorText>{rowErr}</ErrorText></div>}
                        {detailsIsOpen && renderDetailsPanel(connection, device, rowKey)}
                      </div>
                    );
                  })}
                </div>
              )}

              {totalPages > 1 && (
                <div style={{ display: "flex", justifyContent: "center", alignItems: "center", gap: "0.6rem", paddingTop: "0.2rem" }}>
                  <button type="button" disabled={clampedPage === 0} onClick={() => goToPage(clampedPage - 1)}>
                    ◀ Zurück
                  </button>
                  <span style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>
                    Seite {clampedPage + 1} von {totalPages}
                  </span>
                  <button type="button" disabled={clampedPage >= totalPages - 1} onClick={() => goToPage(clampedPage + 1)}>
                    Weiter ▶
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
      </Modal>
    );
  }

  const openConnection = openConnectionId !== null ? connections.find((c) => c.id === openConnectionId) ?? null : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "52rem" }}>
      <section style={cardStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
          <h2 style={{ margin: 0, fontSize: "1rem" }}>netcup-Verbindungen</h2>
          <button type="button" onClick={() => setAddFormOpen(true)}>
            + Neue Verbindung
          </button>
        </div>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Liest ausschließlich Server aus netcups Server Control Panel. Es werden nie Daten dorthin
          zurückgeschrieben. netcup kennt keine Unterkonten: jede Verbindung gehört direkt zu genau einem Kunden. Die
          Serverliste selbst liefert weder Status noch IP-Adresse. Beides erscheint erst, wenn die Details eines
          Servers geöffnet werden.
        </p>
        {connectionsError && <ErrorText>{connectionsError}</ErrorText>}
        {connections.length === 0 && (
          <p style={{ margin: 0, color: "var(--text-muted)", fontSize: "0.85rem" }}>Noch keine Verbindung eingerichtet.</p>
        )}
        {connections.map((connection) => renderConnectionRow(connection))}
      </section>

      {openConnection && renderDeviceModal(openConnection)}

      {addFormOpen && (
        <Modal onClose={closeAddForm}>
          <form onSubmit={(e) => void handleAddConnection(e)} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "22rem" }}>
            <h2 style={{ margin: 0, fontSize: "1rem" }}>Neue netcup-Verbindung</h2>
            <label style={labelStyle}>
              Kunde
              <span style={{ display: "flex", gap: "0.4rem", alignItems: "center" }}>
                <select
                  ref={kundeSelectRef}
                  value={newCustomerId}
                  onChange={(e) => handleKundeSelectChange(e.target.value)}
                  required
                  style={{ flex: 1 }}
                >
                  <option value="">Kunde wählen…</option>
                  {customers.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name} ({c.short_code})
                    </option>
                  ))}
                  <option value={CREATE_NEW_CUSTOMER}>+ Neuen Kunden anlegen…</option>
                </select>
                <button
                  type="button"
                  onClick={() => void handleRefreshCustomers()}
                  disabled={customersRefreshBusy}
                  title="Kundenliste aktualisieren"
                >
                  🔄
                </button>
              </span>
            </label>
            <label style={labelStyle}>
              Label
              <input value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="ACME netcup" required autoFocus />
            </label>
            <label style={labelStyle}>
              API-Token
              <input
                type="password"
                value={newApiToken}
                onChange={(e) => {
                  setNewApiToken(e.target.value);
                  setTestResult(null);
                }}
                required
              />
            </label>
            {testResult && (testResult.ok ? <SuccessText>{testResult.message}</SuccessText> : <ErrorText>{testResult.message}</ErrorText>)}
            {addStatus && <SuccessText>{addStatus}</SuccessText>}
            {addError && <ErrorText>{addError}</ErrorText>}
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "space-between", marginTop: "0.25rem" }}>
              <button type="button" disabled={testBusy || !newApiToken} onClick={() => void handleTestConnection()}>
                {testBusy ? "Teste…" : "Verbindung testen"}
              </button>
              <span style={{ display: "flex", gap: "0.5rem" }}>
                <button type="button" onClick={closeAddForm}>
                  Abbrechen
                </button>
                <button type="submit" disabled={addBusy || newCustomerId === ""}>
                  {addBusy ? "Wird hinzugefügt…" : "Hinzufügen"}
                </button>
              </span>
            </div>
          </form>
        </Modal>
      )}
    </div>
  );
}
