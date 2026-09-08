import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import Modal from "./Modal";

// Ninja (NinjaOne RMM) plugin settings screen. We only ever READ devices from
// Ninja — see docs/PLUGIN_ARCHITECTURE.md: external data is always shown as
// supplementary, read-only information next to a System's own maintained
// fields (name/hostname/ip_address/notes), and copying a value across is
// always a deliberate, manual, per-field action — never an automatic sync
// step. That rule shapes the whole "Details/Aktualisieren" panel below.
//
// A connection is NOT tied to a single Kunde. NinjaOne models
// "Organizations" inside one tenant, and a single connection (one set of
// API credentials) can see multiple Organizations — including an MSP Kunde
// whose own sub-customers are modeled as separate Ninja Organizations. So
// each connection's Organizations are individually mapped to a local Kunde,
// and devices are grouped/shown per-Organization, gated on that
// Organization being mapped.
//
// Devices are no longer fetched live every time this view opens — see
// get_cached_ninja_sync (offline, reads the last known sync) vs.
// sync_ninja_connection (live, also refreshes the cache server-side). The
// cached sync result is also the single source of truth for Organizations
// (organization_id/organization_name/customer_id come straight off each
// cached group), so there's no separate live "list organizations" call
// anymore — opening the overlay below without ever syncing simply shows a
// "Noch nicht synchronisiert" prompt instead of an org list with nothing to
// map yet.
//
// UI structure: the connections list stays inline on the page (identity +
// a couple of buttons per connection, kept short on purpose). Everything
// else — organizations, Kunde-mapping, and the linked/unlinked device lists
// — lives behind a per-connection "Organisationen & Geräte" button that
// opens a Modal overlay, tracked via local `openConnectionId` state (this is
// page-local UI state, not global store state — it only ever opens from a
// button on this same page, unlike the Zustand-driven entity editors
// elsewhere that need to be reachable from multiple places). Likewise
// "+ Neue Verbindung" opens the add-connection form in its own Modal instead
// of it sitting permanently inline.

interface NinjaConnectionDto {
  id: string;
  label: string;
  base_url: string;
}

interface ExternalSystemDto {
  external_id: string;
  name: string;
  hostname: string | null;
  ip_address: string | null;
  ninja_url: string;
  linked_system_id: number | null;
}

interface NinjaOrgDeviceGroupDto {
  organization_id: string;
  organization_name: string;
  customer_id: number | null;
  devices: ExternalSystemDto[];
}

interface CachedNinjaSyncDto {
  synced_at_utc: string;
  groups: NinjaOrgDeviceGroupDto[];
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

const NAME_KEYS = ["displayName", "display_name", "systemName", "system_name", "name"];
const HOSTNAME_KEYS = ["hostname", "host_name", "dnsName", "dns_name"];
const IP_KEYS = ["ipAddress", "ip_address", "ip", "ipv4Address", "ipv4", "primaryIp", "publicIp", "publicIP"];

// Synthetic <option> value for "+ Neuen Kunden anlegen…" inside an
// organization's Kunde-mapping <select>.
const CREATE_NEW_CUSTOMER_VALUE = "__create_new__";

// Heuristic scan of a get_ninja_system_details() payload: the exact shape of
// NinjaOne's response isn't part of the fixed contract (it's just
// Record<string, unknown>), so we look for top-level, non-object fields whose
// key matches one of a handful of common hostname/name/ip-shaped key names.
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

// There's no natural "which timezone" answer for a machine-generated
// synced_at_utc timestamp, so we just use the viewer's own local zone to
// render it via the existing format_timestamp_for_display command.
function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return "UTC";
  }
}

// Devices come back from the Ninja API in whatever order the API returns
// them in — sort alphabetically by name (German collation, e.g. "ä" sorts
// near "a") for display, independent of the text filter.
function sortByName<T extends { name: string }>(items: T[]): T[] {
  return items.slice().sort((a, b) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
}

// How many devices an expanded organization shows per page.
const DEVICE_PAGE_SIZE = 10;

// Slices `items` (already filtered/sorted) to one page of DEVICE_PAGE_SIZE,
// clamping a possibly-stale stored page number into range — e.g. after the
// text filter narrows the list, or after a resync shrinks/grows it — so
// callers never need to worry about "page 4 of 1" themselves; they just
// always render/act on `pageItems` and display `clampedPage`. Shared by
// renderDeviceGroup (mouse) and the keydown handler (keyboard) so both
// agree on what "the current page" contains.
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

const rowStyle: CSSProperties = {
  padding: "0.5rem 0",
  borderTop: "1px solid var(--border-subtle)",
  display: "flex",
  flexDirection: "column",
  gap: "0.3rem",
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

function ErrorText({ children }: { children: React.ReactNode }) {
  return <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {children}</p>;
}

function SuccessText({ children }: { children: React.ReactNode }) {
  return <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{children}</p>;
}

// The opener plugin's JS API (`@tauri-apps/plugin-opener`) is not installed
// in this project (checked package.json/package-lock.json — it isn't a
// dependency, only @tauri-apps/api and @tauri-apps/plugin-dialog are), even
// though the opener plugin IS a Tauri-side (Rust) dependency and the "main"
// window capability already grants it "opener:default" (which does include
// allow-open-url with no pre-configured scope, per the crate's own
// permissions/default.toml). Since importing an uninstalled package would
// break `npm run build`, and this file may only touch PluginsView.tsx, we
// fall back to a plain external link, which Tauri's webview opens via the
// OS default handler.
function NinjaLink({ url }: { url: string }) {
  return (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      // Out of the Tab order — see the device-row keydown handler's comment
      // for why: a focused <a> intercepts Enter as "follow this link" before
      // our own row-level Enter/l/u handling ever gets a look-in.
      tabIndex={-1}
      style={{ fontSize: "0.78rem", color: "var(--accent)", whiteSpace: "nowrap" }}
    >
      In Ninja öffnen ↗
    </a>
  );
}

// Small inline key-binding badge, styled like CommandPalette.tsx's Row hint
// span, so the j/k/Enter/l/u bindings on device rows are discoverable rather
// than hidden knowledge.
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

// "◀ Zurück" / "Seite X von Y" / "Weiter ▶" — rendered under an expanded
// organization's device list. Hides itself entirely when there's only one
// page (e.g. the filter narrowed things down to a handful of matches), per
// the "collapse or show 'Seite 1 von 1' sensibly" ask — hiding is the
// cleaner of the two for this compact layout.
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
      <NinjaLink url={device.ninja_url} />
    </span>
  );
}

export default function NinjaPluginSection() {
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);

  const [connections, setConnections] = useState<NinjaConnectionDto[]>([]);
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [connectionsError, setConnectionsError] = useState<string | null>(null);

  // Add-connection modal.
  const [addFormOpen, setAddFormOpen] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [newBaseUrl, setNewBaseUrl] = useState("");
  const [newClientId, setNewClientId] = useState("");
  const [newClientSecret, setNewClientSecret] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addStatus, setAddStatus] = useState<string | null>(null);
  const [addError, setAddError] = useState<string | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  // Which connection's "Organisationen & Geräte" overlay is open. Page-local
  // UI state — this modal only ever opens from a button on this same page,
  // so it doesn't need to live in the global store.
  const [openConnectionId, setOpenConnectionId] = useState<string | null>(null);

  // Which organization's Kunde-mapping select is waiting on a customer being
  // created via the "+ Neuen Kunden anlegen…" option — keyed the same way as
  // everything else here (`${connectionId}:${organizationId}`). Set right
  // before opening the globally-mounted CustomerForm modal; consumed by the
  // customerEditorTarget-transition-to-null effect below, which auto-maps
  // the freshly-created customer to this organization once the editor
  // closes, so the user doesn't have to manually refresh + re-select it.
  const [pendingCustomerCreationGroupKey, setPendingCustomerCreationGroupKey] = useState<string | null>(null);

  // Org-mapping busy/error, keyed by `${connectionId}:${organizationId}`.
  const [orgMapBusy, setOrgMapBusy] = useState<Record<string, boolean>>({});
  const [orgMapError, setOrgMapError] = useState<Record<string, string | null>>({});

  // Cached (offline) device sync per connection.
  const [cachedSyncByConnection, setCachedSyncByConnection] = useState<Record<string, CachedNinjaSyncDto | null>>({});
  const [cacheLoaded, setCacheLoaded] = useState<Record<string, boolean>>({});
  const [cacheBusy, setCacheBusy] = useState<Record<string, boolean>>({});
  const [cacheError, setCacheError] = useState<Record<string, string | null>>({});
  const [syncedAtDisplay, setSyncedAtDisplay] = useState<Record<string, string>>({});

  // Live "Aktualisieren" per connection.
  const [syncBusy, setSyncBusy] = useState<Record<string, boolean>>({});
  const [syncStatus, setSyncStatus] = useState<Record<string, string | null>>({});
  const [syncError, setSyncError] = useState<Record<string, string | null>>({});

  const [removeBusy, setRemoveBusy] = useState<Record<string, boolean>>({});

  // Device text filter, keyed by `${connectionId}:${organizationId}`.
  const [deviceFilter, setDeviceFilter] = useState<Record<string, string>>({});

  // Which organizations are expanded — collapsed by default (an
  // organization renders as just a header row: name, Kunde-mapping select,
  // and a device-count summary, per <PLUGIN_ARCHITECTURE-adjacent UX ask>
  // "only show the organization, then I can expand it"). Keyed by
  // `${connectionId}:${organizationId}`, same as everything else below.
  // Deliberately NOT an accordion: multiple organizations can be expanded
  // at once. Nothing else in this modal assumes only one org's devices are
  // visible at a time (the keyboard nav below already scopes itself to a
  // single "active" group regardless), so forcing a single-open accordion
  // would only cost the user clicks with no corresponding benefit.
  const [expandedGroupKeys, setExpandedGroupKeys] = useState<Set<string>>(new Set());

  // Device-list pagination, 10 devices/page, keyed the same way
  // (`${connectionId}:${organizationId}`); 0-based, so 0 means "Seite 1".
  // Paginates over the exact same filtered/sorted list the keyboard nav
  // below walks (see computeGroupDevices + paginateDevices), so "current
  // page" means the same thing to the mouse and to j/k/Enter/l/u.
  const [pageByGroup, setPageByGroup] = useState<Record<string, number>>({});

  // Keyboard navigation over the currently-open connection's device lists —
  // mirrors the j/k/Enter pattern used by CustomerListView.tsx/
  // SystemListView.tsx/JournalView.tsx. Each organization's "Nicht
  // verknüpft" + "Bereits verknüpft" devices are treated as ONE flat,
  // ordered list (unlinked first, then linked — see computeGroupDevices),
  // and `activeGroupKey` (`${connectionId}:${organizationId}`) says which
  // organization's list currently owns the keyboard highlight — only one
  // group is keyboard-active at a time; switching groups happens by
  // focusing that group's filter input, hovering one of its rows, or
  // expanding it via its header.
  const [activeGroupKey, setActiveGroupKey] = useState<string | null>(null);
  const [selectedIndexByGroup, setSelectedIndexByGroup] = useState<Record<string, number>>({});

  // Auto-focus the first group's device filter input when a connection's
  // overlay first has data to show, so the user can start typing to narrow
  // the list immediately (see the effect below). `autoFocusedConnectionRef`
  // guards against re-stealing focus on every subsequent re-render/refresh.
  const firstFilterInputRef = useRef<HTMLInputElement | null>(null);
  const autoFocusedConnectionRef = useRef<string | null>(null);

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
    invoke<NinjaConnectionDto[]>("list_ninja_connections")
      .then(setConnections)
      .catch((e) => setConnectionsError(String(e)));
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

  // CustomerForm is globally mounted and driven by the store (see
  // CustomerListView.tsx for the same pattern) — this component doesn't get
  // an onDone callback from it, so it detects "the editor just closed" by
  // watching customerEditorTarget go from non-null to null. When that
  // happens AND a "+ Neuen Kunden anlegen…" selection left
  // pendingCustomerCreationGroupKey set, re-fetch the customer list, diff it
  // against what was in state right before the fetch to find the
  // newly-created customer, and — if exactly one appeared (the user actually
  // saved, and no concurrent creation elsewhere muddies which one is "the"
  // new one) — auto-map it to the pending organization via the same
  // handleOrgMappingChange path the manual dropdown uses. If the user
  // cancelled (no new customer) or more than one new customer showed up, we
  // just refresh the list and leave the selection to the user, per the same
  // "never guess" principle used throughout PLUGIN_ARCHITECTURE.md.
  const prevCustomerEditorTargetRef = useRef(customerEditorTarget);
  useEffect(() => {
    const groupKey = pendingCustomerCreationGroupKey;
    if (prevCustomerEditorTargetRef.current !== null && customerEditorTarget === null && groupKey) {
      const previousCustomers = customers;
      void (async () => {
        try {
          const freshList = await invoke<Customer[]>("list_customers", { includeArchived: false });
          setCustomers(freshList);
          const previousIds = new Set(previousCustomers.map((c) => c.id));
          const newlyCreated = freshList.filter((c) => !previousIds.has(c.id));
          if (newlyCreated.length === 1) {
            const sepIdx = groupKey.indexOf(":");
            const connectionId = groupKey.slice(0, sepIdx);
            const organizationId = groupKey.slice(sepIdx + 1);
            const connection = connections.find((c) => c.id === connectionId);
            const group = cachedSyncByConnection[connectionId]?.groups.find((g) => g.organization_id === organizationId);
            if (connection && group) {
              await handleOrgMappingChange(connection, group, String(newlyCreated[0].id));
            }
          }
        } finally {
          setPendingCustomerCreationGroupKey(null);
        }
      })();
    }
    prevCustomerEditorTargetRef.current = customerEditorTarget;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [customerEditorTarget]);

  async function refreshLocalSystems(customerId: number): Promise<System[]> {
    const list = await invoke<System[]>("list_systems", { customerId, includeArchived: false });
    setLocalSystemsByCustomer((prev) => ({ ...prev, [customerId]: list }));
    return list;
  }

  function getAdoptError(systemId: number): string | null {
    const hit = Object.entries(adoptError).find(([k, v]) => v !== null && k.startsWith(`${systemId}:`));
    return hit ? hit[1] : null;
  }

  async function updateSyncedAtDisplay(connectionId: string, utc: string) {
    try {
      const display = await invoke<string>("format_timestamp_for_display", { utc, tz: localTimeZone() });
      setSyncedAtDisplay((prev) => ({ ...prev, [connectionId]: display }));
    } catch {
      setSyncedAtDisplay((prev) => ({ ...prev, [connectionId]: utc }));
    }
  }

  const loadCachedSync = useCallback(async (connection: NinjaConnectionDto) => {
    const id = connection.id;
    setCacheBusy((prev) => ({ ...prev, [id]: true }));
    setCacheError((prev) => ({ ...prev, [id]: null }));
    try {
      const cached = await invoke<CachedNinjaSyncDto | null>("get_cached_ninja_sync", { connectionId: id });
      setCachedSyncByConnection((prev) => ({ ...prev, [id]: cached }));
      setCacheLoaded((prev) => ({ ...prev, [id]: true }));
      if (cached) {
        await updateSyncedAtDisplay(id, cached.synced_at_utc);
        const customerIds = Array.from(
          new Set(cached.groups.map((g) => g.customer_id).filter((c): c is number => c !== null)),
        );
        await Promise.all(customerIds.map((cid) => refreshLocalSystems(cid).catch(() => {})));
      }
    } catch (err) {
      setCacheError((prev) => ({ ...prev, [id]: String(err) }));
    } finally {
      setCacheBusy((prev) => ({ ...prev, [id]: false }));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function openOrgDeviceModal(connection: NinjaConnectionDto) {
    setOpenConnectionId(connection.id);
    setActiveGroupKey(null);
    setSelectedIndexByGroup({});
    autoFocusedConnectionRef.current = null;
    void loadCachedSync(connection);
  }

  function closeOrgDeviceModal() {
    setOpenConnectionId(null);
  }

  async function handleTestConnection() {
    setTestBusy(true);
    setTestResult(null);
    try {
      await invoke("test_ninja_connection", {
        baseUrl: newBaseUrl,
        clientId: newClientId,
        clientSecret: newClientSecret,
      });
      setTestResult({ ok: true, message: "✓ Verbindung erfolgreich" });
    } catch (err) {
      setTestResult({ ok: false, message: String(err) });
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
      await invoke("add_ninja_connection", {
        label: newLabel,
        baseUrl: newBaseUrl,
        clientId: newClientId,
        clientSecret: newClientSecret,
      });
      setAddStatus(`Verbindung "${newLabel}" hinzugefügt.`);
      setNewLabel("");
      setNewBaseUrl("");
      setNewClientId("");
      setNewClientSecret("");
      setTestResult(null);
      reloadConnections();
      setAddFormOpen(false);
    } catch (err) {
      setAddError(String(err));
    } finally {
      setAddBusy(false);
    }
  }

  async function handleRemoveConnection(connection: NinjaConnectionDto) {
    const confirmed = window.confirm(`Ninja-Verbindung "${connection.label}" wirklich entfernen?`);
    if (!confirmed) return;
    const id = connection.id;
    setRemoveBusy((prev) => ({ ...prev, [id]: true }));
    setConnectionsError(null);
    try {
      await invoke("remove_ninja_connection", { id });
      if (openConnectionId === id) setOpenConnectionId(null);
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

  async function handleSync(connection: NinjaConnectionDto) {
    const id = connection.id;
    setSyncBusy((prev) => ({ ...prev, [id]: true }));
    setSyncStatus((prev) => ({ ...prev, [id]: null }));
    setSyncError((prev) => ({ ...prev, [id]: null }));
    try {
      const groups = await invoke<NinjaOrgDeviceGroupDto[]>("sync_ninja_connection", { connectionId: id });
      const deviceCount = groups.reduce((sum, g) => sum + g.devices.length, 0);
      setSyncStatus((prev) => ({
        ...prev,
        [id]: `${groups.length} Organisation(en), ${deviceCount} Gerät(e) gefunden.`,
      }));
      // Prefer re-reading the cache afterward (authoritative, server-written
      // synced_at_utc) over building the timestamp from the client clock.
      await loadCachedSync(connection);
      // The device list just changed underneath any expanded organization —
      // stale page numbers after a resync would be confusing (e.g. sitting
      // on "page 3" of a list that shrank to one page), so reset every one
      // of this connection's organizations back to page 1.
      setPageByGroup((prev) => {
        const next = { ...prev };
        for (const key of Object.keys(next)) {
          if (key.startsWith(`${id}:`)) next[key] = 0;
        }
        return next;
      });
    } catch (err) {
      setSyncError((prev) => ({ ...prev, [id]: String(err) }));
    } finally {
      setSyncBusy((prev) => ({ ...prev, [id]: false }));
    }
  }

  async function handleOrgMappingChange(connection: NinjaConnectionDto, group: NinjaOrgDeviceGroupDto, rawValue: string) {
    const mapKey = `${connection.id}:${group.organization_id}`;
    setOrgMapBusy((prev) => ({ ...prev, [mapKey]: true }));
    setOrgMapError((prev) => ({ ...prev, [mapKey]: null }));
    try {
      if (rawValue === "") {
        await invoke("unmap_ninja_organization", { connectionId: connection.id, organizationId: group.organization_id });
      } else {
        const customerId = Number(rawValue);
        await invoke("map_ninja_organization", {
          connectionId: connection.id,
          organizationId: group.organization_id,
          organizationName: group.organization_name,
          customerId,
        });
        void refreshLocalSystems(customerId);
      }
      // Optimistic local update so the select reflects the new mapping
      // immediately, before the authoritative cache re-read below lands.
      setCachedSyncByConnection((prev) => {
        const current = prev[connection.id];
        if (!current) return prev;
        return {
          ...prev,
          [connection.id]: {
            ...current,
            groups: current.groups.map((g) =>
              g.organization_id === group.organization_id
                ? { ...g, customer_id: rawValue === "" ? null : Number(rawValue) }
                : g,
            ),
          },
        };
      });
      // Re-read the cache (no network) so already-cached device groups pick
      // up the new/removed mapping without requiring a live re-sync.
      await loadCachedSync(connection);
    } catch (err) {
      setOrgMapError((prev) => ({ ...prev, [mapKey]: String(err) }));
    } finally {
      setOrgMapBusy((prev) => ({ ...prev, [mapKey]: false }));
    }
  }

  function openLinkPicker(connection: NinjaConnectionDto, group: NinjaOrgDeviceGroupDto, device: ExternalSystemDto) {
    if (group.customer_id === null) return;
    const customerId = group.customer_id;
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

  async function confirmLink(connection: NinjaConnectionDto, device: ExternalSystemDto) {
    if (linkPickerSelection === "") return;
    const key = `${connection.id}:${device.external_id}`;
    setLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("link_system_to_ninja", {
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

  async function createAndLink(connection: NinjaConnectionDto, group: NinjaOrgDeviceGroupDto, device: ExternalSystemDto) {
    if (group.customer_id === null) return;
    const customerId = group.customer_id;
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
      await invoke("link_system_to_ninja", {
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

  async function handleUnlink(connection: NinjaConnectionDto, device: ExternalSystemDto) {
    if (device.linked_system_id === null) return;
    const key = `${connection.id}:${device.external_id}`;
    setUnlinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("unlink_system_from_ninja", { systemId: device.linked_system_id, connectionId: connection.id });
      if (detailsOpenKey === key) setDetailsOpenKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: String(err) }));
    } finally {
      setUnlinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function toggleDetails(connection: NinjaConnectionDto, group: NinjaOrgDeviceGroupDto, device: ExternalSystemDto) {
    const key = `${connection.id}:${device.external_id}`;
    if (detailsOpenKey === key) {
      setDetailsOpenKey(null);
      return;
    }
    setDetailsOpenKey(key);
    setDetailsError((prev) => ({ ...prev, [key]: null }));
    setDetailsBusy((prev) => ({ ...prev, [key]: true }));
    try {
      const detailsPromise = invoke<Record<string, unknown>>("get_ninja_system_details", {
        connectionId: connection.id,
        externalId: device.external_id,
      });
      const refreshPromise = group.customer_id !== null ? refreshLocalSystems(group.customer_id) : Promise.resolve();
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

  // Combines an organization's filtered "unlinked" and "linked" devices into
  // one flat, keyboard-navigable list (unlinked first, matching visual
  // order) — shared by renderDeviceGroup (for rendering) and the global
  // keydown handler below (for acting on whichever row is highlighted), so
  // the two can never disagree about ordering/filtering.
  function computeGroupDevices(connectionId: string, group: NinjaOrgDeviceGroupDto) {
    const groupKey = `${connectionId}:${group.organization_id}`;
    const filterText = deviceFilter[groupKey] ?? "";
    const q = filterText.trim().toLowerCase();
    const sortedDevices = sortByName(group.devices);
    const filteredDevices =
      q === ""
        ? sortedDevices
        : sortedDevices.filter(
            (d) => d.name.toLowerCase().includes(q) || (d.hostname ?? "").toLowerCase().includes(q),
          );
    const isMapped = group.customer_id !== null;
    const linked = isMapped ? filteredDevices.filter((d) => d.linked_system_id !== null) : [];
    const unlinked = isMapped ? filteredDevices.filter((d) => d.linked_system_id === null) : [];
    const navItems: { device: ExternalSystemDto; kind: "unlinked" | "linked" }[] = [
      ...unlinked.map((device) => ({ device, kind: "unlinked" as const })),
      ...linked.map((device) => ({ device, kind: "linked" as const })),
    ];
    return { groupKey, filterText, filteredDevices, isMapped, linked, unlinked, navItems };
  }

  // Global j/k (+ ArrowDown/ArrowUp) / Enter / l / u handling for the
  // currently keyboard-active organization's device list — same
  // isTypingTarget-gated pattern as CustomerListView.tsx, so typing in the
  // filter input (or any other field) never fights with these single-letter
  // bindings. Only live while the org/device overlay is actually open.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (!openConnectionId || addFormOpen) return;
      if (isTypingTarget(document.activeElement)) return;
      const connection = connections.find((c) => c.id === openConnectionId);
      const cached = cachedSyncByConnection[openConnectionId];
      if (!connection || !cached) return;
      const activeGroup = activeGroupKey
        ? cached.groups.find((g) => `${openConnectionId}:${g.organization_id}` === activeGroupKey)
        : undefined;
      if (!activeGroup) return;
      const { groupKey, navItems } = computeGroupDevices(connection.id, activeGroup);
      // A collapsed organization shows no rows at all, so there's nothing
      // for j/k/Enter/l/u to act on — and critically, without this guard
      // Enter would silently create-and-link (or toggle details on) a
      // device the user can't even see.
      if (!expandedGroupKeys.has(groupKey)) return;
      if (navItems.length === 0) return;
      const rawPage = pageByGroup[groupKey] ?? 0;
      const { pageItems, totalPages, clampedPage } = paginateDevices(navItems, rawPage);
      if (pageItems.length === 0) return;
      const index = Math.min(selectedIndexByGroup[groupKey] ?? 0, pageItems.length - 1);

      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndexByGroup((prev) => ({ ...prev, [groupKey]: Math.min(index + 1, pageItems.length - 1) }));
      } else if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndexByGroup((prev) => ({ ...prev, [groupKey]: Math.max(index - 1, 0) }));
      } else if (e.key === "Enter") {
        const item = pageItems[index];
        if (item) {
          e.preventDefault();
          if (item.kind === "unlinked") {
            void createAndLink(connection, activeGroup, item.device);
          } else {
            void toggleDetails(connection, activeGroup, item.device);
          }
        }
      } else if (e.key === "l") {
        const item = pageItems[index];
        if (item && item.kind === "unlinked") {
          e.preventDefault();
          openLinkPicker(connection, activeGroup, item.device);
        }
      } else if (e.key === "u") {
        const item = pageItems[index];
        if (item && item.kind === "linked") {
          e.preventDefault();
          void handleUnlink(connection, item.device);
        }
      } else if (e.key === "ArrowRight") {
        // Bonus: page forward without reaching for the mouse. Not one of
        // the required bindings — j/k/Enter/l/u still clamp at the page
        // edge as before — but ArrowLeft/ArrowRight were unused, so paging
        // this way costs nothing.
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
    cachedSyncByConnection,
    activeGroupKey,
    selectedIndexByGroup,
    deviceFilter,
    expandedGroupKeys,
    pageByGroup,
  ]);

  // Default/track which group is keyboard-active: keep the current one if
  // it's still present, otherwise fall back to the first group that has any
  // devices at all (mirrors which group's filter input gets auto-focused
  // below).
  useEffect(() => {
    if (!openConnectionId) return;
    const cached = cachedSyncByConnection[openConnectionId];
    if (!cached) return;
    setActiveGroupKey((prev) => {
      if (prev && cached.groups.some((g) => `${openConnectionId}:${g.organization_id}` === prev)) return prev;
      const firstWithDevices = cached.groups.find((g) => g.devices.length > 0);
      return firstWithDevices ? `${openConnectionId}:${firstWithDevices.organization_id}` : null;
    });
  }, [openConnectionId, cachedSyncByConnection]);

  // Auto-focus the first group's device filter input the first time this
  // connection's overlay actually has device data to show (an actual
  // .focus() call via a ref, since Modal.tsx's own initial-focus effect runs
  // on mount — before the async cache load resolves — and would otherwise
  // land on whatever's focusable first, e.g. the "Schließen" button).
  useEffect(() => {
    if (!openConnectionId) return;
    if (autoFocusedConnectionRef.current === openConnectionId) return;
    const cached = cachedSyncByConnection[openConnectionId];
    if (!cached || !cached.groups.some((g) => g.devices.length > 0)) return;
    if (firstFilterInputRef.current) {
      firstFilterInputRef.current.focus();
      autoFocusedConnectionRef.current = openConnectionId;
    }
  }, [openConnectionId, cachedSyncByConnection]);

  function renderDetailsPanel(group: NinjaOrgDeviceGroupDto, device: ExternalSystemDto, key: string) {
    const busy = detailsBusy[key] ?? false;
    const err = detailsError[key] ?? null;
    const data = detailsData[key];
    const localSystems = group.customer_id !== null ? localSystemsByCustomer[group.customer_id] ?? [] : [];
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
        <div style={{ display: "flex", gap: "0.75rem", alignItems: "center", fontSize: "0.8rem", color: "var(--text-secondary)" }}>
          <span>
            IP-Adresse (Ninja): {device.ip_address ?? <span style={{ color: "var(--text-muted)" }}>—</span>}
          </span>
          <NinjaLink url={device.ninja_url} />
        </div>
        {busy && <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Lade Ninja-Details…</p>}
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
                    <th style={thStyle}>Ninja (extern)</th>
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

  function renderDeviceGroup(connection: NinjaConnectionDto, group: NinjaOrgDeviceGroupDto, isFirstFilterable: boolean) {
    const { groupKey, filterText, filteredDevices, isMapped, linked, unlinked, navItems } = computeGroupDevices(
      connection.id,
      group,
    );
    const localSystems = group.customer_id !== null ? localSystemsByCustomer[group.customer_id] ?? [] : [];
    const mapKey = groupKey;
    const mapBusyFlag = orgMapBusy[mapKey] ?? false;
    const mapErr = orgMapError[mapKey] ?? null;
    const isGroupActive = activeGroupKey === groupKey;
    const isExpanded = expandedGroupKeys.has(groupKey);
    const totalDeviceCount = group.devices.length;
    const linkedDeviceCount = group.devices.filter((d) => d.linked_system_id !== null).length;

    // Paginate the exact list the keyboard handler walks (navItems, once
    // mapped) — or the flat filtered list for a not-yet-mapped organization,
    // which has no linked/unlinked split and isn't keyboard-navigable.
    const rawPage = pageByGroup[groupKey] ?? 0;
    let pageItems: { device: ExternalSystemDto; kind: "unlinked" | "linked" }[] = [];
    let unmappedPageItems: ExternalSystemDto[] = [];
    let totalPages = 1;
    let clampedPage = 0;
    if (isMapped) {
      const paged = paginateDevices(navItems, rawPage);
      pageItems = paged.pageItems;
      totalPages = paged.totalPages;
      clampedPage = paged.clampedPage;
    } else if (filteredDevices.length > 0) {
      const paged = paginateDevices(filteredDevices, rawPage);
      unmappedPageItems = paged.pageItems;
      totalPages = paged.totalPages;
      clampedPage = paged.clampedPage;
    }
    const rawSelectedIndex = selectedIndexByGroup[groupKey] ?? 0;
    const effectiveSelectedIndex = pageItems.length === 0 ? -1 : Math.min(rawSelectedIndex, pageItems.length - 1);

    // Split this page's combined unlinked+linked slice back into the two
    // visually-separated sections below, keeping each device's
    // page-relative index (0..pageItems.length-1) — the same index space
    // the keyboard handler uses, so hover/click and j/k/Enter/l/u agree on
    // "which row is highlighted".
    const pageUnlinked: { device: ExternalSystemDto; navIdx: number }[] = [];
    const pageLinked: { device: ExternalSystemDto; navIdx: number }[] = [];
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
        // Newly expanding: always start at the top, and make this the
        // keyboard-active group so j/k/Enter work immediately without an
        // extra click into the list.
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
        key={group.organization_id}
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
          <span style={{ fontWeight: 600, fontSize: "0.9rem" }}>{group.organization_name}</span>
          <span style={{ fontSize: "0.78rem", color: "var(--text-secondary)" }}>
            {totalDeviceCount} Gerät{totalDeviceCount === 1 ? "" : "e"}
            {isMapped && `, ${linkedDeviceCount} verknüpft`}
          </span>
        </div>

        <div style={{ display: "flex", gap: "0.4rem", alignItems: "center", flexWrap: "wrap" }}>
          <span style={sectionLabelStyle}>Kunde</span>
          <select
            value={group.customer_id ?? ""}
            disabled={mapBusyFlag}
            onChange={(e) => {
              const raw = e.target.value;
              if (raw === CREATE_NEW_CUSTOMER_VALUE) {
                // Reset the DOM value immediately so the select doesn't
                // visually stick on the placeholder option — its React
                // `value` prop won't change (the mapping itself didn't
                // change), so nothing else would force this back on its own.
                e.target.value = group.customer_id === null ? "" : String(group.customer_id);
                setPendingCustomerCreationGroupKey(groupKey);
                openCustomerEditor("new");
                return;
              }
              void handleOrgMappingChange(connection, group, raw);
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
          <button
            type="button"
            title="Kundenliste aktualisieren"
            onClick={() => refreshCustomers()}
            style={{ fontSize: "0.85rem", lineHeight: 1, padding: "0.2rem 0.4rem" }}
          >
            🔄
          </button>
          {mapBusyFlag && <span style={{ fontSize: "0.75rem", color: "var(--text-muted)" }}>speichere…</span>}
        </div>
        {mapErr && <ErrorText>{mapErr}</ErrorText>}

        {isExpanded && (
          <>
            {!isMapped && (
              <p style={mutedStyle}>
                Diese Organisation ist noch keinem Kunden zugeordnet — oben zuordnen, um Geräte zu verknüpfen.
              </p>
            )}
            {group.devices.length === 0 && <p style={mutedStyle}>Keine Geräte in dieser Organisation.</p>}
            {group.devices.length > 0 && (
              <input
                ref={isFirstFilterable ? firstFilterInputRef : undefined}
                value={filterText}
                onChange={(e) => {
                  const value = e.target.value;
                  setDeviceFilter((prev) => ({ ...prev, [groupKey]: value }));
                  // A filter that narrows 50 matches down to 3 shouldn't
                  // leave the user stranded on "page 4 of 1" — restart at
                  // the top whenever the search text itself changes.
                  setPageByGroup((prev) => ({ ...prev, [groupKey]: 0 }));
                }}
                onFocus={() => setActiveGroupKey(groupKey)}
                placeholder={`Geräte filtern (${group.devices.length})…`}
                style={{ maxWidth: "20rem" }}
              />
            )}

            {!isMapped && unmappedPageItems.length > 0 && (
              <>
                {unmappedPageItems.map((device) => (
                  <div key={device.external_id} style={rowStyle}>
                    <DeviceSummaryLine device={device} />
                  </div>
                ))}
                <PaginationBar
                  page={clampedPage}
                  totalPages={totalPages}
                  onPrev={() => goToPage(Math.max(0, clampedPage - 1))}
                  onNext={() => goToPage(Math.min(totalPages - 1, clampedPage + 1))}
                />
              </>
            )}

            {isMapped && (
              <>
                <div>
                  <div style={sectionLabelStyle}>Nicht verknüpft ({unlinked.length})</div>
                  {unlinked.length === 0 && <p style={mutedStyle}>Keine offenen Geräte.</p>}
                  {unlinked.length > 0 && pageUnlinked.length === 0 && (
                    <p style={mutedStyle}>Keine offenen Geräte auf dieser Seite.</p>
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
                              <button type="button" disabled={busyCreate} onClick={() => void createAndLink(connection, group, device)}>
                                {busyCreate ? "Lege an…" : "Neu anlegen"}
                              </button>
                              <KeyHint label="Enter" />
                              <button type="button" disabled={busyCreate} onClick={() => openLinkPicker(connection, group, device)}>
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
                                onClick={() => void confirmLink(connection, device)}
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
                  {linked.length === 0 && <p style={mutedStyle}>Keine verknüpften Geräte.</p>}
                  {linked.length > 0 && pageLinked.length === 0 && (
                    <p style={mutedStyle}>Keine verknüpften Geräte auf dieser Seite.</p>
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
                              <button type="button" onClick={() => void toggleDetails(connection, group, device)}>
                                {detailsIsOpen ? "Details ausblenden" : "Details/Aktualisieren"}
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
                          {detailsIsOpen && renderDetailsPanel(group, device, key)}
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

  function renderConnectionModal(connection: NinjaConnectionDto) {
    const id = connection.id;
    const busy = syncBusy[id] ?? false;
    const status = syncStatus[id] ?? null;
    const err = syncError[id] ?? null;
    const cached = cachedSyncByConnection[id] ?? null;
    const hasCacheLoaded = cacheLoaded[id] ?? false;
    const cacheBusyFlag = cacheBusy[id] ?? false;
    const cacheErr = cacheError[id] ?? null;
    const syncedDisplay = syncedAtDisplay[id];

    return (
      <Modal onClose={closeOrgDeviceModal}>
        <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem", minWidth: "32rem", maxWidth: "60rem", maxHeight: "80vh", overflowY: "auto" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: "0.5rem" }}>
            <div>
              <h2 style={{ margin: 0, fontSize: "1rem" }}>{connection.label}</h2>
              <div style={{ fontSize: "0.8rem", color: "var(--text-secondary)", fontFamily: "var(--font-mono)" }}>
                {connection.base_url}
              </div>
            </div>
            <button type="button" onClick={closeOrgDeviceModal}>
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
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: "0.5rem",
                alignItems: "flex-start",
                padding: "0.75rem",
                background: "var(--bg-elevated)",
                border: "1px solid var(--border-subtle)",
                borderRadius: "var(--radius-sm)",
              }}
            >
              <p style={{ margin: 0, fontSize: "0.85rem", color: "var(--text-secondary)" }}>
                Noch nicht synchronisiert. Auf "Aktualisieren" klicken, um Organisationen und Geräte von Ninja zu
                holen.
              </p>
              <button type="button" disabled={busy} onClick={() => void handleSync(connection)}>
                {busy ? "Aktualisiere…" : "Aktualisieren"}
              </button>
            </div>
          )}

          {cached && cached.groups.length === 0 && <p style={mutedStyle}>Keine Organisationen im Cache.</p>}
          {cached &&
            (() => {
              const firstFilterableOrgId = cached.groups.find((g) => g.devices.length > 0)?.organization_id ?? null;
              return cached.groups.map((group) =>
                renderDeviceGroup(connection, group, group.organization_id === firstFilterableOrgId),
              );
            })()}
        </div>
      </Modal>
    );
  }

  const openConnection = connections.find((c) => c.id === openConnectionId) ?? null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "52rem" }}>
      <section style={cardStyle}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
          <h2 style={{ margin: 0, fontSize: "1rem" }}>Ninja-Verbindungen</h2>
          <button type="button" onClick={() => setAddFormOpen(true)}>
            + Neue Verbindung
          </button>
        </div>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Liest ausschließlich Geräte aus NinjaOne (RMM) — es werden nie Daten dorthin zurückgeschrieben. Eine
          Verbindung kann mehrere Organisationen sehen; jede Organisation wird einzeln einem Kunden zugeordnet.
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
                  {connection.base_url}
                </div>
              </div>
              <div style={{ display: "flex", gap: "0.4rem", flexShrink: 0 }}>
                <button type="button" onClick={() => openOrgDeviceModal(connection)}>
                  Organisationen &amp; Geräte
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
            <h2 style={{ margin: 0, fontSize: "1rem" }}>Neue Ninja-Verbindung</h2>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Label
              <input value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="ACME Ninja" required autoFocus />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Base-URL
              <input
                value={newBaseUrl}
                onChange={(e) => {
                  setNewBaseUrl(e.target.value);
                  setTestResult(null);
                }}
                placeholder="https://app.ninjarmm.com"
                style={{ fontFamily: "var(--font-mono)" }}
                required
              />
              <span style={{ fontSize: "0.75rem", color: "var(--text-muted)" }}>
                Abhängig von der NinjaOne-Region/-Instanz — EU-, CA- und OC-Kunden verwenden einen anderen Host als US.
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
                disabled={testBusy || !newBaseUrl || !newClientId || !newClientSecret}
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
