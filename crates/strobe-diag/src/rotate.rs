// ============================================================================
// Module:       strobe_diag::rotate
// Description:  Which old files to delete when only the newest few are kept:
//               ordering by modification time, never by name.
//
// Dependencies: std::fs, std::time
// ============================================================================

//! Age ordering for rotated files.
//!
//! Session logs and crash reports both keep "the newest N". Their names
//! carry a UTC timestamp, which is tempting to sort by, but a clock that
//! jumps backwards (a dead CMOS battery, no NTP) then makes the newest files
//! sort as the oldest, and the pruning deletes exactly the evidence of the
//! latest run (D396). The file system's modification time is written at the
//! moment of the write, so it orders files by when they were last touched
//! whatever their names say.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

/// When `path` was last modified; the epoch if that cannot be read, which
/// ranks an unreadable file as the oldest.
pub(crate) fn modified(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// The items to delete so that only the newest `keep` remain, oldest first.
/// Items modified at the same instant are told apart by their own order, so
/// the result does not depend on directory listing order.
pub(crate) fn beyond_newest<T: Ord>(mut items: Vec<(T, SystemTime)>, keep: usize) -> Vec<T> {
    items.sort_by(|(a_item, a_time), (b_item, b_time)| {
        a_time.cmp(b_time).then_with(|| a_item.cmp(b_item))
    });
    let excess = items.len().saturating_sub(keep);
    items
        .into_iter()
        .take(excess)
        .map(|(item, _)| item)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::time::Duration;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn the_oldest_by_time_go_whatever_their_names_say() -> Result<()> {
        // `z` is the oldest by time though it sorts last by name.
        let items = vec![("a", at(30)), ("z", at(10)), ("m", at(20))];
        assert_eq!(beyond_newest(items, 2), vec!["z"], "only z is excess");
        Ok(())
    }

    #[test]
    fn equal_times_fall_back_to_name_order() -> Result<()> {
        let items = vec![("c", at(5)), ("a", at(5)), ("b", at(5))];
        assert_eq!(beyond_newest(items, 1), vec!["a", "b"], "a then b go");
        Ok(())
    }

    #[test]
    fn nothing_is_excess_under_the_limit() -> Result<()> {
        assert!(beyond_newest(vec![("a", at(1))], 1).is_empty(), "at limit");
        assert!(
            beyond_newest(Vec::<(&str, SystemTime)>::new(), 0).is_empty(),
            "empty"
        );
        Ok(())
    }
}
