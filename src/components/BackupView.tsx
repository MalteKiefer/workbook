import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

function todayIsoDate(): string {
  return new Date().toISOString().slice(0, 10);
}

export default function BackupView() {
  const [backupBusy, setBackupBusy] = useState(false);
  const [backupStatus, setBackupStatus] = useState<string | null>(null);
  const [backupError, setBackupError] = useState<string | null>(null);

  const [restoreBusy, setRestoreBusy] = useState(false);
  const [restoreStatus, setRestoreStatus] = useState<string | null>(null);
  const [restoreError, setRestoreError] = useState<string | null>(null);

  async function handleCreateBackup() {
    setBackupError(null);
    setBackupStatus(null);
    try {
      const destPath = await save({
        defaultPath: `wartungsdoku-backup-${todayIsoDate()}.zip`,
        filters: [{ name: "Zip", extensions: ["zip"] }],
      });
      if (!destPath) return;
      setBackupBusy(true);
      await invoke("create_backup", { destPath });
      setBackupStatus(`Backup erfolgreich erstellt: ${destPath}`);
    } catch (e) {
      setBackupError(String(e));
    } finally {
      setBackupBusy(false);
    }
  }

  async function handleRestoreBackup() {
    setRestoreError(null);
    setRestoreStatus(null);
    try {
      const sourcePath = await open({ filters: [{ name: "Zip", extensions: ["zip"] }] });
      if (!sourcePath) return;
      const confirmed = window.confirm(
        "Achtung: Dies überschreibt alle aktuellen Daten mit dem Inhalt des Backups. Fortfahren?",
      );
      if (!confirmed) return;
      setRestoreBusy(true);
      setRestoreStatus("Wiederherstellung wird vorbereitet, Anwendung startet danach automatisch neu…");
      await invoke("restore_backup", { sourcePath });
      // In der Praxis unerreichbar: bei Erfolg startet die App neu, bevor
      // dieses Promise aufgelöst wird.
      setRestoreStatus("Wiederherstellung abgeschlossen.");
    } catch (e) {
      setRestoreError(String(e));
    } finally {
      setRestoreBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1.5rem", maxWidth: "36rem" }}>
      <h1 style={{ fontSize: "1.1rem" }}>Backup &amp; Wiederherstellung</h1>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Backup erstellen</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Erstellt eine einzelne Zip-Datei mit einem vollständigen Datenbank-Snapshot, allen Anhängen, den
          Einstellungen (config.toml) und den zwischengespeicherten Plugin-Gerätelisten.
        </p>
        <div>
          <button type="button" disabled={backupBusy} onClick={() => void handleCreateBackup()}>
            {backupBusy ? "Erstelle Backup…" : "Backup erstellen"}
          </button>
        </div>
        {backupStatus && <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{backupStatus}</p>}
        {backupError && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {backupError}</p>}
      </section>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.5rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Backup wiederherstellen</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Ersetzt alle aktuellen Daten (Datenbank, Anhänge, Einstellungen und Plugin-Zwischenspeicher) durch den
          Inhalt einer zuvor erstellten Backup-Datei. Die aktuellen Daten werden vor dem Überschreiben
          sicherheitshalber lokal gesichert.
        </p>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.82rem" }}>
          Hinweis: Zugangsdaten von Plugin-Verbindungen (z.&nbsp;B. API-Schlüssel für Ninja/Level.io) sind aus
          Sicherheitsgründen nie Teil eines Backups — sie liegen ausschließlich im Schlüsselspeicher des
          Betriebssystems. Nach einer Wiederherstellung auf einem anderen Rechner (oder falls der Schlüsselspeicher
          zwischenzeitlich geleert wurde) bleiben die Verbindungen selbst (Bezeichnung, Zuordnung zu Kunden) zwar
          erhalten, müssen aber mit den jeweiligen Zugangsdaten erneut verbunden werden.
        </p>
        <div>
          <button type="button" disabled={restoreBusy} onClick={() => void handleRestoreBackup()}>
            {restoreBusy ? "Bereite Wiederherstellung vor…" : "Backup wiederherstellen…"}
          </button>
        </div>
        {restoreStatus && <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{restoreStatus}</p>}
        {restoreError && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {restoreError}</p>}
      </section>
    </div>
  );
}
