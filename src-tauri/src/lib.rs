pub mod config;
pub mod db;
pub mod error;
pub mod time;

pub use error::AppError;

use std::sync::Mutex;

use crate::config::Config;
use crate::db::pool::{build_pool, DbPool};

pub struct AppState {
    pub pool: DbPool,
    pub config: Mutex<Config>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let data_dir = config::resolve_data_dir();
    let config_path = data_dir.join("config.toml");
    let mut app_config = Config::load_or_default(&config_path).expect("Konfiguration konnte nicht geladen werden");
    app_config.data_dir = data_dir.clone();
    app_config.save(&config_path).expect("Konfiguration konnte nicht gespeichert werden");

    let db_path = data_dir.join("wartungsdoku.db");
    let pool = build_pool(&db_path).expect("Datenbank-Pool konnte nicht erstellt werden");
    {
        let system_tz = time::system_timezone().expect("Systemzeitzone konnte nicht ermittelt werden");
        let mut conn = pool.get().expect("Keine Datenbankverbindung verfügbar");
        db::migrations::run_migrations(&mut conn, &db_path, &system_tz).expect("Migration fehlgeschlagen");
    }

    tauri::Builder::default()
        .manage(AppState { pool, config: Mutex::new(app_config) })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
