import { useEffect } from "react";
import Modal from "./Modal";
import { isTypingTarget } from "../hooks/useGlobalHotkeys";
import { useAppStore } from "../state/appStore";
import { formatShortcut } from "../lib/platform";
import { formatBindingForDisplay, useKeymap } from "../lib/keymap";
import type { Keymap } from "../lib/keymap";

interface ShortcutRow {
  keys: string;
  description: string;
}

// Kept in sync with the real per-view bindings (see SHORTCUTS.md at the repo
// root for the fully detailed, view-by-view reference); this is the compact
// in-app version. A function rather than a module-level constant so the
// Cmd/Strg labels reflect the platform this window is actually running on.
function shortcuts(keymap: Keymap): ShortcutRow[] {
  return [
    { keys: formatBindingForDisplay(keymap.command_palette), description: "Command Palette öffnen" },
    {
      keys: formatBindingForDisplay(keymap.quick_capture),
      description: `Schnellerfassungsfenster öffnen — für einen Eintrag im Hauptfenster: ${formatBindingForDisplay(keymap.command_palette)} → „Neuer Eintrag“`,
    },
    { keys: formatBindingForDisplay(keymap.save), description: "Speichern (im geöffneten Editor)" },
    { keys: "/", description: "Noch nicht gebunden" },
    { keys: keymap.goto_customers, description: "Zu Kundenliste" },
    { keys: keymap.goto_systems, description: "Zu Systemliste des aktuellen Kunden" },
    { keys: keymap.goto_journal, description: "Zum Journal" },
    { keys: `${keymap.list_next} / ${keymap.list_prev}`, description: "Liste abwärts / aufwärts" },
    { keys: "Enter", description: "Auswählen/öffnen — wirkt je nach Ansicht unterschiedlich (siehe SHORTCUTS.md)" },
    { keys: keymap.edit_selected, description: "Ausgewählten Eintrag/Kunde/System bearbeiten" },
    { keys: formatShortcut("V"), description: "Screenshot aus Zwischenablage als Anhang" },
    { keys: "Esc", description: "Abbrechen, Ebene zurück" },
    { keys: "?", description: "Diese Shortcut-Übersicht" },
  ];
}

export default function ShortcutOverview() {
  const open = useAppStore((s) => s.shortcutOverviewOpen);
  const openOverview = useAppStore((s) => s.openShortcutOverview);
  const closeOverview = useAppStore((s) => s.closeShortcutOverview);
  const keymap = useKeymap();

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
          {shortcuts(keymap).map((row) => (
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
      <p style={{ margin: "0.75rem 0 0", fontSize: "0.78rem", color: "var(--text-muted)" }}>
        Anpassbar unter Einstellungen → Tastaturbelegung.
      </p>
    </Modal>
  );
}
