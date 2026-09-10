import type { ImportSummary } from "../lib/csvImport";

// Shared by CustomerListView.tsx and SystemListView.tsx to show the result
// of a CSV import: how many rows were created, and which rows were
// skipped and why. Dismissible since it's not a modal -- the list behind
// it already reflects any successful imports.
export default function ImportSummaryPanel({ summary, onDismiss }: { summary: ImportSummary; onDismiss: () => void }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "0.4rem",
        padding: "0.75rem",
        marginBottom: "0.75rem",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-sm)",
        background: "var(--bg-surface)",
      }}
    >
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <p style={{ margin: 0, fontSize: "0.85rem" }}>
          <strong style={{ color: "var(--success)" }}>{summary.imported}</strong> importiert
          {summary.errors.length > 0 && (
            <>
              , <strong style={{ color: "var(--danger)" }}>{summary.errors.length}</strong> übersprungen
            </>
          )}
        </p>
        <button onClick={onDismiss}>Schließen</button>
      </div>
      {summary.errors.length > 0 && (
        <ul style={{ margin: 0, padding: "0 0 0 1.1rem", fontSize: "0.8rem", color: "var(--text-secondary)", maxHeight: "8rem", overflow: "auto" }}>
          {summary.errors.map((e, i) => (
            <li key={i}>
              Zeile {e.row}: {e.message}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
