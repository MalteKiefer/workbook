use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;

use crate::error::AppError;

pub fn system_timezone() -> Result<Tz, AppError> {
    let name = iana_time_zone::get_timezone()
        .map_err(|e| AppError::Timezone(e.to_string()))?;
    name.parse::<Tz>()
        .map_err(|_| AppError::Timezone(format!("unbekannte Zone: {name}")))
}

pub fn now_with_tz(tz: &Tz) -> (String, String) {
    format_utc_with_tz(Utc::now(), tz)
}

fn format_utc_with_tz(dt: DateTime<Utc>, tz: &Tz) -> (String, String) {
    (
        dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        tz.name().to_string(),
    )
}

pub fn parse_temporal_input(
    input: &str,
    tz: &Tz,
    now_utc: DateTime<Utc>,
) -> Result<(String, String), AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(format_utc_with_tz(now_utc, tz));
    }

    if let Some(caps) = relative_pattern().captures(trimmed) {
        let amount: i64 = caps[1].parse().unwrap();
        let unit = &caps[2];
        let delta = match unit {
            "h" => Duration::hours(amount),
            "m" => Duration::minutes(amount),
            "d" => Duration::days(amount),
            _ => unreachable!(),
        };
        return Ok(format_utc_with_tz(now_utc - delta, tz));
    }

    if let Some(caps) = yesterday_pattern().captures(trimmed) {
        let hour: u32 = caps[1].parse().unwrap();
        let minute: u32 = caps[2].parse().unwrap();
        let local_now = now_utc.with_timezone(tz);
        let yesterday_date = local_now.date_naive() - Duration::days(1);
        return localize(yesterday_date, hour, minute, tz, trimmed);
    }

    if let Some(caps) = absolute_pattern().captures(trimmed) {
        let day: u32 = caps[1].parse().unwrap();
        let month: u32 = caps[2].parse().unwrap();
        let year: i32 = caps[3].parse().unwrap();
        let hour: u32 = caps[4].parse().unwrap();
        let minute: u32 = caps[5].parse().unwrap();
        let date = NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| AppError::InvalidTimestamp(trimmed.to_string()))?;
        return localize(date, hour, minute, tz, trimmed);
    }

    Err(AppError::InvalidTimestamp(trimmed.to_string()))
}

fn localize(
    date: NaiveDate,
    hour: u32,
    minute: u32,
    tz: &Tz,
    original_input: &str,
) -> Result<(String, String), AppError> {
    let naive = date
        .and_hms_opt(hour, minute, 0)
        .ok_or_else(|| AppError::InvalidTimestamp(original_input.to_string()))?;
    let local: NaiveDateTime = naive;
    match tz.from_local_datetime(&local).single() {
        Some(dt) => Ok(format_utc_with_tz(dt.with_timezone(&Utc), tz)),
        None => Err(AppError::InvalidTimestamp(format!(
            "{original_input} (mehrdeutig oder ungültig durch Zeitumstellung)"
        ))),
    }
}

fn relative_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^-(\d+)([hmd])$").unwrap())
}

fn yesterday_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^gestern\s+(\d{1,2}):(\d{2})$").unwrap())
}

fn absolute_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^(\d{2})\.(\d{2})\.(\d{4})\s+(\d{1,2}):(\d{2})$").unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn berlin() -> Tz {
        "Europe/Berlin".parse().unwrap()
    }

    fn fixed_now() -> DateTime<Utc> {
        // 2026-09-07T12:00:00.000Z == 2026-09-07 14:00 Europe/Berlin (CEST, UTC+2)
        Utc.with_ymd_and_hms(2026, 9, 7, 12, 0, 0).unwrap()
    }

    #[test]
    fn empty_input_returns_now() {
        let (utc, tz) = parse_temporal_input("", &berlin(), fixed_now()).unwrap();
        assert_eq!(utc, "2026-09-07T12:00:00.000Z");
        assert_eq!(tz, "Europe/Berlin");
    }

    #[test]
    fn relative_hours_subtracts_from_now() {
        let (utc, _) = parse_temporal_input("-2h", &berlin(), fixed_now()).unwrap();
        assert_eq!(utc, "2026-09-07T10:00:00.000Z");
    }

    #[test]
    fn relative_days_subtracts_from_now() {
        let (utc, _) = parse_temporal_input("-1d", &berlin(), fixed_now()).unwrap();
        assert_eq!(utc, "2026-09-06T12:00:00.000Z");
    }

    #[test]
    fn yesterday_with_time_resolves_to_local_day_before() {
        let (utc, tz) = parse_temporal_input("gestern 9:15", &berlin(), fixed_now()).unwrap();
        // 2026-09-06 09:15 Europe/Berlin (CEST, UTC+2) == 2026-09-06T07:15:00Z
        assert_eq!(utc, "2026-09-06T07:15:00.000Z");
        assert_eq!(tz, "Europe/Berlin");
    }

    #[test]
    fn absolute_date_and_time_parses_in_given_zone() {
        let (utc, tz) = parse_temporal_input("07.09.2026 14:32", &berlin(), fixed_now()).unwrap();
        assert_eq!(utc, "2026-09-07T12:32:00.000Z");
        assert_eq!(tz, "Europe/Berlin");
    }

    #[test]
    fn garbage_input_is_rejected() {
        let result = parse_temporal_input("nicht ein datum", &berlin(), fixed_now());
        assert!(matches!(result, Err(AppError::InvalidTimestamp(_))));
    }

    #[test]
    fn invalid_calendar_date_is_rejected() {
        let result = parse_temporal_input("31.02.2026 10:00", &berlin(), fixed_now());
        assert!(matches!(result, Err(AppError::InvalidTimestamp(_))));
    }
}
