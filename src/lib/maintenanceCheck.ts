import { listen } from "@tauri-apps/api/event";

// Subscribes to "maintenance-check-completed", emitted every ~15 minutes
// by the background scheduler in src-tauri/src/lib.rs (run_maintenance_check)
// with the current cross-customer overdue-systems count, so every open
// window's Dashboard-nav badge stays live without needing the Dashboard
// itself to be open. See src/lib/updateCheck.ts's listenForUpdateCheckCompleted
// for the identical pattern this mirrors.
export function listenForMaintenanceCheckCompleted(onCompleted: (overdueCount: number) => void): void {
  void listen<number>("maintenance-check-completed", (event) => {
    onCompleted(event.payload);
  });
}
