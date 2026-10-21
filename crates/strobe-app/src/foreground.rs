// ============================================================================
// Module:       strobe_app::foreground
// Description:  Notices when the fullscreen output over the control window's
//               monitor is not the foreground window, so the log can say
//               that Windows may be composing it. Pure, no winit.
//
// Dependencies: none
// ============================================================================

//! Whether the fullscreen output has the foreground (D453).
//!
//! On the venue mini PC a soak lost 5-11% of refreshes while console
//! windows, the taskbar and a firewall dialog sat over the fullscreen
//! output, which was then no longer the foreground window: Windows
//! composed it through DWM instead of presenting it directly (MINIPC.md,
//! M26). Nothing in Strobe's own log said so.
//!
//! winit reports no occlusion on Windows, and asking the window system
//! about the z-order each frame is the kind of query the frame loop never
//! makes. Focus is free: winit reports it for both windows as an event. On
//! one monitor, with the output fullscreen over the control window, the
//! output should have the focus: the control window cannot be seen, and a
//! window of another program with the focus is drawn over the show. So the
//! watch looks only at that layout. A normal two-screen setup, where the
//! output is on the projector and the control window on the operator's
//! screen with the focus, is never flagged.
//!
//! The app checks it each time it rereads the monitor list (every 2 s),
//! never per frame. A state has to last [`GRACE`] before it is reported, so
//! focus moving from one window to the other is not. The first report of a
//! run is a warning; later ones and each recovery are info lines, a few a
//! night at most.

use std::time::{Duration, Instant};

/// How long the output must lack the focus before it is reported.
pub const GRACE: Duration = Duration::from_secs(2);

/// Which window has the keyboard focus, as far as Strobe can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The output window.
    Output,
    /// The control window.
    Control,
    /// Neither: a window of another program, or the desktop.
    Elsewhere,
}

impl Focus {
    /// From the focus winit last reported for each window.
    pub fn of(output_focused: bool, control_focused: bool) -> Self {
        if output_focused {
            Self::Output
        } else if control_focused {
            Self::Control
        } else {
            Self::Elsewhere
        }
    }

    /// Who has the focus, for the log.
    pub fn holder(self) -> &'static str {
        match self {
            Self::Output => "the output",
            Self::Control => "the control window",
            Self::Elsewhere => "another program",
        }
    }
}

/// What a check found that the log should say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The output has lacked the focus for [`GRACE`]; `first` in this run.
    Lost {
        /// Who has the focus instead.
        focus: Focus,
        /// Whether this is the run's first report.
        first: bool,
    },
    /// A reported loss ended, after this long.
    Back(Duration),
}

/// The watch over the output's focus.
#[derive(Debug, Default)]
pub struct Watch {
    /// Since when the output has lacked the focus in the watched layout.
    since: Option<Instant>,
    /// Whether the current loss was reported.
    reported: bool,
    /// Whether any loss was reported in this run.
    warned: bool,
}

impl Watch {
    /// One check. `covers_control` is whether the output is shown
    /// fullscreen on the control window's monitor; `focus` is who has the
    /// keyboard focus.
    pub fn check(&mut self, covers_control: bool, focus: Focus, now: Instant) -> Option<Change> {
        if covers_control && focus != Focus::Output {
            let since = *self.since.get_or_insert(now);
            if self.reported || now.duration_since(since) < GRACE {
                return None;
            }
            self.reported = true;
            let first = !std::mem::replace(&mut self.warned, true);
            return Some(Change::Lost { focus, first });
        }
        let since = self.since.take()?;
        std::mem::take(&mut self.reported).then(|| Change::Back(now.duration_since(since)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_window_over_the_fullscreen_output_is_reported_once_and_its_end_too() -> Result<()> {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut watch = Watch::default();
        assert_eq!(
            watch.check(true, Focus::Output, at(0)),
            None,
            "the venue as it should be: the output has the focus"
        );
        // The firewall dialog comes up over the projection.
        assert_eq!(
            watch.check(true, Focus::Elsewhere, at(2)),
            None,
            "not at once"
        );
        assert_eq!(
            watch.check(true, Focus::Elsewhere, at(4)),
            Some(Change::Lost {
                focus: Focus::Elsewhere,
                first: true
            }),
            "after the grace: the run's first report"
        );
        assert_eq!(
            watch.check(true, Focus::Elsewhere, at(6)),
            None,
            "said once while it lasts"
        );
        assert_eq!(
            watch.check(true, Focus::Output, at(64)),
            Some(Change::Back(Duration::from_secs(62))),
            "and its end, with how long it lasted"
        );
        assert_eq!(watch.check(true, Focus::Output, at(66)), None, "once");
        // The control window keeps the focus after a launch.
        assert_eq!(watch.check(true, Focus::Control, at(70)), None, "grace");
        assert_eq!(
            watch.check(true, Focus::Control, at(72)),
            Some(Change::Lost {
                focus: Focus::Control,
                first: false
            }),
            "later reports are not the first"
        );
        Ok(())
    }

    #[test]
    fn a_two_screen_setup_and_a_passing_focus_change_are_never_reported() -> Result<()> {
        let t0 = Instant::now();
        let mut watch = Watch::default();
        // The output on the projector, the control window focused on the
        // operator's screen: the normal show, for hours.
        for s in 0..3600 {
            assert_eq!(
                watch.check(false, Focus::Control, t0 + Duration::from_secs(s)),
                None,
                "a two-screen setup is never flagged ({s} s)"
            );
        }
        // On one screen, the focus passing through the control window for
        // less than the grace (a check lands in the middle of it).
        let mut watch = Watch::default();
        assert_eq!(watch.check(true, Focus::Control, t0), None, "begins");
        assert_eq!(
            watch.check(true, Focus::Output, t0 + Duration::from_secs(1)),
            None,
            "ended within the grace: nothing to say, and no Back either"
        );
        assert_eq!(
            watch.check(true, Focus::Control, t0 + Duration::from_secs(2)),
            None,
            "a new spell starts its own grace"
        );
        Ok(())
    }

    #[test]
    fn the_focus_is_read_from_both_windows() -> Result<()> {
        assert_eq!(Focus::of(true, false), Focus::Output, "output");
        assert_eq!(Focus::of(false, true), Focus::Control, "control");
        assert_eq!(Focus::of(false, false), Focus::Elsewhere, "neither");
        assert_eq!(Focus::Elsewhere.holder(), "another program", "words");
        Ok(())
    }
}
