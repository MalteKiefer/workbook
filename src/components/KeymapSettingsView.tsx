import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";
import { defaultKeymap, type Keymap } from "../lib/keymap";
import KeyRecorder from "./KeyRecorder";

// Kept in sync with src-tauri/src/config.rs::HotkeyConfig field-for-field.
interface HotkeyConfig {
  quick_capture: string;
  search: string;
  clipboard_screenshot: string;
}

const DEFAULT_HOTKEYS: HotkeyConfig = {
  quick_capture: "Ctrl+Alt+Space",
  search: "Ctrl+Alt+F",
  clipboard_screenshot: "Ctrl+Alt+S",
};

interface KeymapRow {
  key: keyof Keymap;
  label: string;
  mode: "combo" | "sequence";
}

const KEYMAP_ROWS: KeymapRow[] = [
  { key: "command_palette", label: "Command Palette öffnen", mode: "combo" },
  { key: "quick_capture", label: "Schnellerfassung öffnen", mode: "combo" },
  { key: "save", label: "Speichern", mode: "combo" },
  { key: "goto_customers", label: "Zu Kundenliste", mode: "sequence" },
  { key: "goto_systems", label: "Zu Systemliste", mode: "sequence" },
  { key: "goto_journal", label: "Zum Journal", mode: "sequence" },
  { key: "list_next", label: "Liste: nächster Eintrag", mode: "combo" },
  { key: "list_prev", label: "Liste: vorheriger Eintrag", mode: "combo" },
  { key: "edit_selected", label: "Ausgewähltes bearbeiten", mode: "combo" },
];

interface HotkeyRow {
  key: keyof HotkeyConfig;
  label: string;
}

const HOTKEY_ROWS: HotkeyRow[] = [
  { key: "quick_capture", label: "Schnellerfassung öffnen" },
  { key: "search", label: "Fenster anzeigen / Suche" },
  { key: "clipboard_screenshot", label: "Zwischenablage-Screenshot" },
];

export default function KeymapSettingsView() {
  const [keymap, setKeymap] = useState<Keymap | null>(null);
  const [hotkeys, setHotkeysState] = useState<HotkeyConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([invoke<Keymap>("get_keymap"), invoke<HotkeyConfig>("get_hotkeys")])
      .then(([loadedKeymap, loadedHotkeys]) => {
        if (cancelled) return;
        setKeymap(loadedKeymap);
        setHotkeysState(loadedHotkeys);
      })
      .catch((e) => {
        if (!cancelled) setError(formatInvokeError(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function saveKeymap() {
    if (keymap === null) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      await invoke("set_keymap", { keymap });
      setStatus("Gespeichert.");
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  async function saveHotkeys() {
    if (hotkeys === null) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      await invoke("set_hotkeys", { hotkeys });
      setStatus("Gespeichert. Wirkt erst nach einem Neustart der Anwendung.");
    } catch (e) {
      setError(formatInvokeError(e));
    } finally {
      setBusy(false);
    }
  }

  function resetToDefaults() {
    setKeymap(defaultKeymap());
    setHotkeysState(DEFAULT_HOTKEYS);
    setStatus(null);
    setError(null);
  }

  if (keymap === null || hotkeys === null) {
    return (
      <div>
        <h1 style={{ fontSize: "1.1rem" }}>Tastaturbelegung</h1>
        {error ? (
          <p style={{ color: "var(--danger)", fontSize: "0.85rem" }}>Fehler: {error}</p>
        ) : (
          <p style={{ color: "var(--text-muted)" }}>Lädt…</p>
        )}
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "36rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Tastaturbelegung</h1>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Global (systemweit)</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Wirkt auch, wenn Wartungsdoku keinen Fokus hat. Eine Änderung hier wirkt erst nach einem Neustart der Anwendung.
        </p>
        <table style={{ borderCollapse: "collapse", marginTop: "0.25rem" }}>
          <tbody>
            {HOTKEY_ROWS.map((row) => (
              <tr key={row.key}>
                <td style={{ padding: "0.3rem 0.75rem 0.3rem 0", fontSize: "0.85rem" }}>{row.label}</td>
                <td style={{ padding: "0.3rem 0" }}>
                  <KeyRecorder
                    value={hotkeys[row.key]}
                    mode="combo"
                    onChange={(newValue) => setHotkeysState({ ...hotkeys, [row.key]: newValue })}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div>
          <button type="button" className="btn-primary" disabled={busy} onClick={() => void saveHotkeys()}>
            Speichern
          </button>
        </div>
      </section>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>App</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Wirkt nur, während Wartungsdoku im Vordergrund ist. Änderungen hier wirken sofort.
        </p>
        <table style={{ borderCollapse: "collapse", marginTop: "0.25rem" }}>
          <tbody>
            {KEYMAP_ROWS.map((row) => (
              <tr key={row.key}>
                <td style={{ padding: "0.3rem 0.75rem 0.3rem 0", fontSize: "0.85rem" }}>{row.label}</td>
                <td style={{ padding: "0.3rem 0" }}>
                  <KeyRecorder
                    value={keymap[row.key]}
                    mode={row.mode}
                    onChange={(newValue) => setKeymap({ ...keymap, [row.key]: newValue })}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div>
          <button type="button" className="btn-primary" disabled={busy} onClick={() => void saveKeymap()}>
            Speichern
          </button>
        </div>
      </section>

      <div>
        <button type="button" disabled={busy} onClick={resetToDefaults}>
          Auf Standard zurücksetzen
        </button>
        <p style={{ margin: "0.3rem 0 0", fontSize: "0.78rem", color: "var(--text-muted)" }}>
          Setzt beide Tabellen oben auf ihre Standardwerte zurück — erst mit „Speichern“ wird das übernommen.
        </p>
      </div>

      {status && <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{status}</p>}
      {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
    </div>
  );
}
