//! Password-based encryption for backup zip files. AES-256-GCM for the
//! actual encryption, Argon2id to derive the key from the user's password --
//! both pure, well-audited Rust implementations (RustCrypto), no external
//! system libraries needed.
//!
//! The password itself never lives in `config.toml`, only in the OS key
//! store (see `plugin::secrets`, reused here under the fixed ID
//! [`SECRET_ID`] -- that module is generic enough to also fit backup
//! purposes).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::Argon2;
use rand::RngCore;

use crate::error::AppError;

/// Key store ID for the backup encryption password, see
/// `plugin::secrets::store_secret`/`load_secret`.
pub const SECRET_ID: &str = "backup-encryption";

/// Marks a file as encrypted by this module, including a format version --
/// if salt/nonce length or KDF parameters ever need to change,
/// `is_encrypted_file` can distinguish based on this.
const MAGIC: &[u8; 8] = b"WDBKENC1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], AppError> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| AppError::Backup(format!("Schlüsselableitung fehlgeschlagen: {e}")))?;
    Ok(key)
}

/// Encrypts the file `src` in full (e.g. a finished backup zip) and writes
/// the result to `dest`. Layout: `MAGIC || salt || nonce || ciphertext`,
/// salt and nonce random on every call so the same password never reuses the
/// same key/nonce twice.
pub fn encrypt_file(src: &Path, dest: &Path, passphrase: &str) -> Result<(), AppError> {
    let plaintext = std::fs::read(src)?;

    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let key_bytes = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| AppError::Backup("Verschlüsselung des Backups fehlgeschlagen".to_string()))?;

    let mut out = Vec::with_capacity(MAGIC.len() + SALT_LEN + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    std::fs::write(dest, out)?;
    Ok(())
}

/// Reverses `encrypt_file`. A wrong password or a corrupted file both lead
/// to the same error -- AES-GCM authenticates decryption, so there's no
/// byte-level difference between "wrong password" and
/// "tampered with/corrupted file".
pub fn decrypt_file(src: &Path, dest: &Path, passphrase: &str) -> Result<(), AppError> {
    let data = std::fs::read(src)?;
    let header_len = MAGIC.len() + SALT_LEN + NONCE_LEN;
    if data.len() < header_len || &data[..MAGIC.len()] != MAGIC {
        return Err(AppError::Backup(
            "Datei ist kein von dieser App verschlüsseltes Backup".to_string(),
        ));
    }
    let salt = &data[MAGIC.len()..MAGIC.len() + SALT_LEN];
    let nonce_bytes = &data[MAGIC.len() + SALT_LEN..header_len];
    let ciphertext = &data[header_len..];

    let key_bytes = derive_key(passphrase, salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        AppError::Backup(
            "Entschlüsselung fehlgeschlagen -- falsches Passwort oder beschädigte Datei"
                .to_string(),
        )
    })?;

    std::fs::write(dest, plaintext)?;
    Ok(())
}

/// Whether `path` was encrypted with `encrypt_file` -- checked via the magic
/// marker at the start of the file, without reading the rest of the file.
pub fn is_encrypted_file(path: &Path) -> Result<bool, AppError> {
    let mut buf = [0u8; 8];
    let mut file = File::open(path)?;
    let read = file.read(&mut buf)?;
    Ok(read == buf.len() && &buf == MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn encrypt_then_decrypt_roundtrips() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("plain.zip");
        std::fs::write(&src, b"fake zip bytes, does not need to be a real zip here").unwrap();
        let encrypted = dir.path().join("plain.wdbk");
        let decrypted = dir.path().join("plain-restored.zip");

        encrypt_file(&src, &encrypted, "correct horse battery staple").unwrap();
        decrypt_file(&encrypted, &decrypted, "correct horse battery staple").unwrap();

        assert_eq!(
            std::fs::read(&src).unwrap(),
            std::fs::read(&decrypted).unwrap()
        );
    }

    #[test]
    fn decrypt_with_wrong_passphrase_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("plain.zip");
        std::fs::write(&src, b"secret backup contents").unwrap();
        let encrypted = dir.path().join("plain.wdbk");
        encrypt_file(&src, &encrypted, "right password").unwrap();

        let decrypted = dir.path().join("out.zip");
        let result = decrypt_file(&encrypted, &decrypted, "wrong password");
        assert!(matches!(result, Err(AppError::Backup(_))));
    }

    #[test]
    fn is_encrypted_file_distinguishes_encrypted_from_plain() {
        let dir = tempdir().unwrap();
        let plain = dir.path().join("plain.zip");
        std::fs::write(&plain, b"PK\x03\x04 not encrypted").unwrap();
        let encrypted = dir.path().join("enc.wdbk");
        encrypt_file(&plain, &encrypted, "pw").unwrap();

        assert!(!is_encrypted_file(&plain).unwrap());
        assert!(is_encrypted_file(&encrypted).unwrap());
    }

    #[test]
    fn two_encryptions_of_same_content_produce_different_ciphertext() {
        // Random salt+nonce on every call -- important so the same password
        // never uses the same keystream twice.
        let dir = tempdir().unwrap();
        let src = dir.path().join("plain.zip");
        std::fs::write(&src, b"identical content").unwrap();
        let out1 = dir.path().join("out1.wdbk");
        let out2 = dir.path().join("out2.wdbk");

        encrypt_file(&src, &out1, "same password").unwrap();
        encrypt_file(&src, &out2, "same password").unwrap();

        assert_ne!(std::fs::read(&out1).unwrap(), std::fs::read(&out2).unwrap());
    }
}
