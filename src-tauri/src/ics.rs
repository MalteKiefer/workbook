//! Generates an RFC 5545 (iCalendar) .ics file covering two kinds of dates
//! this app already tracks: each system's next scheduled maintenance date
//! (see `maintenance::is_overdue` for the underlying "baseline + interval"
//! computation this reuses the same baseline logic from) and each expiring
//! item's expiry date (with a VALARM reminder offset matching its own
//! `reminder_days_before`, so the calendar reminds exactly when the in-app
//! Ablauf-Tracking would). This is a snapshot export, not a live sync --
//! re-export whenever the admin wants an updated calendar.

use chrono::{NaiveDate, Utc};

pub struct MaintenanceEvent {
    pub system_name: String,
    pub customer_name: String,
    pub due_on: NaiveDate,
}

pub struct ExpiryEvent {
    pub label: String,
    pub customer_name: String,
    pub kind_label: String,
    pub expires_on: NaiveDate,
    pub reminder_days_before: i64,
}

/// Escapes the four characters RFC 5545 §3.3.11 requires escaping inside a
/// TEXT value: backslash, comma, semicolon, and newline. Order matters --
/// backslash must be escaped FIRST, or the backslashes this function itself
/// inserts for the other three characters would be double-escaped.
fn escape_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace(';', "\\;")
        .replace('\n', "\\n")
}

/// RFC 5545 §3.1 line folding: a content line longer than 75 octets must be
/// split with a CRLF followed by a single leading space, which readers
/// strip when unfolding. Walks char boundaries (not raw byte offsets) so a
/// multi-byte UTF-8 character in a customer/system name is never split
/// across the fold -- a byte-index fold would corrupt it.
fn fold_line(line: &str) -> String {
    const MAX_OCTETS: usize = 75;
    if line.len() <= MAX_OCTETS {
        return line.to_string();
    }
    let mut result = String::new();
    let mut current_len = 0;
    for ch in line.chars() {
        let ch_len = ch.len_utf8();
        if current_len + ch_len > MAX_OCTETS {
            result.push_str("\r\n ");
            current_len = 0;
        }
        result.push(ch);
        current_len += ch_len;
    }
    result
}

fn uid_for(kind: &str, index: usize) -> String {
    format!("wartungsdoku-{kind}-{index}@wartungsdoku.local")
}

/// Builds the full `.ics` document as a single CRLF-joined string (RFC 5545
/// requires CRLF line endings, not bare `\n`), ending with a trailing CRLF.
pub fn build_calendar(maintenance: &[MaintenanceEvent], expiries: &[ExpiryEvent]) -> String {
    let now_stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Wartungsdoku//Kalender-Export//DE".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
    ];

    for (i, event) in maintenance.iter().enumerate() {
        let summary = escape_text(&format!(
            "Wartung fällig: {} ({})",
            event.system_name, event.customer_name
        ));
        lines.push("BEGIN:VEVENT".to_string());
        lines.push(format!("UID:{}", uid_for("maintenance", i)));
        lines.push(format!("DTSTAMP:{now_stamp}"));
        lines.push(format!(
            "DTSTART;VALUE=DATE:{}",
            event.due_on.format("%Y%m%d")
        ));
        lines.push(fold_line(&format!("SUMMARY:{summary}")));
        lines.push("END:VEVENT".to_string());
    }

    for (i, event) in expiries.iter().enumerate() {
        let summary = escape_text(&format!(
            "{} läuft ab: {} ({})",
            event.kind_label, event.label, event.customer_name
        ));
        lines.push("BEGIN:VEVENT".to_string());
        lines.push(format!("UID:{}", uid_for("expiry", i)));
        lines.push(format!("DTSTAMP:{now_stamp}"));
        lines.push(format!(
            "DTSTART;VALUE=DATE:{}",
            event.expires_on.format("%Y%m%d")
        ));
        lines.push(fold_line(&format!("SUMMARY:{summary}")));
        if event.reminder_days_before > 0 {
            lines.push("BEGIN:VALARM".to_string());
            lines.push("ACTION:DISPLAY".to_string());
            lines.push(fold_line(&format!("DESCRIPTION:{summary}")));
            lines.push(format!("TRIGGER:-P{}D", event.reminder_days_before));
            lines.push("END:VALARM".to_string());
        }
        lines.push("END:VEVENT".to_string());
    }

    lines.push("END:VCALENDAR".to_string());
    lines.join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_calendar_with_no_events_is_still_a_valid_empty_calendar() {
        let ics = build_calendar(&[], &[]);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert!(ics.contains("VERSION:2.0"));
    }

    #[test]
    fn maintenance_event_produces_an_all_day_vevent() {
        let ics = build_calendar(
            &[MaintenanceEvent {
                system_name: "Server1".to_string(),
                customer_name: "ACME GmbH".to_string(),
                due_on: NaiveDate::from_ymd_opt(2026, 12, 24).unwrap(),
            }],
            &[],
        );
        assert!(ics.contains("BEGIN:VEVENT"));
        assert!(ics.contains("DTSTART;VALUE=DATE:20261224"));
        assert!(ics.contains("Wartung fällig"));
        assert!(ics.contains("Server1"));
        assert!(ics.contains("ACME GmbH"));
    }

    #[test]
    fn expiry_event_with_reminder_includes_a_valarm() {
        let ics = build_calendar(
            &[],
            &[ExpiryEvent {
                label: "example.com".to_string(),
                customer_name: "ACME GmbH".to_string(),
                kind_label: "Domain".to_string(),
                expires_on: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
                reminder_days_before: 30,
            }],
        );
        assert!(ics.contains("BEGIN:VALARM"));
        assert!(ics.contains("TRIGGER:-P30D"));
        assert!(ics.contains("ACTION:DISPLAY"));
    }

    #[test]
    fn expiry_event_without_reminder_has_no_valarm() {
        let ics = build_calendar(
            &[],
            &[ExpiryEvent {
                label: "example.com".to_string(),
                customer_name: "ACME GmbH".to_string(),
                kind_label: "Domain".to_string(),
                expires_on: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
                reminder_days_before: 0,
            }],
        );
        assert!(!ics.contains("BEGIN:VALARM"));
    }

    #[test]
    fn escape_text_escapes_commas_semicolons_backslashes_and_newlines() {
        assert_eq!(escape_text("a,b;c\\d\ne"), "a\\,b\\;c\\\\d\\ne");
    }

    #[test]
    fn fold_line_splits_long_lines_with_crlf_space_continuation() {
        let long_summary = format!("SUMMARY:{}", "x".repeat(100));
        let folded = fold_line(&long_summary);
        assert!(folded.contains("\r\n "));
        // Unfolding (strip every "\r\n " occurrence) must reconstruct the
        // original line exactly -- this is the property RFC 5545 readers
        // rely on.
        let unfolded = folded.replace("\r\n ", "");
        assert_eq!(unfolded, long_summary);
    }

    #[test]
    fn fold_line_leaves_short_lines_unchanged() {
        let short = "SUMMARY:kurz";
        assert_eq!(fold_line(short), short);
    }
}
