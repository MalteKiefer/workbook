import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { formatInvokeError } from "../lib/errors";

type CheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up-to-date" }
  | { kind: "available"; update: Update }
  | { kind: "error"; message: string };

type InstallState = { kind: "idle" } | { kind: "installing"; percent: number | null } | { kind: "error"; message: string };

export default function UpdateSettingsView() {
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [checkState, setCheckState] = useState<CheckState>({ kind: "idle" });
  const [installState, setInstallState] = useState<InstallState>({ kind: "idle" });

  useEffect(() => {
    let cancelled = false;
    void getVersion()
      .then((v) => {
        if (!cancelled) setCurrentVersion(v);
      })
      .catch((e) => {
        if (!cancelled) setCheckState({ kind: "error", message: formatInvokeError(e) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleCheck() {
    setCheckState({ kind: "checking" });
    try {
      const update = await check();
      if (update) {
        setCheckState({ kind: "available", update });
      } else {
        setCheckState({ kind: "up-to-date" });
      }
    } catch (e) {
      setCheckState({ kind: "error", message: formatInvokeError(e) });
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
    </div>
  );
}
