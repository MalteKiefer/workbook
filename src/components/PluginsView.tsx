import AbmPluginSection from "./AbmPluginSection";
import AcronisPluginSection from "./AcronisPluginSection";
import IntunePluginSection from "./IntunePluginSection";
import IruPluginSection from "./IruPluginSection";
import JamfPluginSection from "./JamfPluginSection";
import LevelPluginSection from "./LevelPluginSection";
import NinjaPluginSection from "./NinjaPluginSection";
import SnipeitPluginSection from "./SnipeitPluginSection";
import TacticalRmmPluginSection from "./TacticalRmmPluginSection";

// Thin shell: each plugin integration gets its own settings section
// component so this file doesn't become an unmaintainable single file as
// more plugins are added. Sections are mounted alphabetically by the
// section heading each one renders (Acronis-Verbindungen,
// Apple-Business-Manager-Verbindungen, Intune-Verbindungen,
// Iru-Verbindungen, ...) so the list stays predictable as more plugins are
// added, independent of integration order. See NinjaPluginSection.tsx
// (extracted, unchanged behavior), LevelPluginSection.tsx
// (docs/PLUGIN_ARCHITECTURE.md "Level.io-Plugin"), SnipeitPluginSection.tsx
// (docs/PLUGIN_ARCHITECTURE.md "Snipe-IT-Plugin"), IntunePluginSection.tsx
// (docs/PLUGIN_ARCHITECTURE.md "Microsoft-Intune-Plugin"),
// IruPluginSection.tsx (docs/PLUGIN_ARCHITECTURE.md "Iru-Plugin"),
// JamfPluginSection.tsx (docs/PLUGIN_ARCHITECTURE.md "Jamf-Pro-Plugin"),
// AbmPluginSection.tsx (Apple Business Manager, see
// docs/PLUGIN_ARCHITECTURE.md "Apple-Business-Manager-Plugin"),
// TacticalRmmPluginSection.tsx (docs/PLUGIN_ARCHITECTURE.md
// "Tactical-RMM-Plugin"), and AcronisPluginSection.tsx (backup-status, NOT
// device management; see docs/PLUGIN_ARCHITECTURE.md "Acronis-Plugin").
export default function PluginsView() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "2rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Plugins</h1>
      <AbmPluginSection />
      <AcronisPluginSection />
      <IntunePluginSection />
      <IruPluginSection />
      <JamfPluginSection />
      <LevelPluginSection />
      <NinjaPluginSection />
      <SnipeitPluginSection />
      <TacticalRmmPluginSection />
    </div>
  );
}
