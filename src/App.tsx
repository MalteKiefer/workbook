import { useGlobalHotkeys } from "./hooks/useGlobalHotkeys";
import { useAppStore } from "./state/appStore";
import CustomerListView from "./components/CustomerListView";

export default function App() {
  useGlobalHotkeys();
  const view = useAppStore((s) => s.view);

  return (
    <main style={{ fontFamily: "sans-serif", padding: "1rem" }}>
      {view === "customers" && <CustomerListView />}
      {view === "systems" && <p>Systemliste folgt (Phase 4b).</p>}
      {view === "journal" && <p>Journal folgt (Phase 4d).</p>}
    </main>
  );
}
