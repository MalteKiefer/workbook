import NinjaPluginSection from "./NinjaPluginSection";
import LevelPluginSection from "./LevelPluginSection";

// Thin shell: each RMM plugin gets its own settings section component so this
// file doesn't become an unmaintainable single file as more plugins are
// added. See NinjaPluginSection.tsx (extracted, unchanged behavior) and
// LevelPluginSection.tsx (new, docs/PLUGIN_ARCHITECTURE.md "Level.io-Plugin").
export default function PluginsView() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "2rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Plugins</h1>
      <NinjaPluginSection />
      <LevelPluginSection />
    </div>
  );
}
