import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../state/appStore";
import { formatInvokeError } from "../lib/errors";
import { getKeymap, matchesBinding } from "../lib/keymap";
import Modal from "./Modal";
import MarkdownEditor, { type MarkdownEditorHandle } from "./MarkdownEditor";
import AttachmentDropzone, { type PickedFile } from "./AttachmentDropzone";
import AttachmentList from "./AttachmentList";

interface Customer {
  id: number;
  name: string;
  short_code: string;
}

interface System {
  id: number;
  customer_id: number;
  name: string;
  hostname: string;
  // Present on every `list_systems` response (see `db::systems::System`)
  // but not previously typed/searched here -- the System typeahead only
  // matched `name`/`hostname`, even though a user might reasonably type
  // an IP address or a word from a system's own notes to find it.
  ip_address: string;
  notes: string;
}

interface Entry {
  id: number;
  customer_id: number;
  system_id: number | null;
  title: string;
  body_md: string;
  category: string;
  performed_at_utc: string;
  performed_at_tz: string;
  created_at_utc: string;
  created_at_tz: string;
  updated_at_utc: string;
  updated_at_tz: string;
  tags: string[];
}

interface Attachment {
  id: number;
  entry_id: number;
  sha256: string;
  original_filename: string;
  mime_type: string;
  size_bytes: number;
  created_at_utc: string;
  created_at_tz: string;
}

interface PendingAttachment {
  token: string;
  bytesBase64: string;
  filename: string;
  mimeType: string;
}

// Mirrors src-tauri/src/attachments/store.rs's relative_path_for(): deterministic,
// content-addressed naming — attachments/{first 2 hex chars}/{sha256}{.ext}. Used to
// build a Markdown image reference immediately after add_attachment_to_entry returns,
// without a second round-trip to ask the backend for the path it just computed.
function relativePathFor(sha256: string, originalFilename: string): string {
  const dotIndex = originalFilename.lastIndexOf(".");
  const ext = dotIndex >= 0 ? originalFilename.slice(dotIndex) : "";
  return `attachments/${sha256.slice(0, 2)}/${sha256}${ext}`;
}

// Kept local rather than shared — five static entries aren't worth a shared
// module (same convention as JournalView.tsx/QuickCapture.tsx).
const CATEGORIES: { value: string; label: string }[] = [
  { value: "wartung", label: "Wartung" },
  { value: "stoerung", label: "Störung" },
  { value: "aenderung", label: "Änderung" },
  { value: "installation", label: "Installation" },
  { value: "sonstiges", label: "Sonstiges" },
];

// Plugin-sourced device known from a connected RMM/asset-management plugin
// (NinjaOne, Level.io, Snipe-IT) for the selected customer that has no local
// System linked to it yet. Backend reads already-cached plugin data (no
// network call), so it's cheap to fetch alongside the local systems list.
interface UnlinkedExternalSystemDto {
  // A plain `String` on the Rust side (`commands::external_directory::
  // UnlinkedExternalSystemDto.plugin`) -- deliberately NOT a literal union
  // here anymore. It used to list only the three plugins that existed when
  // this typeahead was first built ("ninja" | "level" | "snipeit"), which
  // silently went stale as eleven more plugins were added without anyone
  // updating this type -- a real staleness bug, not just a search gap (see
  // `filteredSystemRows` below for the other half of that same bug: typing
  // a plugin's own name, e.g. "acronis", never matched anything, because
  // this field was never even searched).
  plugin: string;
  connection_id: string;
  external_id: string;
  name: string;
  hostname: string | null;
  ip_address: string | null;
  // `Some(tenant_id)` ONLY for Acronis (`link_system_to_acronis` needs a
  // `tenantId` argument no other plugin's link command has); `null` for
  // every other plugin's devices. See `LINK_COMMAND`/`selectExternalSystemRow`
  // below for how this is used.
  tenant_id: string | null;
}

// Maps a plugin id to its `link_system_to_<plugin>` Tauri command name.
// A real, previously-broken assumption this replaces: `selectExternalSystemRow`
// used to hardcode a 3-way ninja/level/snipeit ternary that silently fell
// through to `link_system_to_snipeit` for every one of the eleven OTHER
// plugins added since -- picking an Acronis (or Kaseya, or any other newer
// plugin's) suggestion here would have called the wrong Tauri command
// entirely. Keep this in sync with `src-tauri/src/lib.rs`'s registered
// `link_system_to_*` commands whenever a new plugin is added.
const LINK_COMMAND: Record<string, string> = {
  ninja: "link_system_to_ninja",
  level: "link_system_to_level",
  snipeit: "link_system_to_snipeit",
  intune: "link_system_to_intune",
  iru: "link_system_to_iru",
  jamf: "link_system_to_jamf",
  abm: "link_system_to_abm",
  tacticalrmm: "link_system_to_tacticalrmm",
  atera: "link_system_to_atera",
  pulseway: "link_system_to_pulseway",
  kaseya: "link_system_to_kaseya",
  action1: "link_system_to_action1",
  dattormm: "link_system_to_dattormm",
  acronis: "link_system_to_acronis",
};

// Falls back to the raw plugin id (still readable, e.g. "kaseya") for any
// plugin not in this map, rather than rendering "undefined".
const PLUGIN_LABEL: Record<string, string> = {
  ninja: "Ninja",
  level: "Level",
  snipeit: "Snipe-IT",
  intune: "Intune",
  iru: "Iru",
  jamf: "Jamf",
  abm: "ABM",
  tacticalrmm: "Tactical RMM",
  atera: "Atera",
  pulseway: "Pulseway",
  kaseya: "Kaseya",
  action1: "Action1",
  dattormm: "Datto RMM",
  acronis: "Acronis",
};

function pluginLabel(plugin: string): string {
  return PLUGIN_LABEL[plugin] ?? plugin;
}

// System typeahead: a filterable dropdown over the already-fetched systems
// list (filtering by name AND hostname, case-insensitive) instead of a plain
// <select>, since a customer can have many systems and hostname is the
// user-recognizable key in practice. A synthetic "clear" row is always first
// so the field can always be reset back to "Kein System" from the keyboard
// or a click, same as the old <select>'s empty option.
//
// A third row kind surfaces devices known from a connected plugin but not
// yet linked to any local System (see UnlinkedExternalSystemDto above) —
// selecting one transparently creates the local System and links it, same
// as picking "Neu anlegen" (create-and-link) in the plugin
// sections (NinjaPluginSection.tsx etc.), just inline in this typeahead.
type SystemRow = { kind: "clear" } | { kind: "system"; system: System } | { kind: "external"; device: UnlinkedExternalSystemDto };

function systemLabel(system: System): string {
  return system.hostname ? `${system.name} — ${system.hostname}` : system.name;
}

export default function EntryEditor() {
  const editorTarget = useAppStore((s) => s.editorTarget);
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const selectedSystemId = useAppStore((s) => s.selectedSystemId);
  const formOpen = useAppStore((s) => s.formOpen);
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);
  const closeEntryEditor = useAppStore((s) => s.closeEntryEditor);

  const [customers, setCustomers] = useState<Customer[]>([]);
  const [systems, setSystems] = useState<System[]>([]);
  const [unlinkedExternalSystems, setUnlinkedExternalSystems] = useState<UnlinkedExternalSystemDto[]>([]);
  const [customerId, setCustomerId] = useState<number | "">("");
  const [systemId, setSystemId] = useState<number | "">("");
  const [systemQuery, setSystemQuery] = useState("");
  const [systemSuggestionsOpen, setSystemSuggestionsOpen] = useState(false);
  const [systemHighlightIndex, setSystemHighlightIndex] = useState(0);
  const [systemCreateLinkBusy, setSystemCreateLinkBusy] = useState(false);
  const [title, setTitle] = useState("");
  const [bodyMd, setBodyMd] = useState("");
  const [category, setCategory] = useState("wartung");
  const [tagsInput, setTagsInput] = useState("");
  const [allTags, setAllTags] = useState<string[]>([]);
  const [tagSuggestionsOpen, setTagSuggestionsOpen] = useState(false);
  const [tagHighlightIndex, setTagHighlightIndex] = useState(0);
  const [performedAtInput, setPerformedAtInput] = useState("");
  const [performedAtUtc, setPerformedAtUtc] = useState("");
  const [performedAtTz, setPerformedAtTz] = useState("");
  const [performedAtPreview, setPerformedAtPreview] = useState("");
  const [loadingEntry, setLoadingEntry] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [pendingAttachments, setPendingAttachments] = useState<PendingAttachment[]>([]);

  const titleRef = useRef<HTMLInputElement>(null);
  const editorRef = useRef<MarkdownEditorHandle>(null);

  // Full customer list, once — used for both the create-mode select and the
  // disabled edit-mode select.
  useEffect(() => {
    invoke<Customer[]>("list_customers", { includeArchived: false }).then(setCustomers);
  }, []);

  // All tag names ever used, once — powers the Tags field's autocomplete.
  useEffect(() => {
    invoke<string[]>("list_tags").then(setAllTags).catch(() => setAllTags([]));
  }, []);

  // The comma-separated Tags input's suggestions are scoped to whatever the
  // user is currently typing after the last comma, excluding tags already
  // present earlier in the same input (no point suggesting a duplicate).
  const tagFragments = tagsInput.split(",");
  const currentTagFragment = tagFragments[tagFragments.length - 1].trim();
  const alreadyEnteredTags = tagFragments
    .slice(0, -1)
    .map((t) => t.trim().toLowerCase())
    .filter(Boolean);
  const tagSuggestions = useMemo(() => {
    if (currentTagFragment === "") return [];
    const q = currentTagFragment.toLowerCase();
    return allTags.filter((t) => t.toLowerCase().includes(q) && !alreadyEnteredTags.includes(t.toLowerCase())).slice(0, 8);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [allTags, currentTagFragment]);

  function applyTagSuggestion(suggestion: string) {
    const parts = tagsInput.split(",");
    parts[parts.length - 1] = ` ${suggestion}`;
    setTagsInput(
      parts
        .map((p) => p.trim())
        .filter(Boolean)
        .join(", ") + ", ",
    );
    setTagSuggestionsOpen(false);
  }

  useEffect(() => {
    setTagHighlightIndex(0);
    setTagSuggestionsOpen(tagSuggestions.length > 0);
  }, [tagSuggestions]);

  function handleTagsKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (!tagSuggestionsOpen || tagSuggestions.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setTagHighlightIndex((i) => Math.min(i + 1, tagSuggestions.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setTagHighlightIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      applyTagSuggestion(tagSuggestions[tagHighlightIndex]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setTagSuggestionsOpen(false);
    }
  }

  // Shared by the customer-change effect below and by the plugin-suggestion
  // create+link handler's post-success refresh — refetches both the local
  // systems list and the plugin-sourced unlinked-devices list for a customer.
  // The latter is secondary/enhancement data: a failure there (no plugin
  // connected, cache not warm yet) must not affect the local systems list.
  const refreshSystemsForCustomer = useCallback((forCustomerId: number) => {
    invoke<System[]>("list_systems", { customerId: forCustomerId, includeArchived: false }).then(setSystems);
    invoke<UnlinkedExternalSystemDto[]>("list_unlinked_external_systems_for_customer", { customerId: forCustomerId })
      .then(setUnlinkedExternalSystems)
      .catch(() => setUnlinkedExternalSystems([]));
  }, []);

  // Refetch systems whenever the customer changes. Deliberately does NOT
  // reset systemId here — this effect also fires when customerId is set
  // programmatically (new-mode default seeding, edit-mode load), and a reset
  // here would clobber a systemId set moments earlier in the same flow. The
  // "reset system when customer changes" UX lives in the customer <select>'s
  // onChange instead, since that is the only place a genuinely user-driven
  // customer change happens. See plan doc for the full rationale.
  useEffect(() => {
    if (customerId === "") {
      setSystems([]);
      setUnlinkedExternalSystems([]);
      return;
    }
    refreshSystemsForCustomer(customerId);
  }, [customerId, refreshSystemsForCustomer]);

  // Filtered dropdown rows for the System typeahead — recomputed as the user
  // types. The "clear" row is unfiltered/always present so "Kein System" stays
  // reachable regardless of the current query text.
  const filteredSystemRows = useMemo<SystemRow[]>(() => {
    const q = systemQuery.trim().toLowerCase();
    const matches =
      q === ""
        ? systems
        : systems.filter(
            (s) =>
              s.name.toLowerCase().includes(q) ||
              s.hostname.toLowerCase().includes(q) ||
              s.ip_address.toLowerCase().includes(q) ||
              s.notes.toLowerCase().includes(q),
          );
    const externalMatches =
      q === ""
        ? unlinkedExternalSystems
        : unlinkedExternalSystems.filter(
            (d) =>
              d.name.toLowerCase().includes(q) ||
              (d.hostname ?? "").toLowerCase().includes(q) ||
              (d.ip_address ?? "").toLowerCase().includes(q) ||
              // Also match on the SOURCE plugin's own name (e.g. typing
              // "acronis" finds every unlinked Acronis resource, not just
              // ones whose device name happens to contain that text) --
              // real user-reported gap: querying a plugin's own name
              // found nothing even when that plugin's sync had genuinely
              // discovered unlinked resources.
              d.plugin.toLowerCase().includes(q),
          );
    return [
      { kind: "clear" },
      ...matches.map((system) => ({ kind: "system" as const, system })),
      ...externalMatches.map((device) => ({ kind: "external" as const, device })),
    ];
  }, [systems, unlinkedExternalSystems, systemQuery]);

  // Keep the input's displayed text in sync with the committed systemId
  // (initial load, edit-mode load, customer-change reset, or a fresh "new"
  // entry seeded from the current selection). Deliberately depends only on
  // systemId/systems, NOT on systemSuggestionsOpen — closing the dropdown via
  // Escape/blur must never re-run this and stomp text the user typed but
  // didn't commit (Escape/click-outside are only supposed to close the list,
  // never discard what's typed).
  useEffect(() => {
    if (systemId === "") {
      setSystemQuery("");
      return;
    }
    const match = systems.find((s) => s.id === systemId);
    setSystemQuery(match ? systemLabel(match) : "");
  }, [systemId, systems]);

  useEffect(() => {
    setSystemHighlightIndex(0);
  }, [systemQuery, systemSuggestionsOpen]);

  // Picking a plugin-sourced suggestion has no local System to point at yet —
  // transparently create one (prefilled from the plugin device) and link it,
  // then commit exactly as if an ordinary existing System had been picked.
  // Mirrors NinjaPluginSection.tsx's (and the Level/Snipe-IT equivalents')
  // "Neu anlegen" (create-and-link) sequence.
  async function selectExternalSystemRow(device: UnlinkedExternalSystemDto) {
    if (customerId === "") return;
    const custId = customerId;
    const previousQuery = systemQuery;
    setSystemSuggestionsOpen(false);
    setSystemCreateLinkBusy(true);
    setSystemQuery(`${device.name} — wird angelegt…`);
    setError(null);
    try {
      const created = await invoke<System>("create_system", {
        input: {
          customer_id: custId,
          name: device.name,
          system_type: "",
          hostname: device.hostname ?? "",
          ip_address: device.ip_address ?? "",
          notes: "",
        },
      });
      const linkCommand = LINK_COMMAND[device.plugin];
      if (linkCommand === undefined) {
        throw new Error(`Unbekanntes Plugin: ${device.plugin}`);
      }
      await invoke(linkCommand, {
        systemId: created.id,
        connectionId: device.connection_id,
        externalId: device.external_id,
        // Only Acronis's link command takes a tenantId -- see the
        // UnlinkedExternalSystemDto.tenant_id comment above. Tauri
        // rejects an unknown named argument, so this is only included
        // when the device actually carries one.
        ...(device.tenant_id !== null ? { tenantId: device.tenant_id } : {}),
      });
      // Reflect the new System locally right away so the query-sync effect
      // below can show it immediately, without depending on the best-effort
      // refresh that follows succeeding.
      setSystems((prev) => [...prev, created]);
      setSystemId(created.id);
      // Nice-to-have, not required for correctness above: refreshes both
      // lists so a reopened dropdown shows this as a normal local System
      // (not still listed as an unlinked plugin suggestion).
      refreshSystemsForCustomer(custId);
    } catch (e) {
      setError(formatInvokeError(e));
      setSystemQuery(previousQuery);
    } finally {
      setSystemCreateLinkBusy(false);
    }
  }

  function selectSystemRow(row: SystemRow) {
    if (row.kind === "clear") {
      setSystemId("");
      setSystemQuery("");
      setSystemSuggestionsOpen(false);
    } else if (row.kind === "system") {
      setSystemId(row.system.id);
      setSystemQuery(systemLabel(row.system));
      setSystemSuggestionsOpen(false);
    } else {
      void selectExternalSystemRow(row.device);
    }
  }

  function handleSystemKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSystemSuggestionsOpen(true);
      setSystemHighlightIndex((i) => Math.min(i + 1, filteredSystemRows.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSystemSuggestionsOpen(true);
      setSystemHighlightIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      if (systemSuggestionsOpen) {
        e.preventDefault();
        const row = filteredSystemRows[systemHighlightIndex];
        if (row) selectSystemRow(row);
      }
    } else if (e.key === "Escape") {
      if (systemSuggestionsOpen) {
        e.preventDefault();
        e.stopPropagation();
        setSystemSuggestionsOpen(false);
      }
    }
  }

  const refreshPreview = useCallback(async (utc: string, tz: string) => {
    try {
      const preview = await invoke<string>("format_timestamp_for_display", { utc, tz });
      setPerformedAtPreview(preview);
    } catch {
      setPerformedAtPreview("");
    }
  }, []);

  // Load/init whenever the editor is opened (or switched to a different target).
  useEffect(() => {
    if (editorTarget === null) return;

    setError(null);

    if (editorTarget === "new") {
      setTitle("");
      setBodyMd("");
      setCategory("wartung");
      setTagsInput("");
      setCustomerId(selectedCustomerId ?? "");
      setSystemId(selectedSystemId ?? "");
      setSystemSuggestionsOpen(false);
      setPerformedAtInput("");
      setAttachments([]);
      setPendingAttachments([]);
      (async () => {
        try {
          const now = await invoke<{ utc: string; tz: string }>("parse_temporal_input", { input: "" });
          setPerformedAtUtc(now.utc);
          setPerformedAtTz(now.tz);
          await refreshPreview(now.utc, now.tz);
        } catch (e) {
          setError(formatInvokeError(e));
        }
      })();
      titleRef.current?.focus();
      return;
    }

    setLoadingEntry(true);
    setPendingAttachments([]);
    invoke<Entry>("get_entry", { id: editorTarget })
      .then(async (entry) => {
        setTitle(entry.title);
        setBodyMd(entry.body_md);
        setCategory(entry.category);
        setTagsInput(entry.tags.join(", "));
        setCustomerId(entry.customer_id);
        setSystemId(entry.system_id ?? "");
        setSystemSuggestionsOpen(false);
        setPerformedAtInput("");
        setPerformedAtUtc(entry.performed_at_utc);
        setPerformedAtTz(entry.performed_at_tz);
        await refreshPreview(entry.performed_at_utc, entry.performed_at_tz);
      })
      .catch((e) => setError(formatInvokeError(e)))
      .finally(() => setLoadingEntry(false));
    invoke<Attachment[]>("list_attachments_for_entry", { entryId: editorTarget })
      .then(setAttachments)
      .catch((e) => setError(formatInvokeError(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorTarget]);

  // Engage the global Esc-closing convention (useGlobalHotkeys watches
  // formOpen) whenever the editor is open, and disengage on close/unmount.
  useEffect(() => {
    if (editorTarget === null) return;
    openForm();
    return () => closeForm();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorTarget]);

  // An Esc-triggered close flips formOpen to false via useGlobalHotkeys; mirror
  // that back into editorTarget so this component's own state stays in sync.
  useEffect(() => {
    if (!formOpen && editorTarget !== null) {
      closeEntryEditor();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formOpen]);

  async function handlePerformedAtBlur() {
    if (performedAtInput.trim() === "") return;
    try {
      const result = await invoke<{ utc: string; tz: string }>("parse_temporal_input", { input: performedAtInput });
      setPerformedAtUtc(result.utc);
      setPerformedAtTz(result.tz);
      await refreshPreview(result.utc, result.tz);
      setError(null);
    } catch (e) {
      setError(formatInvokeError(e));
    }
  }

  // Shared by drag&drop, the file-picker button, and clipboard paste. In edit
  // mode the entry already exists, so a picked/pasted file is attached
  // immediately via the backend (no "pending" state); in create mode the
  // entry doesn't exist yet, so files are held locally and resolved into real
  // attachments server-side when create_entry runs (same pending_attachments
  // mechanism the quick-capture window already uses).
  const handleFilesAdded = useCallback(
    async (files: PickedFile[]) => {
      for (const file of files) {
        if (typeof editorTarget === "number") {
          try {
            const attachment = await invoke<Attachment>("add_attachment_to_entry", {
              entryId: editorTarget,
              bytesBase64: file.bytesBase64,
              originalFilename: file.filename,
              mimeType: file.mimeType,
            });
            const relativePath = relativePathFor(attachment.sha256, attachment.original_filename);
            editorRef.current?.insertAtCursor(`![${file.filename}](${relativePath})`);
            setAttachments((prev) => [...prev, attachment]);
          } catch (e) {
            setError(formatInvokeError(e));
          }
        } else {
          const token = `pending:${crypto.randomUUID()}`;
          editorRef.current?.insertAtCursor(`![${file.filename}](${token})`);
          setPendingAttachments((prev) => [
            ...prev,
            { token, bytesBase64: file.bytesBase64, filename: file.filename, mimeType: file.mimeType },
          ]);
        }
      }
    },
    [editorTarget],
  );

  const handleEditorPaste = useCallback(
    (event: ClipboardEvent) => {
      const items = event.clipboardData?.items;
      if (!items) return;
      for (const item of Array.from(items)) {
        if (item.type.startsWith("image/")) {
          event.preventDefault();
          const file = item.getAsFile();
          if (!file) return;
          void file.arrayBuffer().then((buffer) => {
            const bytes = new Uint8Array(buffer);
            let binary = "";
            for (let i = 0; i < bytes.byteLength; i++) binary += String.fromCharCode(bytes[i]);
            void handleFilesAdded([{ bytesBase64: btoa(binary), filename: "Screenshot.png", mimeType: item.type }]);
          });
          return;
        }
      }
    },
    [handleFilesAdded],
  );

  const handleAttachmentExport = useCallback(
    async (attachmentId: number) => {
      const attachment = attachments.find((a) => a.id === attachmentId);
      if (!attachment) return;
      try {
        const destPath = await saveFileDialog({ defaultPath: attachment.original_filename });
        if (destPath) {
          await invoke("copy_attachment_to", { attachmentId, destPath });
        }
      } catch (e) {
        setError(formatInvokeError(e));
      }
    },
    [attachments],
  );

  const handleAttachmentRemove = useCallback((attachmentId: number) => {
    invoke("remove_attachment", { attachmentId })
      .then(() => setAttachments((prev) => prev.filter((a) => a.id !== attachmentId)))
      .catch((e) => setError(formatInvokeError(e)));
  }, []);

  const save = useCallback(async () => {
    if (editorTarget === null) return;
    if (customerId === "") {
      setError("Kunde ist erforderlich");
      return;
    }
    const tagNames = tagsInput
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    try {
      if (editorTarget === "new") {
        await invoke("create_entry", {
          input: {
            customer_id: customerId,
            system_id: systemId === "" ? null : systemId,
            title,
            body_md: bodyMd,
            category,
            performed_at_utc: performedAtUtc,
            performed_at_tz: performedAtTz,
            tag_names: tagNames,
            pending_attachments: pendingAttachments.map((p) => ({
              placeholder_token: p.token,
              bytes_base64: p.bytesBase64,
              original_filename: p.filename,
              mime_type: p.mimeType,
            })),
          },
        });
      } else {
        await invoke("update_entry", {
          id: editorTarget,
          input: {
            system_id: systemId === "" ? null : systemId,
            title,
            body_md: bodyMd,
            category,
            performed_at_utc: performedAtUtc,
            performed_at_tz: performedAtTz,
            tag_names: tagNames,
            pending_attachments: [],
          },
        });
      }
      closeForm();
      closeEntryEditor();
    } catch (e) {
      setError(formatInvokeError(e));
    }
  }, [
    editorTarget,
    customerId,
    systemId,
    title,
    bodyMd,
    category,
    performedAtUtc,
    performedAtTz,
    tagsInput,
    pendingAttachments,
    closeForm,
    closeEntryEditor,
  ]);

  const cancel = useCallback(() => {
    closeForm();
    closeEntryEditor();
  }, [closeForm, closeEntryEditor]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (editorTarget === null) return;
      if (matchesBinding(e, getKeymap().save)) {
        e.preventDefault();
        void save();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [editorTarget, save]);

  if (editorTarget === null) return null;

  const isEditMode = typeof editorTarget === "number";

  return (
    <Modal onClose={closeForm}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          width: "40rem",
          maxWidth: "90vw",
          maxHeight: "85vh",
          overflowY: "auto",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>{isEditMode ? "Eintrag bearbeiten" : "Neuer Eintrag"}</h2>

        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Titel
          <input ref={titleRef} value={title} onChange={(e) => setTitle(e.target.value)} required autoFocus={!isEditMode} />
        </label>

        <div style={{ display: "flex", gap: "0.5rem" }}>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
            Kunde
            <select
              value={customerId}
              disabled={isEditMode}
              onChange={(e) => {
                const value = e.target.value === "" ? "" : Number(e.target.value);
                setCustomerId(value);
                setSystemId("");
              }}
              required
            >
              <option value="">Kunde wählen…</option>
              {customers.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name} ({c.short_code})
                </option>
              ))}
            </select>
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1, position: "relative" }}>
            System
            <input
              value={systemQuery}
              disabled={systemCreateLinkBusy}
              onChange={(e) => {
                setSystemQuery(e.target.value);
                setSystemSuggestionsOpen(true);
              }}
              onFocus={(e) => {
                setSystemSuggestionsOpen(true);
                e.target.select();
              }}
              onBlur={() => setSystemSuggestionsOpen(false)}
              onKeyDown={handleSystemKeyDown}
              placeholder="Kein System"
              autoComplete="off"
            />
            {systemSuggestionsOpen && (
              <div
                style={{
                  position: "absolute",
                  top: "100%",
                  left: 0,
                  right: 0,
                  marginTop: "0.15rem",
                  background: "var(--bg-elevated)",
                  border: "1px solid var(--border)",
                  borderRadius: "var(--radius-sm)",
                  boxShadow: "var(--shadow-modal)",
                  maxHeight: "12rem",
                  overflowY: "auto",
                  zIndex: 10,
                }}
              >
                {filteredSystemRows.map((row, idx) => {
                  const key =
                    row.kind === "clear"
                      ? "clear"
                      : row.kind === "system"
                        ? `system-${row.system.id}`
                        : `external-${row.device.plugin}-${row.device.connection_id}-${row.device.external_id}`;
                  return (
                    <div
                      key={key}
                      onMouseEnter={() => setSystemHighlightIndex(idx)}
                      onMouseDown={(e) => {
                        e.preventDefault();
                        selectSystemRow(row);
                      }}
                      style={{
                        padding: "0.35rem 0.5rem",
                        cursor: "pointer",
                        fontSize: "0.85rem",
                        background: idx === systemHighlightIndex ? "var(--bg-selected)" : "transparent",
                        color: row.kind === "clear" ? "var(--text-muted)" : "var(--text-primary)",
                      }}
                    >
                      {row.kind === "clear" ? (
                        "Kein System"
                      ) : row.kind === "system" ? (
                        <>
                          {row.system.name}
                          {row.system.hostname && (
                            <span style={{ color: "var(--text-secondary)", fontFamily: "var(--font-mono)", fontSize: "0.85em" }}>
                              {" "}— {row.system.hostname}
                            </span>
                          )}
                        </>
                      ) : (
                        <>
                          {row.device.name}
                          {row.device.hostname && (
                            <span style={{ color: "var(--text-secondary)", fontFamily: "var(--font-mono)", fontSize: "0.85em" }}>
                              {" "}— {row.device.hostname}
                            </span>
                          )}
                          <span style={{ color: "var(--accent)", fontFamily: "var(--font-mono)", fontSize: "0.75em", marginLeft: "0.4rem" }}>
                            · {pluginLabel(row.device.plugin)}
                          </span>
                        </>
                      )}
                    </div>
                  );
                })}
                {filteredSystemRows.length === 1 && (
                  <div style={{ padding: "0.35rem 0.5rem", fontSize: "0.8rem", color: "var(--text-muted)" }}>Keine Treffer</div>
                )}
              </div>
            )}
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
            Kategorie
            <select value={category} onChange={(e) => setCategory(e.target.value)}>
              {CATEGORIES.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
          </label>
        </div>

        <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Tätigkeitszeitpunkt
            <input
              value={performedAtInput}
              onChange={(e) => setPerformedAtInput(e.target.value)}
              onBlur={handlePerformedAtBlur}
              placeholder="jetzt, -2h, gestern 9:15, 07.09.2026 14:32"
              style={{ width: "16rem" }}
            />
          </label>
          <span style={{ fontFamily: "var(--font-mono)", fontSize: "0.85rem", color: "var(--text-secondary)" }}>
            {performedAtPreview}
          </span>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: "0.2rem", flex: 1 }}>
          <span style={{ fontSize: "0.78rem", color: "var(--text-secondary)", fontWeight: 500 }}>
            Beschreibung (Markdown)
          </span>
          <AttachmentDropzone onFilesAdded={handleFilesAdded}>
            <MarkdownEditor
              ref={editorRef}
              value={bodyMd}
              onChange={setBodyMd}
              onPaste={handleEditorPaste}
              placeholder="Markdown…"
            />
          </AttachmentDropzone>
        </div>

        {isEditMode && (
          <AttachmentList
            attachments={attachments}
            onOpen={(id) => void invoke("open_attachment", { attachmentId: id }).catch((e) => setError(formatInvokeError(e)))}
            onExport={handleAttachmentExport}
            onRemove={handleAttachmentRemove}
            resolveImageUrl={(attachment) => invoke<string>("read_attachment_data_url", { attachmentId: attachment.id })}
          />
        )}

        {pendingAttachments.length > 0 && (
          <p style={{ fontSize: "0.8rem", color: "var(--text-muted)" }}>
            {pendingAttachments.length} Anhang/Anhänge werden beim Speichern hinzugefügt.
          </p>
        )}

        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", position: "relative" }}>
          Tags
          <input
            value={tagsInput}
            onChange={(e) => setTagsInput(e.target.value)}
            onKeyDown={handleTagsKeyDown}
            onBlur={() => setTimeout(() => setTagSuggestionsOpen(false), 150)}
            placeholder="Tags, durch Komma getrennt"
            autoComplete="off"
          />
          {tagSuggestionsOpen && tagSuggestions.length > 0 && (
            <ul
              style={{
                position: "absolute",
                top: "100%",
                left: 0,
                right: 0,
                marginTop: "0.2rem",
                zIndex: 10,
                listStyle: "none",
                padding: "0.25rem 0",
                margin: 0,
                background: "var(--bg-elevated)",
                border: "1px solid var(--border)",
                borderRadius: "var(--radius-sm)",
                boxShadow: "var(--shadow-sm)",
                maxHeight: "10rem",
                overflowY: "auto",
              }}
            >
              {tagSuggestions.map((tag, i) => (
                <li
                  key={tag}
                  className="list-row"
                  onMouseEnter={() => setTagHighlightIndex(i)}
                  onMouseDown={(e) => {
                    // mousedown (not click) fires before the input's onBlur, so the
                    // suggestion can still be applied before onBlur closes the list.
                    e.preventDefault();
                    applyTagSuggestion(tag);
                  }}
                  style={{
                    padding: "0.3rem 0.6rem",
                    cursor: "pointer",
                    background: i === tagHighlightIndex ? "var(--bg-selected)" : "transparent",
                    fontSize: "0.85rem",
                  }}
                >
                  {tag}
                </li>
              ))}
            </ul>
          )}
        </label>

        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}

        <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end", marginTop: "0.25rem" }}>
          <button type="button" onClick={cancel}>
            Abbrechen
          </button>
          <button type="submit" disabled={loadingEntry}>
            Speichern
          </button>
        </div>
      </form>
    </Modal>
  );
}
