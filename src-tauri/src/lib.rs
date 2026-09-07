pub mod attachments;
pub mod commands;
pub mod config;
pub mod context_capture;
pub mod db;
pub mod error;
pub mod quickcapture;
pub mod time;
pub mod tray;
pub mod window;

pub use error::AppError;

use std::sync::Mutex;

use tauri::Manager;

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
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            window::show_and_focus_main(app);
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(AppState { pool, config: Mutex::new(app_config) })
        .setup(|app| {
            window::install_hide_on_close(app.handle());
            if let Err(e) = tray::build_tray(app.handle()) {
                eprintln!("Tray konnte nicht eingerichtet werden: {e}");
            }

            let quick_capture_window = tauri::WebviewWindowBuilder::new(
                app,
                "quick-capture",
                tauri::WebviewUrl::App("quick-capture.html".into()),
            )
            .title("Schnellerfassung")
            .inner_size(560.0, 420.0)
            .center()
            .resizable(false)
            .visible(false)
            .build()?;
            window::install_hide_on_close_for(&quick_capture_window);

            let state = app.state::<AppState>();
            let autostart_enabled = state.config.lock().expect("Config-Mutex vergiftet").autostart_enabled;
            use tauri_plugin_autostart::ManagerExt;
            let autolaunch = app.autolaunch();
            let sync_result = if autostart_enabled { autolaunch.enable() } else { autolaunch.disable() };
            if let Err(e) = sync_result {
                eprintln!("Autostart konnte nicht synchronisiert werden: {e}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::customers::list_customers,
            commands::customers::create_customer,
            commands::customers::update_customer,
            commands::customers::archive_customer,
            commands::systems::list_systems,
            commands::systems::create_system,
            commands::systems::update_system,
            commands::systems::archive_system,
            commands::tags::list_tags,
            commands::entries::list_entries,
            commands::entries::get_entry,
            commands::entries::create_entry,
            commands::entries::update_entry,
            commands::entries::parse_temporal_input,
            commands::entries::format_timestamp_for_display,
            commands::quickcapture::get_last_selection,
            commands::search::search_entries,
            commands::search::search_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
