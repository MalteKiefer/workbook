import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";
import type { ThemePreference } from "../lib/theme";

const THEME_OPTIONS: { value: ThemePreference; label: string }[] = [
  { value: "light", label: "Hell" },
  { value: "dark", label: "Dunkel" },
  { value: "system", label: "System" },
];

interface AttachmentStorageSummary {
  distinct_file_count: number;
  total_size_bytes: number;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export default function GeneralSettingsView() {
  const [theme, setTheme] = useState<ThemePreference | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [storageSummary, setStorageSummary] = useState<AttachmentStorageSummary | null>(null);
  const [cleanupBusy, setCleanupBusy] = useState(false);
  const [cleanupStatus, setCleanupStatus] = useState<string | null>(null);
  const [cleanupError, setCleanupError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<ThemePreference>("get_theme_preference")
      .then((preference) => {
        if (!cancelled) setTheme(preference);
      })
      .catch((e) => {
        if (!cancelled) setError(formatInvokeError(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const fetchStorageSummary = useCallback(() => {
    return invoke<AttachmentStorageSummary>("get_attachment_storage_summary")
      .then((summary) => {
        setStorageSummary(summary);
      })
      .catch((e) => {
        setCleanupError(formatInvokeError(e));
      });
  }, []);

  useEffect(() => {
    void fetchStorageSummary();
  }, [fetchStorageSummary]);

  async function handleCleanup() {
    if (cleanupBusy) return;
    setCleanupBusy(true);
    setCleanupError(null);
    setCleanupStatus(null);
    try {
      const result = await invoke<{ removed_count: number; removed_bytes: number }>("cleanup_orphans");
      const kb = (result.removed_bytes / 1024).toFixed(1);
      setCleanupStatus(`${result.removed_count} verwaiste Datei(en) entfernt (${kb} KB freigegeben).`);
      await fetchStorageSummary();
    } catch (e) {
      setCleanupError(formatInvokeError(e));
    } finally {
      setCleanupBusy(false);
    }
  }

  async function handleSelect(preference: ThemePreference) {
    if (preference === theme || busy) return;
    setError(null);
    setBusy(true);
    // Update the displayed selection immediately/locally instead of waiting
    // for the "theme-changed" event to round-trip -- the event still fires
    // (for other windows, e.g. an already-open quick capture), see
    // src/lib/theme.ts::listenForThemeChanges.
    setTheme(preference);
    try {
      await invoke("set_theme_preference", { preference });
    } catch (e) {
      setError(formatInvokeError(e));
      // Failed -- fetch the current server state back instead of leaving a
      // display that diverges from what's actually saved.
      try {
        const current = await invoke<ThemePreference>("get_theme_preference");
        setTheme(current);
      } catch {
        // No second error state -- the first error message stays visible.
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "36rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Allgemein</h1>

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
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Theme</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Legt fest, ob die Oberfläche hell, dunkel oder passend zur Systemeinstellung dargestellt wird.
        </p>
        <div style={{ display: "flex", gap: "0.4rem", marginTop: "0.25rem" }}>
          {THEME_OPTIONS.map((option) => {
            const active = theme === option.value;
            return (
              <button
                key={option.value}
                type="button"
                disabled={busy || theme === null}
                className={active ? "btn-primary" : undefined}
                onClick={() => void handleSelect(option.value)}
              >
                {option.label}
              </button>
            );
          })}
        </div>
        {error && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {error}</p>}
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
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Anhänge</h2>
        {storageSummary === null ? (
          <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade…</p>
        ) : (
          <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
            {storageSummary.distinct_file_count} Datei(en), {formatBytes(storageSummary.total_size_bytes)} insgesamt.
          </p>
        )}
        <div>
          <button type="button" disabled={cleanupBusy} onClick={() => void handleCleanup()}>
            Verwaiste Anhänge bereinigen…
          </button>
        </div>
        {cleanupStatus && <p style={{ margin: 0, color: "var(--success)", fontSize: "0.82rem" }}>{cleanupStatus}</p>}
        {cleanupError && <p style={{ margin: 0, color: "var(--danger)", fontSize: "0.82rem" }}>Fehler: {cleanupError}</p>}
      </section>
    </div>
  );
}
