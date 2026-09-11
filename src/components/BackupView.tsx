import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { formatInvokeError } from "../lib/errors";

type AutoBackupFrequency = "daily" | "weekly" | "monthly";

interface BackupSettingsDto {
  auto_backup_enabled: boolean;
  auto_backup_dir: string | null;
  auto_backup_frequency: AutoBackupFrequency;
  auto_backup_last_run_utc: string | null;
  encryption_enabled: boolean;
  has_encryption_passphrase: boolean;
}

interface CloudStorageSettingsDto {
  enabled: boolean;
  endpoint: string | null;
  region: string | null;
  bucket: string | null;
  access_key_id: string | null;
  has_secret_key: boolean;
}

function todayIsoDate(): string {
  return new Date().toISOString().slice(0, 10);
}

function formatTimestamp(iso: string | null): string {
  if (iso === null) return "noch nie";
  return new Date(iso).toLocaleString("de-DE");
}

export default function BackupView() {
  const [backupBusy, setBackupBusy] = useState(false);
  const [backupStatus, setBackupStatus] = useState<string | null>(null);
  const [backupError, setBackupError] = useState<string | null>(null);

  const [restoreBusy, setRestoreBusy] = useState(false);
  const [restoreStatus, setRestoreStatus] = useState<string | null>(null);
  const [restoreError, setRestoreError] = useState<string | null>(null);

  const [settings, setSettings] = useState<BackupSettingsDto | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [settingsBusy, setSettingsBusy] = useState(false);

  const [cloudSettings, setCloudSettings] = useState<CloudStorageSettingsDto | null>(null);
  const [cloudError, setCloudError] = useState<string | null>(null);
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudTestStatus, setCloudTestStatus] = useState<string | null>(null);

  async function loadSettings() {
    try {
      const loaded = await invoke<BackupSettingsDto>("get_backup_settings");
      setSettings(loaded);
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    }
  }

  async function loadCloudSettings() {
    try {
      const loaded = await invoke<CloudStorageSettingsDto>("get_cloud_storage_settings");
      setCloudSettings(loaded);
    } catch (e) {
      setCloudError(formatInvokeError(e));
    }
  }

  useEffect(() => {
    void loadSettings();
    void loadCloudSettings();
  }, []);

  async function handleCreateBackup() {
    setBackupError(null);
    setBackupStatus(null);
    try {
      const defaultExtension = settings?.encryption_enabled ? "wdbk" : "zip";
      const destPath = await save({
        defaultPath: `wartungsdoku-backup-${todayIsoDate()}.${defaultExtension}`,
        filters: [{ name: "Backup", extensions: [defaultExtension] }],
      });
      if (!destPath) return;
      setBackupBusy(true);
      await invoke("create_backup", { destPath });
      setBackupStatus(`Backup erfolgreich erstellt: ${destPath}`);
    } catch (e) {
      setBackupError(formatInvokeError(e));
    } finally {
      setBackupBusy(false);
    }
  }

  async function handleRestoreBackup() {
    setRestoreError(null);
    setRestoreStatus(null);
    try {
      const sourcePath = await open({ filters: [{ name: "Backup", extensions: ["zip", "wdbk"] }] });
      if (!sourcePath) return;

      let passphrase: string | undefined;
      const encrypted = await invoke<boolean>("is_backup_file_encrypted", { path: sourcePath });
      if (encrypted) {
        const entered = window.prompt("Dieses Backup ist verschlüsselt. Passwort eingeben:");
        if (entered === null) return;
        passphrase = entered;
      }

      const confirmed = window.confirm(
        "Achtung: Dies überschreibt alle aktuellen Daten mit dem Inhalt des Backups. Fortfahren?",
      );
      if (!confirmed) return;
      setRestoreBusy(true);
      setRestoreStatus("Wiederherstellung wird vorbereitet, Anwendung startet danach automatisch neu…");
      await invoke("restore_backup", { sourcePath, passphrase });
      // Unreachable in practice: on success, the app restarts before this
      // promise resolves.
      setRestoreStatus("Wiederherstellung abgeschlossen.");
    } catch (e) {
      setRestoreError(formatInvokeError(e));
    } finally {
      setRestoreBusy(false);
    }
  }

  async function handleChooseAutoBackupDir() {
    if (settings === null) return;
    setSettingsError(null);
    try {
      const dir = await open({ directory: true });
      if (!dir || Array.isArray(dir)) return;
      setSettingsBusy(true);
      await invoke("set_auto_backup_settings", {
        enabled: settings.auto_backup_enabled,
        dir,
        frequency: settings.auto_backup_frequency,
      });
      await loadSettings();
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function handleToggleAutoBackup(enabled: boolean) {
    if (settings === null) return;
    setSettingsError(null);
    setSettingsBusy(true);
    try {
      await invoke("set_auto_backup_settings", {
        enabled,
        dir: settings.auto_backup_dir,
        frequency: settings.auto_backup_frequency,
      });
      await loadSettings();
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function handleFrequencyChange(frequency: AutoBackupFrequency) {
    if (settings === null) return;
    setSettingsError(null);
    setSettingsBusy(true);
    try {
      await invoke("set_auto_backup_settings", {
        enabled: settings.auto_backup_enabled,
        dir: settings.auto_backup_dir,
        frequency,
      });
      await loadSettings();
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function handleToggleEncryption(enabled: boolean) {
    setSettingsError(null);
    setSettingsBusy(true);
    try {
      if (enabled && settings?.has_encryption_passphrase === false) {
        const passphrase = window.prompt("Neues Passwort für die Backup-Verschlüsselung festlegen:");
        if (passphrase === null || passphrase === "") {
          setSettingsBusy(false);
          return;
        }
        await invoke("set_backup_encryption_passphrase", { passphrase });
      }
      await invoke("set_backup_encryption_enabled", { enabled });
      await loadSettings();
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function handleChangePassphrase() {
    setSettingsError(null);
    const passphrase = window.prompt("Neues Passwort für die Backup-Verschlüsselung festlegen:");
    if (passphrase === null || passphrase === "") return;
    setSettingsBusy(true);
    try {
      await invoke("set_backup_encryption_passphrase", { passphrase });
      await loadSettings();
    } catch (e) {
      setSettingsError(formatInvokeError(e));
    } finally {
      setSettingsBusy(false);
    }
  }

  async function saveCloudSettings(next: Partial<CloudStorageSettingsDto>) {
    if (cloudSettings === null) return;
    setCloudError(null);
    setCloudBusy(true);
    try {
      const merged = { ...cloudSettings, ...next };
      await invoke("set_cloud_storage_settings", {
        enabled: merged.enabled,
        endpoint: merged.endpoint,
        region: merged.region,
        bucket: merged.bucket,
        accessKeyId: merged.access_key_id,
      });
      await loadCloudSettings();
    } catch (e) {
      setCloudError(formatInvokeError(e));
    } finally {
      setCloudBusy(false);
    }
  }

  async function handleSetCloudSecretKey() {
    setCloudError(null);
    const secretKey = window.prompt("Secret Access Key für den Cloud-Speicher festlegen:");
    if (secretKey === null || secretKey === "") return;
    setCloudBusy(true);
    try {
      await invoke("set_cloud_storage_secret_key", { secretKey });
      await loadCloudSettings();
    } catch (e) {
      setCloudError(formatInvokeError(e));
    } finally {
      setCloudBusy(false);
    }
  }

  async function handleTestCloudConnection() {
    setCloudError(null);
    setCloudTestStatus(null);
    setCloudBusy(true);
    try {
      await invoke("test_cloud_storage_connection");
      setCloudTestStatus("Verbindung erfolgreich.");
    } catch (e) {
      setCloudError(formatInvokeError(e));
    } finally {
      setCloudBusy(false);
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
          Erstellt eine einzelne Datei mit einem vollständigen Datenbank-Snapshot, allen Anhängen, den
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

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.75rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Automatisches Backup</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Erstellt in einem gewählten Ordner regelmäßig automatisch ein Backup, ohne dass etwas manuell angestoßen
          werden muss. Die App prüft dafür etwa alle 15 Minuten im Hintergrund, ob ein Backup fällig ist.
        </p>

        {settings === null ? (
          <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade Einstellungen…</p>
        ) : (
          <>
            <label style={{ display: "flex", alignItems: "center", gap: "0.5rem", fontSize: "0.9rem" }}>
              <input
                type="checkbox"
                checked={settings.auto_backup_enabled}
                disabled={settingsBusy}
                onChange={(e) => void handleToggleAutoBackup(e.target.checked)}
              />
              Automatische Backups aktivieren
            </label>

            <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", flexWrap: "wrap" }}>
              <span style={{ fontSize: "0.85rem", color: "var(--text-secondary)" }}>
                Ordner: {settings.auto_backup_dir ?? "nicht gewählt"}
              </span>
              <button type="button" disabled={settingsBusy} onClick={() => void handleChooseAutoBackupDir()}>
                Ordner wählen…
              </button>
            </div>

            {settings.auto_backup_enabled && settings.auto_backup_dir !== null && !settings.encryption_enabled && (
              <p style={{ margin: 0, color: "var(--accent)", fontSize: "0.82rem" }}>
                Automatische Backups landen unverschlüsselt in diesem Ordner. Verschlüsselung unten aktivieren, falls
                der Ordner synchronisiert wird oder auf einem externen Laufwerk liegt.
              </p>
            )}

            <label style={{ display: "flex", alignItems: "center", gap: "0.5rem", fontSize: "0.9rem" }}>
              Häufigkeit:
              <select
                value={settings.auto_backup_frequency}
                disabled={settingsBusy}
                onChange={(e) => void handleFrequencyChange(e.target.value as AutoBackupFrequency)}
              >
                <option value="daily">Täglich</option>
                <option value="weekly">Wöchentlich</option>
                <option value="monthly">Monatlich</option>
              </select>
            </label>

            <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.82rem" }}>
              Letztes automatisches Backup: {formatTimestamp(settings.auto_backup_last_run_utc)}
            </p>
          </>
        )}
        {settingsError && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {settingsError}</p>}
      </section>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.75rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Verschlüsselung</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Verschlüsselt sowohl manuell erstellte als auch automatische Backups mit einem Passwort (AES-256-GCM). Das
          Passwort liegt ausschließlich im Schlüsselspeicher des Betriebssystems, nie in der Backup-Datei selbst
          oder in den Einstellungen. Ohne dieses Passwort ist eine verschlüsselte Backup-Datei nicht wiederherstellbar
          — es gibt keinen Weg, es zurückzusetzen.
        </p>

        {settings !== null && (
          <>
            <label style={{ display: "flex", alignItems: "center", gap: "0.5rem", fontSize: "0.9rem" }}>
              <input
                type="checkbox"
                checked={settings.encryption_enabled}
                disabled={settingsBusy}
                onChange={(e) => void handleToggleEncryption(e.target.checked)}
              />
              Backups verschlüsseln
            </label>

            {settings.has_encryption_passphrase && (
              <div>
                <button type="button" disabled={settingsBusy} onClick={() => void handleChangePassphrase()}>
                  Passwort ändern…
                </button>
              </div>
            )}
          </>
        )}
      </section>

      <section
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "0.75rem",
          padding: "1rem",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-sm)",
          background: "var(--bg-surface)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Cloud-Backup (S3/B2)</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Lädt nach jedem erfolgreichen Backup (manuell oder automatisch) zusätzlich eine Kopie in einen
          S3-kompatiblen Bucket hoch -- funktioniert sowohl mit AWS S3 als auch mit Backblaze B2 (B2 stellt eine
          S3-kompatible Schnittstelle bereit). Da ein Backup bereits alle Anhänge enthält, landen damit auch
          Dokumente automatisch in der Cloud, ohne eine zweite Ablage pflegen zu müssen.
        </p>

        {cloudSettings === null ? (
          <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>Lade Einstellungen…</p>
        ) : (
          <>
            <label style={{ display: "flex", alignItems: "center", gap: "0.5rem", fontSize: "0.9rem" }}>
              <input
                type="checkbox"
                checked={cloudSettings.enabled}
                disabled={cloudBusy}
                onChange={(e) => void saveCloudSettings({ enabled: e.target.checked })}
              />
              Cloud-Backup aktivieren
            </label>

            {cloudSettings.enabled && settings !== null && !settings.encryption_enabled && (
              <p style={{ margin: 0, color: "var(--accent)", fontSize: "0.82rem" }}>
                Backups werden unverschlüsselt hochgeladen. Backup-Verschlüsselung oben aktivieren, bevor Daten in
                die Cloud übertragen werden.
              </p>
            )}

            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", fontSize: "0.85rem" }}>
              Endpoint
              <input
                value={cloudSettings.endpoint ?? ""}
                disabled={cloudBusy}
                placeholder="https://s3.us-west-002.backblazeb2.com"
                onChange={(e) => setCloudSettings({ ...cloudSettings, endpoint: e.target.value })}
                onBlur={() => void saveCloudSettings({})}
                style={{ fontFamily: "var(--font-mono)" }}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", fontSize: "0.85rem" }}>
              Region
              <input
                value={cloudSettings.region ?? ""}
                disabled={cloudBusy}
                placeholder="us-west-002"
                onChange={(e) => setCloudSettings({ ...cloudSettings, region: e.target.value })}
                onBlur={() => void saveCloudSettings({})}
                style={{ fontFamily: "var(--font-mono)" }}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", fontSize: "0.85rem" }}>
              Bucket
              <input
                value={cloudSettings.bucket ?? ""}
                disabled={cloudBusy}
                onChange={(e) => setCloudSettings({ ...cloudSettings, bucket: e.target.value })}
                onBlur={() => void saveCloudSettings({})}
                style={{ fontFamily: "var(--font-mono)" }}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem", fontSize: "0.85rem" }}>
              Access Key ID
              <input
                value={cloudSettings.access_key_id ?? ""}
                disabled={cloudBusy}
                onChange={(e) => setCloudSettings({ ...cloudSettings, access_key_id: e.target.value })}
                onBlur={() => void saveCloudSettings({})}
                style={{ fontFamily: "var(--font-mono)" }}
              />
            </label>

            <div style={{ display: "flex", gap: "0.5rem", alignItems: "center", flexWrap: "wrap" }}>
              <button type="button" disabled={cloudBusy} onClick={() => void handleSetCloudSecretKey()}>
                {cloudSettings.has_secret_key ? "Secret Access Key ändern…" : "Secret Access Key festlegen…"}
              </button>
              <button type="button" disabled={cloudBusy} onClick={() => void handleTestCloudConnection()}>
                Verbindung testen
              </button>
            </div>
            {cloudTestStatus && <p style={{ color: "var(--success)", fontSize: "0.85rem", margin: 0 }}>{cloudTestStatus}</p>}
          </>
        )}
        {cloudError && <p style={{ color: "var(--danger)", fontSize: "0.82rem", margin: 0 }}>Fehler: {cloudError}</p>}
      </section>
    </div>
  );
}
