import { useEffect } from "react";
import Modal from "./Modal";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { useAppStore } from "../state/appStore";

interface ShortcutRow {
  keys: string;
  description: string;
}

// Static content — kept in sync with the real per-view bindings (see
// SHORTCUTS.md at the repo root for the fully detailed, view-by-view
// reference; this is the compact in-app version).
const SHORTCUTS: ShortcutRow[] = [
  { keys: "Strg+K", description: "Command Palette öffnen" },
  { keys: "Strg+N", description: "Schnellerfassungsfenster öffnen — für einen Eintrag im Hauptfenster: Strg+K → „Neuer Eintrag“" },
  { keys: "Strg+S", description: "Speichern (im geöffneten Editor)" },
  { keys: "/", description: "Noch nicht gebunden" },
  { keys: "g c", description: "Zu Kundenliste" },
  { keys: "g s", description: "Zu Systemliste des aktuellen Kunden" },
  { keys: "g j", description: "Zum Journal" },
  { keys: "j / k", description: "Liste abwärts / aufwärts" },
  { keys: "Enter", description: "Auswählen/öffnen — wirkt je nach Ansicht unterschiedlich (siehe SHORTCUTS.md)" },
  { keys: "e", description: "Ausgewählten Eintrag/Kunde/System bearbeiten" },
  { keys: "Strg+V", description: "Screenshot aus Zwischenablage als Anhang" },
  { keys: "Esc", description: "Abbrechen, Ebene zurück" },
  { keys: "?", description: "Diese Shortcut-Übersicht" },
];

export default function ShortcutOverview() {
  const open = useAppStore((s) => s.shortcutOverviewOpen);
  const openOverview = useAppStore((s) => s.openShortcutOverview);
  const closeOverview = useAppStore((s) => s.closeShortcutOverview);

  // Self-contained listener, same style as the quick-capture window and the
  // parallel Command Palette work — owns its own "?"/Esc handling rather
  // than being wired into useGlobalHotkeys.ts. The open flag itself lives in
  // the store so Command Palette can also open this dialog.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (open) {
        if (e.key === "Escape") {
          e.preventDefault();
          closeOverview();
        }
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (isTypingTarget(document.activeElement)) return;
      if (e.key === "?") {
        e.preventDefault();
        openOverview();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, openOverview, closeOverview]);

  if (!open) return null;

  return (
    <Modal onClose={closeOverview}>
      <h2 style={{ fontSize: "1rem", marginBottom: "0.75rem" }}>Tastaturbelegung</h2>
      <table style={{ borderCollapse: "collapse" }}>
        <tbody>
          {SHORTCUTS.map((row) => (
            <tr key={row.keys}>
              <td
                style={{
                  fontFamily: "var(--font-mono)",
                  color: "var(--text-primary)",
                  padding: "0.3rem 0.75rem 0.3rem 0",
                  whiteSpace: "nowrap",
                  verticalAlign: "top",
                }}
              >
                {row.keys}
              </td>
              <td style={{ padding: "0.3rem 0", color: "var(--text-secondary)" }}>{row.description}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Modal>
  );
}
