import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { formatInvokeError } from "../lib/errors";
import Modal from "./Modal";

// Microsoft Intune plugin settings screen -- fourth RMM/asset management
// integration after NinjaOne, Level.io, and Snipe-IT (see
// NinjaPluginSection.tsx/LevelPluginSection.tsx/SnipeitPluginSection.tsx),
// following the exact same conventions and the same read-only/manual-adopt
// rule from docs/PLUGIN_ARCHITECTURE.md: external data is always shown as
// supplementary, read-only information next to a System's own maintained
// fields (name/hostname/ip_address/notes), and copying a value across is
// always a deliberate, manual, per-field action -- never an automatic sync
// step.
//
// Structurally closest to LevelPluginSection.tsx: one Azure AD/Entra ID
// tenant is one organization here, so an Intune connection maps 1:1 to
// exactly one local customer (`customer_id` lives directly on the
// connection, see plugin::intune::IntuneConnectionMeta on the Rust side).
// No org-mapping UI, no per-organization customer assignment needed. Unlike
// Level, Intune also has no hierarchical "Groups" concept -- the device list
// below is a genuinely flat, filterable, paginated list (10 per page), not
// grouped at all. The connection dialog authenticates via Azure AD's OAuth2
// client-credentials grant (like NinjaPluginSection.tsx's client ID/secret
// pair), but needs a third value -- the tenant ID.
//
// Page layout: the connections list stays inline (identity + two actions per
// row); both the add-connection form and the per-connection device
// list/compare-and-adopt UI live in on-demand <Modal> overlays so the page
// itself stays short regardless of how many connections or devices exist.

interface IntuneConnectionDto {
  id: string;
  customer_id: number;
  label: string;
}

interface ExternalSystemDto {
  external_id: string;
  name: string;
  hostname: string | null;
  // ALWAYS null -- Microsoft Graph's managed-device response has no IP
  // address field at all (verified against Microsoft's own official
  // schema, see plugin::intune module docs on the Rust side). Kept as a
  // field here (rather than omitted) purely so this DTO can reuse the same
  // compare/adopt panel shape as the other three plugin sections.
  ip_address: string | null;
  linked_system_id: number | null;
  operating_system: string | null;
  os_version: string | null;
  serial_number: string | null;
  manufacturer: string | null;
  model: string | null;
  compliance_state: string | null;
  last_sync_date_time: string | null;
  user_principal_name: string | null;
}

interface CachedIntuneSyncDto {
  synced_at_utc: string;
  devices: ExternalSystemDto[];
}

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

// Mirrors SystemForm.tsx's local System shape exactly -- needed here both to
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
}

type CompareField = "name" | "hostname" | "ip_address";

// Intune's managed-device object has no separate physical-hostname field --
// `deviceName` doubles as both the display name and the hostname-equivalent
// (see plugin::intune module docs), so both candidate lists point at the
// same key. No IP candidate list -- Graph's response has no IP field at all
// (see findExternalIp below), unlike Ninja/Level.
const NAME_KEYS = ["deviceName"];
const HOSTNAME_KEYS = ["deviceName"];

// Sentinel option value for the customer <select>'s "create a new customer"
// convenience entry -- never a real customer id, so it can't collide.
const CREATE_NEW_CUSTOMER = "__create_new__";

// Heuristic scan of a get_intune_system_details() payload, analogous to
// LevelPluginSection.tsx's findExternalValue: looks for top-level,
// non-object fields whose key matches one of a handful of candidate names.
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

// Microsoft Graph's managed-device object has no IP address field anywhere
// (verified against Microsoft's own official schema -- `wiFiMacAddress` is a
// MAC address, not usable as an IP). Always returns null; kept as its own
// function (rather than inlined as a literal null) purely so the compare
// table below can render its "IP-Adresse" row the same way
// LevelPluginSection.tsx/NinjaPluginSection.tsx do, instead of special
// -casing Intune's row rendering. Takes no argument (unlike
// findExternalValue) since the answer never depends on the payload.
function findExternalIp(): string | null {
  return null;
}

// Same reasoning as LevelPluginSection.tsx: no natural "which timezone"
// answer for a machine-generated synced_at_utc timestamp, so we use the
// viewer's own local zone to render it via format_timestamp_for_display.
function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return "UTC";
  }
}

// Devices come back from the Graph API in API/pagination order, not
// alphabetical -- sort by name (German collation) for display, independent
// of the free-text filter above the list.
function sortDevicesByName(devices: ExternalSystemDto[]): ExternalSystemDto[] {
  return devices.slice().sort((a, b) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
}

// Flat pagination (no group layer, unlike Level -- Intune has no group
// concept, see the module doc comment above): 10 devices per page.
const PAGE_SIZE = 10;

// One connection's (filtered+sorted) devices as a single flat,
// keyboard-navigable list (unlinked first, then linked -- matching the two
// subsections' top-to-bottom visual order), analogous to
// LevelPluginSection.tsx's combineGroupRows, just without a group dimension.
function combineRows(devices: ExternalSystemDto[]): { kind: "unlinked" | "linked"; device: ExternalSystemDto }[] {
  const unlinked = devices.filter((d) => d.linked_system_id === null);
  const linked = devices.filter((d) => d.linked_system_id !== null);
  return [
    ...unlinked.map((device) => ({ kind: "unlinked" as const, device })),
    ...linked.map((device) => ({ kind: "linked" as const, device })),
  ];
}

interface PageInfo {
  totalPages: number;
  page: number;
  pageRows: { kind: "unlinked" | "linked"; device: ExternalSystemDto }[];
}

// Slices a connection's combined rows down to the current 10-per-page
// window, clamping a stale/out-of-range page index (e.g. after the filter
// shrank the list) back into range rather than showing a blank page.
function getPageInfo(devices: ExternalSystemDto[], requestedPage: number): PageInfo {
  const combined = combineRows(devices);
  const totalPages = Math.max(1, Math.ceil(combined.length / PAGE_SIZE));
  const page = Math.min(Math.max(requestedPage, 0), totalPages - 1);
  const start = page * PAGE_SIZE;
  return { totalPages, page, pageRows: combined.slice(start, start + PAGE_SIZE) };
}

// One-time notes prefill for "Neu anlegen" (create-and-link), analogous to
// SnipeitPluginSection.tsx's asset_tag/serial prefill: a local System has no
// dedicated field for Intune-specific attributes like serial number or
// compliance state, so a short summary is written into `notes` once at
// creation time -- never overwritten afterward by a later sync.
function buildInitialNotes(device: ExternalSystemDto): string {
  const parts: string[] = [];
  if (device.serial_number) parts.push(`Seriennummer: ${device.serial_number}`);
  if (device.compliance_state) parts.push(`Compliance: ${device.compliance_state}`);
  if (device.operating_system) {
    parts.push(`Betriebssystem: ${device.operating_system}${device.os_version ? ` ${device.os_version}` : ""}`);
  }
  return parts.join("\n");
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

// Small inline keybinding hint, mirroring LevelPluginSection.tsx's
// HintBadge -- makes the row-level keyboard shortcuts (Enter/l/u)
// discoverable next to the mouse-clickable buttons they duplicate, rather
// than hidden knowledge.
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
  const osLabel = device.operating_system
    ? `${device.operating_system}${device.os_version ? ` ${device.os_version}` : ""}`
    : null;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
      <span>
        {device.name}
        {osLabel && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "0.85em" }}>
            {" "}— {osLabel}
          </span>
        )}
        {device.compliance_state && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-muted)", fontSize: "0.85em" }}>
            {" "}({device.compliance_state})
          </span>
        )}
      </span>
    </span>
  );
}

export default function IntunePluginSection() {
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);

  const [connections, setConnections] = useState<IntuneConnectionDto[]>([]);
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [connectionsError, setConnectionsError] = useState<string | null>(null);

  // Add-connection modal.
  const [addFormOpen, setAddFormOpen] = useState(false);
  const [newCustomerId, setNewCustomerId] = useState<number | "">("");
  const [newLabel, setNewLabel] = useState("");
  const [newTenantId, setNewTenantId] = useState("");
  const [newClientId, setNewClientId] = useState("");
  const [newClientSecret, setNewClientSecret] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addStatus, setAddStatus] = useState<string | null>(null);
  const [addError, setAddError] = useState<string | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [customersRefreshBusy, setCustomersRefreshBusy] = useState(false);
  const kundeSelectRef = useRef<HTMLSelectElement>(null);
  // Set when the user picks "+ Neuen Kunden anlegen…" in the add-connection
  // form's customer select -- tells the customerEditorTarget-closed effect
  // below (mirroring CustomerListView.tsx's own reload-on-close pattern)
  // that it should not just refresh the customer list but also try to
  // auto-select whichever customer the CustomerForm modal just created,
  // instead of leaving the user to find and pick it themselves after a
  // manual refresh.
  const [awaitingNewCustomer, setAwaitingNewCustomer] = useState(false);

  // Which connection's device modal is open (page-local UI state -- this
  // modal only ever opens from a button on this same page, so unlike the
  // store-driven entity editors it doesn't need to live in the global
  // Zustand store).
  const [openConnectionId, setOpenConnectionId] = useState<string | null>(null);

  // Cached (offline) device sync per connection.
  const [cachedSyncByConnection, setCachedSyncByConnection] = useState<Record<string, CachedIntuneSyncDto | null>>({});
  const [cacheLoaded, setCacheLoaded] = useState<Record<string, boolean>>({});
  const [cacheBusy, setCacheBusy] = useState<Record<string, boolean>>({});
  const [cacheError, setCacheError] = useState<Record<string, string | null>>({});
  const [syncedAtDisplay, setSyncedAtDisplay] = useState<Record<string, string>>({});

  // "Aktualisieren" (live sync) per connection.
  const [syncBusy, setSyncBusy] = useState<Record<string, boolean>>({});
  const [syncStatus, setSyncStatus] = useState<Record<string, string | null>>({});
  const [syncError, setSyncError] = useState<Record<string, string | null>>({});

  const [removeBusy, setRemoveBusy] = useState<Record<string, boolean>>({});

  // Device text filter, keyed by connection id -- one filter box per
  // connection (there's no group layer to narrow within, unlike Level).
  const [deviceFilter, setDeviceFilter] = useState<Record<string, string>>({});
  const deviceFilterInputRef = useRef<HTMLInputElement>(null);

  // Which page each connection's device list is showing, and which row is
  // keyboard-highlighted on that page -- both keyed by connection id
  // directly (no group dimension, unlike LevelPluginSection.tsx's
  // fullGroupKey).
  const [devicePage, setDevicePage] = useState<Record<string, number>>({});
  const [selectedIndex, setSelectedIndex] = useState<Record<string, number>>({});

  // Local systems cache, keyed by customer id -- used both for the "link to
  // existing system" picker and for the compare/adopt panel.
  const [localSystemsByCustomer, setLocalSystemsByCustomer] = useState<Record<number, System[]>>({});

  // Per-device state, keyed by `${connectionId}:${externalId}`.
  const [linkPickerKey, setLinkPickerKey] = useState<string | null>(null);
  const [linkPickerSelection, setLinkPickerSelection] = useState<number | "">("");
  const [linkBusy, setLinkBusy] = useState<Record<string, boolean>>({});
  const [createLinkBusy, setCreateLinkBusy] = useState<Record<string, boolean>>({});
  const [unlinkBusy, setUnlinkBusy] = useState<Record<string, boolean>>({});
  const [deviceError, setDeviceError] = useState<Record<string, string | null>>({});

  const [detailsOpenKey, setDetailsOpenKey] = useState<string | null>(null);
  const [detailsBusy, setDetailsBusy] = useState<Record<string, boolean>>({});
  const [detailsError, setDetailsError] = useState<Record<string, string | null>>({});
  const [detailsData, setDetailsData] = useState<Record<string, Record<string, unknown>>>({});
  const [showRawJson, setShowRawJson] = useState<Record<string, boolean>>({});

  // Adopt-a-field busy/error, keyed by `${localSystemId}:${field}`.
  const [adoptBusy, setAdoptBusy] = useState<Record<string, boolean>>({});
  const [adoptError, setAdoptError] = useState<Record<string, string | null>>({});

  const reloadConnections = useCallback(() => {
    invoke<IntuneConnectionDto[]>("list_intune_connections")
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

  // CustomerForm is globally mounted and driven by the store, so -- exactly
  // like LevelPluginSection.tsx -- this component learns the editor closed
  // by watching customerEditorTarget transition from non-null back to null,
  // rather than via an onDone callback. Beyond that shared reload, when the
  // close follows the "+ Neuen Kunden anlegen…" option (awaitingNewCustomer)
  // it also diffs the freshly-fetched list against whatever was in state
  // just before the fetch to find the newly-created customer and auto-select
  // it in this form's customer select.
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
        // Exactly one new customer: the common case (the user actually
        // created one) -- auto-select it. Zero (the user cancelled instead)
        // or more than one (rare: concurrent creation elsewhere) both just
        // fall through to leaving the customer select as-is; the list itself
        // is still refreshed either way.
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

  const loadCachedSync = useCallback(async (connection: IntuneConnectionDto) => {
    const id = connection.id;
    setCacheBusy((prev) => ({ ...prev, [id]: true }));
    setCacheError((prev) => ({ ...prev, [id]: null }));
    try {
      const cached = await invoke<CachedIntuneSyncDto | null>("get_cached_intune_sync", { connectionId: id });
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

  function openDeviceModal(connection: IntuneConnectionDto) {
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

  // Same filter+sort logic the device modal renders with -- factored out so
  // the keyboard-navigation effect below can walk the exact same array the
  // user sees, in the exact same order.
  function getFilteredDevices(connectionId: string): ExternalSystemDto[] {
    const cached = cachedSyncByConnection[connectionId] ?? null;
    const filterText = deviceFilter[connectionId] ?? "";
    const q = filterText.trim().toLowerCase();
    const allDevices = cached?.devices ?? [];
    const filtered =
      q === "" ? allDevices : allDevices.filter((d) => d.name.toLowerCase().includes(q) || (d.hostname ?? "").toLowerCase().includes(q));
    return sortDevicesByName(filtered);
  }

  // Resets a connection's device list back to page 1 -- used whenever the
  // filter text changes (a new filter invalidates whatever page was shown)
  // and after a resync (the device set may have changed entirely).
  function resetPage(connectionId: string) {
    setDevicePage((prev) => ({ ...prev, [connectionId]: 0 }));
    setSelectedIndex((prev) => ({ ...prev, [connectionId]: 0 }));
  }

  function selectRow(connectionId: string, rowIndex: number) {
    setSelectedIndex((prev) => ({ ...prev, [connectionId]: rowIndex }));
  }

  function changePage(connectionId: string, nextPage: number) {
    setDevicePage((prev) => ({ ...prev, [connectionId]: Math.max(nextPage, 0) }));
    setSelectedIndex((prev) => ({ ...prev, [connectionId]: 0 }));
  }

  // Auto-focus the device filter input once the modal is open and its cache
  // load has resolved (the input only renders once cached data exists) --
  // lets the user start typing to narrow the list immediately. Keyed on the
  // *value* of cacheLoaded[id] rather than the cacheLoaded object itself, so
  // a later "Aktualisieren" re-sync (which also flips that same flag to
  // true) doesn't steal focus back a second time.
  const activeConnectionCacheLoaded = openConnectionId !== null ? (cacheLoaded[openConnectionId] ?? false) : false;
  useEffect(() => {
    if (!activeConnectionCacheLoaded) return;
    const id = window.setTimeout(() => deviceFilterInputRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, [openConnectionId, activeConnectionCacheLoaded]);

  // Keyboard navigation for the device modal's list: j/k or ArrowDown/Up
  // move the highlight, Enter performs the primary action for the
  // highlighted row (create-and-link for an unlinked device, open/toggle the
  // compare-and-adopt panel for a linked device), l opens the "link to
  // existing system" picker for an unlinked row, u unlinks a linked row.
  // Gated by isTypingTarget exactly like LevelPluginSection.tsx, so typing
  // in the filter input (or the link-picker's <select>, which autofocuses
  // when opened) never triggers row actions.
  //
  // Operates on the open connection's *current page* of rows (pageRows),
  // never its full device list -- with pagination, "the list the user sees"
  // really is just those <=10 rows. j/k clamp at that page's boundaries
  // rather than auto-advancing to the next/previous page, matching
  // LevelPluginSection.tsx's own clamp-don't-page-flip behavior.
  useEffect(() => {
    if (openConnectionId === null) return;
    const found = connections.find((c) => c.id === openConnectionId);
    if (!found) return;
    const connection: IntuneConnectionDto = found;
    const id = openConnectionId;

    function onKeyDown(e: KeyboardEvent) {
      if (isTypingTarget(document.activeElement)) return;
      if (linkPickerKey !== null) return; // let the inline link-picker sub-form own its own keys

      const devices = getFilteredDevices(id);
      const { pageRows } = getPageInfo(devices, devicePage[id] ?? 0);
      if (pageRows.length === 0) return;
      const idx = Math.min(selectedIndex[id] ?? 0, pageRows.length - 1);

      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((prev) => ({ ...prev, [id]: Math.min(idx + 1, pageRows.length - 1) }));
      } else if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((prev) => ({ ...prev, [id]: Math.max(idx - 1, 0) }));
      } else if (e.key === "Enter") {
        const row = pageRows[idx];
        if (row) {
          e.preventDefault();
          if (row.kind === "unlinked") {
            void createAndLink(connection, row.device);
          } else {
            void toggleDetails(connection, row.device);
          }
        }
      } else if (e.key === "l") {
        const row = pageRows[idx];
        if (row && row.kind === "unlinked") {
          e.preventDefault();
          openLinkPicker(connection, row.device);
        }
      } else if (e.key === "u") {
        const row = pageRows[idx];
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
    // listener on every render instead of only when the state above
    // actually changes. Their own bodies read from connection/row (already
    // fresh via the deps above), so a stale reference to the function itself
    // carries no stale-closure risk.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    openConnectionId,
    connections,
    devicePage,
    selectedIndex,
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
    setNewTenantId("");
    setNewClientId("");
    setNewClientSecret("");
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
      // don't change here -- but the browser has already visually flipped
      // the displayed option to the sentinel one, and no state change means
      // no re-render to snap it back. Reset the DOM value imperatively so it
      // doesn't visually stick on "+ Neuen Kunden anlegen…".
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
      await invoke("test_intune_connection", {
        tenantId: newTenantId,
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
    if (newCustomerId === "") return;
    setAddError(null);
    setAddStatus(null);
    setAddBusy(true);
    try {
      await invoke("add_intune_connection", {
        customerId: newCustomerId,
        label: newLabel,
        tenantId: newTenantId,
        clientId: newClientId,
        clientSecret: newClientSecret,
      });
      reloadConnections();
      closeAddForm();
    } catch (err) {
      setAddError(formatInvokeError(err));
    } finally {
      setAddBusy(false);
    }
  }

  async function handleRemoveConnection(connection: IntuneConnectionDto) {
    const confirmed = window.confirm(`Intune-Verbindung "${connection.label}" wirklich entfernen?`);
    if (!confirmed) return;
    const id = connection.id;
    setRemoveBusy((prev) => ({ ...prev, [id]: true }));
    setConnectionsError(null);
    try {
      await invoke("remove_intune_connection", { id });
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

  async function handleSync(connection: IntuneConnectionDto) {
    const id = connection.id;
    setSyncBusy((prev) => ({ ...prev, [id]: true }));
    setSyncStatus((prev) => ({ ...prev, [id]: null }));
    setSyncError((prev) => ({ ...prev, [id]: null }));
    try {
      const devices = await invoke<ExternalSystemDto[]>("sync_intune_connection", { connectionId: id });
      const count = Array.isArray(devices) ? devices.length : 0;
      setSyncStatus((prev) => ({ ...prev, [id]: `${count} Gerät(e) gefunden.` }));
      // The device set (and therefore the page count) may have changed
      // entirely -- reset back to page 1 rather than risk showing a stale,
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

  function openLinkPicker(connection: IntuneConnectionDto, device: ExternalSystemDto) {
    const customerId = connection.customer_id;
    const key = `${connection.id}:${device.external_id}`;
    setLinkPickerKey(key);
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    const localSystems = localSystemsByCustomer[customerId] ?? [];
    const deviceHostname = device.hostname?.trim().toLowerCase() ?? "";
    const match =
      deviceHostname !== "" ? localSystems.find((s) => s.hostname.trim().toLowerCase() === deviceHostname) : undefined;
    setLinkPickerSelection(match ? match.id : "");
    if (!localSystemsByCustomer[customerId]) {
      void refreshLocalSystems(customerId);
    }
  }

  async function confirmLink(connection: IntuneConnectionDto, device: ExternalSystemDto) {
    if (linkPickerSelection === "") return;
    const key = `${connection.id}:${device.external_id}`;
    setLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("link_system_to_intune", {
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

  async function createAndLink(connection: IntuneConnectionDto, device: ExternalSystemDto) {
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
          hostname: device.hostname ?? "",
          ip_address: device.ip_address ?? "",
          notes: buildInitialNotes(device),
        },
      });
      await invoke("link_system_to_intune", {
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

  async function handleUnlink(connection: IntuneConnectionDto, device: ExternalSystemDto) {
    if (device.linked_system_id === null) return;
    const key = `${connection.id}:${device.external_id}`;
    setUnlinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("unlink_system_from_intune", { systemId: device.linked_system_id, connectionId: connection.id });
      if (detailsOpenKey === key) setDetailsOpenKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setUnlinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function toggleDetails(connection: IntuneConnectionDto, device: ExternalSystemDto) {
    const key = `${connection.id}:${device.external_id}`;
    if (detailsOpenKey === key) {
      setDetailsOpenKey(null);
      return;
    }
    setDetailsOpenKey(key);
    setDetailsError((prev) => ({ ...prev, [key]: null }));
    setDetailsBusy((prev) => ({ ...prev, [key]: true }));
    try {
      const detailsPromise = invoke<Record<string, unknown>>("get_intune_system_details", {
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
        },
      });
      await refreshLocalSystems(localSystem.customer_id);
    } catch (err) {
      setAdoptError((prev) => ({ ...prev, [busyKey]: formatInvokeError(err) }));
    } finally {
      setAdoptBusy((prev) => ({ ...prev, [busyKey]: false }));
    }
  }

  function renderDetailsPanel(connection: IntuneConnectionDto, device: ExternalSystemDto, key: string) {
    const busy = detailsBusy[key] ?? false;
    const err = detailsError[key] ?? null;
    const data = detailsData[key];
    const localSystems = localSystemsByCustomer[connection.customer_id] ?? [];
    const localSystem =
      device.linked_system_id !== null ? localSystems.find((s) => s.id === device.linked_system_id) : undefined;

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
        {busy && <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Lade Intune-Details…</p>}
        {err && <ErrorText>{err}</ErrorText>}
        {!busy && data && !localSystem && (
          <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--danger)" }}>
            Lokales System nicht gefunden (evtl. archiviert).
          </p>
        )}
        {!busy && data && localSystem && (
          <>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "0.3rem 1rem", fontSize: "0.8rem", color: "var(--text-secondary)" }}>
              {device.compliance_state && <span>Compliance: {device.compliance_state}</span>}
              {device.operating_system && (
                <span>
                  OS: {device.operating_system}
                  {device.os_version ? ` ${device.os_version}` : ""}
                </span>
              )}
              {device.manufacturer && <span>Hersteller: {device.manufacturer}</span>}
              {device.model && <span>Modell: {device.model}</span>}
              {device.serial_number && <span>Seriennummer: {device.serial_number}</span>}
              {device.user_principal_name && <span>Benutzer: {device.user_principal_name}</span>}
            </div>
            <div style={{ overflowX: "auto" }}>
              <table style={{ borderCollapse: "collapse", fontSize: "0.82rem", width: "100%" }}>
                <thead>
                  <tr>
                    <th style={thStyle}>Feld</th>
                    <th style={thStyle}>Lokal</th>
                    <th style={thStyle}>Intune (extern)</th>
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
                        externalValue: findExternalIp(),
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

  function renderConnectionRow(connection: IntuneConnectionDto) {
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
              Geräte anzeigen
            </button>
            <button type="button" disabled={removing} onClick={() => void handleRemoveConnection(connection)}>
              {removing ? "Entferne…" : "Entfernen"}
            </button>
          </div>
        </div>
      </div>
    );
  }

  function renderDeviceModal(connection: IntuneConnectionDto) {
    const busy = syncBusy[connection.id] ?? false;
    const status = syncStatus[connection.id] ?? null;
    const err = syncError[connection.id] ?? null;
    const cached = cachedSyncByConnection[connection.id] ?? null;
    const hasCacheLoaded = cacheLoaded[connection.id] ?? false;
    const cacheBusyFlag = cacheBusy[connection.id] ?? false;
    const cacheErr = cacheError[connection.id] ?? null;
    const syncedDisplay = syncedAtDisplay[connection.id];

    const filterText = deviceFilter[connection.id] ?? "";
    const allDevices = cached?.devices ?? [];
    const filteredDevices = getFilteredDevices(connection.id);
    const localSystems = localSystemsByCustomer[connection.customer_id] ?? [];
    const { unlinked, linked } = {
      unlinked: filteredDevices.filter((d) => d.linked_system_id === null),
      linked: filteredDevices.filter((d) => d.linked_system_id !== null),
    };
    const { totalPages, page, pageRows } = getPageInfo(filteredDevices, devicePage[connection.id] ?? 0);
    const selectedIdx = selectedIndex[connection.id] ?? 0;
    const unlinkedOnPage = pageRows.filter((r) => r.kind === "unlinked").map((r) => r.device);
    const linkedOnPage = pageRows.filter((r) => r.kind === "linked").map((r) => r.device);

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
            <p style={mutedStyle}>Noch nicht synchronisiert. Auf "Aktualisieren" klicken, um Geräte von Intune zu holen.</p>
          )}

          {cached && allDevices.length > 0 && (
            <input
              ref={deviceFilterInputRef}
              value={filterText}
              onChange={(e) => {
                setDeviceFilter((prev) => ({ ...prev, [connection.id]: e.target.value }));
                resetPage(connection.id);
              }}
              placeholder={`Geräte filtern (${allDevices.length})…`}
              style={{ maxWidth: "20rem" }}
            />
          )}
          {cached && allDevices.length === 0 && <p style={mutedStyle}>Keine Geräte im Cache.</p>}
          {cached && allDevices.length > 0 && filteredDevices.length === 0 && <p style={mutedStyle}>Keine Geräte gefunden.</p>}

          {cached && filteredDevices.length > 0 && (
            <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
              {(unlinked.length === 0 || unlinkedOnPage.length > 0) && (
                <div>
                  <div style={sectionLabelStyle}>Nicht verknüpft ({unlinked.length})</div>
                  {unlinked.length === 0 && <p style={mutedStyle}>Keine offenen Geräte.</p>}
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
                          onMouseEnter={() => selectRow(connection.id, rowIndex)}
                          onClick={() => selectRow(connection.id, rowIndex)}
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
                                  {s.hostname ? ` — ${s.hostname}` : ""}
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
                  {linked.length === 0 && <p style={mutedStyle}>Keine verknüpften Geräte.</p>}
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
                          onMouseEnter={() => selectRow(connection.id, rowIndex)}
                          onClick={() => selectRow(connection.id, rowIndex)}
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
                  <button type="button" disabled={page === 0} onClick={() => changePage(connection.id, page - 1)}>
                    ◀ Zurück
                  </button>
                  <span style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>
                    Seite {page + 1} von {totalPages}
                  </span>
                  <button type="button" disabled={page >= totalPages - 1} onClick={() => changePage(connection.id, page + 1)}>
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
          <h2 style={{ margin: 0, fontSize: "1rem" }}>Intune-Verbindungen</h2>
          <button type="button" onClick={() => setAddFormOpen(true)}>
            + Neue Verbindung
          </button>
        </div>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Liest ausschließlich Geräte aus Microsoft Intune (über Microsoft Graph) — es werden nie Daten dorthin
          zurückgeschrieben. Ein Azure AD/Entra-ID-Mandant entspricht direkt genau einem Kunden.
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
            <h2 style={{ margin: 0, fontSize: "1rem" }}>Neue Intune-Verbindung</h2>
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
              <input value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="ACME Intune" required autoFocus />
            </label>
            <label style={labelStyle}>
              Tenant-ID
              <input
                value={newTenantId}
                onChange={(e) => {
                  setNewTenantId(e.target.value);
                  setTestResult(null);
                }}
                placeholder="z. B. contoso.onmicrosoft.com oder GUID"
                required
              />
            </label>
            <label style={labelStyle}>
              Client-ID (App-ID)
              <input
                value={newClientId}
                onChange={(e) => {
                  setNewClientId(e.target.value);
                  setTestResult(null);
                }}
                required
              />
            </label>
            <label style={labelStyle}>
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
            </label>
            {testResult && (testResult.ok ? <SuccessText>{testResult.message}</SuccessText> : <ErrorText>{testResult.message}</ErrorText>)}
            {addStatus && <SuccessText>{addStatus}</SuccessText>}
            {addError && <ErrorText>{addError}</ErrorText>}
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "space-between", marginTop: "0.25rem" }}>
              <button
                type="button"
                disabled={testBusy || !newTenantId || !newClientId || !newClientSecret}
                onClick={() => void handleTestConnection()}
              >
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
