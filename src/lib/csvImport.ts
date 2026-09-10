import { open } from "@tauri-apps/plugin-dialog";

// Kept in sync with src-tauri/src/import.rs::ImportRowError/ImportSummary.
export interface ImportRowError {
  row: number;
  message: string;
}

export interface ImportSummary {
  imported: number;
  errors: ImportRowError[];
}

// Shared by CustomerListView.tsx and SystemListView.tsx's "CSV
// importieren" buttons -- null means the user cancelled the dialog.
export async function pickCsvFile(): Promise<string | null> {
  const path = await open({ filters: [{ name: "CSV", extensions: ["csv"] }] });
  if (!path || Array.isArray(path)) return null;
  return path;
}
