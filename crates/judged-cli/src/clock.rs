//! The one timestamp the CLI needs, in the one shape the ratchet compares.
//!
//! `judged_ratchet` takes `now` as a caller-supplied RFC 3339 string rather than
//! reading the clock itself, so that a CI run and a local run over the same
//! inputs agree. Somebody still has to read the clock, and that somebody is the
//! process boundary — here.
//!
//! No date crate. `judged_ratchet::rot` compares `expires` against `now`
//! **lexicographically** and never does arithmetic on dates, so the only thing
//! required of this module is that it emit the exact fixed-width
//! `YYYY-MM-DDTHH:MM:SSZ` shape that ordering depends on. Pulling in `chrono` or
//! `time` to produce forty characters would add a dependency whose only job is a
//! format string.
//!
//! The civil-date conversion is Howard Hinnant's `civil_from_days`, which is
//! exact for the whole proleptic Gregorian range and has no branch for leap
//! years — it shifts the year to start in March so that the leap day lands at
//! the end of the cycle instead of in the middle of it.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds in the units the conversion works in.
const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR;

/// Days from 0000-03-01 to 1970-01-01, the shift that puts the leap day last.
const DAYS_FROM_MARCH_ZERO_TO_EPOCH: i64 = 719_468;

/// Days in a 400-year Gregorian era.
const DAYS_PER_ERA: i64 = 146_097;

/// The current instant as an RFC 3339 UTC timestamp.
///
/// A clock reading before the Unix epoch is not an error worth a `Result` here:
/// the value is only ever compared against baseline expiry dates, and a machine
/// whose clock says 1969 would fail that comparison in the safe direction (every
/// amnesty looks unexpired) whatever we did. It is rendered honestly rather than
/// clamped, so the operator sees the absurd date in the report.
pub fn now_rfc3339() -> String {
    let unix_seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() as i64,
        Err(before_epoch) => -(before_epoch.duration().as_secs() as i64),
    };
    rfc3339_from_unix_seconds(unix_seconds)
}

/// Render a Unix timestamp as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Split out from [`now_rfc3339`] because a function that reads the clock cannot
/// be tested, and the thing worth testing is the arithmetic.
fn rfc3339_from_unix_seconds(unix_seconds: i64) -> String {
    // `div_euclid` rather than `/`: negative seconds must floor toward the
    // earlier day, not truncate toward zero, or 1969-12-31T23:59:59Z renders as
    // 1970-01-01T-00:-00:-01.
    let days = unix_seconds.div_euclid(SECONDS_PER_DAY);
    let seconds_into_day = unix_seconds.rem_euclid(SECONDS_PER_DAY);

    let (year, month, day) = civil_from_days(days);
    let hour = seconds_into_day / SECONDS_PER_HOUR;
    let minute = (seconds_into_day % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    let second = seconds_into_day % SECONDS_PER_MINUTE;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Gregorian calendar date for a count of days since 1970-01-01.
///
/// Hinnant's algorithm, unmodified. The internal year runs March..February, so
/// the leap day falls at the end of a cycle rather than in the middle of one and
/// no branch for leap years is needed; January and February are handed back to
/// the previous calendar year at the end.
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let shifted = days_since_epoch + DAYS_FROM_MARCH_ZERO_TO_EPOCH;
    // Floor division, so that dates before 0000-03-01 land in the right era.
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA); // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let month_prime = (5 * day_of_year + 2) / 153; // [0, 11], March = 0
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1; // [1, 31]
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants_render_in_the_shape_the_ratchet_compares() {
        // Pinned against `date -u -r <seconds>`. The exact spelling matters more
        // than the arithmetic does: `judged_ratchet::rot::has_expired` compares
        // this string against a `YYYY-MM-DD` expiry with `<=`, so a missing
        // zero-pad silently grants a longer amnesty than anyone asked for.
        assert_eq!(rfc3339_from_unix_seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339_from_unix_seconds(1_000_000_000),
            "2001-09-09T01:46:40Z"
        );
        assert_eq!(
            rfc3339_from_unix_seconds(1_767_225_600),
            "2026-01-01T00:00:00Z"
        );
        assert_eq!(
            rfc3339_from_unix_seconds(1_769_904_000),
            "2026-02-01T00:00:00Z"
        );
        assert_eq!(
            rfc3339_from_unix_seconds(1_800_000_000),
            "2027-01-15T08:00:00Z"
        );
    }

    #[test]
    fn the_century_leap_day_is_a_real_day() {
        // 2000 is a leap year, 1900 and 2100 are not. This is the one case a
        // hand-rolled conversion gets wrong, and getting it wrong shifts every
        // subsequent date by a day.
        assert_eq!(
            rfc3339_from_unix_seconds(951_782_400),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            rfc3339_from_unix_seconds(951_782_400 + SECONDS_PER_DAY),
            "2000-03-01T00:00:00Z"
        );
    }

    #[test]
    fn instants_before_the_epoch_floor_to_the_earlier_day() {
        // Not a hypothetical: a container with an unset clock reports 0, and a
        // machine mid-NTP-correction reports a negative offset. Truncating
        // toward zero here would emit a timestamp that is not a timestamp.
        assert_eq!(rfc3339_from_unix_seconds(-1), "1969-12-31T23:59:59Z");
        assert_eq!(
            rfc3339_from_unix_seconds(-SECONDS_PER_DAY),
            "1969-12-31T00:00:00Z"
        );
    }

    #[test]
    fn rendered_timestamps_sort_the_way_the_ratchet_assumes() {
        // The whole no-date-library decision rests on this: byte ordering must
        // equal chronological ordering, including across a year boundary.
        let earlier = rfc3339_from_unix_seconds(1_767_225_599);
        let later = rfc3339_from_unix_seconds(1_767_225_600);
        assert!(earlier < later, "{earlier} should sort before {later}");
        assert!(
            "2025-12-31" < later.as_str(),
            "a date-only expiry must compare against a full instant"
        );
    }

    #[test]
    fn the_clock_produces_a_timestamp_of_the_documented_width() {
        let now = now_rfc3339();

        assert_eq!(now.len(), 20, "got {now}");
        assert!(now.ends_with('Z'), "got {now}");
        assert!(
            now.as_str() > "2020-01-01T00:00:00Z",
            "the machine clock reads {now}, which is before this project existed"
        );
    }
}
