import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import Modal from "./Modal";

// Level.io (RMM) plugin settings screen — second RMM integration after
// NinjaOne (see NinjaPluginSection.tsx), following the exact same
// conventions and the same read-only/manual-adopt rule from
// docs/PLUGIN_ARCHITECTURE.md: external data is always shown as
// supplementary, read-only information next to a System's own maintained
// fields (name/hostname/ip_address/notes), and copying a value across is
// always a deliberate, manual, per-field action — never an automatic sync
// step.
//
// Simpler than Ninja in one structural way: Level's own API has no
// organizations/multi-tenancy concept (confirmed against Level's developer
// docs — the API operates at account level, no org/site filtering exists),
// so a Level connection maps 1:1 to exactly one local Kunde
// (`customer_id` lives directly on the connection). No org-mapping UI, no
// per-organization customer assignment needed.
//
// Level DOES have a hierarchical "Groups" concept within one account, and
// each synced device carries a `group_id`/`group_name` (see
// plugin::level::LevelDevice on the Rust side — resolved server-side, but
// the grouping/pagination of the *display* is a pure frontend concern, same
// division of labour as Ninja's organization grouping). The device list
// below is grouped by Level group, client-side, from the same flat
// `ExternalSystemDto[]` the backend already returns — no separate
// "list groups" call, no per-group customer mapping (unlike Ninja's
// organizations, a Level group isn't its own tenant, so there's nothing to
// map to a Kunde). Groups render collapsed by default (click a header to
// expand); an expanded group's devices are paginated 10 per page, with the
// single connection-wide filter box narrowing the device set *before*
// pagination — see getDeviceGroupsList/getGroupPageInfo below. Groups can be
// expanded independently of one another (not an accordion) — the same
// "which group currently owns j/k/Enter/l/u" ambiguity that would create is
// resolved via `activeGroupKey`, updated on hover/click/pagination within a
// group, mirroring how NinjaPluginSection.tsx tracks the keyboard-active
// organization.
//
// Page layout: the connections list stays inline (identity + two actions
// per row); both the add-connection form and the per-connection device
// list/compare-and-adopt UI live in on-demand <Modal> overlays so the page
// itself stays short regardless of how many connections or devices exist.

interface LevelConnectionDto {
  id: string;
  customer_id: number;
  label: string;
}

interface ExternalSystemDto {
  external_id: string;
  name: string;
  hostname: string | null;
  ip_address: string | null;
  linked_system_id: number | null;
  // Level's group concept (see plugin::level module docs): `group_id` is
  // Level's raw, nullable field (`null` means "ungrouped", not an error).
  // `group_name` is the server-resolved plain-text name for it — `null`
  // when `group_id` itself is `null`, OR when `group_id` is set but no
  // matching group was found (e.g. a since-deleted group), in which case
  // the UI falls back to `Gruppe ${group_id}` (see groupLabelFor below).
  group_id: string | null;
  group_name: string | null;
}

interface CachedLevelSyncDto {
  synced_at_utc: string;
  devices: ExternalSystemDto[];
}

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

// Mirrors SystemForm.tsx's local System shape exactly — needed here both to
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

// Level's device object has `nickname` (user-defined, may be null) and
// `hostname` at the top level — analogous to Ninja's NAME_KEYS/HOSTNAME_KEYS
// heuristic scan, but narrower since Level's shape is fixed/documented
// rather than loosely-typed like Ninja's.
const NAME_KEYS = ["nickname", "hostname"];
const HOSTNAME_KEYS = ["hostname"];

// Sentinel option value for the Kunde <select>'s "create a new customer"
// convenience entry — never a real customer id, so it can't collide.
const CREATE_NEW_CUSTOMER = "__create_new__";

// Heuristic scan of a get_level_system_details() payload, analogous to
// NinjaPluginSection.tsx's findExternalValue: looks for top-level,
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

// Level has no flat top-level IP field — it's nested under
// `network_interfaces[].ip_addresses[]` (only present when the backend asks
// for `include_network_interfaces=true`, which get_level_system_details
// always does). Mirrors plugin::level::extract_ip_address on the Rust side.
function findExternalIp(details: Record<string, unknown>): string | null {
  const interfaces = details["network_interfaces"];
  if (!Array.isArray(interfaces)) return null;
  for (const iface of interfaces) {
    if (iface && typeof iface === "object" && Array.isArray((iface as Record<string, unknown>).ip_addresses)) {
      const addresses = (iface as Record<string, unknown>).ip_addresses as unknown[];
      const first = addresses.find((a) => typeof a === "string" && a.length > 0);
      if (typeof first === "string") return first;
    }
  }
  return null;
}

// Same reasoning as NinjaPluginSection.tsx: no natural "which timezone"
// answer for a machine-generated synced_at_utc timestamp, so we use the
// viewer's own local zone to render it via format_timestamp_for_display.
function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return "UTC";
  }
}

// Devices come back from the RMM API in API/pagination order, not
// alphabetical — sort by name (German collation) for display, independent
// of the free-text filter above the list.
function sortDevicesByName(devices: ExternalSystemDto[]): ExternalSystemDto[] {
  return devices.slice().sort((a, b) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
}

// Group-by-Level-group display: collapsed-by-default groups, 10 devices per
// page within an expanded one — mirrors the org-grouping/pagination
// conventions NinjaPluginSection.tsx applies to organizations, adapted for
// Level's simpler "just a display grouping, no per-group Kunde mapping"
// shape (see the module doc comment above).
const GROUP_PAGE_SIZE = 10;

// Sentinel group key for devices with `group_id === null` ("ungrouped" per
// Level's own docs — a legitimate case, not an error). Never collides with a
// real Level group id (those come back as opaque API-assigned strings).
const UNGROUPED_GROUP_KEY = "__ungrouped__";

interface DeviceGroup {
  groupKey: string;
  label: string;
  devices: ExternalSystemDto[];
}

// The label spec from the task: group_name when resolved, "Gruppe
// {group_id}" when the id is set but unresolved (a since-deleted group —
// see ExternalSystemDto's doc comment), "Ohne Gruppe" for the null case.
function groupLabelFor(device: ExternalSystemDto): string {
  if (device.group_id === null) return "Ohne Gruppe";
  return device.group_name ?? `Gruppe ${device.group_id}`;
}

// The full, stable key identifying one group's UI state (expanded/page)
// across renders — connection-scoped, since group ids from different Level
// accounts could otherwise collide.
function fullGroupKey(connectionId: string, groupKey: string): string {
  return `${connectionId}:${groupKey}`;
}

// Buckets an already filtered+sorted device list by group_id, sorted by
// display label (German collation) — with "Ohne Gruppe" always last,
// regardless of where it would otherwise sort alphabetically, since it
// isn't really a "named" group (see groupLabelFor).
function buildDeviceGroups(devices: ExternalSystemDto[]): DeviceGroup[] {
  const byKey = new Map<string, DeviceGroup>();
  for (const device of devices) {
    const groupKey = device.group_id ?? UNGROUPED_GROUP_KEY;
    let group = byKey.get(groupKey);
    if (!group) {
      group = { groupKey, label: groupLabelFor(device), devices: [] };
      byKey.set(groupKey, group);
    }
    group.devices.push(device);
  }
  const groups = Array.from(byKey.values());
  groups.sort((a, b) => {
    if (a.groupKey === UNGROUPED_GROUP_KEY) return b.groupKey === UNGROUPED_GROUP_KEY ? 0 : 1;
    if (b.groupKey === UNGROUPED_GROUP_KEY) return -1;
    return a.label.localeCompare(b.label, "de", { sensitivity: "base" });
  });
  return groups;
}

// Splits one group's (filtered+sorted) devices into "unlinked"/"linked",
// exactly like the flat connection-wide split this replaces.
function splitLinked(devices: ExternalSystemDto[]): { unlinked: ExternalSystemDto[]; linked: ExternalSystemDto[] } {
  return {
    unlinked: devices.filter((d) => d.linked_system_id === null),
    linked: devices.filter((d) => d.linked_system_id !== null),
  };
}

// One group's devices as a single flat, keyboard-navigable list (unlinked
// first, then linked — matching the two subsections' top-to-bottom visual
// order), analogous to the old connection-wide getCombinedDeviceRows.
function combineGroupRows(devices: ExternalSystemDto[]): { kind: "unlinked" | "linked"; device: ExternalSystemDto }[] {
  const { unlinked, linked } = splitLinked(devices);
  return [
    ...unlinked.map((device) => ({ kind: "unlinked" as const, device })),
    ...linked.map((device) => ({ kind: "linked" as const, device })),
  ];
}

interface GroupPageInfo {
  totalPages: number;
  page: number;
  pageRows: { kind: "unlinked" | "linked"; device: ExternalSystemDto }[];
}

// Slices one group's combined rows down to the current 10-per-page window,
// clamping a stale/out-of-range page index (e.g. after the filter shrank the
// group) back into range rather than showing a blank page.
function getGroupPageInfo(devices: ExternalSystemDto[], requestedPage: number): GroupPageInfo {
  const combined = combineGroupRows(devices);
  const totalPages = Math.max(1, Math.ceil(combined.length / GROUP_PAGE_SIZE));
  const page = Math.min(Math.max(requestedPage, 0), totalPages - 1);
  const start = page * GROUP_PAGE_SIZE;
  return { totalPages, page, pageRows: combined.slice(start, start + GROUP_PAGE_SIZE) };
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

// Small inline keybinding hint, mirroring CommandPalette.tsx's Row hint span —
// makes the row-level keyboard shortcuts (Enter/l/u) discoverable next to the
// mouse-clickable buttons they duplicate, rather than hidden knowledge.
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
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
      <span>
        {device.name}
        {device.hostname && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "0.85em" }}>
            {" "}— {device.hostname}
          </span>
        )}
        {device.ip_address && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-muted)", fontSize: "0.85em" }}>
            {" "}({device.ip_address})
          </span>
        )}
      </span>
    </span>
  );
}

export default function LevelPluginSection() {
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);

  const [connections, setConnections] = useState<LevelConnectionDto[]>([]);
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [connectionsError, setConnectionsError] = useState<string | null>(null);

  // Add-connection modal.
  const [addFormOpen, setAddFormOpen] = useState(false);
  const [newCustomerId, setNewCustomerId] = useState<number | "">("");
  const [newLabel, setNewLabel] = useState("");
  const [newApiKey, setNewApiKey] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addStatus, setAddStatus] = useState<string | null>(null);
  const [addError, setAddError] = useState<string | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [customersRefreshBusy, setCustomersRefreshBusy] = useState(false);
  const kundeSelectRef = useRef<HTMLSelectElement>(null);
  // Set when the user picks "+ Neuen Kunden anlegen…" in the add-connection
  // form's Kunde select — tells the customerEditorTarget-closed effect below
  // (mirroring CustomerListView.tsx's own reload-on-close pattern) that it
  // should not just refresh the customer list but also try to auto-select
  // whichever customer the CustomerForm modal just created, instead of
  // leaving the user to find and pick it themselves after a manual refresh.
  const [awaitingNewCustomer, setAwaitingNewCustomer] = useState(false);

  // Which connection's device modal is open (page-local UI state — this
  // modal only ever opens from a button on this same page, so unlike the
  // store-driven entity editors it doesn't need to live in the global
  // Zustand store).
  const [openConnectionId, setOpenConnectionId] = useState<string | null>(null);

  // Cached (offline) device sync per connection.
  const [cachedSyncByConnection, setCachedSyncByConnection] = useState<Record<string, CachedLevelSyncDto | null>>({});
  const [cacheLoaded, setCacheLoaded] = useState<Record<string, boolean>>({});
  const [cacheBusy, setCacheBusy] = useState<Record<string, boolean>>({});
  const [cacheError, setCacheError] = useState<Record<string, string | null>>({});
  const [syncedAtDisplay, setSyncedAtDisplay] = useState<Record<string, string>>({});

  // "Aktualisieren" (live sync) per connection.
  const [syncBusy, setSyncBusy] = useState<Record<string, boolean>>({});
  const [syncStatus, setSyncStatus] = useState<Record<string, string | null>>({});
  const [syncError, setSyncError] = useState<Record<string, string | null>>({});

  const [removeBusy, setRemoveBusy] = useState<Record<string, boolean>>({});

  // Device text filter, keyed by connection id — one filter box per
  // connection (Level groups are a display grouping, not a separate
  // per-group tenant like Ninja's organizations, so a single connection-wide
  // filter narrows every group's devices at once).
  const [deviceFilter, setDeviceFilter] = useState<Record<string, string>>({});
  const deviceFilterInputRef = useRef<HTMLInputElement>(null);

  // Which groups are expanded (collapsed by default), and which page each
  // expanded group is showing — both keyed by fullGroupKey
  // (`${connectionId}:${groupKey}`) so state from different connections (or
  // a previous sync's now-gone groups) never collides. Groups expand
  // independently of one another rather than as an accordion (see the
  // module doc comment) — the user can compare devices across groups
  // without losing one group's page position by opening another.
  const [groupExpanded, setGroupExpanded] = useState<Record<string, boolean>>({});
  const [groupPage, setGroupPage] = useState<Record<string, number>>({});

  // Which group currently owns j/k/Enter/l/u — since groups can be
  // independently expanded, at most one group's *page* of rows is the
  // keyboard-navigation target at a time. Updated on hover/click of a row or
  // its group's pagination controls, and when a group is expanded. Mirrors
  // NinjaPluginSection.tsx's activeGroupKey for the same reason: an
  // "expand this org" UI needs to know which org's list Enter/l/u should
  // act on. groupSelectedIndex is the highlighted row's index *within the
  // active group's current page* (0..9), not a connection-wide index —
  // pagination means "the list" is only ever the current page.
  const [activeGroupKey, setActiveGroupKey] = useState<string | null>(null);
  const [groupSelectedIndex, setGroupSelectedIndex] = useState<Record<string, number>>({});

  // Local systems cache, keyed by customer id — used both for the "link to
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
    invoke<LevelConnectionDto[]>("list_level_connections")
      .then(setConnections)
      .catch((e) => setConnectionsError(String(e)));
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

  // CustomerForm is globally mounted and driven by the store, so — exactly
  // like CustomerListView.tsx — this component learns the editor closed by
  // watching customerEditorTarget transition from non-null back to null,
  // rather than via an onDone callback. Beyond that shared reload, when the
  // close follows the "+ Neuen Kunden anlegen…" option (awaitingNewCustomer)
  // it also diffs the freshly-fetched list against whatever was in state
  // just before the fetch to find the newly-created customer and auto-select
  // it in this form's Kunde select.
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
        // created one) — auto-select it. Zero (the user cancelled instead)
        // or more than one (rare: concurrent creation elsewhere) both just
        // fall through to leaving the Kunde select as-is; the list itself is
        // still refreshed either way.
        if (newlyCreated.length === 1) {
          setNewCustomerId(newlyCreated[0].id);
        }
      } catch (err) {
        setAddError(String(err));
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
      setAddError(String(err));
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

  const loadCachedSync = useCallback(async (connection: LevelConnectionDto) => {
    const id = connection.id;
    setCacheBusy((prev) => ({ ...prev, [id]: true }));
    setCacheError((prev) => ({ ...prev, [id]: null }));
    try {
      const cached = await invoke<CachedLevelSyncDto | null>("get_cached_level_sync", { connectionId: id });
      setCachedSyncByConnection((prev) => ({ ...prev, [id]: cached }));
      setCacheLoaded((prev) => ({ ...prev, [id]: true }));
      if (cached) {
        await updateSyncedAtDisplay(id, cached.synced_at_utc);
      }
      await refreshLocalSystems(connection.customer_id).catch(() => {});
    } catch (err) {
      setCacheError((prev) => ({ ...prev, [id]: String(err) }));
    } finally {
      setCacheBusy((prev) => ({ ...prev, [id]: false }));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function openDeviceModal(connection: LevelConnectionDto) {
    setOpenConnectionId(connection.id);
    setLinkPickerKey(null);
    setDetailsOpenKey(null);
    setActiveGroupKey(null);
    void loadCachedSync(connection);
  }

  function closeDeviceModal() {
    setOpenConnectionId(null);
    setLinkPickerKey(null);
    setDetailsOpenKey(null);
  }

  // Same filter+sort logic the device modal renders with — factored out so
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

  // The filtered+sorted device list, bucketed into groups (see
  // buildDeviceGroups) — the single source of truth both renderDeviceModal
  // and the keyboard-navigation effect below walk, so they can never
  // disagree about which groups/devices are currently visible.
  function getDeviceGroupsList(connectionId: string): DeviceGroup[] {
    return buildDeviceGroups(getFilteredDevices(connectionId));
  }

  // Resets every one of this connection's groups back to page 1 — used
  // whenever the filter text changes (a new filter invalidates whatever
  // page each group was on) and after a resync (device set may have
  // changed entirely).
  function resetAllGroupPages(connectionId: string) {
    const prefix = `${connectionId}:`;
    setGroupPage((prev) => {
      const next = { ...prev };
      for (const key of Object.keys(next)) {
        if (key.startsWith(prefix)) next[key] = 0;
      }
      return next;
    });
  }

  // Expanding a group always resets it to page 1 (whether it was already on
  // a later page from a previous expand, or freshly opened) and makes it
  // the keyboard-navigation target; collapsing just closes it.
  function toggleGroupExpanded(connectionId: string, group: DeviceGroup) {
    const key = fullGroupKey(connectionId, group.groupKey);
    setGroupExpanded((prev) => {
      const nowExpanded = !prev[key];
      return { ...prev, [key]: nowExpanded };
    });
    setGroupPage((prev) => ({ ...prev, [key]: 0 }));
    setGroupSelectedIndex((prev) => ({ ...prev, [key]: 0 }));
    setActiveGroupKey(key);
  }

  // Selecting a row (hover or click) both highlights it and makes its group
  // the keyboard-navigation target — mirrors the selectRow helper
  // NinjaPluginSection.tsx uses for the same activeGroupKey pattern.
  function selectGroupRow(fullKey: string, rowIndex: number) {
    setActiveGroupKey(fullKey);
    setGroupSelectedIndex((prev) => ({ ...prev, [fullKey]: rowIndex }));
  }

  function changeGroupPage(fullKey: string, nextPage: number) {
    setActiveGroupKey(fullKey);
    setGroupPage((prev) => ({ ...prev, [fullKey]: Math.max(nextPage, 0) }));
    setGroupSelectedIndex((prev) => ({ ...prev, [fullKey]: 0 }));
  }

  // Auto-focus the device filter input once the modal is open and its cache
  // load has resolved (the input only renders once cached data exists) —
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

  // Keyboard navigation for the device modal's lists: j/k or ArrowDown/Up
  // move the highlight, Enter performs the primary action for the
  // highlighted row (create-and-link for an unlinked device, open/toggle the
  // compare-and-adopt panel for a linked device), l opens the "link to
  // existing system" picker for an unlinked row, u unlinks a linked row.
  // Gated by isTypingTarget exactly like CustomerListView.tsx, so typing in
  // the filter input (or the link-picker's <select>, which autofocuses when
  // opened) never triggers row actions.
  //
  // Operates on whichever group activeGroupKey names — and only that
  // group's *current page* of rows (pageRows), never the group's full
  // device list: with pagination, "the list the user sees" really is just
  // those <=10 rows. j/k clamp at that page's boundaries rather than
  // auto-advancing to the next/previous page — simple, predictable, and
  // matches the equivalent clamp-don't-page-flip behavior specified for
  // NinjaPluginSection.tsx's own org-grouped pagination.
  useEffect(() => {
    if (openConnectionId === null) return;
    const found = connections.find((c) => c.id === openConnectionId);
    if (!found) return;
    const connection: LevelConnectionDto = found;
    const activeConnectionId = openConnectionId;

    function onKeyDown(e: KeyboardEvent) {
      if (isTypingTarget(document.activeElement)) return;
      if (linkPickerKey !== null) return; // let the inline link-picker sub-form own its own keys
      if (activeGroupKey === null || !activeGroupKey.startsWith(`${activeConnectionId}:`)) return;
      if (!groupExpanded[activeGroupKey]) return; // a collapsed group has no visible rows to act on

      const groupKey = activeGroupKey.slice(activeConnectionId.length + 1);
      const group = getDeviceGroupsList(activeConnectionId).find((g) => g.groupKey === groupKey);
      if (!group) return;
      const { pageRows } = getGroupPageInfo(group.devices, groupPage[activeGroupKey] ?? 0);
      if (pageRows.length === 0) return;
      const idx = Math.min(groupSelectedIndex[activeGroupKey] ?? 0, pageRows.length - 1);

      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        setGroupSelectedIndex((prev) => ({ ...prev, [activeGroupKey]: Math.min(idx + 1, pageRows.length - 1) }));
      } else if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        setGroupSelectedIndex((prev) => ({ ...prev, [activeGroupKey]: Math.max(idx - 1, 0) }));
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
  }, [
    openConnectionId,
    connections,
    activeGroupKey,
    groupExpanded,
    groupPage,
    groupSelectedIndex,
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
    setNewApiKey("");
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
      // don't change here — but the browser has already visually flipped
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
      await invoke("test_level_connection", { apiKey: newApiKey });
      setTestResult({ ok: true, message: "✓ Verbindung erfolgreich" });
    } catch (err) {
      setTestResult({ ok: false, message: String(err) });
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
      await invoke("add_level_connection", {
        customerId: newCustomerId,
        label: newLabel,
        apiKey: newApiKey,
      });
      reloadConnections();
      closeAddForm();
    } catch (err) {
      setAddError(String(err));
    } finally {
      setAddBusy(false);
    }
  }

  async function handleRemoveConnection(connection: LevelConnectionDto) {
    const confirmed = window.confirm(`Level-Verbindung "${connection.label}" wirklich entfernen?`);
    if (!confirmed) return;
    const id = connection.id;
    setRemoveBusy((prev) => ({ ...prev, [id]: true }));
    setConnectionsError(null);
    try {
      await invoke("remove_level_connection", { id });
      if (openConnectionId === id) closeDeviceModal();
      setCachedSyncByConnection((prev) => {
        const next = { ...prev };
        delete next[id];
        return next;
      });
      reloadConnections();
    } catch (err) {
      setConnectionsError(String(err));
    } finally {
      setRemoveBusy((prev) => ({ ...prev, [id]: false }));
    }
  }

  async function handleSync(connection: LevelConnectionDto) {
    const id = connection.id;
    setSyncBusy((prev) => ({ ...prev, [id]: true }));
    setSyncStatus((prev) => ({ ...prev, [id]: null }));
    setSyncError((prev) => ({ ...prev, [id]: null }));
    try {
      const devices = await invoke<ExternalSystemDto[]>("sync_level_connection", { connectionId: id });
      const count = Array.isArray(devices) ? devices.length : 0;
      setSyncStatus((prev) => ({ ...prev, [id]: `${count} Gerät(e) gefunden.` }));
      // The device set (and therefore every group's page count) may have
      // changed entirely — reset every group's page back to 1 rather than
      // risk showing a stale, now out-of-range page.
      resetAllGroupPages(id);
      // Prefer re-reading the cache afterward (authoritative, server-written
      // synced_at_utc) over building the timestamp from the client clock.
      await loadCachedSync(connection);
    } catch (err) {
      setSyncError((prev) => ({ ...prev, [id]: String(err) }));
    } finally {
      setSyncBusy((prev) => ({ ...prev, [id]: false }));
    }
  }

  function openLinkPicker(connection: LevelConnectionDto, device: ExternalSystemDto) {
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

  async function confirmLink(connection: LevelConnectionDto, device: ExternalSystemDto) {
    if (linkPickerSelection === "") return;
    const key = `${connection.id}:${device.external_id}`;
    setLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("link_system_to_level", {
        systemId: linkPickerSelection,
        connectionId: connection.id,
        externalId: device.external_id,
      });
      setLinkPickerKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: String(err) }));
    } finally {
      setLinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function createAndLink(connection: LevelConnectionDto, device: ExternalSystemDto) {
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
          notes: "",
        },
      });
      await invoke("link_system_to_level", {
        systemId: created.id,
        connectionId: connection.id,
        externalId: device.external_id,
      });
      await refreshLocalSystems(customerId);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: String(err) }));
    } finally {
      setCreateLinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function handleUnlink(connection: LevelConnectionDto, device: ExternalSystemDto) {
    if (device.linked_system_id === null) return;
    const key = `${connection.id}:${device.external_id}`;
    setUnlinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("unlink_system_from_level", { systemId: device.linked_system_id, connectionId: connection.id });
      if (detailsOpenKey === key) setDetailsOpenKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: String(err) }));
    } finally {
      setUnlinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function toggleDetails(connection: LevelConnectionDto, device: ExternalSystemDto) {
    const key = `${connection.id}:${device.external_id}`;
    if (detailsOpenKey === key) {
      setDetailsOpenKey(null);
      return;
    }
    setDetailsOpenKey(key);
    setDetailsError((prev) => ({ ...prev, [key]: null }));
    setDetailsBusy((prev) => ({ ...prev, [key]: true }));
    try {
      const detailsPromise = invoke<Record<string, unknown>>("get_level_system_details", {
        connectionId: connection.id,
        externalId: device.external_id,
      });
      const refreshPromise = refreshLocalSystems(connection.customer_id);
      const [data] = await Promise.all([detailsPromise, refreshPromise]);
      setDetailsData((prev) => ({ ...prev, [key]: data }));
    } catch (err) {
      setDetailsError((prev) => ({ ...prev, [key]: String(err) }));
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
      setAdoptError((prev) => ({ ...prev, [busyKey]: String(err) }));
    } finally {
      setAdoptBusy((prev) => ({ ...prev, [busyKey]: false }));
    }
  }

  function renderDetailsPanel(connection: LevelConnectionDto, device: ExternalSystemDto, key: string) {
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
        {busy && <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Lade Level-Details…</p>}
        {err && <ErrorText>{err}</ErrorText>}
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
                    <th style={thStyle}>Level (extern)</th>
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

  function renderConnectionRow(connection: LevelConnectionDto) {
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

  function renderDeviceModal(connection: LevelConnectionDto) {
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
    const groups = getDeviceGroupsList(connection.id);

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
            <p style={mutedStyle}>Noch nicht synchronisiert. Auf "Aktualisieren" klicken, um Geräte von Level zu holen.</p>
          )}

          {cached && allDevices.length > 0 && (
            <input
              ref={deviceFilterInputRef}
              value={filterText}
              onChange={(e) => {
                setDeviceFilter((prev) => ({ ...prev, [connection.id]: e.target.value }));
                resetAllGroupPages(connection.id);
              }}
              placeholder={`Geräte filtern (${allDevices.length})…`}
              style={{ maxWidth: "20rem" }}
            />
          )}
          {cached && allDevices.length === 0 && <p style={mutedStyle}>Keine Geräte im Cache.</p>}
          {cached && allDevices.length > 0 && groups.length === 0 && <p style={mutedStyle}>Keine Geräte gefunden.</p>}

          {cached && groups.length > 0 && (
            <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
              {groups.map((group) => renderDeviceGroup(connection, group))}
            </div>
          )}
        </div>
      </Modal>
    );
  }

  // One collapsed-by-default group: a clickable header (chevron, group
  // label, device count) and — once expanded — that group's devices, using
  // the exact same compact row rendering the flat list used before
  // (unlinked-then-linked sections, keyboard hints, link-picker, compare-
  // and-adopt details panel), just narrowed down to the current 10-device
  // page. A subsection is only rendered when either it's genuinely empty
  // group-wide (so the "keine Geräte" message actually shows) or the
  // current page has at least one row in it — avoids an empty "Nicht
  // verknüpft (3)" header on a page that happens to only contain linked
  // devices.
  function renderDeviceGroup(connection: LevelConnectionDto, group: DeviceGroup) {
    const key = fullGroupKey(connection.id, group.groupKey);
    const expanded = groupExpanded[key] ?? false;
    const localSystems = localSystemsByCustomer[connection.customer_id] ?? [];
    const { unlinked, linked } = splitLinked(group.devices);
    const { totalPages, page, pageRows } = getGroupPageInfo(group.devices, groupPage[key] ?? 0);
    const selectedIdx = groupSelectedIndex[key] ?? 0;
    const unlinkedOnPage = pageRows.filter((r) => r.kind === "unlinked").map((r) => r.device);
    const linkedOnPage = pageRows.filter((r) => r.kind === "linked").map((r) => r.device);

    return (
      <div key={group.groupKey} style={{ border: "1px solid var(--border-subtle)", borderRadius: "var(--radius-sm)", overflow: "hidden" }}>
        <div
          role="button"
          aria-expanded={expanded}
          onClick={() => toggleGroupExpanded(connection.id, group)}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "0.5rem",
            padding: "0.5rem 0.7rem",
            background: "var(--bg-elevated)",
            cursor: "pointer",
            userSelect: "none",
          }}
        >
          <span style={{ fontFamily: "var(--font-mono)", fontSize: "0.75rem", color: "var(--text-muted)", width: "1rem", textAlign: "center" }}>
            {expanded ? "▾" : "▸"}
          </span>
          <span style={{ fontWeight: 600, flex: 1 }}>{group.label}</span>
          <span style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>{group.devices.length} Gerät(e)</span>
        </div>

        {expanded && (
          <div style={{ padding: "0.6rem 0.7rem", display: "flex", flexDirection: "column", gap: "0.5rem" }}>
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
                        onMouseEnter={() => selectGroupRow(key, rowIndex)}
                        onClick={() => selectGroupRow(key, rowIndex)}
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
                        onMouseEnter={() => selectGroupRow(key, rowIndex)}
                        onClick={() => selectGroupRow(key, rowIndex)}
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
                <button type="button" disabled={page === 0} onClick={() => changeGroupPage(key, page - 1)}>
                  ◀ Zurück
                </button>
                <span style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>
                  Seite {page + 1} von {totalPages}
                </span>
                <button type="button" disabled={page >= totalPages - 1} onClick={() => changeGroupPage(key, page + 1)}>
                  Weiter ▶
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    );
  }

  const openConnection = openConnectionId !== null ? connections.find((c) => c.id === openConnectionId) ?? null : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "52rem" }}>
      <section style={cardStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
          <h2 style={{ margin: 0, fontSize: "1rem" }}>Level-Verbindungen</h2>
          <button type="button" onClick={() => setAddFormOpen(true)}>
            + Neue Verbindung
          </button>
        </div>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Liest ausschließlich Geräte aus Level.io (RMM) — es werden nie Daten dorthin zurückgeschrieben. Level kennt
          keine Organisationen: jede Verbindung gehört direkt zu genau einem Kunden.
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
            <h2 style={{ margin: 0, fontSize: "1rem" }}>Neue Level-Verbindung</h2>
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
              <input value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="ACME Level" required autoFocus />
            </label>
            <label style={labelStyle}>
              API-Key
              <input
                type="password"
                value={newApiKey}
                onChange={(e) => {
                  setNewApiKey(e.target.value);
                  setTestResult(null);
                }}
                required
              />
            </label>
            {testResult && (testResult.ok ? <SuccessText>{testResult.message}</SuccessText> : <ErrorText>{testResult.message}</ErrorText>)}
            {addStatus && <SuccessText>{addStatus}</SuccessText>}
            {addError && <ErrorText>{addError}</ErrorText>}
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "space-between", marginTop: "0.25rem" }}>
              <button type="button" disabled={testBusy || !newApiKey} onClick={() => void handleTestConnection()}>
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
