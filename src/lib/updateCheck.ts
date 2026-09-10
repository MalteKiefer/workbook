import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Kept in sync with src-tauri/src/config.rs::AutoUpdateCheckFrequency
// (serde rename_all = "snake_case" -> lowercase strings over the Tauri IPC
// bridge).
export type AutoUpdateCheckFrequency = "daily" | "weekly" | "monthly";

export interface UpdateCheckSettingsDto {
  enabled: boolean;
  frequency: AutoUpdateCheckFrequency;
  last_run_utc: string | null;
  available_version: string | null;
}

export function getUpdateCheckSettings(): Promise<UpdateCheckSettingsDto> {
  return invoke<UpdateCheckSettingsDto>("get_update_check_settings");
}

export function setAutoUpdateCheckSettings(enabled: boolean, frequency: AutoUpdateCheckFrequency): Promise<void> {
  return invoke("set_auto_update_check_settings", { enabled, frequency });
}

export function recordUpdateCheckResult(availableVersion: string | null): Promise<void> {
  return invoke("record_update_check_result", { availableVersion });
}

// Subscribes to the "update-check-completed" event, emitted app-wide by
// both the background scheduler (src-tauri/src/lib.rs::run_auto_update_check_if_due)
// and a manual check (record_update_check_result above) -- so every open
// window's badge reflects the outcome live, regardless of which one
// actually ran the check. See src/lib/theme.ts::listenForThemeChanges for
// the same app-wide-broadcast pattern.
export function listenForUpdateCheckCompleted(onCompleted: (availableVersion: string | null) => void): void {
  void listen<string | null>("update-check-completed", (event) => {
    onCompleted(event.payload);
  });
}

// Subscribes to "open-update-settings", emitted by the tray's "Nach
// Updates suchen" menu item (src-tauri/src/tray.rs) after it shows and
// focuses the main window -- lets that click land directly on the
// Aktualisierung settings tab instead of wherever the user last was.
export function listenForOpenUpdateSettings(onOpen: () => void): void {
  void listen("open-update-settings", () => {
    onOpen();
  });
}
