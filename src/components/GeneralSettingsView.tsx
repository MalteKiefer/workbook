import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";
import type { ThemePreference } from "../lib/theme";

const THEME_OPTIONS: { value: ThemePreference; label: string }[] = [
  { value: "light", label: "Hell" },
  { value: "dark", label: "Dunkel" },
  { value: "system", label: "System" },
];

export default function GeneralSettingsView() {
  const [theme, setTheme] = useState<ThemePreference | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  async function handleSelect(preference: ThemePreference) {
    if (preference === theme || busy) return;
    setError(null);
    setBusy(true);
    // Angezeigte Auswahl aktualisiert sofort lokal, statt auf den Umlauf des
    // "theme-changed"-Ereignisses zu warten -- das Ereignis feuert trotzdem
    // (für andere Fenster, z. B. eine bereits offene Schnellerfassung), siehe
    // src/lib/theme.ts::listenForThemeChanges.
    setTheme(preference);
    try {
      await invoke("set_theme_preference", { preference });
    } catch (e) {
      setError(formatInvokeError(e));
      // Fehlgeschlagen -- aktuellen Serverstand zurückholen statt einer
      // Anzeige, die von der tatsächlich gespeicherten Einstellung abweicht.
      try {
        const current = await invoke<ThemePreference>("get_theme_preference");
        setTheme(current);
      } catch {
        // Kein zweiter Fehlerzustand -- die erste Fehlermeldung bleibt sichtbar.
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
    </div>
  );
}
