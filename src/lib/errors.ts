// Tauri's invoke() rejects with whatever the failing command's Result<T, E>'s
// E serializes to over IPC. This app's AppError (src-tauri/src/error.rs) has a
// custom Serialize impl producing {"code": "...", "message": "..."} — a plain
// object, not a string or Error — so naively doing `String(err)` on it yields
// the useless literal "[object Object]" instead of the real message.
export function formatInvokeError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (
    typeof err === "object" &&
    err !== null &&
    "message" in err &&
    typeof (err as { message: unknown }).message === "string"
  ) {
    return (err as { message: string }).message;
  }
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}
