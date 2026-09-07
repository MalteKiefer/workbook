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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            autostart_enabled: true,
            context_capture_enabled: false,
            late_entry_threshold_hours: 24,
            hotkeys: HotkeyConfig::default(),
        }
    }
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wartungsdoku")
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
}
