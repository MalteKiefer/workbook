import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { formatInvokeError } from "../lib/errors";
import { getKeymap, matchesBinding } from "../lib/keymap";
import Modal from "./Modal";

// Vultr (cloud VPS/server hosting) plugin settings screen -- structurally
// closest to IntunePluginSection.tsx/LevelPluginSection.tsx: a Vultr
// connection (one API key) maps 1:1 to exactly one local customer
// (`customer_id` lives directly on the connection, see
// plugin::vultr::VultrConnectionMeta on the Rust side) -- a single Vultr API
// key sees exactly one account's instances, no further sub-tenant concept
// visible through it (Vultr's genuine "Sub-Accounts" feature means separate
// API keys, not something this connection could see through one key, see
// docs/PLUGIN_ARCHITECTURE.md "Vultr-Plugin"). No org-mapping UI, no
// per-organization customer assignment needed.
//
// Different in *purpose* from every RMM/MDM plugin here: this is server
// inventory (a customer's Vultr cloud instances), not device-management
// telemetry -- no group/site/organization concept to resolve at all (unlike
// Level's groups), and, like Intune, a genuinely flat, filterable, paginated
// list (10 per page), not grouped at all. The connection dialog
// authenticates with a single API key (like LevelPluginSection.tsx), not an
// OAuth2 client-credentials triple like Intune's.
//
// Page layout: the connections list stays inline (identity + two actions per
// row); both the add-connection form and the per-connection instance
// list/compare-and-adopt UI live in on-demand <Modal> overlays so the page
// itself stays short regardless of how many connections or instances exist.

interface VultrConnectionDto {
  id: string;
  customer_id: number;
  label: string;
}

interface ExternalSystemDto {
  external_id: string;
  name: string;
  ip_address: string | null;
  // See plugin::vultr::VultrInstance::ipv6_address on the Rust side -- the
  // empty-string-means-absent handling already happened server-side, this
  // is never an empty string here, only a real address or null.
  ipv6_address: string | null;
  // Vultr's power_status ("is it on"), not its own separate status field --
  // see docs/PLUGIN_ARCHITECTURE.md "Vultr-Plugin" for why.
  status: string | null;
  // Vultr's plan (instance size/flavor), used as a platform-equivalent
  // display string, like Datto RMM's deviceClass.
  platform: string | null;
  region: string | null;
  linked_system_id: number | null;
}

interface CachedVultrSyncDto {
  synced_at_utc: string;
  instances: ExternalSystemDto[];
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
  maintenance_interval_days: number | null;
  operating_system: string | null;
}

type CompareField = "name" | "hostname" | "ip_address";

// Vultr's instance object has no separate hostname field surfaced in
// ExternalSystemDto (the display-name fallback chain -- label, then
// hostname, then the external ID -- already folds it into `name`
// server-side, see plugin::vultr::map_instance). The compare panel below
// nonetheless still scans the raw get_vultr_system_details() payload for a
// literal "hostname"/"label" field (Vultr's real API response does carry
// both, even though this plugin's own DTO doesn't surface hostname
// separately) -- analogous to DattoRmmPluginSection.tsx's/
// TacticalRmmPluginSection.tsx's NAME_KEYS/HOSTNAME_KEYS/IP_KEYS heuristic
// scan.
const NAME_KEYS = ["label", "hostname"];
const HOSTNAME_KEYS = ["hostname"];
const IP_KEYS = ["main_ip", "ip_address", "ip"];

// Sentinel option value for the customer <select>'s "create a new customer"
// convenience entry -- never a real customer id, so it can't collide.
const CREATE_NEW_CUSTOMER = "__create_new__";

// Heuristic scan of a get_vultr_system_details() payload, analogous to
// LevelPluginSection.tsx's/IntunePluginSection.tsx's findExternalValue:
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

// Same reasoning as every other plugin section: no natural "which timezone"
// answer for a machine-generated synced_at_utc timestamp, so we use the
// viewer's own local zone to render it via format_timestamp_for_display.
function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return "UTC";
  }
}

// Instances come back from the Vultr API in API/pagination order, not
// alphabetical -- sort by name (German collation) for display, independent
// of the free-text filter above the list.
function sortInstancesByName(instances: ExternalSystemDto[]): ExternalSystemDto[] {
  return instances.slice().sort((a, b) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
}

// Flat pagination (no group layer -- Vultr has no group/site/organization
// concept, see the module doc comment above): 10 instances per page.
const PAGE_SIZE = 10;

// One connection's (filtered+sorted) instances as a single flat,
// keyboard-navigable list (unlinked first, then linked -- matching the two
// subsections' top-to-bottom visual order), analogous to
// IntunePluginSection.tsx's combineRows.
function combineRows(instances: ExternalSystemDto[]): { kind: "unlinked" | "linked"; instance: ExternalSystemDto }[] {
  const unlinked = instances.filter((i) => i.linked_system_id === null);
  const linked = instances.filter((i) => i.linked_system_id !== null);
  return [
    ...unlinked.map((instance) => ({ kind: "unlinked" as const, instance })),
    ...linked.map((instance) => ({ kind: "linked" as const, instance })),
  ];
}

interface PageInfo {
  totalPages: number;
  page: number;
  pageRows: { kind: "unlinked" | "linked"; instance: ExternalSystemDto }[];
}

// Slices a connection's combined rows down to the current 10-per-page
// window, clamping a stale/out-of-range page index (e.g. after the filter
// shrank the list) back into range rather than showing a blank page.
function getPageInfo(instances: ExternalSystemDto[], requestedPage: number): PageInfo {
  const combined = combineRows(instances);
  const totalPages = Math.max(1, Math.ceil(combined.length / PAGE_SIZE));
  const page = Math.min(Math.max(requestedPage, 0), totalPages - 1);
  const start = page * PAGE_SIZE;
  return { totalPages, page, pageRows: combined.slice(start, start + PAGE_SIZE) };
}

// One-time notes prefill for "Neu anlegen" (create-and-link), analogous to
// IntunePluginSection.tsx's buildInitialNotes: a local System has no
// dedicated field for Vultr-specific attributes like region or plan, so a
// short summary is written into `notes` once at creation time -- never
// overwritten afterward by a later sync.
function buildInitialNotes(instance: ExternalSystemDto): string {
  const parts: string[] = [];
  if (instance.platform) parts.push(`Plan: ${instance.platform}`);
  if (instance.region) parts.push(`Region: ${instance.region}`);
  if (instance.status) parts.push(`Status: ${instance.status}`);
  if (instance.ipv6_address) parts.push(`IPv6: ${instance.ipv6_address}`);
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

// Small inline keybinding hint, mirroring LevelPluginSection.tsx's/
// IntunePluginSection.tsx's HintBadge -- makes the row-level keyboard
// shortcuts (Enter/l/u) discoverable next to the mouse-clickable buttons
// they duplicate, rather than hidden knowledge.
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

function InstanceSummaryLine({ instance }: { instance: ExternalSystemDto }) {
  const detailBits = [instance.platform, instance.region].filter((v): v is string => Boolean(v)).join(" · ");
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
      <span>
        {instance.name}
        {detailBits && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "0.85em" }}>
            {" "}— {detailBits}
          </span>
        )}
        {instance.status && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-muted)", fontSize: "0.85em" }}>
            {" "}({instance.status})
          </span>
        )}
        {instance.ip_address && (
          <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-muted)", fontSize: "0.85em" }}>
            {" "}[{instance.ip_address}]
          </span>
        )}
      </span>
    </span>
  );
}

export default function VultrPluginSection() {
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);

  const [connections, setConnections] = useState<VultrConnectionDto[]>([]);
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
  // form's customer select -- tells the customerEditorTarget-closed effect
  // below (mirroring CustomerListView.tsx's own reload-on-close pattern)
  // that it should not just refresh the customer list but also try to
  // auto-select whichever customer the CustomerForm modal just created,
  // instead of leaving the user to find and pick it themselves after a
  // manual refresh.
  const [awaitingNewCustomer, setAwaitingNewCustomer] = useState(false);

  // Which connection's instance modal is open (page-local UI state -- this
  // modal only ever opens from a button on this same page, so unlike the
  // store-driven entity editors it doesn't need to live in the global
  // Zustand store).
  const [openConnectionId, setOpenConnectionId] = useState<string | null>(null);

  // Cached (offline) instance sync per connection.
  const [cachedSyncByConnection, setCachedSyncByConnection] = useState<Record<string, CachedVultrSyncDto | null>>({});
  const [cacheLoaded, setCacheLoaded] = useState<Record<string, boolean>>({});
  const [cacheBusy, setCacheBusy] = useState<Record<string, boolean>>({});
  const [cacheError, setCacheError] = useState<Record<string, string | null>>({});
  const [syncedAtDisplay, setSyncedAtDisplay] = useState<Record<string, string>>({});

  // "Aktualisieren" (live sync) per connection.
  const [syncBusy, setSyncBusy] = useState<Record<string, boolean>>({});
  const [syncStatus, setSyncStatus] = useState<Record<string, string | null>>({});
  const [syncError, setSyncError] = useState<Record<string, string | null>>({});

  const [removeBusy, setRemoveBusy] = useState<Record<string, boolean>>({});

  // Instance text filter, keyed by connection id -- one filter box per
  // connection (there's no group layer to narrow within, unlike Level).
  const [instanceFilter, setInstanceFilter] = useState<Record<string, string>>({});
  const instanceFilterInputRef = useRef<HTMLInputElement>(null);

  // Which page each connection's instance list is showing, and which row is
  // keyboard-highlighted on that page -- both keyed by connection id
  // directly (no group dimension, unlike LevelPluginSection.tsx's
  // fullGroupKey).
  const [instancePage, setInstancePage] = useState<Record<string, number>>({});
  const [selectedIndex, setSelectedIndex] = useState<Record<string, number>>({});

  // Local systems cache, keyed by customer id -- used both for the "link to
  // existing system" picker and for the compare/adopt panel.
  const [localSystemsByCustomer, setLocalSystemsByCustomer] = useState<Record<number, System[]>>({});

  // Per-instance state, keyed by `${connectionId}:${externalId}`.
  const [linkPickerKey, setLinkPickerKey] = useState<string | null>(null);
  const [linkPickerSelection, setLinkPickerSelection] = useState<number | "">("");
  const [linkBusy, setLinkBusy] = useState<Record<string, boolean>>({});
  const [createLinkBusy, setCreateLinkBusy] = useState<Record<string, boolean>>({});
  const [unlinkBusy, setUnlinkBusy] = useState<Record<string, boolean>>({});
  const [instanceError, setInstanceError] = useState<Record<string, string | null>>({});
  // Keyed by connection id directly (a connection maps 1:1 to one customer,
  // so there's no group/tenant-scoped key here) -- disables the "Alle
  // anlegen" bulk button for one connection while it works through that
  // connection's unlinked instances, independent of the per-instance
  // `createLinkBusy` map (both are set during a bulk run, so an instance's
  // own row also shows busy).
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
    invoke<VultrConnectionDto[]>("list_vultr_connections")
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
  // like LevelPluginSection.tsx/IntunePluginSection.tsx -- this component
  // learns the editor closed by watching customerEditorTarget transition
  // from non-null back to null, rather than via an onDone callback. Beyond
  // that shared reload, when the close follows the "+ Neuen Kunden
  // anlegen…" option (awaitingNewCustomer) it also diffs the
  // freshly-fetched list against whatever was in state just before the
  // fetch to find the newly-created customer and auto-select it in this
  // form's customer select.
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

  const loadCachedSync = useCallback(async (connection: VultrConnectionDto) => {
    const id = connection.id;
    setCacheBusy((prev) => ({ ...prev, [id]: true }));
    setCacheError((prev) => ({ ...prev, [id]: null }));
    try {
      const cached = await invoke<CachedVultrSyncDto | null>("get_cached_vultr_sync", { connectionId: id });
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

  function openInstanceModal(connection: VultrConnectionDto) {
    setOpenConnectionId(connection.id);
    setLinkPickerKey(null);
    setDetailsOpenKey(null);
    void loadCachedSync(connection);
  }

  function closeInstanceModal() {
    setOpenConnectionId(null);
    setLinkPickerKey(null);
    setDetailsOpenKey(null);
  }

  // Same filter+sort logic the instance modal renders with -- factored out
  // so the keyboard-navigation effect below can walk the exact same array
  // the user sees, in the exact same order.
  function getFilteredInstances(connectionId: string): ExternalSystemDto[] {
    const cached = cachedSyncByConnection[connectionId] ?? null;
    const filterText = instanceFilter[connectionId] ?? "";
    const q = filterText.trim().toLowerCase();
    const allInstances = cached?.instances ?? [];
    const filtered =
      q === ""
        ? allInstances
        : allInstances.filter(
            (i) =>
              i.name.toLowerCase().includes(q) ||
              (i.ip_address ?? "").toLowerCase().includes(q) ||
              (i.region ?? "").toLowerCase().includes(q),
          );
    return sortInstancesByName(filtered);
  }

  // Resets a connection's instance list back to page 1 -- used whenever the
  // filter text changes (a new filter invalidates whatever page was shown)
  // and after a resync (the instance set may have changed entirely).
  function resetPage(connectionId: string) {
    setInstancePage((prev) => ({ ...prev, [connectionId]: 0 }));
    setSelectedIndex((prev) => ({ ...prev, [connectionId]: 0 }));
  }

  function selectRow(connectionId: string, rowIndex: number) {
    setSelectedIndex((prev) => ({ ...prev, [connectionId]: rowIndex }));
  }

  function changePage(connectionId: string, nextPage: number) {
    setInstancePage((prev) => ({ ...prev, [connectionId]: Math.max(nextPage, 0) }));
    setSelectedIndex((prev) => ({ ...prev, [connectionId]: 0 }));
  }

  // Auto-focus the instance filter input once the modal is open and its
  // cache load has resolved (the input only renders once cached data
  // exists) -- lets the user start typing to narrow the list immediately.
  // Keyed on the *value* of cacheLoaded[id] rather than the cacheLoaded
  // object itself, so a later "Aktualisieren" re-sync (which also flips
  // that same flag to true) doesn't steal focus back a second time.
  const activeConnectionCacheLoaded = openConnectionId !== null ? (cacheLoaded[openConnectionId] ?? false) : false;
  useEffect(() => {
    if (!activeConnectionCacheLoaded) return;
    const id = window.setTimeout(() => instanceFilterInputRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, [openConnectionId, activeConnectionCacheLoaded]);

  // Keyboard navigation for the instance modal's list: j/k or ArrowDown/Up
  // move the highlight, Enter performs the primary action for the
  // highlighted row (create-and-link for an unlinked instance, open/toggle
  // the compare-and-adopt panel for a linked instance), l opens the "link to
  // existing system" picker for an unlinked row, u unlinks a linked row.
  // Gated by isTypingTarget exactly like every other plugin section, so
  // typing in the filter input (or the link-picker's <select>, which
  // autofocuses when opened) never triggers row actions.
  //
  // Operates on the open connection's *current page* of rows (pageRows),
  // never its full instance list -- with pagination, "the list the user
  // sees" really is just those <=10 rows. j/k clamp at that page's
  // boundaries rather than auto-advancing to the next/previous page,
  // matching every other plugin section's own clamp-don't-page-flip
  // behavior.
  useEffect(() => {
    if (openConnectionId === null) return;
    const found = connections.find((c) => c.id === openConnectionId);
    if (!found) return;
    const connection: VultrConnectionDto = found;
    const id = openConnectionId;

    function onKeyDown(e: KeyboardEvent) {
      if (isTypingTarget(document.activeElement)) return;
      if (linkPickerKey !== null) return; // let the inline link-picker sub-form own its own keys

      const instances = getFilteredInstances(id);
      const { pageRows } = getPageInfo(instances, instancePage[id] ?? 0);
      if (pageRows.length === 0) return;
      const idx = Math.min(selectedIndex[id] ?? 0, pageRows.length - 1);

      const keymap = getKeymap();
      if (matchesBinding(e, keymap.list_next) || e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((prev) => ({ ...prev, [id]: Math.min(idx + 1, pageRows.length - 1) }));
      } else if (matchesBinding(e, keymap.list_prev) || e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((prev) => ({ ...prev, [id]: Math.max(idx - 1, 0) }));
      } else if (e.key === "Enter") {
        const row = pageRows[idx];
        if (row) {
          e.preventDefault();
          if (row.kind === "unlinked") {
            void createAndLink(connection, row.instance);
          } else {
            void toggleDetails(connection, row.instance);
          }
        }
      } else if (e.key === "l") {
        const row = pageRows[idx];
        if (row && row.kind === "unlinked") {
          e.preventDefault();
          openLinkPicker(connection, row.instance);
        }
      } else if (e.key === "u") {
        const row = pageRows[idx];
        if (row && row.kind === "linked") {
          e.preventDefault();
          void handleUnlink(connection, row.instance);
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
    instancePage,
    selectedIndex,
    linkPickerKey,
    cachedSyncByConnection,
    instanceFilter,
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
      await invoke("test_vultr_connection", { apiKey: newApiKey });
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
      await invoke("add_vultr_connection", {
        customerId: newCustomerId,
        label: newLabel,
        apiKey: newApiKey,
      });
      reloadConnections();
      closeAddForm();
    } catch (err) {
      setAddError(formatInvokeError(err));
    } finally {
      setAddBusy(false);
    }
  }

  async function handleRemoveConnection(connection: VultrConnectionDto) {
    const confirmed = window.confirm(`Vultr-Verbindung "${connection.label}" wirklich entfernen?`);
    if (!confirmed) return;
    const id = connection.id;
    setRemoveBusy((prev) => ({ ...prev, [id]: true }));
    setConnectionsError(null);
    try {
      await invoke("remove_vultr_connection", { id });
      if (openConnectionId === id) closeInstanceModal();
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

  async function handleSync(connection: VultrConnectionDto) {
    const id = connection.id;
    setSyncBusy((prev) => ({ ...prev, [id]: true }));
    setSyncStatus((prev) => ({ ...prev, [id]: null }));
    setSyncError((prev) => ({ ...prev, [id]: null }));
    try {
      const instances = await invoke<ExternalSystemDto[]>("sync_vultr_connection", { connectionId: id });
      const count = Array.isArray(instances) ? instances.length : 0;
      setSyncStatus((prev) => ({ ...prev, [id]: `${count} Instanz(en) gefunden.` }));
      // The instance set (and therefore the page count) may have changed
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

  function openLinkPicker(connection: VultrConnectionDto, instance: ExternalSystemDto) {
    const customerId = connection.customer_id;
    const key = `${connection.id}:${instance.external_id}`;
    setLinkPickerKey(key);
    setInstanceError((prev) => ({ ...prev, [key]: null }));
    const localSystems = localSystemsByCustomer[customerId] ?? [];
    const instanceIp = instance.ip_address?.trim().toLowerCase() ?? "";
    const match =
      instanceIp !== "" ? localSystems.find((s) => s.ip_address.trim().toLowerCase() === instanceIp) : undefined;
    setLinkPickerSelection(match ? match.id : "");
    if (!localSystemsByCustomer[customerId]) {
      void refreshLocalSystems(customerId);
    }
  }

  async function confirmLink(connection: VultrConnectionDto, instance: ExternalSystemDto) {
    if (linkPickerSelection === "") return;
    const key = `${connection.id}:${instance.external_id}`;
    setLinkBusy((prev) => ({ ...prev, [key]: true }));
    setInstanceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("link_system_to_vultr", {
        systemId: linkPickerSelection,
        connectionId: connection.id,
        externalId: instance.external_id,
      });
      setLinkPickerKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setInstanceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setLinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function createAndLink(connection: VultrConnectionDto, instance: ExternalSystemDto) {
    const customerId = connection.customer_id;
    const key = `${connection.id}:${instance.external_id}`;
    setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
    setInstanceError((prev) => ({ ...prev, [key]: null }));
    try {
      const created = await invoke<System>("create_system", {
        input: {
          customer_id: customerId,
          name: instance.name,
          system_type: "",
          hostname: "",
          ip_address: instance.ip_address ?? "",
          notes: buildInitialNotes(instance),
        },
      });
      await invoke("link_system_to_vultr", {
        systemId: created.id,
        connectionId: connection.id,
        externalId: instance.external_id,
      });
      await refreshLocalSystems(customerId);
      await loadCachedSync(connection);
    } catch (err) {
      setInstanceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setCreateLinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  // Bulk version of `createAndLink`: works through every unlinked instance on
  // this connection sequentially. A failure on one instance does not abort
  // the rest -- it is recorded in the same `instanceError` map that already
  // surfaces per-instance errors, so a partial run still leaves the row's
  // own "Neu anlegen" button as the retry path. `refreshLocalSystems`/
  // `loadCachedSync` run once at the end, not per instance, so a connection
  // with many instances doesn't refetch the whole customer's system list N
  // times.
  async function createAndLinkAll(connection: VultrConnectionDto, instances: ExternalSystemDto[]) {
    if (instances.length === 0) return;
    const customerId = connection.customer_id;
    setBulkCreateBusy((prev) => ({ ...prev, [connection.id]: true }));
    try {
      for (const instance of instances) {
        const key = `${connection.id}:${instance.external_id}`;
        setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
        setInstanceError((prev) => ({ ...prev, [key]: null }));
        try {
          const created = await invoke<System>("create_system", {
            input: {
              customer_id: customerId,
              name: instance.name,
              system_type: "",
              hostname: "",
              ip_address: instance.ip_address ?? "",
              notes: buildInitialNotes(instance),
            },
          });
          await invoke("link_system_to_vultr", {
            systemId: created.id,
            connectionId: connection.id,
            externalId: instance.external_id,
          });
        } catch (err) {
          setInstanceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
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

  async function handleUnlink(connection: VultrConnectionDto, instance: ExternalSystemDto) {
    if (instance.linked_system_id === null) return;
    const key = `${connection.id}:${instance.external_id}`;
    setUnlinkBusy((prev) => ({ ...prev, [key]: true }));
    setInstanceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("unlink_system_from_vultr", { systemId: instance.linked_system_id, connectionId: connection.id });
      if (detailsOpenKey === key) setDetailsOpenKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setInstanceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setUnlinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function toggleDetails(connection: VultrConnectionDto, instance: ExternalSystemDto) {
    const key = `${connection.id}:${instance.external_id}`;
    if (detailsOpenKey === key) {
      setDetailsOpenKey(null);
      return;
    }
    setDetailsOpenKey(key);
    setDetailsError((prev) => ({ ...prev, [key]: null }));
    setDetailsBusy((prev) => ({ ...prev, [key]: true }));
    try {
      const detailsPromise = invoke<Record<string, unknown>>("get_vultr_system_details", {
        connectionId: connection.id,
        externalId: instance.external_id,
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

  function renderDetailsPanel(connection: VultrConnectionDto, instance: ExternalSystemDto, key: string) {
    const busy = detailsBusy[key] ?? false;
    const err = detailsError[key] ?? null;
    const data = detailsData[key];
    const localSystems = localSystemsByCustomer[connection.customer_id] ?? [];
    const localSystem =
      instance.linked_system_id !== null ? localSystems.find((s) => s.id === instance.linked_system_id) : undefined;

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
        {busy && <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Lade Vultr-Details…</p>}
        {err && <ErrorText>{err}</ErrorText>}
        {!busy && data && !localSystem && (
          <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--danger)" }}>
            Lokales System nicht gefunden (evtl. archiviert).
          </p>
        )}
        {!busy && data && localSystem && (
          <>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "0.3rem 1rem", fontSize: "0.8rem", color: "var(--text-secondary)" }}>
              {instance.status && <span>Status: {instance.status}</span>}
              {instance.platform && <span>Plan: {instance.platform}</span>}
              {instance.region && <span>Region: {instance.region}</span>}
              {instance.ipv6_address && <span>IPv6: {instance.ipv6_address}</span>}
            </div>
            <div style={{ overflowX: "auto" }}>
              <table style={{ borderCollapse: "collapse", fontSize: "0.82rem", width: "100%" }}>
                <thead>
                  <tr>
                    <th style={thStyle}>Feld</th>
                    <th style={thStyle}>Lokal</th>
                    <th style={thStyle}>Vultr (extern)</th>
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

  function renderConnectionRow(connection: VultrConnectionDto) {
    const removing = removeBusy[connection.id] ?? false;

    return (
      <div key={connection.id} style={connectionRowStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
          <div>
            <div style={{ fontWeight: 600 }}>{connection.label}</div>
            <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>{customerLabel(connection.customer_id)}</div>
          </div>
          <div style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
            <button type="button" onClick={() => openInstanceModal(connection)}>
              Instanzen anzeigen
            </button>
            <button type="button" disabled={removing} onClick={() => void handleRemoveConnection(connection)}>
              {removing ? "Entferne…" : "Entfernen"}
            </button>
          </div>
        </div>
      </div>
    );
  }

  function renderInstanceModal(connection: VultrConnectionDto) {
    const busy = syncBusy[connection.id] ?? false;
    const status = syncStatus[connection.id] ?? null;
    const err = syncError[connection.id] ?? null;
    const cached = cachedSyncByConnection[connection.id] ?? null;
    const hasCacheLoaded = cacheLoaded[connection.id] ?? false;
    const cacheBusyFlag = cacheBusy[connection.id] ?? false;
    const cacheErr = cacheError[connection.id] ?? null;
    const syncedDisplay = syncedAtDisplay[connection.id];

    const filterText = instanceFilter[connection.id] ?? "";
    const allInstances = cached?.instances ?? [];
    const filteredInstances = getFilteredInstances(connection.id);
    const localSystems = localSystemsByCustomer[connection.customer_id] ?? [];
    const { unlinked, linked } = {
      unlinked: filteredInstances.filter((i) => i.linked_system_id === null),
      linked: filteredInstances.filter((i) => i.linked_system_id !== null),
    };
    const { totalPages, page, pageRows } = getPageInfo(filteredInstances, instancePage[connection.id] ?? 0);
    const selectedIdx = selectedIndex[connection.id] ?? 0;
    const unlinkedOnPage = pageRows.filter((r) => r.kind === "unlinked").map((r) => r.instance);
    const linkedOnPage = pageRows.filter((r) => r.kind === "linked").map((r) => r.instance);

    return (
      <Modal onClose={closeInstanceModal}>
        <div style={{ display: "flex", flexDirection: "column", gap: "0.6rem", minWidth: "34rem", maxWidth: "48rem", maxHeight: "80vh", overflowY: "auto" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: "0.5rem" }}>
            <div>
              <h2 style={{ margin: 0, fontSize: "1rem" }}>{connection.label}</h2>
              <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)" }}>{customerLabel(connection.customer_id)}</div>
            </div>
            <button type="button" onClick={closeInstanceModal}>
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
            <p style={mutedStyle}>Noch nicht synchronisiert. Auf "Aktualisieren" klicken, um Instanzen von Vultr zu holen.</p>
          )}

          {cached && allInstances.length > 0 && (
            <input
              ref={instanceFilterInputRef}
              value={filterText}
              onChange={(e) => {
                setInstanceFilter((prev) => ({ ...prev, [connection.id]: e.target.value }));
                resetPage(connection.id);
              }}
              placeholder={`Instanzen filtern (${allInstances.length})…`}
              style={{ maxWidth: "20rem" }}
            />
          )}
          {cached && allInstances.length === 0 && <p style={mutedStyle}>Keine Instanzen im Cache.</p>}
          {cached && allInstances.length > 0 && filteredInstances.length === 0 && <p style={mutedStyle}>Keine Instanzen gefunden.</p>}

          {cached && filteredInstances.length > 0 && (
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
                  {unlinked.length === 0 && <p style={mutedStyle}>Keine offenen Instanzen.</p>}
                  {unlinkedOnPage.map((instance, i) => {
                    const rowKey = `${connection.id}:${instance.external_id}`;
                    const rowIndex = i;
                    const isSelected = rowIndex === selectedIdx;
                    const pickerOpen = linkPickerKey === rowKey;
                    const busyLink = linkBusy[rowKey] ?? false;
                    const busyCreate = createLinkBusy[rowKey] ?? false;
                    const rowErr = instanceError[rowKey] ?? null;
                    return (
                      <div key={instance.external_id}>
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
                          <InstanceSummaryLine instance={instance} />
                          <span style={{ display: "flex", alignItems: "center", gap: "0.6rem", flexShrink: 0 }}>
                            <span style={{ display: "inline-flex", alignItems: "center", gap: "0.3rem" }}>
                              <button
                                type="button"
                                disabled={busyCreate}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void createAndLink(connection, instance);
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
                                  openLinkPicker(connection, instance);
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
                              onClick={() => void confirmLink(connection, instance)}
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
                  {linked.length === 0 && <p style={mutedStyle}>Keine verknüpften Instanzen.</p>}
                  {linkedOnPage.map((instance, i) => {
                    const rowKey = `${connection.id}:${instance.external_id}`;
                    const rowIndex = unlinkedOnPage.length + i;
                    const isSelected = rowIndex === selectedIdx;
                    const detailsIsOpen = detailsOpenKey === rowKey;
                    const busyUnlink = unlinkBusy[rowKey] ?? false;
                    const rowErr = instanceError[rowKey] ?? null;
                    const localSystem = localSystems.find((s) => s.id === instance.linked_system_id);
                    return (
                      <div key={instance.external_id}>
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
                            <InstanceSummaryLine instance={instance} />
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
                                  void toggleDetails(connection, instance);
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
                                  void handleUnlink(connection, instance);
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
                        {detailsIsOpen && renderDetailsPanel(connection, instance, rowKey)}
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
          <h2 style={{ margin: 0, fontSize: "1rem" }}>Vultr-Verbindungen</h2>
          <button type="button" onClick={() => setAddFormOpen(true)}>
            + Neue Verbindung
          </button>
        </div>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Liest ausschließlich Server-Instanzen aus Vultr — es werden nie Daten dorthin zurückgeschrieben. Ein
          Vultr-API-Schlüssel entspricht direkt genau einem Kunden.
        </p>
        {connectionsError && <ErrorText>{connectionsError}</ErrorText>}
        {connections.length === 0 && (
          <p style={{ margin: 0, color: "var(--text-muted)", fontSize: "0.85rem" }}>Noch keine Verbindung eingerichtet.</p>
        )}
        {connections.map((connection) => renderConnectionRow(connection))}
      </section>

      {openConnection && renderInstanceModal(openConnection)}

      {addFormOpen && (
        <Modal onClose={closeAddForm}>
          <form onSubmit={(e) => void handleAddConnection(e)} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "22rem" }}>
            <h2 style={{ margin: 0, fontSize: "1rem" }}>Neue Vultr-Verbindung</h2>
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
              <input value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="ACME Vultr" required autoFocus />
            </label>
            <label style={labelStyle}>
              API-Schlüssel
              <input
                type="password"
                value={newApiKey}
                onChange={(e) => {
                  setNewApiKey(e.target.value);
                  setTestResult(null);
                }}
                placeholder="Account → API im Vultr-Kundenportal"
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
