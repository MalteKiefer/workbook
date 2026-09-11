export type DiffLine = { type: "same" | "added" | "removed"; text: string };

// Minimal LCS-based line diff -- O(n*m) on line COUNT (not character
// count), which is fine here since journal entries are short structured
// notes, not large documents. Good enough for "show what changed
// between two revisions of a maintenance note"; not a general-purpose
// diff tool.
export function diffLines(oldText: string, newText: string): DiffLine[] {
  const oldLines = oldText.split("\n");
  const newLines = newText.split("\n");
  const m = oldLines.length;
  const n = newLines.length;

  const lcs: number[][] = Array.from({ length: m + 1 }, () => new Array<number>(n + 1).fill(0));
  for (let i = m - 1; i >= 0; i--) {
    for (let j = n - 1; j >= 0; j--) {
      lcs[i][j] = oldLines[i] === newLines[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const result: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < m && j < n) {
    if (oldLines[i] === newLines[j]) {
      result.push({ type: "same", text: oldLines[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      result.push({ type: "removed", text: oldLines[i] });
      i++;
    } else {
      result.push({ type: "added", text: newLines[j] });
      j++;
    }
  }
  while (i < m) {
    result.push({ type: "removed", text: oldLines[i] });
    i++;
  }
  while (j < n) {
    result.push({ type: "added", text: newLines[j] });
    j++;
  }
  return result;
}
