import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { formatInvokeError } from "../lib/errors";
import {
  getUpdateCheckSettings,
  recordUpdateCheckResult,
  setAutoUpdateCheckSettings,
  type AutoUpdateCheckFrequency,
  type UpdateCheckSettingsDto,
} from "../lib/updateCheck";

type CheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up-to-date" }
  | { kind: "available"; update: Update }
  | { kind: "error"; message: string };

type InstallState = { kind: "idle" } | { kind: "installing"; percent: number | null } | { kind: "error"; message: string };

function formatCheckTimestamp(iso: string | null): string {
  if (iso === null) return "noch nie";
  return new Date(iso).toLocaleString("de-DE");
}

export default function UpdateSettingsView() {
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [checkState, setCheckState] = useState<CheckState>({ kind: "idle" });
  const [installState, setInstallState] = useState<InstallState>({ kind: "idle" });

  const [settings, setSettings] = useState<UpdateCheckSettingsDto | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [settingsBusy, setSettingsBusy] = useState(false);

  async function loadSettings() {
    try {
      const loaded = await getUpdateCheckSettings();
      setSettings(loaded);
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    }
  }

  useEffect(() => {
    let cancelled = false;
    void getVersion()
      .then((v) => {
        if (!cancelled) setCurrentVersion(v);
      })
      .catch((e) => {
        if (!cancelled) setCheckState({ kind: "error", message: formatInvokeError(e) });
      });
    void loadSettings();
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleCheck() {
    setCheckState({ kind: "checking" });
    try {
      const update = await check();
      // Records the outcome the same way an automatic check would, so
      // config.toml, the tray tooltip, and every window's badge stay in
      // sync regardless of whether this manual check or the background
      // scheduler found it -- see src/lib/updateCheck.ts's module doc.
      await recordUpdateCheckResult(update ? update.version : null);
      void loadSettings();
      if (update) {
        setCheckState({ kind: "available", update });
      } else {
        setCheckState({ kind: "up-to-date" });
      }
    } catch (e) {
      setCheckState({ kind: "error", message: formatInvokeError(e) });
    }
  }

  async function handleToggleAutoCheck(enabled: boolean) {
    if (settings === null) return;
    setSettingsError(null);
    setSettingsBusy(true);
    try {
      await setAutoUpdateCheckSettings(enabled, settings.frequency);
      await loadSettings();
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function handleFrequencyChange(frequency: AutoUpdateCheckFrequency) {
    if (settings === null) return;
    setSettingsError(null);
    setSettingsBusy(true);
    try {
      await setAutoUpdateCheckSettings(settings.enabled, frequency);
      await loadSettings();
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function handleInstall(update: Update) {
    setInstallState({ kind: "installing", percent: null });
    let downloaded = 0;
    let contentLength = 0;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          contentLength = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setInstallState({
            kind: "installing",
            percent: contentLength > 0 ? Math.round((downloaded / contentLength) * 100) : null,
          });
        }
      });
      await relaunch();
    } catch (e) {
      setInstallState({ kind: "error", message: formatInvokeError(e) });
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "36rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Aktualisierung</h1>

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
        <p style={{ margin: 0, fontSize: "0.85rem", color: "var(--text-secondary)" }}>
          Installierte Version: <strong style={{ color: "var(--text-primary)" }}>{currentVersion ?? "…"}</strong>
        </p>

        <div>
          <button type="button" className="btn-primary" disabled={checkState.kind === "checking"} onClick={() => void handleCheck()}>
            Nach Updates suchen
          </button>
        </div>

        {checkState.kind === "checking" && <p style={{ margin: 0, color: "var(--text-muted)", fontSize: "0.85rem" }}>Suche…</p>}
        {checkState.kind === "up-to-date" && (
          <p style={{ margin: 0, color: "var(--success)", fontSize: "0.85rem" }}>Aktuell — keine Updates verfügbar.</p>
        )}
        {checkState.kind === "error" && (
          <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.85rem" }}>Fehler: {checkState.message}</p>
        )}
        {checkState.kind === "available" && (
          <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
            <p style={{ margin: 0, fontSize: "0.85rem" }}>
              Version <strong>{checkState.update.version}</strong> verfügbar (aktuell {currentVersion}).
            </p>
            {checkState.update.body && (
              <p style={{ margin: 0, fontSize: "0.8rem", color: "var(--text-secondary)", whiteSpace: "pre-wrap" }}>
                {checkState.update.body}
              </p>
            )}
            <div>
              <button
                type="button"
                className="btn-primary"
                disabled={installState.kind === "installing"}
                onClick={() => void handleInstall(checkState.update)}
              >
                Installieren und neu starten
              </button>
            </div>
            {installState.kind === "installing" && (
              <p style={{ margin: 0, color: "var(--text-muted)", fontSize: "0.85rem" }}>
                Installiere{installState.percent !== null ? ` (${installState.percent}%)` : "…"}
              </p>
            )}
            {installState.kind === "error" && (
              <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.85rem" }}>Fehler: {installState.message}</p>
            )}
          </div>
        )}
      </section>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.75rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Automatische Suche</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Sucht regelmäßig im Hintergrund nach neuen Versionen, ohne etwas zu installieren. Ist eine neuere Version
          verfügbar, zeigen Navigation, Tray-Symbol und dieser Tab einen Hinweis; installiert wird weiterhin nur
          manuell über den Knopf oben.
        </p>

        {settings === null ? (
          <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade Einstellungen…</p>
        ) : (
          <>
            <label style={{ display: "flex", alignItems: "center", gap: "0.5rem", fontSize: "0.9rem" }}>
              <input
                type="checkbox"
                checked={settings.enabled}
                disabled={settingsBusy}
                onChange={(e) => void handleToggleAutoCheck(e.target.checked)}
              />
              Automatische Suche aktivieren
            </label>

            <label style={{ display: "flex", alignItems: "center", gap: "0.5rem", fontSize: "0.9rem" }}>
              Häufigkeit:
              <select
                value={settings.frequency}
                disabled={settingsBusy}
                onChange={(e) => void handleFrequencyChange(e.target.value as AutoUpdateCheckFrequency)}
              >
                <option value="daily">Täglich</option>
                <option value="weekly">Wöchentlich</option>
                <option value="monthly">Monatlich</option>
              </select>
            </label>

            <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.82rem" }}>
              Letzte Suche: {formatCheckTimestamp(settings.last_run_utc)}
            </p>
          </>
        )}
        {settingsError && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {settingsError}</p>}
      </section>
    </div>
  );
}
