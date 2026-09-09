//! Thin wrapper around the `keyring` crate for plugin credentials. Stores
//! exclusively in the OS keyring (Windows Credential Manager / Secret
//! Service on Linux) -- per spec, plugin secrets must never end up in
//! `config.toml` or the database, only here.
//!
//! Deliberately no `#[cfg(test)]` block that calls `store_secret`/
//! `load_secret` against the real OS keyring: that would leave an orphaned,
//! hard-to-clean-up credential entry on this machine, and sandbox/CI
//! environments often have no real keyring available at all. This module's
//! correctness relies on the `keyring` crate's own test suite plus clean
//! compiling/type-checking against its real API (verified against `keyring`
//! 4.2.0, see `docs/PLUGIN_ARCHITECTURE.md`).

use crate::error::AppError;

const SERVICE_NAME: &str = "wartungsdoku";

/// Stores a plugin credential in the OS keyring, keyed by the plugin ID.
/// Never call this function with a value that should also live in
/// `config.toml` or the database -- per spec, plugin secrets live
/// EXCLUSIVELY here.
pub fn store_secret(plugin_id: &str, secret: &str) -> Result<(), AppError> {
    let entry = keyring::Entry::new(SERVICE_NAME, plugin_id)
        .map_err(|e| AppError::Config(format!("Schlüsselspeicher nicht verfügbar: {e}")))?;
    entry.set_password(secret).map_err(|e| {
        AppError::Config(format!(
            "Anmeldedaten konnten nicht gespeichert werden: {e}"
        ))
    })
}

/// Reads a previously stored plugin credential. `Ok(None)` if nothing has
/// been stored yet for this plugin ID.
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
