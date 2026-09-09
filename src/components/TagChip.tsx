interface TagChipProps {
  name: string;
}

// Small read-only pill for showing a tag inline (entry rows, detail view).
// For a clickable, size-by-frequency version see TagCloud.tsx.
export function TagChip({ name }: TagChipProps) {
  return (
    <span
      style={{
        display: "inline-block",
        padding: "0.05rem 0.5rem",
        borderRadius: "999px",
        border: "1px solid var(--border-subtle)",
        background: "var(--bg-hover)",
        color: "var(--text-secondary)",
        fontSize: "0.75rem",
        lineHeight: 1.6,
        whiteSpace: "nowrap",
      }}
    >
      {name}
    </span>
  );
}

interface TagChipListProps {
  names: string[];
}

export function TagChipList({ names }: TagChipListProps) {
  if (names.length === 0) return null;
  return (
    <span style={{ display: "inline-flex", gap: "0.3rem", flexWrap: "wrap" }}>
      {names.map((name) => (
        <TagChip key={name} name={name} />
      ))}
    </span>
  );
}
