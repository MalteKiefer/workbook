import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatInvokeError } from "../lib/errors";

interface CloudStorageSettingsDto {
  enabled: boolean;
  endpoint: string | null;
  region: string | null;
  bucket: string | null;
  access_key_id: string | null;
  has_secret_key: boolean;
}

export default function StorageSettingsView() {
  const [cloudSettings, setCloudSettings] = useState<CloudStorageSettingsDto | null>(null);
  const [cloudError, setCloudError] = useState<string | null>(null);
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudTestStatus, setCloudTestStatus] = useState<string | null>(null);

  async function loadCloudSettings() {
    try {
      const loaded = await invoke<CloudStorageSettingsDto>("get_cloud_storage_settings");
      setCloudSettings(loaded);
    } catch (e) {
      setCloudError(formatInvokeError(e));
    }
  }

  useEffect(() => {
    void loadCloudSettings();
  }, []);

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
      <h1 style={{ fontSize: "1.1rem" }}>Speicher</h1>

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
        <h2 style={{ margin: 0, fontSize: "1rem" }}>Cloud-Speicher (S3/B2)</h2>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.85rem" }}>
          Lädt Kopien in einen S3-kompatiblen Bucket hoch -- funktioniert sowohl mit AWS S3 als auch mit Backblaze B2
          (B2 stellt eine S3-kompatible Schnittstelle bereit). Gilt für zwei unabhängige Vorgänge, die dieselbe
          Konfiguration hier teilen: jedes neu erstellte Backup (siehe Einstellungen &gt; Backup) und jeder einzelne
          neue Anhang/jedes Dokument, sobald er hochgeladen wird -- beides läuft automatisch im Hintergrund, sobald
          diese Einstellung aktiviert und vollständig ausgefüllt ist.
        </p>
        <p style={{ margin: 0, color: "var(--text-secondary)", fontSize: "0.82rem" }}>
          Backups landen so verschlüsselt oder unverschlüsselt in der Cloud, wie sie lokal erstellt werden -- siehe
          Einstellungen &gt; Backup für die Backup-Verschlüsselung. Einzelne Anhänge/Dokumente werden unverändert
          hochgeladen (nur der Übertragungsweg ist per TLS abgesichert).
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
              Cloud-Speicher aktivieren
            </label>

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
