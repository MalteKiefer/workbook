use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::plugin::level::LevelConnectionMeta;
use crate::plugin::ninja::{NinjaConnectionMeta, NinjaOrgMapping};
use crate::plugin::snipeit::{SnipeitCompanyMapping, SnipeitConnectionMeta};

/// Theme-Präferenz für die Oberfläche (siehe `src/styles/theme.css` und
/// `src/lib/theme.ts` auf der Frontend-Seite). Reines TOML/JSON-Serde --
/// keine DB-Spalte -- daher genügen einfache Serde-Derives; `rename_all =
/// "snake_case"` sorgt dafür, dass die Werte in `config.toml` und über den
/// Tauri-Command als `"light"`/`"dark"`/`"system"` erscheinen statt in Rusts
/// Standard-Schreibweise `Light`/`Dark`/`System` (dieselbe Konvention wie
/// `db::entries::Category`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    Light,
    // Die App war bislang ausschließlich dunkel -- eine config.toml, die
    // dieses Feld zum ersten Mal bekommt (bestehender Nutzer, altes
    // Backup), darf sich dadurch NICHT optisch verändern. Nur eine
    // explizite künftige Auswahl darf das Erscheinungsbild umstellen.
    #[default]
    Dark,
    System,
}

/// Häufigkeit automatischer Backups (Einstellungen → Backup). Reines
/// TOML/Serde-Enum, dieselbe Konvention wie `ThemePreference`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoBackupFrequency {
    #[default]
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub quick_capture: String,
    pub search: String,
    pub clipboard_screenshot: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            quick_capture: "Ctrl+Alt+Space".to_string(),
            search: "Ctrl+Alt+F".to_string(),
            clipboard_screenshot: "Ctrl+Alt+S".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub data_dir: PathBuf,
    pub autostart_enabled: bool,
    pub context_capture_enabled: bool,
    pub late_entry_threshold_hours: i64,
    pub hotkeys: HotkeyConfig,
    pub last_customer_id: Option<i64>,
    pub last_system_id: Option<i64>,
    /// Nicht-geheime Metadaten je konfigurierter Ninja-Verbindung (ein Satz
    /// OAuth2-Zugangsdaten für genau einen Ninja-Mandanten; ein Nutzer kann
    /// beliebig viele Verbindungen anlegen). Eine Verbindung ist NICHT an
    /// genau einen lokalen Kunden gebunden -- siehe `ninja_org_mappings`.
    /// Zugehörige Client-ID/-Secret liegen ausschließlich im
    /// OS-Schlüsselspeicher, siehe `plugin::secrets`.
    pub ninja_connections: Vec<NinjaConnectionMeta>,
    /// Zuordnung einzelner Ninja-"Organizations" (innerhalb einer Verbindung)
    /// zu lokalen Kunden. Ein einzelner Ninja-Mandant (eine Verbindung) kann
    /// mehrere Organisationen sehen -- z. B. weil der Nutzer selbst ein MSP
    /// ist, der seinerseits mehrere eigene Kunden als getrennte
    /// Organisationen in Ninja führt --, deshalb diese separate, granulare
    /// Zuordnungstabelle statt eines `customer_id`-Felds direkt an der
    /// Verbindung. `#[serde(default)]`-kompatibel mit Konfigurationen von vor
    /// dieser Änderung, die dieses Feld noch nicht kennen (siehe
    /// `ninja_connections` oben für dasselbe Muster).
    pub ninja_org_mappings: Vec<NinjaOrgMapping>,
    /// Nicht-geheime Metadaten je konfigurierter Level.io-Verbindung. Anders
    /// als eine Ninja-Verbindung ist eine Level-Verbindung direkt an genau
    /// einen lokalen Kunden gebunden (`LevelConnectionMeta.customer_id`) --
    /// Level kennt kein Organisationskonzept, siehe `plugin::level`. Der
    /// zugehörige API-Key liegt ausschließlich im OS-Schlüsselspeicher, siehe
    /// `plugin::secrets`. `#[serde(default)]`-kompatibel mit
    /// Konfigurationen von vor dieser Änderung, analog zu
    /// `ninja_connections` oben.
    pub level_connections: Vec<LevelConnectionMeta>,
    /// Nicht-geheime Metadaten je konfigurierter Snipe-IT-Verbindung (eine
    /// selbst gehostete Snipe-IT-Instanz; ein Nutzer kann beliebig viele
    /// Verbindungen anlegen). Wie eine Ninja-Verbindung ist eine
    /// Snipe-IT-Verbindung NICHT an genau einen lokalen Kunden gebunden --
    /// siehe `snipeit_company_mappings`. Der zugehörige Personal Access
    /// Token liegt ausschließlich im OS-Schlüsselspeicher, siehe
    /// `plugin::secrets`. `#[serde(default)]`-kompatibel mit Konfigurationen
    /// von vor dieser Änderung, analog zu `ninja_connections` oben.
    pub snipeit_connections: Vec<SnipeitConnectionMeta>,
    /// Zuordnung einzelner Snipe-IT-"Companies" (innerhalb einer Verbindung)
    /// zu lokalen Kunden. Eine einzelne Snipe-IT-Instanz (eine Verbindung)
    /// kann mehrere Firmen verwalten -- z. B. weil der Nutzer selbst ein MSP
    /// ist, der mehrere eigene Kunden als getrennte Firmen in einer
    /// gemeinsamen Snipe-IT-Instanz führt --, deshalb diese separate,
    /// granulare Zuordnungstabelle statt eines `customer_id`-Felds direkt an
    /// der Verbindung -- exakt dasselbe Prinzip wie `ninja_org_mappings`.
    /// `#[serde(default)]`-kompatibel mit Konfigurationen von vor dieser
    /// Änderung.
    pub snipeit_company_mappings: Vec<SnipeitCompanyMapping>,
    /// Vom Nutzer gewählte Theme-Präferenz (Einstellungen → Allgemein).
    /// `#[serde(default)]`-kompatibel mit Konfigurationen von vor Einführung
    /// dieses Feldes, analog zu `ninja_connections` oben -- fehlt es, greift
    /// `ThemePreference::default()` (= `Dark`), NICHT `System`, damit
    /// bestehende Installationen optisch unverändert bleiben.
    pub theme_preference: ThemePreference,
    /// Ob der Hintergrund-Scheduler (siehe `backup::schedule_auto_backups`)
    /// automatisch Backups erstellen soll. `auto_backup_dir` muss zusätzlich
    /// gesetzt sein, sonst bleibt die Funktion trotz `true` inaktiv (siehe
    /// `backup::is_auto_backup_due`-Aufrufstelle im Scheduler).
    pub auto_backup_enabled: bool,
    /// Zielordner für automatische Backups. Getrennt vom manuellen
    /// "Backup erstellen"-Dialog, der den Zielpfad jedes Mal explizit abfragt.
    pub auto_backup_dir: Option<PathBuf>,
    pub auto_backup_frequency: AutoBackupFrequency,
    /// RFC3339-Zeitstempel (UTC) des letzten erfolgreichen automatischen
    /// Backups. `None` heißt "noch nie" -- der Scheduler behandelt das wie
    /// eine sofort fällige erste Ausführung.
    pub auto_backup_last_run_utc: Option<String>,
    /// Ob sowohl manuell erstellte als auch automatische Backups mit dem im
    /// OS-Schlüsselspeicher hinterlegten Passwort verschlüsselt werden (siehe
    /// `backup::crypto`). Das Passwort selbst steht nie hier in
    /// `config.toml`, nur dieses Flag.
    pub backup_encryption_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            autostart_enabled: true,
            context_capture_enabled: false,
            late_entry_threshold_hours: 24,
            hotkeys: HotkeyConfig::default(),
            last_customer_id: None,
            last_system_id: None,
            ninja_connections: Vec::new(),
            ninja_org_mappings: Vec::new(),
            level_connections: Vec::new(),
            snipeit_connections: Vec::new(),
            snipeit_company_mappings: Vec::new(),
            theme_preference: ThemePreference::default(),
            auto_backup_enabled: false,
            auto_backup_dir: None,
            auto_backup_frequency: AutoBackupFrequency::default(),
            auto_backup_last_run_utc: None,
            backup_encryption_enabled: false,
        }
    }
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wartungsdoku")
}

pub fn resolve_data_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("WARTUNGSDOKU_DATA_DIR") {
        return PathBuf::from(override_dir);
    }
    default_data_dir()
}

impl Config {
    pub fn load_or_default(config_path: &Path) -> Result<Self, AppError> {
        if !config_path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(config_path)
            .map_err(|e| AppError::Config(format!("config.toml lesen fehlgeschlagen: {e}")))?;
        toml::from_str(&text).map_err(|e| AppError::Config(format!("config.toml ungültig: {e}")))
    }

    pub fn save(&self, config_path: &Path) -> Result<(), AppError> {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| {
            AppError::Config(format!("config.toml serialisieren fehlgeschlagen: {e}"))
        })?;
        std::fs::write(config_path, text)?;
        Ok(())
    }
}

#[cfg(test)]
// Tests build fixtures by mutating a couple of fields on a `Config::default()`
// binding; that reads clearer here than a full struct literal with
// `..Default::default()` and would only get more brittle as fields are added.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_or_default_returns_defaults_when_file_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config::load_or_default(&path).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.hotkeys.quick_capture, "Ctrl+Alt+Space");
        assert!(config.autostart_enabled);
        assert!(!config.context_capture_enabled);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let mut config = Config::default();
        config.autostart_enabled = false;
        config.context_capture_enabled = true;
        config.hotkeys.search = "Ctrl+Shift+F".to_string();

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn invalid_toml_returns_config_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not = [valid toml").unwrap();

        let result = Config::load_or_default(&path);
        assert!(matches!(result, Err(AppError::Config(_))));
    }

    #[test]
    fn save_then_load_roundtrips_last_selection() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.last_customer_id = Some(7);
        config.last_system_id = Some(3);

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.last_customer_id, Some(7));
        assert_eq!(loaded.last_system_id, Some(3));
    }

    #[test]
    fn save_then_load_roundtrips_ninja_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.ninja_connections.push(NinjaConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Ninja".to_string(),
            base_url: "https://eu.ninjarmm.com".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.ninja_connections.len(), 1);
        assert_eq!(
            loaded.ninja_connections[0].base_url,
            "https://eu.ninjarmm.com"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_ninja_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simuliert eine config.toml von vor Einführung der Ninja-Integration --
        // das Feld fehlt komplett und muss dank `#[serde(default)]` klaglos auf
        // eine leere Liste zurückfallen statt das Laden scheitern zu lassen.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.ninja_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_ninja_org_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.ninja_connections.push(NinjaConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Ninja".to_string(),
            base_url: "https://eu.ninjarmm.com".to_string(),
        });
        config.ninja_org_mappings.push(NinjaOrgMapping {
            connection_id: "acme-1700000000000".to_string(),
            organization_id: "1".to_string(),
            organization_name: "ACME Hauptsitz".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.ninja_org_mappings.len(), 1);
        assert_eq!(loaded.ninja_org_mappings[0].organization_id, "1");
        assert_eq!(loaded.ninja_org_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_ninja_org_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simuliert eine config.toml von vor Einführung der Organisations-
        // Zuordnung -- das Feld fehlt komplett und muss dank
        // `#[serde(default)]` klaglos auf eine leere Liste zurückfallen statt
        // das Laden scheitern zu lassen.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.ninja_org_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_level_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.level_connections.push(LevelConnectionMeta {
            id: "acme-1700000000000".to_string(),
            customer_id: 7,
            label: "ACME Level".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.level_connections.len(), 1);
        assert_eq!(loaded.level_connections[0].customer_id, 7);
        assert_eq!(loaded.level_connections[0].label, "ACME Level");
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_level_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simuliert eine config.toml von vor Einführung der Level.io-Integration
        // -- das Feld fehlt komplett und muss dank `#[serde(default)]` klaglos
        // auf eine leere Liste zurückfallen statt das Laden scheitern zu
        // lassen.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.level_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_snipeit_connections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.snipeit_connections.len(), 1);
        assert_eq!(
            loaded.snipeit_connections[0].base_url,
            "https://assets.example.com"
        );
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_snipeit_connections_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simuliert eine config.toml von vor Einführung der Snipe-IT-Integration
        // -- das Feld fehlt komplett und muss dank `#[serde(default)]` klaglos
        // auf eine leere Liste zurückfallen statt das Laden scheitern zu
        // lassen.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.snipeit_connections.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_snipeit_company_mappings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.snipeit_connections.push(SnipeitConnectionMeta {
            id: "acme-1700000000000".to_string(),
            label: "ACME Snipe-IT".to_string(),
            base_url: "https://assets.example.com".to_string(),
        });
        config.snipeit_company_mappings.push(SnipeitCompanyMapping {
            connection_id: "acme-1700000000000".to_string(),
            company_id: "1".to_string(),
            company_name: "ACME Hauptsitz".to_string(),
            customer_id: 7,
        });

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.snipeit_company_mappings.len(), 1);
        assert_eq!(loaded.snipeit_company_mappings[0].company_id, "1");
        assert_eq!(loaded.snipeit_company_mappings[0].customer_id, 7);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_snipeit_company_mappings_field_defaults_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simuliert eine config.toml von vor Einführung der Firmen-Zuordnung --
        // das Feld fehlt komplett und muss dank `#[serde(default)]` klaglos auf
        // eine leere Liste zurückfallen statt das Laden scheitern zu lassen.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(loaded.snipeit_company_mappings.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips_theme_preference() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.theme_preference = ThemePreference::Light;

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.theme_preference, ThemePreference::Light);
        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_theme_preference_field_defaults_to_dark() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simuliert eine config.toml von vor Einführung der Theme-Auswahl --
        // das Feld fehlt komplett und muss dank `#[serde(default)]` klaglos
        // auf `Dark` zurückfallen (NICHT `System`), damit sich das
        // Erscheinungsbild bestehender Installationen nicht ungefragt ändert.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded.theme_preference, ThemePreference::Dark);
    }

    #[test]
    fn theme_preference_serializes_as_lowercase_snake_case() {
        assert_eq!(
            serde_json::to_string(&ThemePreference::Light).unwrap(),
            "\"light\""
        );
        assert_eq!(
            serde_json::to_string(&ThemePreference::Dark).unwrap(),
            "\"dark\""
        );
        assert_eq!(
            serde_json::to_string(&ThemePreference::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn save_then_load_roundtrips_auto_backup_settings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.auto_backup_enabled = true;
        config.auto_backup_dir = Some(PathBuf::from("D:/Backups"));
        config.auto_backup_frequency = AutoBackupFrequency::Weekly;
        config.auto_backup_last_run_utc = Some("2026-09-01T10:00:00Z".to_string());
        config.backup_encryption_enabled = true;

        config.save(&path).unwrap();
        let loaded = Config::load_or_default(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn config_without_auto_backup_fields_defaults_to_disabled() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Simuliert eine config.toml von vor Einführung des Auto-Backups --
        // muss dank `#[serde(default)]` klaglos auf "deaktiviert" zurückfallen.
        std::fs::write(&path, "autostart_enabled = true\n").unwrap();

        let loaded = Config::load_or_default(&path).unwrap();

        assert!(!loaded.auto_backup_enabled);
        assert_eq!(loaded.auto_backup_dir, None);
        assert_eq!(loaded.auto_backup_frequency, AutoBackupFrequency::Daily);
        assert_eq!(loaded.auto_backup_last_run_utc, None);
        assert!(!loaded.backup_encryption_enabled);
    }

    #[test]
    fn resolve_data_dir_honours_env_override() {
        // SAFETY: Tests laufen sequenziell innerhalb dieses Prozesses für diese eine Variable.
        std::env::set_var("WARTUNGSDOKU_DATA_DIR", "/tmp/wartungsdoku-test-override");
        let resolved = resolve_data_dir();
        std::env::remove_var("WARTUNGSDOKU_DATA_DIR");
        assert_eq!(resolved, PathBuf::from("/tmp/wartungsdoku-test-override"));
    }
}
