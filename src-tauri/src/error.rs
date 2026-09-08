use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Konfigurationsfehler: {0}")]
    Config(String),
    #[error("Datenbankfehler: {0}")]
    Database(String),
    #[error("Migrationsfehler: {0}")]
    Migration(String),
    #[error("Ungültiger Zeitstempel: {0}")]
    InvalidTimestamp(String),
    #[error("Zeitzone konnte nicht ermittelt werden: {0}")]
    Timezone(String),
    #[error("I/O-Fehler: {0}")]
    Io(String),
    #[error("Nicht gefunden: {0}")]
    NotFound(String),
    #[error("Backup-Fehler: {0}")]
    Backup(String),
    #[error("Plugin-Fehler: {0}")]
    Plugin(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Config(_) => "config",
            AppError::Database(_) => "database",
            AppError::Migration(_) => "migration",
            AppError::InvalidTimestamp(_) => "invalid_timestamp",
            AppError::Timezone(_) => "timezone",
            AppError::Io(_) => "io",
            AppError::NotFound(_) => "not_found",
            AppError::Backup(_) => "backup",
            AppError::Plugin(_) => "plugin",
        }
    }
}

impl From<crate::plugin::PluginError> for AppError {
    fn from(e: crate::plugin::PluginError) -> Self {
        AppError::Plugin(e.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Database(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<zip::result::ZipError> for AppError {
    fn from(e: zip::result::ZipError) -> Self {
        AppError::Backup(e.to_string())
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_error_has_database_code_and_readable_message() {
        let err = AppError::Database("UNIQUE constraint failed".to_string());
        assert_eq!(err.code(), "database");
        assert_eq!(err.to_string(), "Datenbankfehler: UNIQUE constraint failed");
    }

    #[test]
    fn serializes_to_code_and_message_json() {
        let err = AppError::InvalidTimestamp("07.09.2026".to_string());
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(
            json,
            r#"{"code":"invalid_timestamp","message":"Ungültiger Zeitstempel: 07.09.2026"}"#
        );
    }

    #[test]
    fn io_error_converts_from_std_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "datei fehlt");
        let app_err: AppError = io_err.into();
        assert_eq!(app_err.code(), "io");
    }

    #[test]
    fn not_found_has_not_found_code() {
        let err = AppError::NotFound("Kunde 42".to_string());
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn plugin_error_converts_from_plugin_error_type() {
        let plugin_err = crate::plugin::PluginError::Unreachable(
            "Ninja-API antwortete mit Status 500".to_string(),
        );
        let app_err: AppError = plugin_err.into();
        assert_eq!(app_err.code(), "plugin");
        assert!(app_err.to_string().contains("Status 500"));
    }
}
