import type { ComponentType } from "react";
import {
  Printer,
  Server,
  Shield,
  Cpu,
  Camera,
  HardDrive,
  Router,
  Laptop,
  ScanLine,
  Smartphone,
  Network,
  BatteryCharging,
  Box,
  Monitor,
  Cloud,
  Wifi,
  HelpCircle,
} from "lucide-react";

interface IconProps {
  size?: number;
  style?: React.CSSProperties;
}

// Keyed by the exact Typ strings in SystemForm.tsx's CURATED_SYSTEM_TYPES --
// keep these two lists in sync if either changes. A Typ not in this map
// (a custom "Sonstiges" value, or a legacy value from before the curated
// list existed) falls back to HelpCircle rather than rendering nothing.
const DEVICE_TYPE_ICONS: Record<string, ComponentType<IconProps>> = {
  "Access Point": Wifi,
  Drucker: Printer,
  Firewall: Shield,
  IoT: Cpu,
  Kamera: Camera,
  Multifunktionsgerät: Printer,
  NAS: HardDrive,
  Netzwerkgerät: Router,
  Notebook: Laptop,
  Scanner: ScanLine,
  Server: Server,
  "Smartphone/Tablet": Smartphone,
  Switch: Network,
  USV: BatteryCharging,
  VM: Box,
  Workstation: Monitor,
  "SaaS / Cloud-Dienst": Cloud,
};

// Renders the icon for `type` (a System's `system_type` string), falling
// back to a generic help-circle icon for anything not in the curated map
// (a freeform "Sonstiges" value, or a value from before the curated list
// existed).
export function DeviceTypeIcon({ type, size = 14 }: { type: string; size?: number }) {
  const Icon = DEVICE_TYPE_ICONS[type] ?? HelpCircle;
  return <Icon size={size} style={{ flexShrink: 0, color: "var(--text-muted)" }} />;
}
