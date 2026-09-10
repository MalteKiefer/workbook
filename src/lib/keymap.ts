import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { isMac } from "./platform";

// Kept in sync with src-tauri/src/config.rs::KeymapConfig field-for-field.
export interface Keymap {
  command_palette: string;
  quick_capture: string;
  save: string;
  goto_customers: string;
  goto_systems: string;
  goto_journal: string;
  list_next: string;
  list_prev: string;
  edit_selected: string;
}

interface ParsedBinding {
  ctrl: boolean;
  meta: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
}

function parseBinding(binding: string): ParsedBinding {
  const parts = binding.split("+").map((p) => p.trim());
  const rawKey = parts[parts.length - 1].toLowerCase();
  const key = rawKey === "space" ? " " : rawKey === "plus" ? "+" : rawKey;
  const mods = parts.slice(0, -1).map((p) => p.toLowerCase());
  return {
    ctrl: mods.includes("ctrl"),
    meta: mods.includes("cmd") || mods.includes("meta"),
    alt: mods.includes("alt"),
    shift: mods.includes("shift"),
    key,
  };
}

// Exact match: a stored "Ctrl+K" requires e.ctrlKey and NOT e.metaKey/altKey/
// shiftKey, never "Ctrl or Cmd" aliasing (see the design spec's self-review
// note — the platform-appropriate literal is what KeymapConfig::default()
// seeds instead). A binding with no modifier token (e.g. "j") requires
// every modifier flag to be false.
export function matchesBinding(e: KeyboardEvent, binding: string): boolean {
  const parsed = parseBinding(binding);
  return (
    e.key.toLowerCase() === parsed.key &&
    e.ctrlKey === parsed.ctrl &&
    e.metaKey === parsed.meta &&
    e.altKey === parsed.alt &&
    e.shiftKey === parsed.shift
  );
}

// Formats a KeyboardEvent's modifiers + key back into the same shape
// matchesBinding parses, e.g. Cmd+Shift+K -> "Cmd+Shift+K". Used by the
// KeyRecorder settings control (Task 12) — matchesBinding and this function
// must stay exact inverses for every combination the recorder can produce.
export function formatBinding(e: KeyboardEvent): string {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.metaKey) mods.push("Cmd");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  const key = e.key === " " ? "Space" : e.key === "+" ? "Plus" : e.key.length === 1 ? e.key.toUpperCase() : e.key;
  return [...mods, key].join("+");
}

// Display label for a stored binding, e.g. "Cmd+K" -> "⌘+K", "Ctrl+K" ->
// "Strg+K". Distinct from src/lib/platform.ts's formatShortcut, which
// formats a hardcoded default from a bare letter rather than an arbitrary
// (possibly user-customized) stored binding string.
export function formatBindingForDisplay(binding: string): string {
  return binding
    .split("+")
    .map((part) => (part === "Cmd" ? "⌘" : part === "Ctrl" ? "Strg" : part))
    .join("+");
}

// Mirrors src-tauri/src/config.rs::KeymapConfig::default() exactly — used
// as this module's initial value before loadKeymap()'s async fetch
// resolves, and by the Settings UI's "Auf Standard zurücksetzen" button.
export function defaultKeymap(): Keymap {
  const primary = isMac() ? "Cmd" : "Ctrl";
  return {
    command_palette: `${primary}+K`,
    quick_capture: `${primary}+N`,
    save: `${primary}+S`,
    goto_customers: "g c",
    goto_systems: "g s",
    goto_journal: "g j",
    list_next: "j",
    list_prev: "k",
    edit_selected: "e",
  };
}

// Module-level singleton, same non-React-state pattern as
// src/lib/theme.ts's DOM attribute — every keydown handler reads this
// synchronously via getKeymap(), not through a React subscription. Each
// window (main, quick capture) is a separate JS bundle/module instance
// (see src/main.tsx / src/quick-capture/main.tsx), so each keeps its own
// copy, kept in sync via loadKeymap()/listenForKeymapChanges() below —
// exactly like applyPersistedTheme()/listenForThemeChanges() already do.
let currentKeymap: Keymap = defaultKeymap();

export function getKeymap(): Keymap {
  return currentKeymap;
}

export async function loadKeymap(): Promise<void> {
  try {
    currentKeymap = await invoke<Keymap>("get_keymap");
  } catch {
    // Keep the platform-appropriate default from defaultKeymap() above
    // rather than crash startup over a failed keymap lookup — same
    // fail-open convention as src/lib/theme.ts::applyPersistedTheme.
  }
}

const subscribers = new Set<() => void>();

function notifySubscribers(): void {
  for (const callback of subscribers) callback();
}

function subscribe(callback: () => void): () => void {
  subscribers.add(callback);
  return () => subscribers.delete(callback);
}

// React hook for components that display a binding at render time (as
// opposed to keydown handlers, which read getKeymap() fresh on every event
// and don't need this) -- re-renders the calling component whenever the
// keymap changes, e.g. after a live "keymap-changed" event.
export function useKeymap(): Keymap {
  return useSyncExternalStore(subscribe, getKeymap);
}

export function listenForKeymapChanges(): void {
  void listen<Keymap>("keymap-changed", (event) => {
    currentKeymap = event.payload;
    notifySubscribers();
  });
}
