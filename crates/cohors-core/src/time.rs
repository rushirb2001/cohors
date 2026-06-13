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

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

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
