pub mod attachments;
pub mod backup;
pub mod cli;
pub mod clipboard;
pub mod commands;
pub mod config;
pub mod context_capture;
pub mod db;
pub mod error;
pub mod export;
pub mod hotkeys;
pub mod import;
pub mod maintenance;
pub mod plugin;
pub mod quickcapture;
pub mod time;
pub mod tray;
pub mod updater;
pub mod window;

pub use error::AppError;

use std::sync::Mutex;

use tauri::{Emitter, Manager};

use crate::config::Config;
use crate::db::pool::{build_pool, DbPool};

pub struct AppState {
    pub pool: DbPool,
    pub config: Mutex<Config>,
    pub previous_foreground: Mutex<Option<context_capture::ForegroundHandle>>,
    /// Cross-customer count of overdue systems (`commands::systems::list_overdue_systems`),
    /// recomputed every `MAINTENANCE_CHECK_INTERVAL` by `run_maintenance_check` and
    /// reflected in the tray tooltip (`tray::sync_tray_tooltip`) and the
    /// `maintenance-check-completed` event. Deliberately NOT persisted to
    /// `config.toml` -- it's cheap to recompute on every launch, and there's no
    /// reason to serialize it.
    pub overdue_systems_count: Mutex<usize>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let data_dir = config::resolve_data_dir();
    let config_path = data_dir.join("config.toml");
    let mut app_config =
        Config::load_or_default(&config_path).expect("Konfiguration konnte nicht geladen werden");
    app_config.data_dir = data_dir.clone();
    app_config
        .save(&config_path)
        .expect("Konfiguration konnte nicht gespeichert werden");

    if let Err(e) = backup::apply_pending_restore_if_present(&data_dir) {
        eprintln!("Ausstehende Wiederherstellung konnte nicht angewendet werden: {e}");
    }

    let db_path = data_dir.join(backup::DB_FILE_NAME);
    let pool = build_pool(&db_path).expect("Datenbank-Pool konnte nicht erstellt werden");
    {
        let system_tz =
            time::system_timezone().expect("Systemzeitzone konnte nicht ermittelt werden");
        let mut conn = pool.get().expect("Keine Datenbankverbindung verfügbar");
        db::migrations::run_migrations(&mut conn, &db_path, &system_tz)
            .expect("Migration fehlgeschlagen");
    }

    let hotkey_config = app_config.hotkeys.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let action = cli::parse_args(&argv);
            if action == cli::CliAction::None {
                window::show_and_focus_main(app);
            } else {
                cli::dispatch(app, action);
            }
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            pool,
            config: Mutex::new(app_config),
            previous_foreground: Mutex::new(None),
            overdue_systems_count: Mutex::new(0),
        })
        .setup(move |app| {
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
            .inner_size(520.0, 560.0)
            .min_inner_size(420.0, 320.0)
            .center()
            .resizable(true)
            .visible(false)
            .build()?;
            window::install_hide_on_close_for(&quick_capture_window);

            let state = app.state::<AppState>();
            let autostart_enabled = state
                .config
                .lock()
                .expect("Config-Mutex vergiftet")
                .autostart_enabled;
            use tauri_plugin_autostart::ManagerExt;
            let autolaunch = app.autolaunch();
            let sync_result = if autostart_enabled {
                autolaunch.enable()
            } else {
                autolaunch.disable()
            };
            if let Err(e) = sync_result {
                eprintln!("Autostart konnte nicht synchronisiert werden: {e}");
            }

            if let Err(e) = hotkeys::register(app.handle(), &hotkey_config) {
                eprintln!("Globale Hotkeys konnten nicht registriert werden: {e}");
            }

            let first_launch_args: Vec<String> = std::env::args().collect();
            cli::dispatch(app.handle(), cli::parse_args(&first_launch_args));

            spawn_auto_backup_scheduler(app.handle().clone());
            spawn_auto_update_check_scheduler(app.handle().clone());
            spawn_maintenance_check_scheduler(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::customers::list_customers,
            commands::customers::create_customer,
            commands::customers::update_customer,
            commands::customers::archive_customer,
            commands::customers::archive_customers,
            commands::customers::import_customers_from_csv,
            commands::systems::list_systems,
            commands::systems::create_system,
            commands::systems::update_system,
            commands::systems::archive_system,
            commands::systems::archive_systems,
            commands::systems::bulk_set_maintenance_interval,
            commands::systems::list_systems_with_maintenance_status,
            commands::systems::list_overdue_systems,
            commands::systems::import_systems_from_csv,
            commands::tags::list_tags,
            commands::tags::list_tags_with_counts,
            commands::entries::list_entries,
            commands::entries::get_entry,
            commands::entries::create_entry,
            commands::entries::update_entry,
            commands::entries::parse_temporal_input,
            commands::entries::format_timestamp_for_display,
            commands::entry_templates::list_entry_templates,
            commands::entry_templates::create_entry_template,
            commands::entry_templates::update_entry_template,
            commands::entry_templates::delete_entry_template,
            commands::expiring_items::list_expiring_items,
            commands::expiring_items::create_expiring_item,
            commands::expiring_items::update_expiring_item,
            commands::expiring_items::delete_expiring_item,
            commands::settings::get_theme_preference,
            commands::settings::set_theme_preference,
            commands::settings::get_late_entry_threshold_hours,
            commands::keymap::get_keymap,
            commands::keymap::set_keymap,
            commands::keymap::get_hotkeys,
            commands::keymap::set_hotkeys,
            commands::quickcapture::get_last_selection,
            commands::quickcapture::quick_capture_close,
            commands::quickcapture::open_quick_capture_with_context,
            commands::search::search_entries,
            commands::search::search_directory,
            commands::attachments::list_attachments_for_entry,
            commands::attachments::add_attachment_to_entry,
            commands::attachments::remove_attachment,
            commands::attachments::read_attachment_data_url,
            commands::attachments::copy_attachment_to,
            commands::attachments::cleanup_orphans,
            commands::attachments::get_attachment_storage_summary,
            commands::attachments::open_attachment,
            commands::export::export_markdown,
            commands::export::export_pdf,
            commands::export::export_markdown_all_customers,
            commands::export::export_pdf_all_customers,
            commands::backup::create_backup,
            commands::backup::restore_backup,
            commands::backup::is_backup_file_encrypted,
            commands::backup::get_backup_settings,
            commands::backup::set_auto_backup_settings,
            commands::backup::set_backup_encryption_enabled,
            commands::backup::set_backup_encryption_passphrase,
            commands::audit_log::list_audit_log_for_entity,
            commands::plugins::test_ninja_connection,
            commands::plugins::list_ninja_connections,
            commands::plugins::add_ninja_connection,
            commands::plugins::remove_ninja_connection,
            commands::plugins::list_ninja_organizations,
            commands::plugins::map_ninja_organization,
            commands::plugins::unmap_ninja_organization,
            commands::plugins::sync_ninja_connection,
            commands::plugins::get_cached_ninja_sync,
            commands::plugins::link_system_to_ninja,
            commands::plugins::unlink_system_from_ninja,
            commands::plugins::get_ninja_system_details,
            commands::level::test_level_connection,
            commands::level::list_level_connections,
            commands::level::add_level_connection,
            commands::level::remove_level_connection,
            commands::level::sync_level_connection,
            commands::level::get_cached_level_sync,
            commands::level::link_system_to_level,
            commands::level::unlink_system_from_level,
            commands::level::get_level_system_details,
            commands::snipeit::test_snipeit_connection,
            commands::snipeit::list_snipeit_connections,
            commands::snipeit::add_snipeit_connection,
            commands::snipeit::remove_snipeit_connection,
            commands::snipeit::list_snipeit_companies,
            commands::snipeit::map_snipeit_company,
            commands::snipeit::unmap_snipeit_company,
            commands::snipeit::sync_snipeit_connection,
            commands::snipeit::get_cached_snipeit_sync,
            commands::snipeit::link_system_to_snipeit,
            commands::snipeit::unlink_system_from_snipeit,
            commands::snipeit::get_snipeit_system_details,
            commands::intune::test_intune_connection,
            commands::intune::list_intune_connections,
            commands::intune::add_intune_connection,
            commands::intune::remove_intune_connection,
            commands::intune::sync_intune_connection,
            commands::intune::get_cached_intune_sync,
            commands::intune::link_system_to_intune,
            commands::intune::unlink_system_from_intune,
            commands::intune::get_intune_system_details,
            commands::iru::test_iru_connection,
            commands::iru::list_iru_connections,
            commands::iru::add_iru_connection,
            commands::iru::remove_iru_connection,
            commands::iru::sync_iru_connection,
            commands::iru::get_cached_iru_sync,
            commands::iru::link_system_to_iru,
            commands::iru::unlink_system_from_iru,
            commands::iru::get_iru_system_details,
            commands::jamf::test_jamf_connection,
            commands::jamf::list_jamf_connections,
            commands::jamf::add_jamf_connection,
            commands::jamf::remove_jamf_connection,
            commands::jamf::list_jamf_sites,
            commands::jamf::map_jamf_site,
            commands::jamf::unmap_jamf_site,
            commands::jamf::sync_jamf_connection,
            commands::jamf::get_cached_jamf_sync,
            commands::jamf::link_system_to_jamf,
            commands::jamf::unlink_system_from_jamf,
            commands::jamf::get_jamf_system_details,
            commands::abm::test_abm_connection,
            commands::abm::list_abm_connections,
            commands::abm::add_abm_connection,
            commands::abm::remove_abm_connection,
            commands::abm::sync_abm_connection,
            commands::abm::get_cached_abm_sync,
            commands::abm::link_system_to_abm,
            commands::abm::unlink_system_from_abm,
            commands::abm::get_abm_system_details,
            commands::tacticalrmm::test_tacticalrmm_connection,
            commands::tacticalrmm::list_tacticalrmm_connections,
            commands::tacticalrmm::add_tacticalrmm_connection,
            commands::tacticalrmm::remove_tacticalrmm_connection,
            commands::tacticalrmm::list_tacticalrmm_clients,
            commands::tacticalrmm::map_tacticalrmm_client,
            commands::tacticalrmm::unmap_tacticalrmm_client,
            commands::tacticalrmm::sync_tacticalrmm_connection,
            commands::tacticalrmm::get_cached_tacticalrmm_sync,
            commands::tacticalrmm::link_system_to_tacticalrmm,
            commands::tacticalrmm::unlink_system_from_tacticalrmm,
            commands::tacticalrmm::get_tacticalrmm_system_details,
            commands::atera::test_atera_connection,
            commands::atera::list_atera_connections,
            commands::atera::add_atera_connection,
            commands::atera::remove_atera_connection,
            commands::atera::list_atera_customers,
            commands::atera::map_atera_customer,
            commands::atera::unmap_atera_customer,
            commands::atera::sync_atera_connection,
            commands::atera::get_cached_atera_sync,
            commands::atera::link_system_to_atera,
            commands::atera::unlink_system_from_atera,
            commands::atera::get_atera_system_details,
            commands::pulseway::test_pulseway_connection,
            commands::pulseway::list_pulseway_connections,
            commands::pulseway::add_pulseway_connection,
            commands::pulseway::remove_pulseway_connection,
            commands::pulseway::list_pulseway_organizations,
            commands::pulseway::map_pulseway_organization,
            commands::pulseway::unmap_pulseway_organization,
            commands::pulseway::sync_pulseway_connection,
            commands::pulseway::get_cached_pulseway_sync,
            commands::pulseway::link_system_to_pulseway,
            commands::pulseway::unlink_system_from_pulseway,
            commands::pulseway::get_pulseway_system_details,
            commands::kaseya::test_kaseya_connection,
            commands::kaseya::list_kaseya_connections,
            commands::kaseya::add_kaseya_connection,
            commands::kaseya::remove_kaseya_connection,
            commands::kaseya::list_kaseya_organizations,
            commands::kaseya::map_kaseya_organization,
            commands::kaseya::unmap_kaseya_organization,
            commands::kaseya::sync_kaseya_connection,
            commands::kaseya::get_cached_kaseya_sync,
            commands::kaseya::link_system_to_kaseya,
            commands::kaseya::unlink_system_from_kaseya,
            commands::kaseya::get_kaseya_system_details,
            commands::action1::test_action1_connection,
            commands::action1::list_action1_connections,
            commands::action1::add_action1_connection,
            commands::action1::remove_action1_connection,
            commands::action1::list_action1_organizations,
            commands::action1::map_action1_organization,
            commands::action1::unmap_action1_organization,
            commands::action1::sync_action1_connection,
            commands::action1::get_cached_action1_sync,
            commands::action1::link_system_to_action1,
            commands::action1::unlink_system_from_action1,
            commands::action1::get_action1_system_details,
            commands::dattormm::test_dattormm_connection,
            commands::dattormm::list_dattormm_connections,
            commands::dattormm::add_dattormm_connection,
            commands::dattormm::remove_dattormm_connection,
            commands::dattormm::list_dattormm_sites,
            commands::dattormm::map_dattormm_site,
            commands::dattormm::unmap_dattormm_site,
            commands::dattormm::sync_dattormm_connection,
            commands::dattormm::get_cached_dattormm_sync,
            commands::dattormm::link_system_to_dattormm,
            commands::dattormm::unlink_system_from_dattormm,
            commands::dattormm::get_dattormm_system_details,
            commands::acronis::test_acronis_connection,
            commands::acronis::list_acronis_connections,
            commands::acronis::add_acronis_connection,
            commands::acronis::remove_acronis_connection,
            commands::acronis::list_acronis_tenants,
            commands::acronis::map_acronis_tenant,
            commands::acronis::unmap_acronis_tenant,
            commands::acronis::sync_acronis_connection,
            commands::acronis::get_cached_acronis_sync,
            commands::acronis::link_system_to_acronis,
            commands::acronis::unlink_system_from_acronis,
            commands::acronis::get_acronis_system_details,
            commands::hetzner::test_hetzner_connection,
            commands::hetzner::list_hetzner_connections,
            commands::hetzner::add_hetzner_connection,
            commands::hetzner::remove_hetzner_connection,
            commands::hetzner::sync_hetzner_connection,
            commands::hetzner::get_cached_hetzner_sync,
            commands::hetzner::link_system_to_hetzner,
            commands::hetzner::unlink_system_from_hetzner,
            commands::hetzner::get_hetzner_system_details,
            commands::netcup::test_netcup_connection,
            commands::netcup::list_netcup_connections,
            commands::netcup::add_netcup_connection,
            commands::netcup::remove_netcup_connection,
            commands::netcup::sync_netcup_connection,
            commands::netcup::get_cached_netcup_sync,
            commands::netcup::link_system_to_netcup,
            commands::netcup::unlink_system_from_netcup,
            commands::netcup::get_netcup_system_details,
            commands::vultr::test_vultr_connection,
            commands::vultr::list_vultr_connections,
            commands::vultr::add_vultr_connection,
            commands::vultr::remove_vultr_connection,
            commands::vultr::sync_vultr_connection,
            commands::vultr::get_cached_vultr_sync,
            commands::vultr::link_system_to_vultr,
            commands::vultr::unlink_system_from_vultr,
            commands::vultr::get_vultr_system_details,
            commands::external_directory::list_unlinked_external_systems_for_customer,
            commands::updater::get_update_check_settings,
            commands::updater::set_auto_update_check_settings,
            commands::updater::record_update_check_result,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Interval between checks for a due automatic backup. Doesn't need to be
/// anywhere near as fine-grained as the shortest configurable frequency
/// (daily) -- this just needs to notice "due" reasonably soon after it
/// becomes true, not immediately.
const AUTO_BACKUP_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Starts a background thread that periodically checks whether an automatic
/// backup is due (per `backup::is_auto_backup_due`) and, if so, creates one.
/// A plain `std::thread` loop rather than an async task -- this app has no
/// other need for an async runtime, and a sleep-then-check loop needs none
/// either.
fn spawn_auto_backup_scheduler(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        run_auto_backup_if_due(&app);
        std::thread::sleep(AUTO_BACKUP_CHECK_INTERVAL);
    });
}

fn run_auto_backup_if_due(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();

    let (dest_dir, frequency, last_run_utc, encryption_enabled, data_dir) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        if !config.auto_backup_enabled {
            return;
        }
        let Some(dest_dir) = config.auto_backup_dir.clone() else {
            return;
        };
        (
            dest_dir,
            config.auto_backup_frequency,
            config.auto_backup_last_run_utc.clone(),
            config.backup_encryption_enabled,
            config.data_dir.clone(),
        )
    };

    let now = chrono::Utc::now();
    if !backup::is_auto_backup_due(frequency, last_run_utc.as_deref(), now) {
        return;
    }

    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        eprintln!("Automatisches Backup: Zielordner konnte nicht angelegt werden: {e}");
        return;
    }

    let extension = if encryption_enabled { "wdbk" } else { "zip" };
    let dest_path = dest_dir.join(format!(
        "wartungsdoku-auto-{}.{extension}",
        now.format("%Y%m%d-%H%M%S")
    ));

    let conn = match state.pool.get() {
        Ok(conn) => conn,
        Err(e) => {
            eprintln!("Automatisches Backup: keine Datenbankverbindung verfügbar: {e}");
            return;
        }
    };

    let result = if encryption_enabled {
        match plugin::secrets::load_secret(backup::crypto::SECRET_ID) {
            Ok(Some(passphrase)) => {
                backup::create_backup_encrypted(&conn, &data_dir, &dest_path, &passphrase)
            }
            Ok(None) => {
                eprintln!(
                    "Automatisches Backup übersprungen: Verschlüsselung aktiviert, aber kein Passwort gesetzt."
                );
                return;
            }
            Err(e) => {
                eprintln!("Automatisches Backup: Passwort konnte nicht gelesen werden: {e}");
                return;
            }
        }
    } else {
        backup::create_backup(&conn, &data_dir, &dest_path)
    };
    drop(conn);

    match result {
        Ok(()) => {
            let mut config = state.config.lock().expect("Config-Mutex vergiftet");
            config.auto_backup_last_run_utc = Some(now.to_rfc3339());
            let config_path = config.data_dir.join("config.toml");
            if let Err(e) = config.save(&config_path) {
                eprintln!("Automatisches Backup: Zeitstempel konnte nicht gespeichert werden: {e}");
            }
        }
        Err(e) => eprintln!("Automatisches Backup fehlgeschlagen: {e}"),
    }
}

/// Interval between checks for a due automatic update check. Same
/// reasoning as `AUTO_BACKUP_CHECK_INTERVAL`: coarse-grained is fine, this
/// just needs to notice "due" reasonably soon, not immediately.
const AUTO_UPDATE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Starts a background thread that periodically checks whether an
/// automatic update check is due (per `updater::is_auto_update_check_due`)
/// and, if so, asks the updater plugin whether a newer version exists.
/// Same plain-`std::thread` sleep-poll design as
/// `spawn_auto_backup_scheduler`, for the same reason.
fn spawn_auto_update_check_scheduler(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        run_auto_update_check_if_due(&app);
        std::thread::sleep(AUTO_UPDATE_CHECK_INTERVAL);
    });
}

fn run_auto_update_check_if_due(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();

    let (frequency, last_run_utc) = {
        let config = state.config.lock().expect("Config-Mutex vergiftet");
        if !config.auto_update_check_enabled {
            return;
        }
        (
            config.auto_update_check_frequency,
            config.auto_update_check_last_run_utc.clone(),
        )
    };

    let now = chrono::Utc::now();
    if !updater::is_auto_update_check_due(frequency, last_run_utc.as_deref(), now) {
        return;
    }

    use tauri_plugin_updater::UpdaterExt;
    let plugin_updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Automatische Update-Suche: Updater konnte nicht initialisiert werden: {e}");
            return;
        }
    };

    // `check()` is async (it makes an HTTP request); this scheduler is a
    // plain `std::thread` loop with no async runtime of its own, so it
    // blocks on Tauri's own async runtime the same way the plugin's own
    // doc examples do (`tauri::async_runtime::spawn` there, `block_on`
    // here since this call site isn't already inside an async task).
    match tauri::async_runtime::block_on(plugin_updater.check()) {
        Ok(Some(update)) => updater::apply_update_check_result(app, Some(update.version)),
        Ok(None) => updater::apply_update_check_result(app, None),
        Err(e) => eprintln!("Automatische Update-Suche fehlgeschlagen: {e}"),
    }
}

/// Interval between recomputations of the cross-customer overdue-systems
/// count. Same coarse-grained reasoning as `AUTO_BACKUP_CHECK_INTERVAL` and
/// `AUTO_UPDATE_CHECK_INTERVAL`.
const MAINTENANCE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Starts a background thread that periodically recomputes the
/// cross-customer overdue-systems count (same query
/// `commands::systems::list_overdue_systems` uses) and reflects it in the
/// tray tooltip plus a live event for the frontend badge -- see the module
/// doc on `AppState::overdue_systems_count`. Unlike the backup/update-check
/// schedulers there's no "is it due" gate: this is a cheap local DB query,
/// not a network call, so it just always runs on the same interval.
fn spawn_maintenance_check_scheduler(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        run_maintenance_check(&app);
        std::thread::sleep(MAINTENANCE_CHECK_INTERVAL);
    });
}

fn run_maintenance_check(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let overdue = match commands::systems::list_overdue_systems(state.clone()) {
        Ok(list) => list.len(),
        Err(e) => {
            eprintln!("Wartungs-Überfälligkeitsprüfung fehlgeschlagen: {e}");
            return;
        }
    };
    *state
        .overdue_systems_count
        .lock()
        .expect("Overdue-Mutex vergiftet") = overdue;

    tray::sync_tray_tooltip(app);

    if let Err(e) = app.emit("maintenance-check-completed", overdue) {
        eprintln!("Ereignis \"maintenance-check-completed\" konnte nicht gesendet werden: {e}");
    }
}
