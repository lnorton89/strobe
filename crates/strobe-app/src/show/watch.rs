// ============================================================================
// Module:       strobe_app::show::watch
// Description:  Noticing that the playlist file was saved, by polling its
//               modification time at most once a second.
//
// Dependencies: std::fs
// ============================================================================

//! The playlist file watcher.
//!
//! "Text mode" (PLAN section 7.3) means opening the TOML in an editor and
//! having the show pick the change up when the file is saved. A watcher
//! crate would be a dependency, a thread and a platform backend for one
//! file, so this polls instead: [`Watch::due`] says yes once a second
//! (counted from frame `dt`, no timer thread), and the caller reads the
//! file's [`Stamp`] (a `stat`, the one syscall a poll costs) and gives it to
//! [`Watch::check`].
//!
//! ## Settling
//!
//! Editors save in several steps (truncate, write, rename). A change is
//! therefore reported only when the stamp has been **the same on two polls
//! in a row** after it changed, so the show reloads the finished file and
//! not the half-written one. A parse error in the finished file is the
//! caller's to show; it keeps the old playlist, and the next save is a new
//! change.
//!
//! The app's own saves (the Generate dialog's Save) call
//! [`Watch::accept`] so they do not reload themselves.

use std::path::Path;
use std::time::SystemTime;

/// Seconds between polls.
pub const POLL_EVERY: f64 = 1.0;

/// What identifies a version of the file: when it was modified and how big
/// it is (a save inside the clock's resolution still changes the size
/// almost always).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// Modification time.
    pub modified: SystemTime,
    /// Length in bytes.
    pub len: u64,
}

/// Reads `path`'s stamp, or `None` if it cannot be read (missing,
/// permissions).
pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok()?,
        len: meta.len(),
    })
}

/// The watcher's state.
#[derive(Debug)]
pub struct Watch {
    since: f64,
    /// The stamp of the version the show has loaded.
    loaded: Option<Stamp>,
    /// A different stamp seen on the previous poll, waiting to settle.
    pending: Option<Stamp>,
}

impl Watch {
    /// A watcher for a file whose current version is `loaded`.
    pub fn new(loaded: Option<Stamp>) -> Self {
        Self {
            since: 0.0,
            loaded,
            pending: None,
        }
    }

    /// Counts `dt` and says whether it is time to poll.
    pub fn due(&mut self, dt: f64) -> bool {
        if dt.is_finite() && dt > 0.0 {
            self.since += dt;
        }
        if self.since + 1e-6 < POLL_EVERY {
            return false;
        }
        self.since = 0.0;
        true
    }

    /// Feeds a poll's stamp; returns true when the file changed and has
    /// stayed unchanged since, so it is safe to load. An unreadable file
    /// (`None`) is a file in the middle of being replaced: it changes
    /// nothing.
    pub fn check(&mut self, now: Option<Stamp>) -> bool {
        let Some(now) = now else {
            return false;
        };
        if Some(now) == self.loaded {
            self.pending = None;
            return false;
        }
        if self.pending == Some(now) {
            self.pending = None;
            self.loaded = Some(now);
            return true;
        }
        self.pending = Some(now);
        false
    }

    /// Records `now` as the version the show has, without reporting it as a
    /// change (the app wrote it, or loaded it by another route).
    pub fn accept(&mut self, now: Option<Stamp>) {
        self.loaded = now;
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn stamp_at(seconds: u64, len: u64) -> Option<Stamp> {
        Some(Stamp {
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
            len,
        })
    }

    #[test]
    fn polls_come_once_a_second_whatever_the_frame_rate() {
        let mut watch = Watch::new(None);
        let polls = (0..(60 * 10)).filter(|_| watch.due(1.0 / 60.0)).count();
        assert_eq!(polls, 10, "ten seconds, ten polls");
        assert!(!watch.due(f64::NAN), "nonsense adds no time");
    }

    #[test]
    fn a_change_is_reported_once_it_has_settled() {
        let mut watch = Watch::new(stamp_at(100, 500));
        assert!(!watch.check(stamp_at(100, 500)), "unchanged");
        assert!(
            !watch.check(stamp_at(105, 520)),
            "first sight of the change"
        );
        assert!(watch.check(stamp_at(105, 520)), "the same on the next poll");
        assert!(!watch.check(stamp_at(105, 520)), "and only reported once");
    }

    #[test]
    fn a_file_still_being_written_is_not_loaded_half_done() {
        let mut watch = Watch::new(stamp_at(100, 500));
        assert!(!watch.check(stamp_at(105, 100)), "truncated");
        assert!(!watch.check(stamp_at(106, 480)), "still growing");
        assert!(watch.check(stamp_at(106, 480)), "finished");
    }

    #[test]
    fn an_unreadable_file_changes_nothing_and_our_own_save_is_not_a_change() {
        let mut watch = Watch::new(stamp_at(100, 500));
        assert!(!watch.check(None), "missing for a moment");
        watch.accept(stamp_at(200, 900));
        assert!(!watch.check(stamp_at(200, 900)), "our own write");
        assert!(!watch.check(stamp_at(200, 900)), "still not a change");
    }

    #[test]
    fn a_real_file_has_a_stamp_and_a_missing_one_does_not() -> anyhow::Result<()> {
        let file = std::env::temp_dir().join(format!("strobe-watch-{}.toml", std::process::id()));
        std::fs::write(&file, "name = \"x\"\n")?;
        let found = stamp(&file);
        let _ = std::fs::remove_file(&file);
        assert!(found.is_some_and(|s| s.len == 11), "{found:?}");
        assert!(stamp(&file).is_none(), "gone");
        Ok(())
    }
}
