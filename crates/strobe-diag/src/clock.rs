// ============================================================================
// Module:       strobe_diag::clock
// Description:  Process start time, uptime, and UTC timestamps for file names
//               and report headers, with no date/time dependency.
//
// Dependencies: std::time
// ============================================================================

//! Time for diagnostics.
//!
//! Log lines carry time since the process started (monotonic, immune to
//! clock changes); the session header and file names carry wall-clock time.
//! Wall-clock time is UTC: the standard library cannot read the local UTC
//! offset, and a date/time crate for one file name is not worth a
//! dependency. UTC also means a report from another time zone sorts and
//! compares correctly.

use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<Instant> = OnceLock::new();

/// Records the process start. Idempotent; the first call wins.
pub fn start() {
    START.get_or_init(Instant::now);
}

/// Time since [`start`], or zero if it was never called.
pub fn uptime() -> Duration {
    START.get().map_or(Duration::ZERO, Instant::elapsed)
}

/// A UTC calendar time, second resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utc {
    /// Calendar year.
    pub year: i64,
    /// Month, 1 to 12.
    pub month: u32,
    /// Day of month, 1 to 31.
    pub day: u32,
    /// Hour, 0 to 23.
    pub hour: u32,
    /// Minute, 0 to 59.
    pub minute: u32,
    /// Second, 0 to 59.
    pub second: u32,
}

impl Utc {
    /// The current time. A clock set before 1970 reads as the epoch.
    pub fn now() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0));
        Self::from_unix(secs)
    }

    /// Converts seconds since 1970-01-01T00:00:00Z.
    pub fn from_unix(secs: i64) -> Self {
        let days = secs.div_euclid(86_400);
        let rest = secs.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: (rest / 3600) as u32,
            minute: (rest % 3600 / 60) as u32,
            second: (rest % 60) as u32,
        }
    }

    /// `YYYYMMDD-HHMMSS`, as used in file names.
    pub fn file_stamp(&self) -> String {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `YYYY-MM-DDTHH:MM:SSZ`.
    pub fn iso(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_epoch_is_1970() -> Result<()> {
        assert_eq!(Utc::from_unix(0).iso(), "1970-01-01T00:00:00Z", "epoch");
        Ok(())
    }

    #[test]
    fn a_known_timestamp_converts_exactly() -> Result<()> {
        let stamp = Utc::from_unix(1_791_462_896);
        assert_eq!(stamp.iso(), "2026-10-08T12:34:56Z", "iso form");
        assert_eq!(stamp.file_stamp(), "20261008-123456", "file form");
        Ok(())
    }

    #[test]
    fn leap_days_are_handled() -> Result<()> {
        assert_eq!(
            Utc::from_unix(1_709_251_199).iso(),
            "2024-02-29T23:59:59Z",
            "leap day"
        );
        assert_eq!(
            Utc::from_unix(1_709_251_200).iso(),
            "2024-03-01T00:00:00Z",
            "day after the leap day"
        );
        Ok(())
    }

    #[test]
    fn uptime_is_monotonic_after_start() -> Result<()> {
        start();
        let first = uptime();
        let second = uptime();
        assert!(second >= first, "uptime went backwards");
        Ok(())
    }
}
