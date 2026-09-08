import { useGlobalHotkeys } from "./hooks/useGlobalHotkeys";
import { useAppStore } from "./state/appStore";
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

function NavButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      style={{
        background: active ? "var(--bg-hover)" : "transparent",
        border: "1px solid transparent",
        color: active ? "var(--text-primary)" : "var(--text-secondary)",
        fontWeight: active ? 600 : 500,
      }}
    >
      {children}
    </button>
  );
}

export default function App() {
  useGlobalHotkeys();
  const view = useAppStore((s) => s.view);
  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const goToJournal = useAppStore((s) => s.goToJournal);
  const goToSettings = useAppStore((s) => s.goToSettings);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <header
        style={{
          display: "flex",
          alignItems: "center",
          gap: "0.5rem",
          padding: "0 0.75rem",
          height: "44px",
          flexShrink: 0,
          borderBottom: "1px solid var(--border)",
          background: "var(--bg-surface)",
        }}
      >
        <span style={{ fontWeight: 600, fontSize: "0.85rem", letterSpacing: "-0.01em", marginRight: "0.5rem" }}>
          Wartungsdoku
        </span>
        <NavButton active={view === "customers" || view === "systems"} onClick={goToCustomers}>
          Kunden
        </NavButton>
        <NavButton active={view === "journal"} onClick={goToJournal}>
          Journal
        </NavButton>
        <NavButton active={view === "settings"} onClick={() => goToSettings()}>
          Einstellungen
        </NavButton>
        <span style={{ marginLeft: "auto", fontSize: "0.75rem", color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>
          Strg+K
        </span>
      </header>
      <main style={{ flex: 1, overflow: "auto", padding: "1.25rem" }}>
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
