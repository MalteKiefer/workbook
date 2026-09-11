// Frontend mirror of maintenance::is_expiring_soon (src-tauri/src/maintenance.rs) --
// same "duplicate the simple pure check on both sides" precedent as
// lib/lateEntry.ts's isLateEntry, though here there's no truncation subtlety
// to match: expires_on is a plain YYYY-MM-DD calendar date (no time-of-day),
// so this compares whole calendar days at midnight boundaries rather than
// fractional hours -- "today" is normalized to midnight before the
// reminder-window offset is added, the same way the Rust side operates on
// chrono::NaiveDate values rather than instants.
export function isExpiringSoon(expiresOn: string, reminderDaysBefore: number): boolean {
  const now = new Date();
  const today = new Date(Date.UTC(now.getFullYear(), now.getMonth(), now.getDate()));
  const threshold = new Date(today.getTime() + reminderDaysBefore * 86400000);
  return new Date(expiresOn) <= threshold;
}

// German labels for db::expiring_items::ExpiringItemKind's 6 variants --
// shared between DashboardView.tsx (read-only rendering) and
// ExpiringItemsPanel.tsx (the <select> in the create/edit form) so the
// mapping isn't duplicated across both.
export const EXPIRING_ITEM_KIND_LABELS: Record<string, string> = {
  ssl_certificate: "SSL-Zertifikat",
  domain: "Domain",
  license: "Lizenz",
  contract: "Vertrag",
  warranty: "Garantie",
  sonstiges: "Sonstiges",
};

export const EXPIRING_ITEM_KIND_OPTIONS: { value: string; label: string }[] = Object.entries(
  EXPIRING_ITEM_KIND_LABELS,
).map(([value, label]) => ({ value, label }));
