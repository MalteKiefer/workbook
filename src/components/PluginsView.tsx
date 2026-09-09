import NinjaPluginSection from "./NinjaPluginSection";
import LevelPluginSection from "./LevelPluginSection";
import SnipeitPluginSection from "./SnipeitPluginSection";
import IntunePluginSection from "./IntunePluginSection";
import IruPluginSection from "./IruPluginSection";
import JamfPluginSection from "./JamfPluginSection";
import AbmPluginSection from "./AbmPluginSection";
import TacticalRmmPluginSection from "./TacticalRmmPluginSection";

// Thin shell: each plugin integration gets its own settings section
// component so this file doesn't become an unmaintainable single file as
// more plugins are added. See NinjaPluginSection.tsx (extracted, unchanged
// behavior), LevelPluginSection.tsx (docs/PLUGIN_ARCHITECTURE.md
// "Level.io-Plugin"), SnipeitPluginSection.tsx (third integration, see
// docs/PLUGIN_ARCHITECTURE.md "Snipe-IT-Plugin"), IntunePluginSection.tsx
// (fourth integration, see docs/PLUGIN_ARCHITECTURE.md
// "Microsoft-Intune-Plugin"), IruPluginSection.tsx (fifth integration, see
// docs/PLUGIN_ARCHITECTURE.md "Iru-Plugin"), JamfPluginSection.tsx (sixth
// integration, see docs/PLUGIN_ARCHITECTURE.md "Jamf-Pro-Plugin"),
// AbmPluginSection.tsx (seventh integration, Apple Business Manager, see
// docs/PLUGIN_ARCHITECTURE.md "Apple-Business-Manager-Plugin"), and
// TacticalRmmPluginSection.tsx (eighth integration, see
// docs/PLUGIN_ARCHITECTURE.md "Tactical-RMM-Plugin").
export default function PluginsView() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "2rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Plugins</h1>
      <NinjaPluginSection />
      <LevelPluginSection />
      <SnipeitPluginSection />
      <IntunePluginSection />
      <IruPluginSection />
      <JamfPluginSection />
      <AbmPluginSection />
      <TacticalRmmPluginSection />
    </div>
  );
}
