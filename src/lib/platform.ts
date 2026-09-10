// macOS uses Cmd (⌘) where Windows/Linux use Ctrl for the same shortcuts —
// they are different physical keys, so a keydown handler that only checks
// `e.ctrlKey` never fires for a Mac user pressing Cmd. `navigator.userAgent`
// sniffing is the standard (if slightly old-school) way to tell them apart
// in a webview; Tauri doesn't inject anything more specific into `window`
// for this, and pulling in `@tauri-apps/plugin-os` for one boolean isn't
// worth the extra dependency.
export function isMac(): boolean {
  return typeof navigator !== "undefined" && /Mac|iPhone|iPad|iPod/.test(navigator.userAgent);
}

// Display label for a single-letter shortcut using this platform's primary
// modifier, e.g. formatShortcut("K") -> "⌘K" on macOS, "Strg+K" elsewhere.
export function formatShortcut(key: string): string {
  return isMac() ? `⌘${key}` : `Strg+${key}`;
}
