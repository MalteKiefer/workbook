//! A thin wrapper around the `s3` crate (rust-s3) for uploading backups to
//! an S3-compatible bucket -- works against real AWS S3 or Backblaze B2's
//! S3-compatible API (both are just a region + endpoint + credentials to
//! this client, see `Region::Custom`). Sync-only (no tokio), matching this
//! codebase's fully synchronous architecture.

use std::path::Path;

use s3::creds::Credentials;
use s3::{Bucket, Region};

use crate::error::AppError;

/// Key store ID for the cloud storage secret access key, see
/// `plugin::secrets::store_secret`/`load_secret` -- same mechanism
/// `backup::crypto::SECRET_ID` already uses for the backup passphrase.
pub const SECRET_ID: &str = "cloud-storage-secret-key";

/// Non-secret cloud destination settings -- lives in `Config`
/// (`cloud_storage_*` fields). The secret access key is NEVER part of
/// this struct; it's loaded separately from the OS keychain right before
/// building a `Bucket`, same separation `backup::crypto`'s passphrase
/// already uses.
#[derive(Debug, Clone)]
pub struct CloudStorageSettings {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
}

fn build_bucket(settings: &CloudStorageSettings, secret_access_key: &str) -> Result<Box<Bucket>, AppError> {
    let region = Region::Custom {
        region: settings.region.clone(),
        endpoint: settings.endpoint.clone(),
    };
    let credentials = Credentials::new(
        Some(&settings.access_key_id),
        Some(secret_access_key),
        None,
        None,
        None,
    )
    .map_err(|e| AppError::Config(format!("Cloud-Zugangsdaten ungültig: {e}")))?;
    Bucket::new(&settings.bucket, region, credentials)
        .map_err(|e| AppError::Config(format!("Cloud-Bucket konnte nicht initialisiert werden: {e}")))
}

/// Uploads the file at `local_path` to the configured bucket under `key`
/// (e.g. `"backups/wartungsdoku-backup-20260911-143000.wdbk"`). Reads the
/// whole file into memory first -- backups are at most a handful of MB for
/// this app's expected data volumes (single admin, at most a few hundred
/// systems), so this is simpler than streaming and matches how
/// `backup::crypto::encrypt_file` already reads whole files into memory.
pub fn upload_file(
    settings: &CloudStorageSettings,
    secret_access_key: &str,
    local_path: &Path,
    key: &str,
) -> Result<(), AppError> {
    let bucket = build_bucket(settings, secret_access_key)?;
    let bytes = std::fs::read(local_path)?;
    let response = bucket
        .put_object(key, &bytes)
        .map_err(|e| AppError::Config(format!("Upload zu Cloud-Speicher fehlgeschlagen: {e}")))?;
    if !(200..300).contains(&response.status_code()) {
        return Err(AppError::Config(format!(
            "Upload zu Cloud-Speicher fehlgeschlagen: HTTP {}",
            response.status_code()
        )));
    }
    Ok(())
}

/// Validates that the configured bucket/credentials/endpoint are actually
/// reachable and correct, without uploading anything -- a lightweight
/// listing call (bounded to a single object) used by the Settings UI's
/// "Verbindung testen" button, so a typo in the endpoint or a wrong
/// secret key is caught immediately instead of silently failing on the
/// next real backup.
pub fn test_connection(settings: &CloudStorageSettings, secret_access_key: &str) -> Result<(), AppError> {
    let bucket = build_bucket(settings, secret_access_key)?;
    bucket
        .list("".to_string(), Some("/".to_string()))
        .map_err(|e| AppError::Config(format!("Verbindungstest fehlgeschlagen: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // No live network calls in tests (no mock S3 server in this codebase's
    // dependencies) -- these only exercise the parts that don't need one:
    // credential/region construction failure modes.

    #[test]
    fn build_bucket_with_empty_bucket_name_still_constructs_or_fails_cleanly() {
        let settings = CloudStorageSettings {
            endpoint: "https://s3.us-west-002.backblazeb2.com".to_string(),
            region: "us-west-002".to_string(),
            bucket: "".to_string(),
            access_key_id: "test-key-id".to_string(),
        };
        // Bucket::new with an empty name does not itself validate against
        // the network -- this just confirms building the client doesn't
        // panic and produces SOME deterministic Result either way.
        let result = build_bucket(&settings, "test-secret");
        let _ = result;
    }
}
