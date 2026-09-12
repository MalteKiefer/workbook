import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { formatInvokeError } from "../lib/errors";
import { getKeymap, matchesBinding } from "../lib/keymap";
import Modal from "./Modal";

// Kaseya VSA (self-hosted/single-tenant RMM -- each customer runs their own
// VSA server) plugin settings screen -- ninth integration, mechanically
// closest to Tactical RMM (TacticalRmmPluginSection.tsx, see
// docs/PLUGIN_ARCHITECTURE.md "Tactical-RMM-Plugin"): connections list stays
// inline on the page, everything else (organizations, Kunde-mapping, and the
// linked/unlinked device lists) lives behind a per-connection "Organisationen
// & Geräte" overlay, cache-first (get_cached_kaseya_sync on open,
// sync_kaseya_connection only on "Aktualisieren"), collapsed-by-default
// groups, keyboard-navigable (j/k/Enter/l/u) filtered/10-per-page-paginated
// device list. Same read-only/manual-adopt rule as every other plugin
// section: external data is always shown as supplementary, read-only
// information next to a System's own maintained fields
// (name/hostname/ip_address/notes), and copying a value across is always a
// deliberate, manual, per-field action -- never an automatic sync step.
//
// A connection is NOT tied to a single Kunde, mirroring Ninja/Snipe-IT/
// Tactical RMM: a VSA instance models a real "Organization" tenancy layer,
// and a single connection (one Base-URL + Token ID/Secret) can see multiple
// Organizations -- a realistic MSP scenario is one shared VSA instance
// managing several customers' devices, each as its own Organization. So
// each connection's Organizations are individually mapped to a local Kunde,
// exactly like Tactical RMM's Clients/NinjaOne's Organizations.
//
// IMPORTANT, verified difference from Tactical RMM (see plugin::kaseya
// module docs and docs/PLUGIN_ARCHITECTURE.md "Kaseya-VSA-Plugin"): unlike
// Tactical RMM's agents (joined to their Client by NAME, no numeric ID at
// all), a Kaseya device carries a genuine OrganizationId foreign key -- the
// backend (commands::kaseya::group_devices_by_organization) already resolves
// the organization grouping via that ID before this component ever sees the
// data, so this file just consumes the resulting
// KaseyaOrgDeviceGroupDto[] the same way it would consume any ID-joined
// group. One thing IS special-cased backend-side and simply shows up here as
// an ordinary extra group: devices with NO organization id at all (Kaseya's
// own documentation doesn't fully guarantee every device object carries
// OrganizationId, see module docs) are bucketed under a synthetic
// organization named "Nicht zugeordnet" -- it behaves exactly like any other
// group here (can be mapped to a Kunde, expanded, filtered, etc.), no
// special-case rendering needed.
//
// Kaseya VSA's confirmed device fields are notably leaner than Tactical
// RMM's or NinjaOne's -- explicitly NO confirmed hostname or IP address
// field (see module docs on why those are honest omissions, not guesses),
// just Name, GroupId, IsAgentInstalled, IsMdmEnrolled. DeviceSummaryLine
// therefore shows the raw Group-ID (if present) and two small Ja/Nein
// badges for the install/MDM-enrollment flags instead of an IP/status/
// platform readout.
//
// There is deliberately no "open in Kaseya" link on a device row: no
// reachable VSA X documentation describes a stable, base-URL-derivable web
// dashboard link, exactly the same honest-omission principle as Tactical
// RMM's missing dashboard link.

interface KaseyaConnectionDto {
  id: string;
  label: string;
  base_url: string;
}

interface ExternalSystemDto {
  external_id: string;
  name: string;
  // NOT necessarily equal to the enclosing group's organization_id -- see
  // module docs on the "Nicht zugeordnet" bucket (there this is always
  // null). Informational only.
  organization_id: string | null;
  organization_name: string | null;
  group_id: string | null;
  is_agent_installed: boolean;
  is_mdm_enrolled: boolean;
  linked_system_id: number | null;
}

interface KaseyaOrgDeviceGroupDto {
  organization_id: string;
  organization_name: string;
  customer_id: number | null;
  devices: ExternalSystemDto[];
}

interface CachedKaseyaSyncDto {
  synced_at_utc: string;
  groups: KaseyaOrgDeviceGroupDto[];
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

// Kaseya's confirmed schema (see plugin::kaseya module docs) guarantees
// none of hostname/IP on a device -- get_kaseya_system_details' raw JSON is
// therefore scanned the same defensive, candidate-key way every other
// plugin section here does for genuinely uncertain shapes, not because any
// of these keys are confirmed to exist.
const NAME_KEYS = ["Name", "name"];
const HOSTNAME_KEYS = ["Hostname", "hostname", "ComputerName", "computer_name"];
const IP_KEYS = ["IpAddress", "ip_address", "IPAddress", "ip"];

// Synthetic <option> value for "+ Neuen Kunden anlegen…" inside an
// organization's Kunde-mapping <select>.
const CREATE_NEW_CUSTOMER_VALUE = "__create_new__";

// Heuristic scan of a get_kaseya_system_details() payload: the exact shape
// isn't part of the fixed contract (it's just Record<string, unknown>), so
// we look for top-level, non-object fields whose key matches one of a
// handful of common name/hostname/ip-shaped key names. Identical in spirit
// to every other plugin section's findExternalValue.
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

// Devices come back from the Kaseya API in whatever order the API returns
// them in -- sort alphabetically by name (German collation, e.g. "ä" sorts
// near "a") for display, independent of the text filter.
function sortByName<T extends { name: string }>(items: T[]): T[] {
  return items.slice().sort((a, b) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
}

// How many devices an expanded organization shows per page.
const DEVICE_PAGE_SIZE = 10;

// Slices `items` (already filtered/sorted) to one page of DEVICE_PAGE_SIZE,
// clamping a possibly-stale stored page number into range -- e.g. after the
// text filter narrows the list, or after a resync shrinks/grows it -- so
// callers never need to worry about "page 4 of 1" themselves; they just
// always render/act on `pageItems` and display `clampedPage`. Shared by
// renderDeviceGroup (mouse) and the keydown handler (keyboard) so both agree
// on what "the current page" contains.
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

// Small colored dot + Ja/Nein label for a boolean device flag
// (is_agent_installed/is_mdm_enrolled) -- the closest equivalent this
// integration has to Tactical RMM's online/offline StatusBadge, since
// Kaseya's confirmed device fields carry no status field at all (see module
// docs).
function BoolBadge({ label, value }: { label: string; value: boolean }) {
  const color = value ? "var(--success)" : "var(--text-muted)";
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "0.3rem", fontSize: "0.78rem", color }}>
      <span style={{ width: "0.5em", height: "0.5em", borderRadius: "50%", background: color, display: "inline-block" }} />
      {label}: {value ? "Ja" : "Nein"}
    </span>
  );
}

// "◀ Zurück" / "Seite X von Y" / "Weiter ▶" -- rendered under an expanded
// organization's device list. Hides itself entirely when there's only one
// page.
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
        {device.group_id && (
          <span style={{ color: "var(--text-secondary)", fontSize: "0.85em" }}> · Gruppe {device.group_id}</span>
        )}
      </span>
      <span style={{ display: "inline-flex", alignItems: "center", gap: "0.5rem" }}>
        <BoolBadge label="Agent" value={device.is_agent_installed} />
        <BoolBadge label="MDM" value={device.is_mdm_enrolled} />
      </span>
    </span>
  );
}

// Kaseya VSA's confirmed device schema carries no hostname/asset-tag/serial
// field at all -- only Name (see module docs and plugin::kaseya's own
// docs). The suggested match key for "Mit bestehendem System verknüpfen"
// therefore falls back to the device's own Name, compared against the only
// free-text field a local system has for it -- System.hostname -- exactly
// the same "best available candidate, not a confirmed guarantee" spirit as
// SnipeitPluginSection.tsx's own asset_tag/serial-based matchKeyForDevice.
function matchKeyForDevice(device: ExternalSystemDto): string {
  return (device.name || "").trim().toLowerCase();
}

export default function KaseyaPluginSection() {
  const openCustomerEditor = useAppStore((s) => s.openCustomerEditor);
  const customerEditorTarget = useAppStore((s) => s.customerEditorTarget);

  const [connections, setConnections] = useState<KaseyaConnectionDto[]>([]);
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [connectionsError, setConnectionsError] = useState<string | null>(null);

  // Which organization's Kunde-mapping select is waiting on the
  // globally-mounted CustomerForm modal to finish creating a customer,
  // keyed the same way as orgMapBusy/orgMapError below
  // (`${connectionId}:${organizationId}`). Set when the user picks "+ Neuen
  // Kunden anlegen…"; consumed and cleared by the customerEditorTarget-
  // closed effect further down, which then auto-maps the newly created
  // customer to this organization.
  const [pendingCustomerCreationGroupKey, setPendingCustomerCreationGroupKey] = useState<string | null>(null);

  // Add-connection modal.
  const [addFormOpen, setAddFormOpen] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [newBaseUrl, setNewBaseUrl] = useState("");
  const [newTokenId, setNewTokenId] = useState("");
  const [newTokenSecret, setNewTokenSecret] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addStatus, setAddStatus] = useState<string | null>(null);
  const [addError, setAddError] = useState<string | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  // Which connection's "Organisationen & Geräte" overlay is open. Page-local
  // UI state -- this modal only ever opens from a button on this same page,
  // so it doesn't need to live in the global store.
  const [openConnectionId, setOpenConnectionId] = useState<string | null>(null);

  // Organization-mapping busy/error, keyed by `${connectionId}:${organizationId}`.
  const [orgMapBusy, setOrgMapBusy] = useState<Record<string, boolean>>({});
  const [orgMapError, setOrgMapError] = useState<Record<string, string | null>>({});

  // Cached (offline) device sync per connection.
  const [cachedSyncByConnection, setCachedSyncByConnection] = useState<Record<string, CachedKaseyaSyncDto | null>>({});
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

  // Which organizations are expanded -- collapsed by default (an
  // organization renders as just a header row: name, Kunde-mapping select,
  // and a device-count summary). Keyed by
  // `${connectionId}:${organizationId}`, same as everything else below.
  // Deliberately NOT an accordion: multiple organizations can be expanded
  // at once, mirroring Tactical RMM's/NinjaOne's clients/organizations.
  const [expandedGroupKeys, setExpandedGroupKeys] = useState<Set<string>>(new Set());

  // Device-list pagination, 10 devices/page, keyed the same way
  // (`${connectionId}:${organizationId}`); 0-based, so 0 means "Seite 1".
  // Paginates over the exact same filtered/sorted list the keyboard nav
  // below walks (see computeGroupDevices + paginateDevices), so "current
  // page" means the same thing to the mouse and to j/k/Enter/l/u.
  const [pageByGroup, setPageByGroup] = useState<Record<string, number>>({});

  // Keyboard navigation over the currently-open connection's device lists --
  // mirrors the j/k/Enter pattern used by every other plugin section. Each
  // organization's "Nicht verknüpft" + "Bereits verknüpft" devices are
  // treated as ONE flat, ordered list (unlinked first, then linked -- see
  // computeGroupDevices), and `activeGroupKey`
  // (`${connectionId}:${organizationId}`) says which organization's list
  // currently owns the keyboard highlight -- only one group is
  // keyboard-active at a time; switching groups happens by focusing that
  // group's filter input, hovering one of its rows, or expanding it via its
  // header.
  const [activeGroupKey, setActiveGroupKey] = useState<string | null>(null);
  const [selectedIndexByGroup, setSelectedIndexByGroup] = useState<Record<string, number>>({});

  // Auto-focus the first group's device filter input when a connection's
  // overlay first has data to show, so the user can start typing to narrow
  // the list immediately (see the effect below). `autoFocusedConnectionRef`
  // guards against re-stealing focus on every subsequent re-render/refresh.
  const firstFilterInputRef = useRef<HTMLInputElement | null>(null);
  const autoFocusedConnectionRef = useRef<string | null>(null);

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
  // Group-scoped, keyed by `${connectionId}:${organization_id}` — disables
  // the "Alle anlegen" bulk button for one group while it works through that
  // group's unlinked devices, independent of the per-device `createLinkBusy`
  // map (both are set during a bulk run, so a device's own row also shows
  // busy).
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
    invoke<KaseyaConnectionDto[]>("list_kaseya_connections")
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

  // CustomerForm is globally mounted and driven by the store (see
  // CustomerListView.tsx for the same pattern) -- this component has no
  // onDone callback from it, so it detects "the customer editor just
  // closed" by watching customerEditorTarget transition from non-null to
  // null. When that happens right after the user picked "+ Neuen Kunden
  // anlegen…" on one of the organization selects below (tracked via
  // pendingCustomerCreationGroupKey), re-fetch the customer list, diff it
  // against the list from before the fetch to find the newly-created
  // customer, and -- if exactly one appeared -- auto-map it to the
  // organization that was waiting, via the exact same
  // handleOrgMappingChange path the manual dropdown selection already uses.
  // If the user cancelled instead of saving, no new customer will be found
  // -- the list is simply refreshed and the pending flag cleared, no error
  // surfaced.
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
            const organizationId = groupKey.slice(separatorIdx + 1);
            const connection = connections.find((c) => c.id === connectionId);
            const group = cachedSyncByConnection[connectionId]?.groups.find((g) => g.organization_id === organizationId);
            if (connection && group) {
              await handleOrgMappingChange(connection, group, String(newlyCreated[0].id));
            }
          }
          // More than one new customer (rare -- e.g. created elsewhere in
          // the same window) can't be disambiguated: the list refresh above
          // already covers that case, deliberately without guessing.
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
  }, [customerEditorTarget, pendingCustomerCreationGroupKey, customers, connections, cachedSyncByConnection]);

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

  const loadCachedSync = useCallback(async (connection: KaseyaConnectionDto) => {
    const id = connection.id;
    setCacheBusy((prev) => ({ ...prev, [id]: true }));
    setCacheError((prev) => ({ ...prev, [id]: null }));
    try {
      const cached = await invoke<CachedKaseyaSyncDto | null>("get_cached_kaseya_sync", { connectionId: id });
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
      setCacheError((prev) => ({ ...prev, [id]: formatInvokeError(err) }));
    } finally {
      setCacheBusy((prev) => ({ ...prev, [id]: false }));
    }
  }, []);

  function openOrgDeviceModal(connection: KaseyaConnectionDto) {
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
      await invoke("test_kaseya_connection", { baseUrl: newBaseUrl, tokenId: newTokenId, tokenSecret: newTokenSecret });
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
      await invoke("add_kaseya_connection", {
        label: newLabel,
        baseUrl: newBaseUrl,
        tokenId: newTokenId,
        tokenSecret: newTokenSecret,
      });
      setAddStatus(`Verbindung "${newLabel}" hinzugefügt.`);
      setNewLabel("");
      setNewBaseUrl("");
      setNewTokenId("");
      setNewTokenSecret("");
      setTestResult(null);
      reloadConnections();
      setAddFormOpen(false);
    } catch (err) {
      setAddError(formatInvokeError(err));
    } finally {
      setAddBusy(false);
    }
  }

  async function handleRemoveConnection(connection: KaseyaConnectionDto) {
    const confirmed = window.confirm(`Kaseya-VSA-Verbindung "${connection.label}" wirklich entfernen?`);
    if (!confirmed) return;
    const id = connection.id;
    setRemoveBusy((prev) => ({ ...prev, [id]: true }));
    setConnectionsError(null);
    try {
      await invoke("remove_kaseya_connection", { id });
      if (openConnectionId === id) setOpenConnectionId(null);
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

  async function handleSync(connection: KaseyaConnectionDto) {
    const id = connection.id;
    setSyncBusy((prev) => ({ ...prev, [id]: true }));
    setSyncStatus((prev) => ({ ...prev, [id]: null }));
    setSyncError((prev) => ({ ...prev, [id]: null }));
    try {
      const groups = await invoke<KaseyaOrgDeviceGroupDto[]>("sync_kaseya_connection", { connectionId: id });
      const deviceCount = groups.reduce((sum, g) => sum + g.devices.length, 0);
      setSyncStatus((prev) => ({
        ...prev,
        [id]: `${groups.length} Organisation(en), ${deviceCount} Gerät(e) gefunden.`,
      }));
      // Prefer re-reading the cache afterward (authoritative, server-written
      // synced_at_utc) over building the timestamp from the client clock.
      await loadCachedSync(connection);
      // The device list just changed underneath any expanded organization --
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
      setSyncError((prev) => ({ ...prev, [id]: formatInvokeError(err) }));
    } finally {
      setSyncBusy((prev) => ({ ...prev, [id]: false }));
    }
  }

  async function handleOrgMappingChange(
    connection: KaseyaConnectionDto,
    group: KaseyaOrgDeviceGroupDto,
    rawValue: string,
  ) {
    const mapKey = `${connection.id}:${group.organization_id}`;
    setOrgMapBusy((prev) => ({ ...prev, [mapKey]: true }));
    setOrgMapError((prev) => ({ ...prev, [mapKey]: null }));
    try {
      if (rawValue === "") {
        await invoke("unmap_kaseya_organization", { connectionId: connection.id, organizationId: group.organization_id });
      } else {
        const customerId = Number(rawValue);
        await invoke("map_kaseya_organization", {
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
      setOrgMapError((prev) => ({ ...prev, [mapKey]: formatInvokeError(err) }));
    } finally {
      setOrgMapBusy((prev) => ({ ...prev, [mapKey]: false }));
    }
  }

  function openLinkPicker(connection: KaseyaConnectionDto, group: KaseyaOrgDeviceGroupDto, device: ExternalSystemDto) {
    if (group.customer_id === null) return;
    const customerId = group.customer_id;
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

  async function confirmLink(connection: KaseyaConnectionDto, device: ExternalSystemDto) {
    if (linkPickerSelection === "") return;
    const key = `${connection.id}:${device.external_id}`;
    setLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("link_system_to_kaseya", {
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

  async function createAndLink(connection: KaseyaConnectionDto, group: KaseyaOrgDeviceGroupDto, device: ExternalSystemDto) {
    if (group.customer_id === null) return;
    const customerId = group.customer_id;
    const key = `${connection.id}:${device.external_id}`;
    setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      // A local System has no group/install/MDM-enrollment field of its
      // own -- rather than silently losing that Kaseya-native context on
      // creation, it's seeded into "Notizen" once, up front. This is a
      // one-time default at creation time, not an automatic overwrite of an
      // existing System's fields later -- consistent with
      // docs/PLUGIN_ARCHITECTURE.md's "external data never auto-overwrites
      // self-maintained fields" rule (there is no existing System yet here).
      const noteLines = [
        device.group_id ? `Kaseya-Gruppe: ${device.group_id}` : null,
        `Agent installiert: ${device.is_agent_installed ? "Ja" : "Nein"}`,
        `MDM verwaltet: ${device.is_mdm_enrolled ? "Ja" : "Nein"}`,
      ].filter((line): line is string => line !== null);
      const created = await invoke<System>("create_system", {
        input: {
          customer_id: customerId,
          name: device.name,
          system_type: "",
          // No confirmed hostname/IP field on a Kaseya device (see module
          // docs) -- left blank, exactly like SnipeitPluginSection.tsx does
          // for its own hostname-less device shape.
          hostname: "",
          ip_address: "",
          notes: noteLines.join("\n"),
        },
      });
      await invoke("link_system_to_kaseya", {
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

  // Bulk version of `createAndLink`: works through every unlinked device in
  // one group sequentially. A failure on one device does not abort the
  // rest — it is recorded in the same `deviceError` map that already
  // surfaces per-device errors, so a partial run still leaves the row's own
  // "Neu anlegen" button as the retry path. `refreshLocalSystems`/
  // `loadCachedSync` run once at the end, not per device, so a group with
  // many devices doesn't refetch the whole customer's system list N times.
  async function createAndLinkAll(connection: KaseyaConnectionDto, group: KaseyaOrgDeviceGroupDto, devices: ExternalSystemDto[]) {
    if (group.customer_id === null || devices.length === 0) return;
    const customerId = group.customer_id;
    const groupKey = `${connection.id}:${group.organization_id}`;
    setBulkCreateBusy((prev) => ({ ...prev, [groupKey]: true }));
    try {
      for (const device of devices) {
        const key = `${connection.id}:${device.external_id}`;
        setCreateLinkBusy((prev) => ({ ...prev, [key]: true }));
        setDeviceError((prev) => ({ ...prev, [key]: null }));
        try {
          const noteLines = [
            device.group_id ? `Kaseya-Gruppe: ${device.group_id}` : null,
            `Agent installiert: ${device.is_agent_installed ? "Ja" : "Nein"}`,
            `MDM verwaltet: ${device.is_mdm_enrolled ? "Ja" : "Nein"}`,
          ].filter((line): line is string => line !== null);
          const created = await invoke<System>("create_system", {
            input: {
              customer_id: customerId,
              name: device.name,
              system_type: "",
              hostname: "",
              ip_address: "",
              notes: noteLines.join("\n"),
            },
          });
          await invoke("link_system_to_kaseya", {
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
      setBulkCreateBusy((prev) => ({ ...prev, [groupKey]: false }));
    }
  }

  async function handleUnlink(connection: KaseyaConnectionDto, device: ExternalSystemDto) {
    if (device.linked_system_id === null) return;
    const key = `${connection.id}:${device.external_id}`;
    setUnlinkBusy((prev) => ({ ...prev, [key]: true }));
    setDeviceError((prev) => ({ ...prev, [key]: null }));
    try {
      await invoke("unlink_system_from_kaseya", { systemId: device.linked_system_id, connectionId: connection.id });
      if (detailsOpenKey === key) setDetailsOpenKey(null);
      await loadCachedSync(connection);
    } catch (err) {
      setDeviceError((prev) => ({ ...prev, [key]: formatInvokeError(err) }));
    } finally {
      setUnlinkBusy((prev) => ({ ...prev, [key]: false }));
    }
  }

  async function toggleDetails(connection: KaseyaConnectionDto, group: KaseyaOrgDeviceGroupDto, device: ExternalSystemDto) {
    const key = `${connection.id}:${device.external_id}`;
    if (detailsOpenKey === key) {
      setDetailsOpenKey(null);
      return;
    }
    setDetailsOpenKey(key);
    setDetailsError((prev) => ({ ...prev, [key]: null }));
    setDetailsBusy((prev) => ({ ...prev, [key]: true }));
    try {
      const detailsPromise = invoke<Record<string, unknown>>("get_kaseya_system_details", {
        connectionId: connection.id,
        externalId: device.external_id,
      });
      const refreshPromise = group.customer_id !== null ? refreshLocalSystems(group.customer_id) : Promise.resolve();
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

  // Combines an organization's filtered "unlinked" and "linked" devices into
  // one flat, keyboard-navigable list (unlinked first, matching visual
  // order) -- shared by renderDeviceGroup (for rendering) and the global
  // keydown handler below (for acting on whichever row is highlighted), so
  // the two can never disagree about ordering/filtering.
  function computeGroupDevices(connectionId: string, group: KaseyaOrgDeviceGroupDto) {
    const groupKey = `${connectionId}:${group.organization_id}`;
    const filterText = deviceFilter[groupKey] ?? "";
    const q = filterText.trim().toLowerCase();
    const sortedDevices = sortByName(group.devices);
    const filteredDevices =
      q === ""
        ? sortedDevices
        : sortedDevices.filter((d) => d.name.toLowerCase().includes(q) || (d.group_id ?? "").toLowerCase().includes(q));
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
  // currently keyboard-active organization's device list -- same
  // isTypingTarget-gated pattern as every other plugin section, so typing
  // in the filter input (or any other field) never fights with these
  // single-letter bindings. Only live while the organization/device overlay
  // is actually open.
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
      // for j/k/Enter/l/u to act on -- and critically, without this guard
      // Enter would silently create-and-link (or toggle details on) a
      // device the user can't even see.
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
        // the required bindings -- j/k/Enter/l/u still clamp at the page
        // edge as before -- but ArrowLeft/ArrowRight were unused, so paging
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
  // on mount -- before the async cache load resolves -- and would otherwise
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

  function renderDetailsPanel(group: KaseyaOrgDeviceGroupDto, device: ExternalSystemDto, key: string) {
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
        <div style={{ display: "flex", gap: "0.75rem", alignItems: "center", flexWrap: "wrap", fontSize: "0.8rem", color: "var(--text-secondary)" }}>
          <span>Gruppe (Kaseya): {device.group_id ?? <span style={{ color: "var(--text-muted)" }}>—</span>}</span>
          <BoolBadge label="Agent installiert" value={device.is_agent_installed} />
          <BoolBadge label="MDM verwaltet" value={device.is_mdm_enrolled} />
        </div>
        {busy && <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)" }}>Lade Kaseya-Details…</p>}
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
                    <th style={thStyle}>Kaseya VSA (extern)</th>
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

  function renderDeviceGroup(connection: KaseyaConnectionDto, group: KaseyaOrgDeviceGroupDto, isFirstFilterable: boolean) {
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
    // mapped) -- or the flat filtered list for a not-yet-mapped
    // organization, which has no linked/unlinked split and isn't
    // keyboard-navigable.
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
    // page-relative index (0..pageItems.length-1) -- the same index space
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
                // visually stick on the placeholder option -- its React
                // `value` prop won't change (the mapping itself didn't
                // change), so nothing else would force this back on its own.
                e.target.value = group.customer_id === null ? "" : String(group.customer_id);
                // Remember which organization is waiting so the
                // customerEditorTarget-closed effect above can auto-map the
                // newly created customer once the CustomerForm modal
                // closes.
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
              <p style={mutedStyle}>Diese Organisation ist noch keinem Kunden zugeordnet — oben zuordnen, um Geräte zu verknüpfen.</p>
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
                  // leave the user stranded on "page 4 of 1" -- restart at
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
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "0.5rem" }}>
                    <div style={sectionLabelStyle}>Nicht verknüpft ({unlinked.length})</div>
                    {unlinked.length > 0 && (
                      <button
                        type="button"
                        disabled={bulkCreateBusy[groupKey] ?? false}
                        onClick={() => void createAndLinkAll(connection, group, unlinked)}
                      >
                        {bulkCreateBusy[groupKey] ? "Lege an…" : `Alle anlegen (${unlinked.length})`}
                      </button>
                    )}
                  </div>
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

  function renderConnectionModal(connection: KaseyaConnectionDto) {
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
                Noch nicht synchronisiert. Auf "Aktualisieren" klicken, um Organisationen und Geräte von Kaseya VSA zu
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
              return cached.groups.map((group) => renderDeviceGroup(connection, group, group.organization_id === firstFilterableOrgId));
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
          <h2 style={{ margin: 0, fontSize: "1rem" }}>Kaseya-VSA-Verbindungen</h2>
          <button type="button" onClick={() => setAddFormOpen(true)}>
            + Neue Verbindung
          </button>
        </div>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Liest ausschließlich Geräte aus Kaseya VSA (selbst gehostetes RMM-Tool, jede Kundin betreibt ihren eigenen
          VSA-Server) — es werden nie Daten dorthin zurückgeschrieben. Eine Verbindung kann mehrere Organisationen
          sehen; jede Organisation wird einzeln einem Kunden zugeordnet.
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
            <h2 style={{ margin: 0, fontSize: "1rem" }}>Neue Kaseya-VSA-Verbindung</h2>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Label
              <input value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="ACME Kaseya VSA" required autoFocus />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Base-URL
              <input
                value={newBaseUrl}
                onChange={(e) => {
                  setNewBaseUrl(e.target.value);
                  setTestResult(null);
                }}
                placeholder="https://ihr-server.beispiel.de/api"
                style={{ fontFamily: "var(--font-mono)" }}
                required
              />
              <span style={{ fontSize: "0.75rem", color: "var(--text-muted)" }}>
                Vollständige API-Basis-URL Ihrer VSA-Instanz (z. B. https://ihr-server.beispiel.de/api oder .../api/v3
                — je nach Version testen).
              </span>
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Token-ID
              <input value={newTokenId} onChange={(e) => { setNewTokenId(e.target.value); setTestResult(null); }} required />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Token-Secret
              <input
                type="password"
                value={newTokenSecret}
                onChange={(e) => {
                  setNewTokenSecret(e.target.value);
                  setTestResult(null);
                }}
                required
              />
              <span style={{ fontSize: "0.75rem", color: "var(--text-muted)" }}>
                Erzeugt in Kaseya VSA unter Configuration → API Access → Third Party Tokens → Create Token.
              </span>
            </label>
            {testResult && (testResult.ok ? <SuccessText>{testResult.message}</SuccessText> : <ErrorText>{testResult.message}</ErrorText>)}
            {addStatus && <SuccessText>{addStatus}</SuccessText>}
            {addError && <ErrorText>{addError}</ErrorText>}
            <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end", marginTop: "0.25rem" }}>
              <button type="button" onClick={() => setAddFormOpen(false)}>
                Abbrechen
              </button>
              <button type="button" disabled={testBusy || !newBaseUrl || !newTokenId || !newTokenSecret} onClick={() => void handleTestConnection()}>
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
