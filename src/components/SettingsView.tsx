import { useAppStore } from "../state/appStore";
import GeneralSettingsView from "./GeneralSettingsView";
import BackupView from "./BackupView";
import PluginsView from "./PluginsView";
import KeymapSettingsView from "./KeymapSettingsView";
import UpdateSettingsView from "./UpdateSettingsView";
import TemplatesSettingsView from "./TemplatesSettingsView";

function TabButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      style={{
        background: active ? "var(--bg-hover)" : "transparent",
        border: "1px solid " + (active ? "var(--border)" : "transparent"),
        color: active ? "var(--text-primary)" : "var(--text-secondary)",
        fontWeight: active ? 600 : 500,
      }}
    >
      {children}
    </button>
  );
}

export default function SettingsView() {
  const settingsTab = useAppStore((s) => s.settingsTab);
  const setSettingsTab = useAppStore((s) => s.setSettingsTab);
  const updateAvailableVersion = useAppStore((s) => s.updateAvailableVersion);

  return (
    <div>
      <h1 style={{ fontSize: "1.1rem", marginBottom: "0.75rem" }}>Einstellungen</h1>
      <div
        style={{
          display: "flex",
          gap: "0.4rem",
          marginBottom: "1rem",
          paddingBottom: "0.6rem",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <TabButton active={settingsTab === "general"} onClick={() => setSettingsTab("general")}>
          Allgemein
        </TabButton>
        <TabButton active={settingsTab === "backup"} onClick={() => setSettingsTab("backup")}>
          Backup
        </TabButton>
        <TabButton active={settingsTab === "plugins"} onClick={() => setSettingsTab("plugins")}>
          Plugins
        </TabButton>
        <TabButton active={settingsTab === "keymap"} onClick={() => setSettingsTab("keymap")}>
          Tastaturbelegung
        </TabButton>
        <TabButton active={settingsTab === "templates"} onClick={() => setSettingsTab("templates")}>
          Vorlagen
        </TabButton>
        <TabButton active={settingsTab === "update"} onClick={() => setSettingsTab("update")}>
          Aktualisierung
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
        </TabButton>
      </div>
      {settingsTab === "general" && <GeneralSettingsView />}
      {settingsTab === "backup" && <BackupView />}
      {settingsTab === "plugins" && <PluginsView />}
      {settingsTab === "keymap" && <KeymapSettingsView />}
      {settingsTab === "templates" && <TemplatesSettingsView />}
      {settingsTab === "update" && <UpdateSettingsView />}
    </div>
  );
}
