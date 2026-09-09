import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Kept in sync with src-tauri/src/config.rs::ThemePreference (serde
// rename_all = "snake_case" -> lowercase strings over the Tauri IPC bridge).
export type ThemePreference = "light" | "dark" | "system";

function applyThemeAttribute(preference: ThemePreference): void {
  const root = document.documentElement;
  if (preference === "system") {
    // No attribute at all -- src/styles/theme.css's
    // `@media (prefers-color-scheme: light) { :root:not([data-theme="dark"]) {...} }`
    // then decides based on the OS preference, defaulting to the dark
    // :root block when the OS is dark (or reports no preference).
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", preference);
  }
}

// Reads the persisted theme preference and applies it to <html> as
// data-theme. Fired-and-forgotten from both entry points (src/main.tsx and
// src/quick-capture/main.tsx) right away, without blocking first render --
// each is its own separate webview/document in Tauri, so each has to apply
// this independently. If the read fails for any reason, do nothing and keep
// the default dark appearance (src/styles/theme.css's unconditional :root
// block) rather than crash startup over a theme preference read.
export async function applyPersistedTheme(): Promise<void> {
  try {
    const preference = await invoke<ThemePreference>("get_theme_preference");
    applyThemeAttribute(preference);
  } catch {
    // No crash on startup because of a failed theme lookup -- default stays
    // dark (see comment above).
  }
}

// Subscribes to the "theme-changed" event (emitted app-wide by the
// set_theme_preference command, see src-tauri/src/commands/settings.rs) so
// an already-open window updates live when the setting is changed elsewhere
// -- e.g. the main window changes it while the Schnellerfassung
// (quick-capture) popup is already open, or vice versa.
export function listenForThemeChanges(): void {
  void listen<ThemePreference>("theme-changed", (event) => {
    applyThemeAttribute(event.payload);
  });
}
