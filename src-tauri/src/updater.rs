//! Automatic update-check scheduling logic (Settings -> Aktualisierung).
//! Mirrors `backup::is_auto_backup_due`'s pure due-check plus a shared
//! "apply a check's result" step used by both the background scheduler
//! (`spawn_auto_update_check_scheduler` in `lib.rs`) and a manual check
//! from the frontend (`commands::updater::record_update_check_result`),
//! so both paths update `config.toml`, the tray hint, and the frontend
//! badge identically. Never installs anything -- see the module doc on
//! `Config::auto_update_check_enabled`.

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Emitter, Manager};

use crate::config::AutoUpdateCheckFrequency;
use crate::{tray, AppState};

pub fn is_auto_update_check_due(
    frequency: AutoUpdateCheckFrequency,
    last_run_utc: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    let Some(last_run_utc) = last_run_utc else {
        return true;
    };
    let Ok(last_run) = DateTime::parse_from_rfc3339(last_run_utc) else {
        return true;
    };
    let interval = match frequency {
        AutoUpdateCheckFrequency::Daily => chrono::Duration::days(1),
        AutoUpdateCheckFrequency::Weekly => chrono::Duration::days(7),
        AutoUpdateCheckFrequency::Monthly => chrono::Duration::days(30),
    };
    now.signed_duration_since(last_run) >= interval
}

/// Records the result of an update check (automatic or manual): updates
/// `auto_update_check_last_run_utc` (so a manual check also postpones the
/// next automatic one) and `auto_update_check_available_version`, saves
/// `config.toml`, syncs the tray tooltip, and emits `update-check-completed`
/// so any open window updates its badge live.
pub fn apply_update_check_result(app: &AppHandle, available_version: Option<String>) {
    let state = app.state::<AppState>();
    {
        let mut config = state.config.lock().expect("Config-Mutex vergiftet");
        config.auto_update_check_last_run_utc = Some(Utc::now().to_rfc3339());
        config.auto_update_check_available_version = available_version.clone();
        let config_path = config.data_dir.join("config.toml");
        if let Err(e) = config.save(&config_path) {
            eprintln!("Update-Suche: Ergebnis konnte nicht gespeichert werden: {e}");
        }
    }

    tray::sync_tray_tooltip(app);

    if let Err(e) = app.emit("update-check-completed", &available_version) {
        eprintln!("Update-Suche: Ereignis konnte nicht gesendet werden: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_auto_update_check_due_when_never_run_before() {
        assert!(is_auto_update_check_due(
            AutoUpdateCheckFrequency::Daily,
            None,
            Utc::now()
        ));
    }

    #[test]
    fn is_auto_update_check_due_when_last_run_timestamp_is_unparseable() {
        assert!(is_auto_update_check_due(
            AutoUpdateCheckFrequency::Daily,
            Some("not a timestamp"),
            Utc::now()
        ));
    }

    #[test]
    fn is_auto_update_check_not_due_before_daily_interval_elapsed() {
        let now = Utc::now();
        let last_run = (now - chrono::Duration::hours(2)).to_rfc3339();
        assert!(!is_auto_update_check_due(
            AutoUpdateCheckFrequency::Daily,
            Some(&last_run),
            now
        ));
    }

    #[test]
    fn is_auto_update_check_due_after_daily_interval_elapsed() {
        let now = Utc::now();
        let last_run =
            (now - chrono::Duration::days(1) - chrono::Duration::minutes(1)).to_rfc3339();
        assert!(is_auto_update_check_due(
            AutoUpdateCheckFrequency::Daily,
            Some(&last_run),
            now
        ));
    }

    #[test]
    fn is_auto_update_check_not_due_before_weekly_interval_elapsed() {
        let now = Utc::now();
        let last_run = (now - chrono::Duration::days(6)).to_rfc3339();
        assert!(!is_auto_update_check_due(
            AutoUpdateCheckFrequency::Weekly,
            Some(&last_run),
            now
        ));
    }

    #[test]
    fn is_auto_update_check_due_after_weekly_interval_elapsed() {
        let now = Utc::now();
        let last_run =
            (now - chrono::Duration::days(7) - chrono::Duration::minutes(1)).to_rfc3339();
        assert!(is_auto_update_check_due(
            AutoUpdateCheckFrequency::Weekly,
            Some(&last_run),
            now
        ));
    }

    #[test]
    fn is_auto_update_check_not_due_before_monthly_interval_elapsed() {
        let now = Utc::now();
        let last_run = (now - chrono::Duration::days(29)).to_rfc3339();
        assert!(!is_auto_update_check_due(
            AutoUpdateCheckFrequency::Monthly,
            Some(&last_run),
            now
        ));
    }

    #[test]
    fn is_auto_update_check_due_after_monthly_interval_elapsed() {
        let now = Utc::now();
        let last_run =
            (now - chrono::Duration::days(30) - chrono::Duration::minutes(1)).to_rfc3339();
        assert!(is_auto_update_check_due(
            AutoUpdateCheckFrequency::Monthly,
            Some(&last_run),
            now
        ));
    }
}
