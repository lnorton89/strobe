// ============================================================================
// Module:       strobe_scene::date
// Description:  A calendar date and time of day in Shadertoy's `iDate` shape,
//               computed from seconds since the Unix epoch.
//
// Dependencies: std only
// ============================================================================

//! `strobe_Date`, the uniform behind Shadertoy's `iDate`.
//!
//! `iDate` is a `vec4`: year, month (0 to 11), day of the month (1 to 31),
//! and seconds since midnight. This crate reads no clock, so the caller
//! supplies the instant; [`from_unix_seconds`] does the calendar
//! arithmetic with no dependency, using Howard Hinnant's civil-from-days
//! algorithm.
//!
//! The standard library has no time zone database, so this is UTC unless
//! the caller adds its own offset to the seconds first. A host that knows
//! the local offset should; the renderer's default is UTC (OPEN_QUESTIONS
//! Q15).

/// Seconds in a day.
const DAY: f64 = 86_400.0;

/// `[year, month 0..=11, day 1..=31, seconds since midnight]` for an
/// instant `seconds` after 1970-01-01 00:00:00 in the caller's time zone.
pub fn from_unix_seconds(seconds: f64) -> [f32; 4] {
    let days = (seconds / DAY).floor();
    let in_day = seconds - days * DAY;
    let (year, month, day) = civil_from_days(days as i64);
    [year as f32, (month - 1) as f32, day as f32, in_day as f32]
}

/// Year, month (1 to 12) and day (1 to 31) of the day `days` after
/// 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_is_january_first_1970() {
        assert_eq!(from_unix_seconds(0.0), [1970.0, 0.0, 1.0, 0.0], "epoch");
    }

    #[test]
    fn known_instants_convert() {
        // 2000-02-29 12:30:15 UTC (a leap day) is 951_827_415.
        assert_eq!(
            from_unix_seconds(951_827_415.0),
            [2000.0, 1.0, 29.0, 45_015.0],
            "leap day"
        );
        // 2026-10-09 00:00:00 UTC is 1_791_504_000.
        assert_eq!(
            from_unix_seconds(1_791_504_000.0),
            [2026.0, 9.0, 9.0, 0.0],
            "a day in the present"
        );
    }

    #[test]
    fn instants_before_the_epoch_stay_in_range() {
        let [year, month, day, seconds] = from_unix_seconds(-1.0);
        assert_eq!(
            [year, month, day, seconds],
            [1969.0, 11.0, 31.0, 86_399.0],
            "one second before the epoch"
        );
    }
}
