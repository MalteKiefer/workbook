import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";
import Modal from "./Modal";

interface System {
  id: number;
  customer_id: number;
  name: string;
  system_type: string;
  hostname: string;
  ip_address: string;
  notes: string;
}

// Curated Typ choices covering both physical/network assets (which have a
// meaningful Hostname/IP) and cloud/SaaS services (which don't) -- "Sonstiges"
// keeps the field freeform for anything that doesn't fit, and also catches
// legacy values (e.g. a system created before this list existed, or via the
// Ninja/Level plugins' "Als neues System anlegen" flow with a type string
// that isn't one of these). system_type stays a plain string column in the
// DB either way; this is purely a form-presentation choice.
const CURATED_SYSTEM_TYPES = ["Server", "Workstation", "Netzwerkgerät", "Drucker", "Firewall", "SaaS / Cloud-Dienst"];
const OTHER_TYPE = "Sonstiges";

// Hostname/IP-Adresse don't apply to a cloud/SaaS service the way they do to
// a physical or network asset -- hiding them for that one Typ keeps the form
// honest about which fields are actually meaningful, rather than always
// showing two fields that are irrelevant for e.g. "ESET PROTECT Cloud".
function typeHasNetworkFields(typeSelection: string): boolean {
  return typeSelection !== "SaaS / Cloud-Dienst";
}

// Globally mounted (see App.tsx), same pattern as CustomerForm.tsx/EntryEditor.tsx:
// driven entirely by the store's systemEditorTarget/systemEditorCustomerId rather
// than local per-view state, so it can be opened from anywhere (list row,
// keyboard shortcut, Command Palette) regardless of which view is active.
export default function SystemForm() {
  const systemEditorTarget = useAppStore((s) => s.systemEditorTarget);
  const systemEditorCustomerId = useAppStore((s) => s.systemEditorCustomerId);
  const closeSystemEditor = useAppStore((s) => s.closeSystemEditor);
  const formOpenInStore = useAppStore((s) => s.formOpen);
  const openForm = useAppStore((s) => s.openForm);
  const closeForm = useAppStore((s) => s.closeForm);

  const [name, setName] = useState("");
  const [typeSelection, setTypeSelection] = useState("");
  const [customType, setCustomType] = useState("");
  const [hostname, setHostname] = useState("");
  const [ipAddress, setIpAddress] = useState("");
  const [notes, setNotes] = useState("");
  const [error, setError] = useState<string | null>(null);

  const isEditMode = typeof systemEditorTarget === "number";

  useEffect(() => {
    if (systemEditorTarget === null || systemEditorCustomerId === null) return;
    setError(null);

    if (systemEditorTarget === "new") {
      setName("");
      setTypeSelection("");
      setCustomType("");
      setHostname("");
      setIpAddress("");
      setNotes("");
      return;
    }

    invoke<System[]>("list_systems", { customerId: systemEditorCustomerId, includeArchived: true })
      .then((list) => {
        const match = list.find((s) => s.id === systemEditorTarget);
        if (match) {
          setName(match.name);
          if (match.system_type === "") {
            setTypeSelection("");
            setCustomType("");
          } else if (CURATED_SYSTEM_TYPES.includes(match.system_type)) {
            setTypeSelection(match.system_type);
            setCustomType("");
          } else {
            setTypeSelection(OTHER_TYPE);
            setCustomType(match.system_type);
          }
          setHostname(match.hostname);
          setIpAddress(match.ip_address);
          setNotes(match.notes);
        }
      })
      .catch((e) => setError(String(e)));
  }, [systemEditorTarget, systemEditorCustomerId]);

  useEffect(() => {
    if (systemEditorTarget === null) return;
    openForm();
    return () => closeForm();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [systemEditorTarget]);

  useEffect(() => {
    if (!formOpenInStore && systemEditorTarget !== null) {
      closeSystemEditor();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formOpenInStore]);

  function cancel() {
    closeForm();
    closeSystemEditor();
  }

  const showNetworkFields = typeHasNetworkFields(typeSelection);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (systemEditorCustomerId === null) return;
    const systemType = typeSelection === OTHER_TYPE ? customType : typeSelection;
    // A SaaS/Cloud-Dienst system has no meaningful Hostname/IP -- clear
    // whatever was there before (e.g. left over from switching Typ after
    // already having typed something) rather than silently persisting it.
    const effectiveHostname = showNetworkFields ? hostname : "";
    const effectiveIpAddress = showNetworkFields ? ipAddress : "";
    try {
      if (isEditMode) {
        await invoke("update_system", {
          id: systemEditorTarget,
          input: { name, system_type: systemType, hostname: effectiveHostname, ip_address: effectiveIpAddress, notes },
        });
      } else {
        await invoke("create_system", {
          input: {
            customer_id: systemEditorCustomerId,
            name,
            system_type: systemType,
            hostname: effectiveHostname,
            ip_address: effectiveIpAddress,
            notes,
          },
        });
      }
      closeForm();
      closeSystemEditor();
    } catch (err) {
      setError(String(err));
    }
  }

  if (systemEditorTarget === null) return null;

  return (
    <Modal onClose={cancel}>
      <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "0.5rem", minWidth: "20rem" }}>
        <h2 style={{ margin: 0, fontSize: "1rem" }}>{isEditMode ? "System bearbeiten" : "Neues System"}</h2>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} required autoFocus />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Typ
          <select value={typeSelection} onChange={(e) => setTypeSelection(e.target.value)} required>
            <option value="" disabled>
              Typ wählen…
            </option>
            {CURATED_SYSTEM_TYPES.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
            <option value={OTHER_TYPE}>{OTHER_TYPE}</option>
          </select>
        </label>
        {typeSelection === OTHER_TYPE && (
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            Typ (frei)
            <input value={customType} onChange={(e) => setCustomType(e.target.value)} required autoFocus />
          </label>
        )}
        {showNetworkFields && (
          <>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              Hostname
              <input value={hostname} onChange={(e) => setHostname(e.target.value)} style={{ fontFamily: "var(--font-mono)" }} />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
              IP-Adresse
              <input value={ipAddress} onChange={(e) => setIpAddress(e.target.value)} style={{ fontFamily: "var(--font-mono)" }} />
            </label>
          </>
        )}
        <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
          Notizen
          <textarea value={notes} onChange={(e) => setNotes(e.target.value)} rows={3} />
        </label>
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
        <div style={{ display: "flex", gap: "0.5rem", justifyContent: "flex-end", marginTop: "0.25rem" }}>
          <button type="button" onClick={cancel}>
            Abbrechen
          </button>
          <button type="submit">Speichern</button>
        </div>
      </form>
    </Modal>
  );
}
