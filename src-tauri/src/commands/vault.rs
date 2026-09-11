use base64::prelude::*;
use tauri::State;

use crate::db::vault::{self, NewVaultEntry, UpdateVaultEntry, VaultEntry};
use crate::vault::crypto;
use crate::{time, AppError, AppState};

#[tauri::command]
pub fn has_vault_passphrase(state: State<AppState>) -> bool {
    state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .vault_salt
        .is_some()
}

#[tauri::command]
pub fn is_vault_unlocked(state: State<AppState>) -> bool {
    state
        .vault_key
        .lock()
        .expect("Vault-Key-Mutex vergiftet")
        .is_some()
}

/// First-time setup: generates a salt, derives the key from `passphrase`,
/// stores the salt + an encrypted canary in `config.toml`, stores
/// `passphrase` itself in the OS keychain (same mechanism as the backup
/// encryption passphrase, see `backup::crypto::SECRET_ID`), and leaves the
/// vault unlocked for this session.
#[tauri::command]
pub fn set_vault_passphrase(state: State<AppState>, passphrase: String) -> Result<(), AppError> {
    if state
        .config
        .lock()
        .expect("Config-Mutex vergiftet")
        .vault_salt
        .is_some()
    {
        return Err(AppError::Validation(
            "Tresor ist bereits eingerichtet.".to_string(),
        ));
    }
    if passphrase.trim().is_empty() {
        return Err(AppError::Validation(
            "Master-Passwort darf nicht leer sein.".to_string(),
        ));
    }
    let salt = crypto::generate_salt();
    let key = crypto::derive_key(&passphrase, &salt)?;
    let canary = crypto::encrypt_canary(&key)?;

    crate::plugin::secrets::store_secret("credential-vault", &passphrase)?;

    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.vault_salt = Some(BASE64_STANDARD.encode(salt));
    config.vault_canary = Some(canary);
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)?;
    drop(config);

    *state.vault_key.lock().expect("Vault-Key-Mutex vergiftet") = Some(key);
    Ok(())
}

/// Unlocks an already-set-up vault for this session. Wrong `passphrase`
/// surfaces as `AppError::Validation` (via the canary check), never
/// silently "unlocks" with a key that would just fail to decrypt every
/// entry later.
#[tauri::command]
pub fn unlock_vault(state: State<AppState>, passphrase: String) -> Result<(), AppError> {
    let (salt_b64, canary) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        let salt_b64 = config.vault_salt.clone().ok_or_else(|| {
            AppError::Validation("Tresor ist noch nicht eingerichtet.".to_string())
        })?;
        let canary = config.vault_canary.clone().ok_or_else(|| {
            AppError::Validation("Tresor ist noch nicht eingerichtet.".to_string())
        })?;
        (salt_b64, canary)
    };
    let salt = BASE64_STANDARD
        .decode(&salt_b64)
        .map_err(|e| AppError::Validation(format!("Ungültiges Salt in der Konfiguration: {e}")))?;
    let key = crypto::derive_key(&passphrase, &salt)?;
    if !crypto::verify_canary(&canary, &key)? {
        return Err(AppError::Validation(
            "Falsches Master-Passwort.".to_string(),
        ));
    }
    *state.vault_key.lock().expect("Vault-Key-Mutex vergiftet") = Some(key);
    Ok(())
}

#[tauri::command]
pub fn lock_vault(state: State<AppState>) {
    *state.vault_key.lock().expect("Vault-Key-Mutex vergiftet") = None;
}

fn require_unlocked_key(state: &State<AppState>) -> Result<[u8; 32], AppError> {
    state
        .vault_key
        .lock()
        .expect("Vault-Key-Mutex vergiftet")
        .ok_or_else(|| AppError::Validation("Tresor ist gesperrt.".to_string()))
}

#[tauri::command]
pub fn list_vault_entries_for_customer(
    state: State<AppState>,
    customer_id: i64,
) -> Result<Vec<VaultEntry>, AppError> {
    let key = require_unlocked_key(&state)?;
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    vault::list_for_customer(&conn, customer_id, &key)
}

#[tauri::command]
pub fn create_vault_entry(
    state: State<AppState>,
    input: NewVaultEntry,
) -> Result<VaultEntry, AppError> {
    let key = require_unlocked_key(&state)?;
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    vault::create(&conn, input, &key, &tz)
}

#[tauri::command]
pub fn update_vault_entry(
    state: State<AppState>,
    id: i64,
    input: UpdateVaultEntry,
) -> Result<VaultEntry, AppError> {
    let key = require_unlocked_key(&state)?;
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let tz = time::system_timezone()?;
    vault::update(&conn, id, input, &key, &tz)
}

#[tauri::command]
pub fn delete_vault_entry(state: State<AppState>, id: i64) -> Result<(), AppError> {
    // No key needed to delete a row -- there's nothing to decrypt -- but
    // still gate it on the vault being unlocked so a locked vault can't
    // be modified (destructively or otherwise) by someone with only
    // physical/IPC access to the running app, not the passphrase.
    require_unlocked_key(&state)?;
    let conn = state
        .pool
        .get()
        .map_err(|e| AppError::Database(e.to_string()))?;
    vault::delete(&conn, id)
}
