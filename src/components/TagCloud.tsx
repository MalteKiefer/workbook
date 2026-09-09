import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface TagCount {
  name: string;
  count: number;
}

interface TagCloudProps {
  selected: string;
  onSelect: (name: string) => void;
}

const MIN_FONT_REM = 0.72;
const MAX_FONT_REM = 1.25;

// Font size grows with usage, but on a log scale -- a tag used 50 times
// shouldn't visually drown out one used 5 times as hard as a linear scale
// would, and it keeps a single outlier from making every other tag
// unreadably tiny by comparison.
function fontSizeFor(count: number, minCount: number, maxCount: number): string {
  if (maxCount === minCount) return `${(MIN_FONT_REM + MAX_FONT_REM) / 2}rem`;
  const logMin = Math.log(minCount);
  const logMax = Math.log(maxCount);
  const t = (Math.log(count) - logMin) / (logMax - logMin);
  return `${MIN_FONT_REM + t * (MAX_FONT_REM - MIN_FONT_REM)}rem`;
}

// Clickable tag cloud for the Journal filter bar: every tag currently in use,
// sized by how often it's used, click to filter by it (click again to clear).
// Complements JournalView's exact-match tag filter rather than replacing it.
export function TagCloud({ selected, onSelect }: TagCloudProps) {
  const [tags, setTags] = useState<TagCount[]>([]);

  useEffect(() => {
    invoke<TagCount[]>("list_tags_with_counts").then(setTags);
  }, []);

  if (tags.length === 0) return null;

  const counts = tags.map((t) => t.count);
  const minCount = Math.min(...counts);
  const maxCount = Math.max(...counts);

  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: "0.4rem", alignItems: "baseline", marginBottom: "0.75rem" }}>
      {tags.map((t) => {
        const isSelected = t.name === selected;
        return (
          <button
            key={t.name}
            type="button"
            onClick={() => onSelect(isSelected ? "" : t.name)}
            title={`${t.count} ${t.count === 1 ? "Eintrag" : "Einträge"}`}
            style={{
              fontSize: fontSizeFor(t.count, minCount, maxCount),
              padding: "0.1rem 0.55rem",
              borderRadius: "999px",
              border: isSelected ? "1px solid var(--accent)" : "1px solid var(--border-subtle)",
              background: isSelected ? "var(--accent)" : "var(--bg-hover)",
              color: isSelected ? "var(--accent-text)" : "var(--text-secondary)",
              cursor: "pointer",
              lineHeight: 1.5,
            }}
          >
            {t.name}
          </button>
        );
      })}
    </div>
  );
}
