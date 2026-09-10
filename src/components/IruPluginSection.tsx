import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { formatInvokeError } from "../lib/errors";
import { getKeymap, matchesBinding } from "../lib/keymap";
import Modal from "./Modal";

// Iru (Apple MDM, formerly "Kandji") plugin settings screen — fourth
// integration after NinjaOne (NinjaPluginSection.tsx), Level.io
// (LevelPluginSection.tsx) and Snipe-IT (SnipeitPluginSection.tsx). Same
// read-only/manual-adopt rule as all three: external data is always shown as
// supplementary, read-only information next to a System's own maintained
// fields (name/hostname/ip_address/notes), and copying a value across is
// always a deliberate, manual, per-field action — never an automatic sync
// step.
//
// Naming note: Iru's product was rebranded from "Kandji" — its own public
// API docs (api-docs.iru.com) still reference the old kandji.io domain in
// places (see plugin::iru's own module doc comment on the Rust side). Not a
// typo anywhere in this file either.
//
// Structurally closest to Level.io: like Level, Iru's API has no
// organization/tenant concept, so a connection maps 1:1 to exactly one local
// customer (customer_id lives directly on the connection) — no org-mapping
// UI, no per-organization customer assignment, no per-group nesting like
// Level's own "Groups" concept either (Iru has nothing analogous). The
// device list below is therefore a single flat, filterable, paginated list
// per connection — simpler than LevelPluginSection.tsx in that one regard.
//
// Unlike Level (fixed API host) but like Snipe-IT: Iru is subdomain-per-
// tenant (`https://{sub_domain}.api.kandji.io` or the EU equivalent), so a
// connection also carries a user-supplied base URL, entered as one whole
// string exactly like SnipeitPluginSection.tsx's Base-URL field.
//
// Also unlike Level (which uses `hostname` as the "link to existing system"
// match key) but like Snipe-IT: Iru's device object has no hostname/IP
// address field at all (verified against Iru's own live API docs, see
// plugin::iru module docs). So this file follows Snipe-IT's own precedent
// (SnipeitPluginSection.tsx::matchKeyForDevice) — the first available
// identifying Iru field wins, in order serial_number then asset_tag,
// compared against the only free-text field a local System has for it,
// System.hostname. For the same reason, creating-and-linking a new local
// System seeds those two Iru-only identifiers into the new System's
// "Notizen" once, up front (see createAndLink) — exactly Snipe-IT's own
// one-time notes-prefill convention, since a local System has no dedicated
// serial_number/asset_tag field of its own.
//
// Page layout: the connections list stays inline (identity + two actions
// per row); both the add-connection form and the per-connection device
// list/compare-and-adopt UI live in on-demand <Modal> overlays, exactly like
// LevelPluginSection.tsx/SnipeitPluginSection.tsx.

interface IruConnectionDto {
  id: string;
  customer_id: number;
  label: string;
  base_url: string;
}

interface ExternalSystemDto {
  external_id: string;
  name: string;
  // Always `null` — Iru's device object has no hostname/IP field (verified
  // against Iru's own live API docs, see plugin::iru module docs). Kept as
  // fields anyway for structural parity with Ninja/Level/Snipe-IT's own
  // ExternalSystemDto.
  hostname: string | null;
  ip_address: string | null;
  // Iru's own natural identification fields — the "real" identifying
  // fields for this plugin, unlike hostname/ip_address above.
  serial_number: string | null;
  asset_tag: string | null;
  model: string | null;
  platform: string | null;
  os_version: string | null;
  linked_system_id: number | null;
}

interface CachedIruSyncDto {
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
// create_system input objects the same way SystemForm.tsx does. Note there
// is no serial_number/asset_tag field here — a local System only ever has
// name/hostname/ip_address/notes, so those two Iru-only fields never become
// their own compare/adopt row (see CompareField below); they're folded into
// the "Notizen" field on creation instead (see createAndLink), exactly like
// SnipeitPluginSection.tsx.
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

// Iru's own device JSON (from get_iru_system_details) reliably has
// "device_name"/"model"/"serial_number"/"asset_tag" at the top level, but
// nothing resembling a hostname/IP (see module docs above) — the HOSTNAME/IP
// candidate lists are kept as a narrow, best-effort scan anyway (in case a
// future Iru API version exposes one), never a guess at anything else.
const NAME_KEYS = ["device_name", "model", "serial_number", "asset_tag"];
const HOSTNAME_KEYS = ["hostname", "host_name"];
const IP_KEYS = ["ip_address", "ip", "ipAddress"];

// Sentinel option value for the customer <select>'s "create a new customer"
// convenience entry — never a real customer id, so it can't collide.
const CREATE_NEW_CUSTOMER = "__create_new__";

// Heuristic scan of a get_iru_system_details() payload: looks for top-level,
// non-object fields whose key matches one of a handful of candidate names.
// Identical in spirit to LevelPluginSection.tsx's/SnipeitPluginSection.tsx's
// own findExternalValue.
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

// Devices come back from Iru's API in whatever order the API returns them in
// — sort alphabetically by name (German collation) for display, independent
// of the free-text filter below.
function sortDevicesByName(devices: ExternalSystemDto[]): ExternalSystemDto[] {
  return devices.slice().sort((a, b) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
}

// Iru has no hostname/IP field on the core device object (see module docs
// above) -- so the suggested match key for "Link to existing system" isn't
// compared against device.hostname like it is for Ninja/Level; instead, the
// first present, genuinely identifying Iru field is taken in this order:
// serial_number first, then asset_tag — following Snipe-IT's own precedent
// for the identical situation (SnipeitPluginSection.tsx::matchKeyForDevice),
// just with the field priority Iru's own API docs suggest (serial_number is
// Iru's primary per-device identifier, asset_tag is a free-text field an
// admin may or may not have filled in). This value is still compared against
// the only free-text field a local system has for it — System.hostname —
// exactly as with Ninja/Level/Snipe-IT.
function matchKeyForDevice(device: ExternalSystemDto): string {
  return (device.serial_number || device.asset_tag || "").trim().toLowerCase();
}

// How many devices a connection's device list shows per page.
const DEVICE_PAGE_SIZE = 10;

// Slices `items` (already filtered/sorted) to one page of DEVICE_PAGE_SIZE,
// clamping a possibly-stale stored page number into range — e.g. after the
// text filter narrows the list, or after a resync shrinks/grows it. Shared
// by renderDeviceModal (mouse) and the keydown handler (keyboard) so both
// agree on what "the current page" contains — same helper shape as
// SnipeitPluginSection.tsx's paginateDevices.
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

// Small inline keybinding hint, mirroring CommandPalette.tsx's Row hint span
// — makes the row-level keyboard shortcuts (Enter/l/u) discoverable next to
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
  // Mirrors SnipeitPluginSection.tsx's own redundancy check: the backend
  // already falls back the display name to model/serial_number when
  // device_name is blank (see plugin::iru::map_device), so re-showing the
  // same value a second time would be noise, not information.
  const showAssetTag = device.asset_tag && device.asset_tag !== device.name;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
      <span>
        {device.name}
        {device.model && device.model !== device.name && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "0.85em" }}>
            {" "}— {device.model}
          </span>
        )}
        {device.serial_number && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-muted)", fontSize: "0.85em" }}>
            {" "}(SN: {device.serial_number})
          </span>
        )}
        {showAssetTag && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-muted)", fontSize: "0.85em" }}>
            {" "}[{device.asset_tag}]
          </span>
        )}
      </span>
    </span>
  );
}

export default function IruPluginSection() {
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);

  const [connections, setConnections] = useState<IruConnectionDto[]>([]);
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [connectionsError, setConnectionsError] = useState<string | null>(null);

  // Add-connection modal.
  const [addFormOpen, setAddFormOpen] = useState(false);
  const [newCustomerId, setNewCustomerId] = useState<number | "">("");
  const [newLabel, setNewLabel] = useState("");
  const [newBaseUrl, setNewBaseUrl] = useState("");
  const [newToken, setNewToken] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addStatus, setAddStatus] = useState<string | null>(null);
  const [addError, setAddError] = useState<string | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [customersRefreshBusy, setCustomersRefreshBusy] = useState(false);
  const kundeSelectRef = useRef<HTMLSelectElement>(null);
  // Set when the user picks "+ Neuen Kunden anlegen…" in the add-connection
  // form's customer select — mirrors LevelPluginSection.tsx's own
  // awaitingNewCustomer flag exactly.
  const [awaitingNewCustomer, setAwaitingNewCustomer] = useState(false);

  // Which connection's device modal is open (page-local UI state).
  const [openConnectionId, setOpenConnectionId] = useState<string | null>(null);

  // Cached (offline) device sync per connection.
  const [cachedSyncByConnection, setCachedSyncByConnection] = useState<Record<string, CachedIruSyncDto | null>>({});
  const [cacheLoaded, setCacheLoaded] = useState<Record<string, boolean>>({});
  const [cacheBusy, setCacheBusy] = useState<Record<string, boolean>>({});
  const [cacheError, setCacheError] = useState<Record<string, string | null>>({});
  const [syncedAtDisplay, setSyncedAtDisplay] = useState<Record<string, string>>({});

  // "Aktualisieren" (live sync) per connection.
  const [syncBusy, setSyncBusy] = useState<Record<string, boolean>>({});
  const [syncStatus, setSyncStatus] = useState<Record<string, string | null>>({});
  const [syncError, setSyncError] = useState<Record<string, string | null>>({});

  const [removeBusy, setRemoveBusy] = useState<Record<string, boolean>>({});

  // Device text filter and pagination, both keyed by connection id — Iru has
  // no group/organization concept, so (unlike LevelPluginSection.tsx) there
  // is exactly one flat device list per connection, not one per group.
  const [deviceFilter, setDeviceFilter] = useState<Record<string, string>>({});
  const deviceFilterInputRef = useRef<HTMLInputElement>(null);
  const [pageByConnection, setPageByConnection] = useState<Record<string, number>>({});
  const [selectedIndexByConnection, setSelectedIndexByConnection] = useState<Record<string, number>>({});

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
  // Keyed by connection id directly (a connection maps 1:1 to one customer,
  // so there's no group/tenant-scoped key here) — disables the "Alle
  // anlegen" bulk button for one connection while it works through that
  // connection's unlinked devices, independent of the per-device
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
    invoke<IruConnectionDto[]>("list_iru_connections")
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

  // CustomerForm is globally mounted and driven by the store, so — exactly
  // like LevelPluginSection.tsx/CustomerListView.tsx — this component learns
  // the editor closed by watching customerEditorTarget transition from
  // non-null back to null, rather than via an onDone callback.
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

  const loadCachedSync = useCallback(async (connection: IruConnectionDto) => {
    const id = connection.id;
    setCacheBusy((prev) => ({ ...prev, [id]: true }));
    setCacheError((prev) => ({ ...prev, [id]: null }));
    try {
      const cached = await invoke<CachedIruSyncDto | null>("get_cached_iru_sync", { connectionId: id });
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

  function openDeviceModal(connection: IruConnectionDto) {
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
  // linked) — the single source of truth both renderDeviceModal and the
  // keyboard-navigation effect below walk, so they can never disagree about
  // which devices/rows are currently visible. Analogous to
  // SnipeitPluginSection.tsx's computeGroupDevices, just without any
  // group/company layer (Iru has none).
  function computeConnectionDevices(connectionId: string) {
    const cached = cachedSyncByConnection[connectionId] ?? null;
    const filterText = deviceFilter[connectionId] ?? "";
    const q = filterText.trim().toLowerCase();
    const allDevices = cached?.devices ?? [];
    const sorted = sortDevicesByName(allDevices);
    const filteredDevices =
      q === ""
        ? sorted
        : sorted.filter(
            (d) =>
              d.name.toLowerCase().includes(q) ||
              (d.serial_number ?? "").toLowerCase().includes(q) ||
              (d.asset_tag ?? "").toLowerCase().includes(q) ||
              (d.model ?? "").toLowerCase().includes(q),
          );
    const unlinked = filteredDevices.filter((d) => d.linked_system_id === null);
    const linked = filteredDevices.filter((d) => d.linked_system_id !== null);
    const navItems: { device: ExternalSystemDto; kind: "unlinked" | "linked" }[] = [
      ...unlinked.map((device) => ({ device, kind: "unlinked" as const })),
      ...linked.map((device) => ({ device, kind: "linked" as const })),
    ];
    return { allDevices, filterText, filteredDevices, unlinked, linked, navItems };
  }

  // Resets one connection's device list back to page 1 — used whenever the
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
  // Gated by isTypingTarget exactly like LevelPluginSection.tsx, so typing
  // in the filter input (or the link-picker's <select>, which autofocuses
  // when opened) never triggers row actions. Unlike LevelPluginSection.tsx
  // there is no activeGroupKey layer — the currently open connection's list
  // is unambiguously "the list", since Iru has no groups.
  useEffect(() => {
    if (openConnectionId === null) return;
    const found = connections.find((c) => c.id === openConnectionId);
    if (!found) return;
    const connection: IruConnectionDto = found;
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
    setNewBaseUrl("");
    setNewToken("");
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
      await invoke("test_iru_connection", { baseUrl: newBaseUrl, token: newToken });
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
      await invoke("add_iru_connection", {
        customerId: newCustomerId,
        label: newLabel,
        baseUrl: newBaseUrl,
        token: newToken,
      });
      reloadConnections();
      closeAddForm();
    } catch (err) {
      setAddError(formatInvokeError(err));
    } finally {
      setAddBusy(false);
    }
  }

  async function handleRemoveConnection(connection: IruConnectionDto) {
    const confirmed = window.confirm(`Iru-Verbindung "${connection.label}" wirklich entfernen?`);
    if (!confirmed) return;
    const id = connection.id;
    setRemoveBusy((prev) => ({ ...prev, [id]: true }));
    setConnectionsError(null);
    try {
      await invoke("remove_iru_connection", { id });
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

  async function handleSync(connection: IruConnectionDto) {
    const id = connection.id;
    setSyncBusy((prev) => ({ ...prev, [id]: true }));
    setSyncStatus((prev) => ({ ...prev, [id]: null }));
    setSyncError((prev) => ({ ...prev, [id]: null }));
    try {
      const devices = await invoke<ExternalSystemDto[]>("sync_iru_connection", { connectionId: id });
      const count = Array.isArray(devices) ? devices.length : 0;
      setSyncStatus((prev) => ({ ...prev, [id]: `${count} Gerät(e) gefunden.` }));
      // The device set (and therefore the page count) may have changed
      // entirely — reset back to page 1 rather than risk showing a stale,
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

  function openLinkPicker(connection: IruConnectionDto, device: ExternalSystemDto) {
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

  async function confirmLink(connection: IruConnectionDto, device: ExternalSystemDto) {
    if (linkPickerSelection === "") return;
    const key = `${connection.id}:${device.external_id}`;
    setLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("link_system_to_iru", {
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

  async function createAndLink(connection: IruConnectionDto, device: ExternalSystemDto) {
    const customerId = connection.customer_id;
    const key = `${connection.id}:${device.external_id}`;
    setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      // A local System has no serial_number/asset_tag field of its own —
      // rather than silently losing those two Iru-native identifiers on
      // creation, they're seeded into "Notizen" once, up front. One-time
      // default at creation time, not an automatic overwrite of an existing
      // System's fields later — the same convention
      // SnipeitPluginSection.tsx::createAndLink uses for the identical
      // situation, consistent with docs/PLUGIN_ARCHITECTURE.md's "external
      // data never auto-overwrites self-maintained fields" rule (there is no
      // existing System yet here).
      const noteLines = [
        device.serial_number ? `Iru Seriennummer: ${device.serial_number}` : null,
        device.asset_tag ? `Iru Asset-Tag: ${device.asset_tag}` : null,
      ].filter((line): line is string => line !== null);
      const created = await invoke<System>("create_system", {
        input: {
          customer_id: customerId,
          name: device.name,
          system_type: "",
          hostname: device.hostname ?? "",
          ip_address: device.ip_address ?? "",
          notes: noteLines.join("\n"),
        },
      });
      await invoke("link_system_to_iru", {
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

  // Bulk version of `createAndLink`: works through every unlinked device on
  // this connection sequentially. A failure on one device does not abort
  // the rest — it is recorded in the same `deviceError` map that already
  // surfaces per-device errors, so a partial run still leaves the row's own
  // "Neu anlegen" button as the retry path. `refreshLocalSystems`/
  // `loadCachedSync` run once at the end, not per device, so a connection
  // with many devices doesn't refetch the whole customer's system list N
  // times.
  async function createAndLinkAll(connection: IruConnectionDto, devices: ExternalSystemDto[]) {
    if (devices.length === 0) return;
    const customerId = connection.customer_id;
    setBulkCreateBusy((prev) => ({ ...prev, [connection.id]: true }));
    try {
      for (const device of devices) {
        const key = `${connection.id}:${device.external_id}`;
        setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
        setDeviceError((prev) => ({ ...prev, [key]: null }));
        try {
          const noteLines = [
            device.serial_number ? `Iru Seriennummer: ${device.serial_number}` : null,
            device.asset_tag ? `Iru Asset-Tag: ${device.asset_tag}` : null,
          ].filter((line): line is string => line !== null);
          const created = await invoke<System>("create_system", {
            input: {
              customer_id: customerId,
              name: device.name,
              system_type: "",
              hostname: device.hostname ?? "",
              ip_address: device.ip_address ?? "",
              notes: noteLines.join("\n"),
            },
          });
          await invoke("link_system_to_iru", {
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

  async function handleUnlink(connection: IruConnectionDto, device: ExternalSystemDto) {
    if (device.linked_system_id === null) return;
    const key = `${connection.id}:${device.external_id}`;
    setUnlinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("unlink_system_from_iru", { systemId: device.linked_system_id, connectionId: connection.id });
      if (detailsOpenKey === key) setDetailsOpenKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setUnlinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function toggleDetails(connection: IruConnectionDto, device: ExternalSystemDto) {
    const key = `${connection.id}:${device.external_id}`;
    if (detailsOpenKey === key) {
      setDetailsOpenKey(null);
      return;
    }
    setDetailsOpenKey(key);
    setDetailsError((prev) => ({ ...prev, [key]: null }));
    setDetailsBusy((prev) => ({ ...prev, [key]: true }));
    try {
      const detailsPromise = invoke<Record<string, unknown>>("get_iru_system_details", {
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

  function renderDetailsPanel(connection: IruConnectionDto, device: ExternalSystemDto, key: string) {
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
        <div style={{ display: "flex", gap: "0.75rem", alignItems: "center", flexWrap: "wrap", fontSize: "0.8rem", color: "var(--text-secondary)" }}>
          <span>Seriennummer (Iru): {device.serial_number ?? <span style={{ color: "var(--text-muted)" }}>—</span>}</span>
          <span>Asset-Tag (Iru): {device.asset_tag ?? <span style={{ color: "var(--text-muted)" }}>—</span>}</span>
          {device.platform && <span>Plattform: {device.platform}</span>}
          {device.os_version && <span>OS-Version: {device.os_version}</span>}
        </div>
        {busy && <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Lade Iru-Details…</p>}
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
                    <th style={thStyle}>Iru (extern)</th>
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
                        externalValue: findExternalValue(data, IP_KEYS),
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

  function renderConnectionRow(connection: IruConnectionDto) {
    const removing = removeBusy[connection.id] ?? false;

    return (
      <div key={connection.id} style={connectionRowStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
          <div>
            <div style={{ fontWeight: 600 }}>{connection.label}</div>
            <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>{customerLabel(connection.customer_id)}</div>
            <div style={{ fontSize: "0.78rem", color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>{connection.base_url}</div>
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

  function renderDeviceModal(connection: IruConnectionDto) {
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
            <p style={mutedStyle}>Noch nicht synchronisiert. Auf "Aktualisieren" klicken, um Geräte von Iru zu holen.</p>
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
          {cached && allDevices.length > 0 && navItems.length === 0 && <p style={mutedStyle}>Keine Geräte gefunden.</p>}

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
          <h2 style={{ margin: 0, fontSize: "1rem" }}>Iru-Verbindungen</h2>
          <button type="button" onClick={() => setAddFormOpen(true)}>
            + Neue Verbindung
          </button>
        </div>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Liest ausschließlich Geräte aus Iru (Apple-MDM, früher "Kandji") — es werden nie Daten dorthin
          zurückgeschrieben. Iru kennt keine Organisationen: jede Verbindung gehört direkt zu genau einem Kunden.
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
            <h2 style={{ margin: 0, fontSize: "1rem" }}>Neue Iru-Verbindung</h2>
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
              <input value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="ACME Iru" required autoFocus />
            </label>
            <label style={labelStyle}>
              Base-URL
              <input
                value={newBaseUrl}
                onChange={(e) => {
                  setNewBaseUrl(e.target.value);
                  setTestResult(null);
                }}
                placeholder="https://acme.api.kandji.io"
                required
              />
            </label>
            <label style={labelStyle}>
              API-Token
              <input
                type="password"
                value={newToken}
                onChange={(e) => {
                  setNewToken(e.target.value);
                  setTestResult(null);
                }}
                required
              />
            </label>
            {testResult && (testResult.ok ? <SuccessText>{testResult.message}</SuccessText> : <ErrorText>{testResult.message}</ErrorText>)}
            {addStatus && <SuccessText>{addStatus}</SuccessText>}
            {addError && <ErrorText>{addError}</ErrorText>}
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "space-between", marginTop: "0.25rem" }}>
              <button type="button" disabled={testBusy || !newBaseUrl || !newToken} onClick={() => void handleTestConnection()}>
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
