use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::AppError;

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
    fn resolve_data_dir_honours_env_override() {
        // SAFETY: Tests laufen sequenziell innerhalb dieses Prozesses für diese eine Variable.
        std::env::set_var("WARTUNGSDOKU_DATA_DIR", "/tmp/wartungsdoku-test-override");
        let resolved = resolve_data_dir();
        std::env::remove_var("WARTUNGSDOKU_DATA_DIR");
        assert_eq!(resolved, PathBuf::from("/tmp/wartungsdoku-test-override"));
    }
}
