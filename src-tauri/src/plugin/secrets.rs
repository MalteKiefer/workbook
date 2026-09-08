//! Dünner Wrapper um die `keyring`-Crate für Plugin-Zugangsdaten. Speichert
//! ausschließlich im OS-Schlüsselspeicher (Windows Credential Manager /
//! Secret Service unter Linux) -- laut Spec dürfen Plugin-Secrets nie in
//! `config.toml` oder der Datenbank landen, nur hier.
//!
//! Bewusst kein `#[cfg(test)]`-Block, der `store_secret`/`load_secret` gegen
//! den echten OS-Schlüsselspeicher aufruft: das würde auf diesem Rechner
//! einen verwaisten, schwer aufzuräumenden Credential-Eintrag hinterlassen,
//! und Sandbox-/CI-Umgebungen haben oft gar keinen echten Schlüsselspeicher
//! verfügbar. Die Korrektheit dieses Moduls stützt sich auf die Testsuite der
//! `keyring`-Crate selbst plus sauberes Kompilieren/Typchecking gegen deren
//! reale API (verifiziert gegen `keyring` 4.2.0, siehe
//! `docs/PLUGIN_ARCHITECTURE.md`).

use crate::error::AppError;

const SERVICE_NAME: &str = "wartungsdoku";

/// Speichert ein Plugin-Zugangsdatum im OS-Schlüsselspeicher, geschlüsselt
/// über die Plugin-ID. Diese Funktion nie mit einem Wert aufrufen, der auch
/// in `config.toml` oder der Datenbank stehen sollte -- laut Spec leben
/// Plugin-Secrets AUSSCHLIESSLICH hier.
pub fn store_secret(plugin_id: &str, secret: &str) -> Result<(), AppError> {
    let entry = keyring::Entry::new(SERVICE_NAME, plugin_id)
        .map_err(|e| AppError::Config(format!("Schlüsselspeicher nicht verfügbar: {e}")))?;
    entry.set_password(secret).map_err(|e| {
        AppError::Config(format!(
            "Anmeldedaten konnten nicht gespeichert werden: {e}"
        ))
    })
}

/// Liest ein zuvor gespeichertes Plugin-Zugangsdatum. `Ok(None)`, wenn für
/// diese Plugin-ID noch nichts hinterlegt wurde.
pub fn load_secret(plugin_id: &str) -> Result<Option<String>, AppError> {
    let entry = keyring::Entry::new(SERVICE_NAME, plugin_id)
        .map_err(|e| AppError::Config(format!("Schlüsselspeicher nicht verfügbar: {e}")))?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Config(format!(
            "Anmeldedaten konnten nicht gelesen werden: {e}"
        ))),
    }
}
