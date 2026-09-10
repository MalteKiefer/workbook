import { useEffect } from "react";
import { useGlobalHotkeys } from "./hooks/useGlobalHotkeys";
import { useAppStore } from "./state/appStore";
import { getUpdateCheckSettings, listenForOpenUpdateSettings, listenForUpdateCheckCompleted } from "./lib/updateCheck";
import DashboardView from "./components/DashboardView";
import CustomerListView from "./components/CustomerListView";
import SystemListView from "./components/SystemListView";
import JournalView from "./components/JournalView";
import SettingsView from "./components/SettingsView";
import CommandPalette from "./components/CommandPalette";
import ShortcutOverview from "./components/ShortcutOverview";
import EntryEditor from "./components/EntryEditor";
import EntryDetailModal from "./components/EntryDetailModal";
import ExportDialog from "./components/ExportDialog";
import CustomerForm from "./components/CustomerForm";
import SystemForm from "./components/SystemForm";
import { useKeymap, formatBindingForDisplay } from "./lib/keymap";

function NavLink({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      style={{
        display: "block",
        width: "100%",
        textAlign: "left",
        background: active ? "var(--bg-hover)" : "transparent",
        border: "none",
        borderLeft: active ? "2px solid var(--accent)" : "2px solid transparent",
        borderRadius: 0,
        color: active ? "var(--accent)" : "var(--text-secondary)",
        fontWeight: active ? 600 : 500,
        padding: "0.45rem 0.7rem",
      }}
    >
      {children}
    </button>
  );
}

export default function App() {
  useGlobalHotkeys();
  const keymap = useKeymap();
  const view = useAppStore((s) => s.view);
  const goToDashboard = useAppStore((s) => s.goToDashboard);
  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const goToJournal = useAppStore((s) => s.goToJournal);
  const goToSettings = useAppStore((s) => s.goToSettings);
  const updateAvailableVersion = useAppStore((s) => s.updateAvailableVersion);
  const setUpdateAvailableVersion = useAppStore((s) => s.setUpdateAvailableVersion);

  useEffect(() => {
    void getUpdateCheckSettings()
      .then((settings) => setUpdateAvailableVersion(settings.available_version))
      .catch(() => {
        // No crash on startup because of a failed settings lookup -- the
        // badge just stays hidden until the next successful check.
      });
    listenForUpdateCheckCompleted(setUpdateAvailableVersion);
    listenForOpenUpdateSettings(() => goToSettings("update"));
  }, [setUpdateAvailableVersion, goToSettings]);

  return (
    <div style={{ display: "flex", height: "100vh" }}>
      <nav
        style={{
          display: "flex",
          flexDirection: "column",
          width: "180px",
          flexShrink: 0,
          background: "var(--bg-surface)",
          borderRight: "1px solid var(--border)",
          padding: "0.75rem 0.6rem",
          gap: "0.15rem",
        }}
      >
        <div
          style={{
            fontWeight: 600,
            fontSize: "0.85rem",
            fontFamily: "var(--font-mono)",
            letterSpacing: "-0.01em",
            padding: "0 0.7rem 0.9rem",
          }}
        >
          Wartungsdoku
        </div>
        <NavLink active={view === "dashboard"} onClick={goToDashboard}>
          Dashboard
        </NavLink>
        <NavLink active={view === "customers" || view === "systems"} onClick={goToCustomers}>
          Kunden
        </NavLink>
        <NavLink active={view === "journal"} onClick={goToJournal}>
          Journal
        </NavLink>
        <NavLink active={view === "settings"} onClick={() => goToSettings()}>
          Einstellungen
          {updateAvailableVersion !== null && (
            <span
              title={`Update ${updateAvailableVersion} verfügbar`}
              style={{
                display: "inline-block",
                width: "6px",
                height: "6px",
                borderRadius: "50%",
                background: "var(--accent)",
                marginLeft: "0.4rem",
                verticalAlign: "middle",
              }}
            />
          )}
        </NavLink>
        <div
          style={{
            marginTop: "auto",
            paddingTop: "0.6rem",
            borderTop: "1px solid var(--border-subtle)",
            display: "flex",
            flexDirection: "column",
            gap: "0.3rem",
            fontSize: "0.7rem",
            color: "var(--text-muted)",
            fontFamily: "var(--font-mono)",
            padding: "0.6rem 0.7rem 0",
          }}
        >
          <span>{formatBindingForDisplay(keymap.command_palette)} · Befehle</span>
          <span>? · Hilfe</span>
        </div>
      </nav>
      <main style={{ flex: 1, minWidth: 0, overflow: "auto", padding: "1.25rem" }}>
        {view === "dashboard" && <DashboardView />}
        {view === "customers" && <CustomerListView />}
        {view === "systems" && <SystemListView />}
        {view === "journal" && <JournalView />}
        {view === "settings" && <SettingsView />}
      </main>
      <CommandPalette />
      <ShortcutOverview />
      <EntryEditor />
      <EntryDetailModal />
      <ExportDialog />
      <CustomerForm />
      <SystemForm />
    </div>
  );
}
