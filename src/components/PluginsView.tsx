import NinjaPluginSection from "./NinjaPluginSection";
import LevelPluginSection from "./LevelPluginSection";
import SnipeitPluginSection from "./SnipeitPluginSection";
import JamfPluginSection from "./JamfPluginSection";

// Thin shell: each plugin integration gets its own settings section
// component so this file doesn't become an unmaintainable single file as
// more plugins are added. See NinjaPluginSection.tsx (extracted, unchanged
// behavior), LevelPluginSection.tsx (docs/PLUGIN_ARCHITECTURE.md
// "Level.io-Plugin"), SnipeitPluginSection.tsx (third integration, see
// docs/PLUGIN_ARCHITECTURE.md "Snipe-IT-Plugin"), and JamfPluginSection.tsx
// (fourth integration, see docs/PLUGIN_ARCHITECTURE.md "Jamf-Pro-Plugin").
export default function PluginsView() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "2rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Plugins</h1>
      <NinjaPluginSection />
      <LevelPluginSection />
      <SnipeitPluginSection />
      <JamfPluginSection />
    </div>
  );
}
