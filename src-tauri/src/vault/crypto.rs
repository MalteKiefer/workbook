//! Field-level encryption for the credential vault. Same primitives as
//! `backup::crypto` (AES-256-GCM, Argon2id), adapted from whole-file
//! encryption to short string values (passwords, notes) stored as base64
//! text in SQLite columns instead of on disk.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::Argon2;
use base64::prelude::*;
use rand::Rng;

use crate::error::AppError;

const NONCE_LEN: usize = 12;
/// Encrypted and stored in `Config::vault_canary` at vault setup time
/// (`commands::vault::set_vault_passphrase`). A later unlock attempt
/// (`commands::vault::unlock_vault`) decrypts this with the newly
/// entered passphrase's derived key -- AES-GCM's authentication tag
/// fails decryption on any wrong key, giving a clean "falsches
/// Master-Passwort" error the same way `backup::crypto::decrypt_file`
/// already does for backups.
const CANARY_PLAINTEXT: &str = "wartungsdoku-vault-ok";

pub fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], AppError> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| AppError::Validation(format!("Schlüsselableitung fehlgeschlagen: {e}")))?;
    Ok(key)
}

pub fn generate_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    salt
}

/// Encrypts `plaintext`, returning base64(nonce || ciphertext). A fresh
/// random nonce every call (same convention as
/// `backup::crypto::encrypt_file`), so the same key never reuses a
/// nonce.
pub fn encrypt(plaintext: &str, key: &[u8; 32]) -> Result<String, AppError> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key));
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| AppError::Validation("Verschlüsselung fehlgeschlagen".to_string()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(BASE64_STANDARD.encode(out))
}

/// Reverses `encrypt`. A wrong key and corrupted data both surface as
/// the same error (AES-GCM authenticates, same as
/// `backup::crypto::decrypt_file`).
pub fn decrypt(encoded: &str, key: &[u8; 32]) -> Result<String, AppError> {
    let data = BASE64_STANDARD
        .decode(encoded)
        .map_err(|e| AppError::Validation(format!("Ungültige Tresordaten: {e}")))?;
    if data.len() < NONCE_LEN {
        return Err(AppError::Validation("Ungültige Tresordaten".to_string()));
    }
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key));
    let nonce = Nonce::try_from(nonce_bytes)
        .map_err(|_| AppError::Validation("Ungültige Nonce-Länge".to_string()))?;
    let plaintext = cipher.decrypt(&nonce, ciphertext).map_err(|_| {
        AppError::Validation(
            "Entschlüsselung fehlgeschlagen -- falsches Master-Passwort oder beschädigte Daten"
                .to_string(),
        )
    })?;
    String::from_utf8(plaintext)
        .map_err(|e| AppError::Validation(format!("Ungültige UTF-8-Daten im Tresor: {e}")))
}

pub fn encrypt_canary(key: &[u8; 32]) -> Result<String, AppError> {
    encrypt(CANARY_PLAINTEXT, key)
}

/// `Ok(false)` (not an error) for a wrong passphrase -- the caller turns
/// that into a clean "falsches Master-Passwort" `AppError::Validation`
/// at the command layer, see Task 5.
pub fn verify_canary(encoded: &str, key: &[u8; 32]) -> Result<bool, AppError> {
    match decrypt(encoded, key) {
        Ok(plaintext) => Ok(plaintext == CANARY_PLAINTEXT),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_roundtrips() {
        let salt = generate_salt();
        let key = derive_key("correct horse battery staple", &salt).unwrap();
        let encrypted = encrypt("hunter2", &key).unwrap();
        assert_eq!(decrypt(&encrypted, &key).unwrap(), "hunter2");
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let salt = generate_salt();
        let key = derive_key("right password", &salt).unwrap();
        let wrong_key = derive_key("wrong password", &salt).unwrap();
        let encrypted = encrypt("secret value", &key).unwrap();
        assert!(decrypt(&encrypted, &wrong_key).is_err());
    }

    #[test]
    fn two_encryptions_of_same_value_produce_different_ciphertext() {
        let salt = generate_salt();
        let key = derive_key("pw", &salt).unwrap();
        let a = encrypt("identical content", &key).unwrap();
        let b = encrypt("identical content", &key).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn canary_verifies_correct_key_and_rejects_wrong_key() {
        let salt = generate_salt();
        let key = derive_key("correct horse battery staple", &salt).unwrap();
        let wrong_key = derive_key("wrong horse", &salt).unwrap();
        let canary = encrypt_canary(&key).unwrap();
        assert!(verify_canary(&canary, &key).unwrap());
        assert!(!verify_canary(&canary, &wrong_key).unwrap());
    }
}
