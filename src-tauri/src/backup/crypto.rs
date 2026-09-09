//! Passwortbasierte Verschlüsselung für Backup-Zip-Dateien. AES-256-GCM für
//! die eigentliche Verschlüsselung, Argon2id zur Ableitung des Schlüssels aus
//! dem Nutzerpasswort -- beides reine, gut geprüfte Rust-Implementierungen
//! (RustCrypto), keine externen Systembibliotheken nötig.
//!
//! Das Passwort selbst liegt nie in `config.toml`, nur im OS-Schlüsselspeicher
//! (siehe `plugin::secrets`, hier unter der festen ID [`SECRET_ID`]
//! wiederverwendet -- das Modul ist generisch genug, um auch für
//! Backup-Zwecke zu passen).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::Argon2;
use rand::RngCore;

use crate::error::AppError;

/// Schlüsselspeicher-ID für das Backup-Verschlüsselungspasswort, siehe
/// `plugin::secrets::store_secret`/`load_secret`.
pub const SECRET_ID: &str = "backup-encryption";

/// Kennzeichnet eine Datei als von diesem Modul verschlüsselt, inklusive
/// Formatversion -- falls sich Salt-/Nonce-Länge oder KDF-Parameter je einmal
/// ändern müssen, kann `is_encrypted_file` anhand davon unterscheiden.
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

/// Verschlüsselt die Datei `src` komplett (z. B. ein fertiges Backup-Zip) und
/// schreibt das Ergebnis nach `dest`. Layout: `MAGIC || salt || nonce ||
/// ciphertext`, Salt und Nonce zufällig je Aufruf, damit dasselbe Passwort nie
/// denselben Schlüssel/Nonce zweimal verwendet.
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

/// Kehrt `encrypt_file` um. Ein falsches Passwort oder eine beschädigte Datei
/// führen beide zu demselben Fehler -- AES-GCM authentifiziert die
/// Entschlüsselung, sodass es keinen Unterschied zwischen "falsches Passwort"
/// und "manipulierte/kaputte Datei" auf Byte-Ebene gibt.
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

/// Ob `path` mit `encrypt_file` verschlüsselt wurde -- geprüft anhand der
/// magischen Kennung am Dateianfang, ohne die restliche Datei zu lesen.
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
        // Zufälliges Salt+Nonce je Aufruf -- wichtig, damit dasselbe Passwort
        // niemals denselben Schlüsselstrom zweimal benutzt.
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
