use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::plugin::level::LevelConnectionMeta;
use crate::plugin::ninja::{NinjaConnectionMeta, NinjaOrgMapping};
use crate::plugin::snipeit::{SnipeitCompanyMapping, SnipeitConnectionMeta};

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
        toml::from_str(&text)
            .map_err(|e| AppError::Config(format!("config.toml ungültig: {e}")))
    }

    pub fn save(&self, config_path: &Path) -> Result<(), AppError> {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| AppError::Config(format!("config.toml serialisieren fehlgeschlagen: {e}")))?;
        std::fs::write(config_path, text)?;
        Ok(())
    }
}

#[cfg(test)]
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
        assert_eq!(loaded.ninja_connections[0].base_url, "https://eu.ninjarmm.com");
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
        assert_eq!(loaded.snipeit_connections[0].base_url, "https://assets.example.com");
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
    fn resolve_data_dir_honours_env_override() {
        // SAFETY: Tests laufen sequenziell innerhalb dieses Prozesses für diese eine Variable.
        std::env::set_var("WARTUNGSDOKU_DATA_DIR", "/tmp/wartungsdoku-test-override");
        let resolved = resolve_data_dir();
        std::env::remove_var("WARTUNGSDOKU_DATA_DIR");
        assert_eq!(resolved, PathBuf::from("/tmp/wartungsdoku-test-override"));
    }
}
