// ============================================================================
// Module:       strobe_playlist::schedule
// Description:  Wall-clock scheduling: which cue should be playing at a
//               time of day, the wall time of each cue, and the late-start
//               catch-up, all as pure functions of a timeline.
//
// Dependencies: crate::timeline, crate::model
// ============================================================================

//! The wall clock, kept apart from the scene clock.
//!
//! A playlist with an anchor ("starts at 09:00") has two notions of time:
//!
//! - the **scene clock**, which the [`PlaylistClock`](crate::PlaylistClock)
//!   advances by frame time, which stops while held, and which a person can
//!   push around with next and extend; and
//! - the **wall clock**, a time of day the schedule is defined against.
//!
//! This module is the second one and it only ever *reads* a [`Timeline`].
//! Given the anchor and "now" it says which cue should be playing and how
//! far into it ([`Timeline::place`]); the app hands that to the clock to
//! jump forward on a late start. It never edits the playlist, and the clock
//! never consults the wall clock behind the app's back: "now" is always an
//! argument.
//!
//! Wall times are computed from the *static* timeline (fallback BPM, nominal
//! lengths for manual cues). Beat-locked endings and manual advances move
//! the real show away from them over time; the schedule is the plan, not a
//! promise.
//!
//! [`LoopMode::LoopSection`] places like [`LoopMode::Loop`]: a section loop
//! is something a person turns on during the show, not part of the plan.

use crate::model::{LoopMode, SECONDS_PER_DAY, WallClockAnchor};
use crate::timeline::Timeline;
use std::fmt;

/// A time on the wall clock, possibly days after the anchor's day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallTime {
    /// Whole days after the day the show starts (0 = same day).
    pub day: u32,
    /// Seconds after local midnight.
    pub seconds_of_day: f64,
}

impl fmt::Display for WallTime {
    /// `HH:MM:SS`, with ` (+Nd)` when it falls on a later day.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.seconds_of_day.floor().max(0.0) as u64;
        write!(
            f,
            "{:02}:{:02}:{:02}",
            whole / 3600,
            whole % 3600 / 60,
            whole % 60
        )?;
        if self.day > 0 {
            write!(f, " (+{}d)", self.day)?;
        }
        Ok(())
    }
}

/// What the schedule says should be happening at a moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// The show has not started yet.
    Waiting {
        /// Seconds until the anchor.
        seconds_until_start: f64,
    },
    /// The show is on, in this cue.
    Playing {
        /// Which pass over the playlist, from 0.
        pass: u64,
        /// Timeline index of the cue.
        index: usize,
        /// Seconds into that cue.
        offset: f64,
    },
    /// A non-looping show has ended (or there was nothing to play).
    Finished {
        /// Seconds since it ended.
        seconds_over: f64,
    },
}

/// Seconds from the anchor to `now_seconds_of_day`: negative before the
/// anchor on the same day, positive after.
pub fn elapsed_since_anchor(anchor: WallClockAnchor, now_seconds_of_day: f64) -> f64 {
    now_seconds_of_day - f64::from(anchor.seconds_of_day)
}

impl Timeline {
    /// The wall time cue `index` starts, if the playlist has an anchor.
    pub fn wall_time(&self, index: usize) -> Option<WallTime> {
        let anchor = self.anchor()?;
        let cue = self.cues().get(index)?;
        Some(wall_time_of(anchor, cue.start))
    }

    /// The wall time one pass ends, if the playlist has an anchor.
    pub fn wall_end(&self) -> Option<WallTime> {
        Some(wall_time_of(self.anchor()?, self.total_runtime()))
    }

    /// What should be playing `elapsed` seconds after the anchor.
    ///
    /// For a looping playlist the answer repeats every
    /// [`total_runtime`](Self::total_runtime): lengths are fixed per cue
    /// (see [`crate::timeline`]), so `elapsed` modulo the total is exact.
    pub fn place(&self, elapsed: f64) -> Placement {
        if !elapsed.is_finite() || elapsed < 0.0 {
            let wait = if elapsed.is_finite() { -elapsed } else { 0.0 };
            return Placement::Waiting {
                seconds_until_start: wait,
            };
        }
        let total = self.total_runtime();
        if self.is_empty() || total <= 0.0 {
            return Placement::Finished {
                seconds_over: elapsed,
            };
        }
        let looping = match self.loop_mode() {
            LoopMode::Stop => false,
            LoopMode::Loop | LoopMode::LoopSection => true,
        };
        if !looping {
            return match self.locate(elapsed) {
                Some((index, offset)) => Placement::Playing {
                    pass: 0,
                    index,
                    offset,
                },
                None => Placement::Finished {
                    seconds_over: elapsed - total,
                },
            };
        }
        let mut pass = (elapsed / total).floor();
        let mut within = elapsed - pass * total;
        // Float rounding can leave `within` a hair outside [0, total).
        if within >= total {
            pass += 1.0;
            within = 0.0;
        } else if within < 0.0 {
            within = 0.0;
        }
        match self.locate(within) {
            Some((index, offset)) => Placement::Playing {
                pass: pass as u64,
                index,
                offset,
            },
            None => Placement::Playing {
                pass: pass as u64,
                index: 0,
                offset: 0.0,
            },
        }
    }
}

/// Anchor plus an offset, folded into a day and a time of day.
fn wall_time_of(anchor: WallClockAnchor, offset: f64) -> WallTime {
    let absolute = f64::from(anchor.seconds_of_day) + offset.max(0.0);
    let day_seconds = f64::from(SECONDS_PER_DAY);
    let day = (absolute / day_seconds).floor();
    WallTime {
        day: day as u32,
        seconds_of_day: absolute - day * day_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::DurationSpec;
    use crate::model::{Cue, Playlist, SceneRef, Section};
    use crate::timeline::TimelineConfig;
    use anyhow::Result;

    fn timeline(
        durations: &[f64],
        loop_mode: LoopMode,
        anchor: Option<WallClockAnchor>,
    ) -> Timeline {
        let mut playlist = Playlist::new("t");
        playlist.loop_mode = loop_mode;
        playlist.anchor = anchor;
        let mut section = Section::new("s");
        for (i, seconds) in durations.iter().enumerate() {
            let mut cue = Cue::new(SceneRef::Library(format!("c{i}")));
            cue.settings.duration = Some(DurationSpec::Seconds(*seconds));
            section.cues.push(cue);
        }
        playlist.sections.push(section);
        Timeline::from_playlist(&playlist, &TimelineConfig::default())
    }

    #[test]
    fn before_the_anchor_the_show_waits() {
        let anchor = WallClockAnchor::from_hm(9, 0);
        let t = timeline(&[60.0], LoopMode::Stop, anchor);
        let elapsed = anchor.map_or(0.0, |a| elapsed_since_anchor(a, 8.0 * 3600.0));
        assert_eq!(elapsed, -3600.0, "an hour before the anchor");
        assert_eq!(
            t.place(elapsed),
            Placement::Waiting {
                seconds_until_start: 3600.0
            },
            "waiting an hour"
        );
    }

    #[test]
    fn a_late_start_lands_mid_cue() {
        let t = timeline(&[100.0, 100.0, 100.0], LoopMode::Stop, None);
        assert_eq!(
            t.place(250.0),
            Placement::Playing {
                pass: 0,
                index: 2,
                offset: 50.0
            },
            "250 s in is 50 s into the third cue"
        );
        assert_eq!(
            t.place(0.0),
            Placement::Playing {
                pass: 0,
                index: 0,
                offset: 0.0
            },
            "on time"
        );
    }

    #[test]
    fn a_stopping_show_finishes_and_a_looping_one_wraps() {
        let stop = timeline(&[100.0, 100.0], LoopMode::Stop, None);
        assert_eq!(
            stop.place(230.0),
            Placement::Finished { seconds_over: 30.0 },
            "30 s after the end"
        );
        let looped = timeline(&[100.0, 100.0], LoopMode::Loop, None);
        assert_eq!(
            looped.place(230.0),
            Placement::Playing {
                pass: 1,
                index: 0,
                offset: 30.0
            },
            "wrapped into pass 1"
        );
        assert_eq!(
            looped.place(400.0),
            Placement::Playing {
                pass: 2,
                index: 0,
                offset: 0.0
            },
            "exactly on a pass boundary"
        );
        let section = timeline(&[100.0], LoopMode::LoopSection, None);
        assert_eq!(
            section.place(150.0),
            Placement::Playing {
                pass: 1,
                index: 0,
                offset: 50.0
            },
            "a section loop places like a loop"
        );
    }

    #[test]
    fn an_empty_playlist_has_nothing_to_place() {
        let t = timeline(&[], LoopMode::Loop, None);
        assert_eq!(
            t.place(5.0),
            Placement::Finished { seconds_over: 5.0 },
            "empty"
        );
    }

    #[test]
    fn bad_elapsed_values_wait_rather_than_misplace() {
        let t = timeline(&[10.0], LoopMode::Loop, None);
        assert_eq!(
            t.place(f64::NAN),
            Placement::Waiting {
                seconds_until_start: 0.0
            },
            "NaN waits"
        );
    }

    #[test]
    fn placing_never_changes_the_timeline() {
        let t = timeline(&[10.0, 20.0], LoopMode::Loop, None);
        let before = t.clone();
        for elapsed in [0.0, 5.0, 31.0, 1e6] {
            let _ = t.place(elapsed);
        }
        assert_eq!(
            t, before,
            "scheduling must not mutate the playlist representation"
        );
    }

    #[test]
    fn wall_times_run_from_the_anchor_and_carry_into_the_next_day() -> Result<()> {
        let anchor = WallClockAnchor::from_hm(23, 0);
        let t = timeline(&[1800.0, 3600.0], LoopMode::Stop, anchor);
        assert_eq!(
            t.wall_time(0).map(|w| w.to_string()).as_deref(),
            Some("23:00:00"),
            "first cue"
        );
        assert_eq!(
            t.wall_time(1).map(|w| w.to_string()).as_deref(),
            Some("23:30:00"),
            "second cue"
        );
        assert_eq!(
            t.wall_end().map(|w| w.to_string()).as_deref(),
            Some("00:30:00 (+1d)"),
            "the end rolls past midnight"
        );
        assert!(
            timeline(&[1.0], LoopMode::Stop, None)
                .wall_time(0)
                .is_none(),
            "no anchor, no wall time"
        );
        Ok(())
    }
}
