//! Relative-time formatting.
//!
//! Pure and clock-free: the caller passes the current time, so this stays
//! WASM-safe (no `SystemTime::now` / `Instant`). The TUI passes the real wall
//! clock; tests pass a fixed `now`.

const MIN: i64 = 60;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
const WEEK: i64 = 7 * DAY;
const MONTH: i64 = 30 * DAY; // approximate — fine for an at-a-glance age
const YEAR: i64 = 365 * DAY;

/// Format `timestamp` (seconds since the Unix epoch) relative to `now`, as a
/// short label: `now`, `5m`, `2h`, `3d`, `1w`, `4mo`, `2y`.
///
/// A timestamp at or after `now` (e.g. clock skew, or a commit dated in the
/// future) collapses to `now`.
pub fn relative(timestamp: i64, now: i64) -> String {
    let secs = now - timestamp;
    if secs < MIN {
        "now".to_string()
    } else if secs < HOUR {
        format!("{}m", secs / MIN)
    } else if secs < DAY {
        format!("{}h", secs / HOUR)
    } else if secs < WEEK {
        format!("{}d", secs / DAY)
    } else if secs < MONTH {
        format!("{}w", secs / WEEK)
    } else if secs < YEAR {
        format!("{}mo", secs / MONTH)
    } else {
        format!("{}y", secs / YEAR)
    }
}

/// Format seconds since the Unix epoch as an ISO-8601 UTC timestamp
/// (`2026-01-31T09:05:00Z`). Used to stamp `updated` on generated files.
///
/// Done by hand to keep the core dependency-free: the date part is the
/// well-known "civil from days" algorithm (proleptic Gregorian calendar).
pub fn iso8601_utc(secs: i64) -> String {
    let days = secs.div_euclid(DAY);
    let rem = secs.rem_euclid(DAY);
    // Shift the epoch to 0000-03-01 so leap days fall at the end of a "year".
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153; // 0 = March
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / HOUR,
        (rem % HOUR) / MIN,
        rem % MIN
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn iso8601_matches_known_instants() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_utc(NOW), "2023-11-14T22:13:20Z");
        // Leap day, and the last second of a year.
        assert_eq!(iso8601_utc(1_709_210_096), "2024-02-29T12:34:56Z");
        assert_eq!(iso8601_utc(1_735_689_599), "2024-12-31T23:59:59Z");
        assert_eq!(iso8601_utc(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn buckets_render_expected_labels() {
        assert_eq!(relative(NOW, NOW), "now");
        assert_eq!(relative(NOW - 30, NOW), "now");
        assert_eq!(relative(NOW - 90, NOW), "1m");
        assert_eq!(relative(NOW - 2 * HOUR, NOW), "2h");
        assert_eq!(relative(NOW - 3 * DAY, NOW), "3d");
        assert_eq!(relative(NOW - 2 * WEEK, NOW), "2w");
        assert_eq!(relative(NOW - 4 * MONTH, NOW), "4mo");
        assert_eq!(relative(NOW - 2 * YEAR, NOW), "2y");
    }

    #[test]
    fn future_timestamps_collapse_to_now() {
        assert_eq!(relative(NOW + 5000, NOW), "now");
    }

    #[test]
    fn boundaries_round_down() {
        assert_eq!(relative(NOW - (HOUR - 1), NOW), "59m");
        assert_eq!(relative(NOW - HOUR, NOW), "1h");
        assert_eq!(relative(NOW - (DAY - 1), NOW), "23h");
        assert_eq!(relative(NOW - DAY, NOW), "1d");
    }
}
