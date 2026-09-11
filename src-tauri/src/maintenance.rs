//! Pure overdue-check logic for a system's optional maintenance interval
//! (Kunden/Systeme -> "Wartungsintervall"). Mirrors
//! `backup::is_auto_backup_due`/`updater::is_auto_update_check_due`'s
//! shape: a single pure function taking `now` as a parameter (so it's
//! trivially testable without mocking the clock), fail-safe on an
//! unparseable timestamp.

use chrono::{DateTime, Utc};

/// `None` interval -- system has no maintenance schedule -- is never
/// overdue. `Some(n)`: overdue once `n` days have passed since the later
/// of the system's last entry (`last_performed_at_utc`) or, if it has
/// never had one, the system's own `created_at_utc`. An unparseable
/// baseline timestamp is treated as overdue (fail-safe, same convention
/// as `backup::is_auto_backup_due`/`updater::is_auto_update_check_due`).
pub fn is_overdue(
    interval_days: Option<i64>,
    last_performed_at_utc: Option<&str>,
    system_created_at_utc: &str,
    now: DateTime<Utc>,
) -> bool {
    let Some(interval_days) = interval_days else {
        return false;
    };
    let baseline_str = last_performed_at_utc.unwrap_or(system_created_at_utc);
    let Ok(baseline) = DateTime::parse_from_rfc3339(baseline_str) else {
        return true;
    };
    now.signed_duration_since(baseline) >= chrono::Duration::days(interval_days)
}

/// Whether an expiring item (SSL cert, domain, license, ...) is inside
/// its reminder window -- `today` is on or after
/// `expires_on - reminder_days_before` days. Unlike `is_overdue`, this
/// does not treat "already expired" specially -- an item overdue by 400
/// days is still simply "due" (the UI distinguishes "expired" from
/// "expiring soon" by comparing `expires_on` to today directly, this
/// function only answers "should it show up in the warning list at
/// all").
pub fn is_expiring_soon(
    expires_on: &str,
    reminder_days_before: i64,
    today: chrono::NaiveDate,
) -> bool {
    let Ok(expires) = chrono::NaiveDate::parse_from_str(expires_on, "%Y-%m-%d") else {
        return true; // unparseable date -- fail-safe, same convention as is_overdue
    };
    today >= expires - chrono::Duration::days(reminder_days_before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_overdue_when_no_interval_is_set() {
        let now = Utc::now();
        let long_ago = (now - chrono::Duration::days(10_000)).to_rfc3339();
        assert!(!is_overdue(None, None, &long_ago, now));
    }

    #[test]
    fn overdue_when_no_entries_ever_and_system_created_long_ago() {
        let now = Utc::now();
        let created_at = (now - chrono::Duration::days(200)).to_rfc3339();
        assert!(is_overdue(Some(90), None, &created_at, now));
    }

    #[test]
    fn not_overdue_when_no_entries_ever_and_system_created_recently() {
        let now = Utc::now();
        let created_at = (now - chrono::Duration::days(5)).to_rfc3339();
        assert!(!is_overdue(Some(90), None, &created_at, now));
    }

    #[test]
    fn not_overdue_when_last_entry_is_recent() {
        let now = Utc::now();
        let created_at = (now - chrono::Duration::days(500)).to_rfc3339();
        let last_performed_at = (now - chrono::Duration::days(5)).to_rfc3339();
        assert!(!is_overdue(
            Some(90),
            Some(&last_performed_at),
            &created_at,
            now
        ));
    }

    #[test]
    fn overdue_when_last_entry_is_old() {
        let now = Utc::now();
        let created_at = (now - chrono::Duration::days(500)).to_rfc3339();
        let last_performed_at = (now - chrono::Duration::days(200)).to_rfc3339();
        assert!(is_overdue(
            Some(90),
            Some(&last_performed_at),
            &created_at,
            now
        ));
    }

    #[test]
    fn overdue_exactly_at_the_interval_boundary() {
        let now = Utc::now();
        let created_at = (now - chrono::Duration::days(500)).to_rfc3339();
        let last_performed_at = (now - chrono::Duration::days(90)).to_rfc3339();
        assert!(is_overdue(
            Some(90),
            Some(&last_performed_at),
            &created_at,
            now
        ));
    }

    #[test]
    fn overdue_when_baseline_timestamp_is_unparseable() {
        let now = Utc::now();
        assert!(is_overdue(
            Some(90),
            Some("not a timestamp"),
            "also not one",
            now
        ));
    }
}

#[cfg(test)]
mod expiring_soon_tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn not_expiring_when_far_in_the_future() {
        assert!(!is_expiring_soon("2027-12-31", 30, date("2026-09-11")));
    }

    #[test]
    fn expiring_when_inside_reminder_window() {
        assert!(is_expiring_soon("2026-09-30", 30, date("2026-09-11")));
    }

    #[test]
    fn expiring_when_already_past() {
        assert!(is_expiring_soon("2026-01-01", 30, date("2026-09-11")));
    }

    #[test]
    fn expiring_exactly_at_the_reminder_boundary() {
        assert!(is_expiring_soon("2026-10-11", 30, date("2026-09-11")));
    }

    #[test]
    fn unparseable_date_is_treated_as_expiring() {
        assert!(is_expiring_soon("not-a-date", 30, date("2026-09-11")));
    }
}
