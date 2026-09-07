import { useGlobalHotkeys } from "./hooks/useGlobalHotkeys";
import { useAppStore } from "./state/appStore";
import CustomerListView from "./components/CustomerListView";
import SystemListView from "./components/SystemListView";
import JournalView from "./components/JournalView";
import CommandPalette from "./components/CommandPalette";
import ShortcutOverview from "./components/ShortcutOverview";
import EntryEditor from "./components/EntryEditor";

export default function App() {
  useGlobalHotkeys();
  const view = useAppStore((s) => s.view);

  return (
    <main style={{ fontFamily: "sans-serif", padding: "1rem" }}>
      {view === "customers" && <CustomerListView />}
      {view === "systems" && <SystemListView />}
      {view === "journal" && <JournalView />}
      <CommandPalette />
      <ShortcutOverview />
      <EntryEditor />
    </main>
  );
}
