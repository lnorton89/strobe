// ============================================================================
// Module:       strobe_app::show::start
// Description:  Where a playlist begins when it has been compiled: the wall
//               clock if it is anchored, else the saved resume point, else
//               the first cue.
//
// Dependencies: strobe-playlist (PlaylistClock, ResumePoint, Placement)
// ============================================================================

//! Choosing where the show starts.
//!
//! Three things can say where a playlist begins, and they have a fixed
//! order of authority:
//!
//! 1. **The wall clock**, for a playlist with an anchor ("starts at 09:00
//!    every day"). The schedule says what *should* be playing now, so a
//!    late start jumps to that cue (`sync_to_wall_clock`, D76), a start
//!    before the anchor waits for it, and a start after a non-looping show
//!    has finished plays nothing. A saved resume point is ignored for an
//!    anchored playlist: it would put the show somewhere the schedule says
//!    it is not.
//! 2. **The resume point** saved by the last run (every five seconds, D155),
//!    if it was saved for *this* playlist (the identity matches). A point
//!    for another playlist, or for a cue that no longer exists, is refused
//!    with the reason and the show starts from the top.
//! 3. **The first cue**, when the clock is first ticked.
//!
//! [`plan`] applies that order to a clock and says what it did. It reads no
//! clock of its own: "now" is an argument, like everywhere in the playlist
//! crate.

use strobe_playlist::{Placement, PlaylistClock, ResumeError, ResumePoint};

/// What starting the show did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Start {
    /// Nothing moved the clock: the first tick starts the first cue.
    Fresh,
    /// The clock was put back where a resume point saved it.
    Resumed {
        /// Seconds into the cue it resumed.
        offset: f64,
    },
    /// A saved point was refused and the show starts from the top.
    ResumeRefused(ResumeError),
    /// The anchor put the clock in the cue the schedule says is playing.
    OnSchedule {
        /// Seconds into that cue.
        offset: f64,
    },
    /// The anchor is later today: do not tick the clock yet.
    WaitingForAnchor {
        /// Seconds until the anchor.
        seconds: f64,
    },
    /// The anchored show already finished for today.
    Finished,
}

/// Applies the order above to `clock`. `resume` is the saved point, if one
/// was found, and `now_seconds_of_day` is local seconds since midnight.
///
/// `lenient` skips the identity check on the point: a **reload** of an
/// edited playlist is meant to be the same show (adding a cue changes the
/// identity, and the position should survive it), where a point read from
/// the resume file at launch must belong to this playlist.
pub fn plan(
    clock: &mut PlaylistClock,
    resume: Option<&ResumePoint>,
    now_seconds_of_day: f64,
    lenient: bool,
) -> Start {
    if let Some(placement) = clock.sync_to_wall_clock(now_seconds_of_day) {
        return match placement {
            Placement::Waiting {
                seconds_until_start,
            } => Start::WaitingForAnchor {
                seconds: seconds_until_start,
            },
            Placement::Playing { offset, .. } => Start::OnSchedule { offset },
            Placement::Finished { .. } => Start::Finished,
        };
    }
    match resume {
        Some(point) => match if lenient {
            clock.restore_lenient(point)
        } else {
            clock.restore(point)
        } {
            Ok(()) => Start::Resumed {
                offset: point.offset,
            },
            Err(refused) => Start::ResumeRefused(refused),
        },
        None => Start::Fresh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use strobe_playlist::{
        ClockConfig, Cue, DurationSpec, LoopMode, Playlist, SceneRef, Section, Timeline,
        TimelineConfig, WallClockAnchor,
    };

    /// Three 10-second cues, optionally anchored at 09:00 and looping.
    fn clock(anchor: bool, looping: bool) -> Result<PlaylistClock> {
        let mut playlist = Playlist::new("show");
        playlist.defaults.duration = Some(DurationSpec::Seconds(10.0));
        let mut section = Section::new("s");
        for name in ["a", "b", "c"] {
            section.cues.push(Cue::new(SceneRef::Library(name.into())));
        }
        playlist.sections.push(section);
        if anchor {
            playlist.anchor = Some(
                WallClockAnchor::from_hm(9, 0).ok_or_else(|| anyhow!("09:00 is a time of day"))?,
            );
        }
        if looping {
            playlist.loop_mode = LoopMode::Loop;
        }
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
        Ok(PlaylistClock::new(timeline, ClockConfig::default()))
    }

    const NINE: f64 = 9.0 * 3600.0;

    #[test]
    fn with_nothing_saved_and_no_anchor_the_show_starts_fresh() -> Result<()> {
        let mut c = clock(false, false)?;
        assert_eq!(
            plan(&mut c, None, NINE, false),
            Start::Fresh,
            "nothing to go on"
        );
        assert_eq!(c.current(), None, "the first tick starts the first cue");
        Ok(())
    }

    #[test]
    fn a_matching_resume_point_puts_the_clock_back() -> Result<()> {
        let mut c = clock(false, false)?;
        let point = ResumePoint {
            identity: c.timeline().identity(),
            section: 0,
            cue: 2,
            pass: 0,
            offset: 4.5,
        };
        assert_eq!(
            plan(&mut c, Some(&point), NINE, false),
            Start::Resumed { offset: 4.5 },
            "resumed"
        );
        assert_eq!(c.current(), Some(2), "on the saved cue");
        assert!((c.elapsed() - 4.5).abs() < 1e-9, "at the saved offset");
        Ok(())
    }

    #[test]
    fn a_point_for_another_playlist_is_refused_with_the_reason() -> Result<()> {
        let mut c = clock(false, false)?;
        let point = ResumePoint {
            identity: c.timeline().identity() ^ 1,
            section: 0,
            cue: 1,
            pass: 0,
            offset: 2.0,
        };
        let start = plan(&mut c, Some(&point), NINE, false);
        assert!(
            matches!(
                start,
                Start::ResumeRefused(ResumeError::IdentityMismatch { .. })
            ),
            "{start:?}"
        );
        assert_eq!(c.current(), None, "the clock was not moved");
        Ok(())
    }

    #[test]
    fn a_reload_keeps_the_position_even_though_the_playlist_changed() -> Result<()> {
        let mut c = clock(false, false)?;
        let point = ResumePoint {
            identity: c.timeline().identity() ^ 0xff,
            section: 0,
            cue: 1,
            pass: 0,
            offset: 3.0,
        };
        assert_eq!(
            plan(&mut c, Some(&point), NINE, true),
            Start::Resumed { offset: 3.0 },
            "lenient accepts another identity"
        );
        assert_eq!(c.current(), Some(1), "on the same cue");
        Ok(())
    }

    #[test]
    fn an_anchored_show_follows_the_wall_clock_and_ignores_a_saved_point() -> Result<()> {
        let mut c = clock(true, true)?;
        let point = ResumePoint {
            identity: c.timeline().identity(),
            section: 0,
            cue: 0,
            pass: 0,
            offset: 1.0,
        };
        // 09:00:25: 25 seconds in is five seconds into the third cue.
        let start = plan(&mut c, Some(&point), NINE + 25.0, false);
        assert_eq!(start, Start::OnSchedule { offset: 5.0 }, "{start:?}");
        assert_eq!(c.current(), Some(2), "the cue the schedule names");
        Ok(())
    }

    #[test]
    fn before_the_anchor_the_clock_waits_and_after_a_one_shot_show_it_is_over() -> Result<()> {
        let mut early = clock(true, false)?;
        assert_eq!(
            plan(&mut early, None, NINE - 60.0, false),
            Start::WaitingForAnchor { seconds: 60.0 },
            "a minute early"
        );
        assert_eq!(early.current(), None, "not started");

        let mut late = clock(true, false)?;
        assert_eq!(
            plan(&mut late, None, NINE + 3600.0, false),
            Start::Finished,
            "an hour after a 30-second show"
        );
        Ok(())
    }
}
