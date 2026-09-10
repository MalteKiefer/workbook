import { useEffect, useState } from "react";
import { formatBinding } from "../lib/keymap";

const IGNORED_KEYS = ["Control", "Shift", "Alt", "Meta"];

interface KeyRecorderProps {
  value: string;
  // "combo": records one keydown (modifiers + key) via formatBinding, e.g.
  // "Ctrl+Shift+K". "sequence": records two consecutive keydowns with no
  // modifiers held on either, joined by a space, e.g. "g c" — mirrors
  // useGlobalHotkeys.ts's actual "prefix key, then follow-up key" mechanism
  // for goto_customers/goto_systems/goto_journal.
  mode: "combo" | "sequence";
  onChange: (value: string) => void;
}

export default function KeyRecorder({ value, mode, onChange }: KeyRecorderProps) {
  const [recording, setRecording] = useState(false);
  const [pendingFirst, setPendingFirst] = useState<string | null>(null);

  useEffect(() => {
    if (!recording) return;

    function onKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
        setRecording(false);
        setPendingFirst(null);
        return;
      }
      if (IGNORED_KEYS.includes(e.key)) return;

      if (mode === "combo") {
        onChange(formatBinding(e));
        setRecording(false);
        return;
      }

      if (pendingFirst === null) {
        setPendingFirst(e.key.toLowerCase());
        return;
      }
      onChange(`${pendingFirst} ${e.key.toLowerCase()}`);
      setRecording(false);
      setPendingFirst(null);
    }

    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [recording, mode, pendingFirst, onChange]);

  return (
    <button
      type="button"
      onClick={() => {
        setRecording(true);
        setPendingFirst(null);
      }}
      style={{
        fontFamily: "var(--font-mono)",
        minWidth: "9rem",
        borderColor: recording ? "var(--accent)" : undefined,
      }}
    >
      {recording ? (pendingFirst ? `${pendingFirst} …` : "Taste(n) drücken…") : value}
    </button>
  );
}
