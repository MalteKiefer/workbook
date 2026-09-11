use tauri::State;

use crate::cloud_storage::{self, CloudStorageSettings};
use crate::{plugin::secrets, AppError, AppState};

#[derive(Debug, serde::Serialize)]
pub struct CloudStorageSettingsDto {
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub access_key_id: Option<String>,
    pub has_secret_key: bool,
}

#[tauri::command]
pub fn get_cloud_storage_settings(state: State<AppState>) -> Result<CloudStorageSettingsDto, AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    Ok(CloudStorageSettingsDto {
        enabled: config.cloud_storage_enabled,
        endpoint: config.cloud_storage_endpoint.clone(),
        region: config.cloud_storage_region.clone(),
        bucket: config.cloud_storage_bucket.clone(),
        access_key_id: config.cloud_storage_access_key_id.clone(),
        has_secret_key: secrets::load_secret(cloud_storage::SECRET_ID)?.is_some(),
    })
}

#[tauri::command]
pub fn set_cloud_storage_settings(
    state: State<AppState>,
    enabled: bool,
    endpoint: Option<String>,
    region: Option<String>,
    bucket: Option<String>,
    access_key_id: Option<String>,
) -> Result<(), AppError> {
    let mut config = state.config.lock().expect("Config-Mutex vergiftet");
    config.cloud_storage_enabled = enabled;
    config.cloud_storage_endpoint = endpoint;
    config.cloud_storage_region = region;
    config.cloud_storage_bucket = bucket;
    config.cloud_storage_access_key_id = access_key_id;
    let config_path = config.data_dir.join("config.toml");
    config.save(&config_path)
}

#[tauri::command]
pub fn set_cloud_storage_secret_key(secret_key: String) -> Result<(), AppError> {
    secrets::store_secret(cloud_storage::SECRET_ID, &secret_key)
}

pub(crate) fn current_cloud_storage_settings(
    state: &State<AppState>,
) -> Result<(CloudStorageSettings, String), AppError> {
    let config = state.config.lock().expect("Config-Mutex vergiftet");
    let settings = CloudStorageSettings {
        endpoint: config
            .cloud_storage_endpoint
            .clone()
            .ok_or_else(|| AppError::Validation("Endpoint ist nicht gesetzt.".to_string()))?,
        region: config
            .cloud_storage_region
            .clone()
            .ok_or_else(|| AppError::Validation("Region ist nicht gesetzt.".to_string()))?,
        bucket: config
            .cloud_storage_bucket
            .clone()
            .ok_or_else(|| AppError::Validation("Bucket ist nicht gesetzt.".to_string()))?,
        access_key_id: config
            .cloud_storage_access_key_id
            .clone()
            .ok_or_else(|| AppError::Validation("Access Key ID ist nicht gesetzt.".to_string()))?,
    };
    drop(config);
    let secret_key = secrets::load_secret(cloud_storage::SECRET_ID)?
        .ok_or_else(|| AppError::Validation("Secret Access Key ist nicht gesetzt.".to_string()))?;
    Ok((settings, secret_key))
}

#[tauri::command]
pub fn test_cloud_storage_connection(state: State<AppState>) -> Result<(), AppError> {
    let (settings, secret_key) = current_cloud_storage_settings(&state)?;
    cloud_storage::test_connection(&settings, &secret_key)
}
