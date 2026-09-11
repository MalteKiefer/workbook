import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Mirrors src-tauri/src/commands/export.rs's to_pdf_entry exactly: an entry
// is "late" when the gap between when it was actually performed and when
// it was recorded exceeds the configured threshold. Previously this was
// only ever surfaced in the PDF export's "Nachträglich erfasst" note --
// this is the same check, computed client-side so it can show live in the
// Journal list and entry detail too.
//
// The Rust side computes `(created_at - performed_at).num_hours().abs()` --
// chrono's `num_hours()` TRUNCATES the duration to a whole number of hours
// (toward zero) before the threshold comparison, it does not use a
// fractional-hour value. `Math.trunc(...)` here reproduces that truncation;
// without it, e.g. a 24h54m gap against a 24h threshold would round up to
// "24.9 > 24" (late) here while the PDF's own truncated-to-24 comparison
// says "24 > 24" (not late) -- the exact live-vs-export disagreement this
// feature exists to avoid.
export function isLateEntry(performedAtUtc: string, createdAtUtc: string, thresholdHours: number): boolean {
  const performedAt = new Date(performedAtUtc).getTime();
  const createdAt = new Date(createdAtUtc).getTime();
  const diffHours = Math.trunc(Math.abs(createdAt - performedAt) / (1000 * 60 * 60));
  return diffHours > thresholdHours;
}

// Fetches Config::late_entry_threshold_hours once on mount via the
// get_late_entry_threshold_hours command (src-tauri/src/commands/settings.rs)
// -- shared by JournalView.tsx and EntryDetailModal.tsx so neither duplicates
// the invoke() call. Read-only: there is deliberately no settings UI to
// change this value from the frontend (still config.toml-only).
export function useLateEntryThreshold(): number | null {
  const [thresholdHours, setThresholdHours] = useState<number | null>(null);
  useEffect(() => {
    invoke<number>("get_late_entry_threshold_hours").then(setThresholdHours);
  }, []);
  return thresholdHours;
}
