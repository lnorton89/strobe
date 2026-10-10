// ============================================================================
// Module:       strobe_app::localtime
// Description:  Local wall-clock time for `strobe_Date` (Shadertoy's iDate):
//               a cached UTC offset plus the system clock, no allocation.
//
// Dependencies: chrono (the platform's time-zone lookup); strobe-scene (date)
// ============================================================================

//! Local time for `strobe_Date`.
//!
//! Shadertoy's `iDate` is the **local** time of the person watching, and a
//! shader that draws a clock shows it. The standard library has no time
//! zone support, so the offset comes from `chrono`'s `Local`, which asks
//! the operating system (the time-zone calls on Windows; `TZ` and
//! `/etc/localtime` on Unix). `chrono` is used for that one question only;
//! the calendar arithmetic is `strobe_scene::date`, which the renderer
//! already tests.
//!
//! ## Cost
//!
//! The offset is looked up **off the frame path**: [`LocalDate::refresh`]
//! runs with the app's other housekeeping (every couple of seconds, with
//! the monitor list), because that lookup can touch the time-zone database
//! on disk. [`LocalDate::now`], the per-frame call, is one read of the
//! system clock, one integer add and the calendar arithmetic: no
//! allocation, no syscall beyond the clock, no lock. A daylight-saving
//! change is picked up at the next refresh, so the displayed clock is wrong
//! by an hour for at most that long once or twice a year.

use std::time::{SystemTime, UNIX_EPOCH};

/// The local time zone's offset from UTC, cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDate {
    /// Seconds to add to UTC to get local time (east of Greenwich positive).
    offset_seconds: i32,
}

impl LocalDate {
    /// A source that reads the platform's current offset.
    pub fn from_system() -> Self {
        Self {
            offset_seconds: system_offset_seconds(),
        }
    }

    /// A source with a fixed offset, for tests and for hosts that know it.
    pub fn with_offset(offset_seconds: i32) -> Self {
        Self { offset_seconds }
    }

    /// Reads the platform's offset again (a daylight-saving change, or the
    /// person moved the clock's zone). Not for the frame path.
    pub fn refresh(&mut self) {
        self.offset_seconds = system_offset_seconds();
    }

    /// The cached offset, in seconds east of UTC.
    pub fn offset_seconds(&self) -> i32 {
        self.offset_seconds
    }

    /// `strobe_Date` for the instant `unix_seconds` after the epoch:
    /// `[year, month 0..=11, day 1..=31, seconds since local midnight]`.
    pub fn at(&self, unix_seconds: f64) -> [f32; 4] {
        strobe_scene::date::from_unix_seconds(unix_seconds + f64::from(self.offset_seconds))
    }

    /// `strobe_Date` now. Allocation-free; see the module docs.
    pub fn now(&self) -> [f32; 4] {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64());
        self.at(seconds)
    }
}

/// The platform's current UTC offset in seconds.
fn system_offset_seconds() -> i32 {
    use chrono::Offset as _;
    chrono::Local::now().offset().fix().local_minus_utc()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// 2026-10-09 23:30:00 UTC.
    const LATE_EVENING_UTC: f64 = 1_791_504_000.0 + 23.5 * 3600.0;

    #[test]
    fn utc_is_the_identity() {
        assert_eq!(
            LocalDate::with_offset(0).at(LATE_EVENING_UTC),
            [2026.0, 9.0, 9.0, 84_600.0],
            "utc"
        );
    }

    #[test]
    fn an_offset_east_of_utc_moves_the_clock_and_can_roll_the_day() {
        // UTC+2: 01:30 the next day, October 10th.
        assert_eq!(
            LocalDate::with_offset(2 * 3600).at(LATE_EVENING_UTC),
            [2026.0, 9.0, 10.0, 5_400.0],
            "east of utc"
        );
    }

    #[test]
    fn an_offset_west_of_utc_can_roll_back_across_a_month() {
        // 2026-11-01 02:00 UTC at UTC-5 is October 31st, 21:00.
        let unix = 1_791_504_000.0 + 23.0 * 86_400.0 + 2.0 * 3600.0;
        assert_eq!(
            LocalDate::with_offset(-5 * 3600).at(unix),
            [2026.0, 9.0, 31.0, 75_600.0],
            "west of utc"
        );
    }

    #[test]
    fn the_system_offset_is_a_real_zone_offset_and_now_is_a_valid_date() -> Result<()> {
        let local = LocalDate::from_system();
        // No zone is further than UTC-12 or UTC+14.
        assert!(
            (-12 * 3600..=14 * 3600).contains(&local.offset_seconds()),
            "offset {} s is not a time zone",
            local.offset_seconds()
        );
        let now = local.now();
        assert!(now[0] >= 2026.0, "the year is {}", now[0]);
        assert!((0.0..12.0).contains(&now[1]), "month {}", now[1]);
        assert!((1.0..=31.0).contains(&now[2]), "day {}", now[2]);
        assert!((0.0..86_400.0).contains(&now[3]), "seconds {}", now[3]);
        Ok(())
    }
}
