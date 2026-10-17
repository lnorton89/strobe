// ============================================================================
// Module:       strobe_io::takeover
// Description:  Soft takeover (pickup) for faders, so a control does not
//               jump when a hardware fader that is somewhere else is touched.
//
// Dependencies: serde
// ============================================================================

//! Soft takeover.
//!
//! A fader has a position; the control it drives has a value; after a scene
//! change, a preset recall or a drag in the inspector the two disagree. In
//! [`Takeover::Jump`] the control leaps to the fader the moment it moves,
//! which on a projector is a visible lurch. In [`Takeover::Pickup`] the
//! fader is ignored until it *reaches* the control's value, either by
//! landing within [`TOLERANCE`] of it or by crossing it between two
//! readings; from then on it drives the control. If something else moves the
//! control (the UI, a preset, another mapping), the fader is dropped again
//! and must be picked up anew.
//!
//! The comparison is in the control's own normalised terms, after the
//! mapping's range, curve and inversion, so it works for any of them. A
//! control outside what the mapping can reach is compared at the nearest end
//! of the reach, so a narrowed fader still picks it up there (D373).
//!
//! [`Pickup`] holds the state for one mapping and is a pure function of the
//! values it is shown.

use serde::{Deserialize, Serialize};

/// How close counts as having reached the value, in normalised units
/// (about three steps of a 7-bit controller).
pub const TOLERANCE: f32 = 0.025;

/// What a mapping does about a fader that is somewhere else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Takeover {
    /// Follow the fader at once, even if that is a jump.
    Jump,
    /// Ignore the fader until it reaches the control's value.
    #[default]
    Pickup,
}

impl Takeover {
    /// The mode's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Jump => "Jump",
            Self::Pickup => "Pickup",
        }
    }
}

/// One mapping's pickup state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pickup {
    /// The fader is driving the control.
    engaged: bool,
    /// The previous reading, to detect a crossing.
    last_input: Option<f32>,
    /// The value this mapping last wrote, to notice someone else's write.
    last_sent: Option<f32>,
}

impl Pickup {
    /// Whether the fader is currently driving the control.
    pub fn engaged(&self) -> bool {
        self.engaged
    }

    /// Shows the filter a reading. `wanted` is where the mapping would put
    /// the control, `current` where the control is now, both `0..=1`.
    /// Returns the value to write, or `None` while the fader has not yet
    /// reached the control.
    pub fn filter(&mut self, mode: Takeover, wanted: f32, current: f32) -> Option<f32> {
        let previous = self.last_input.replace(wanted);
        if mode == Takeover::Jump {
            self.engaged = true;
            self.last_sent = Some(wanted);
            return Some(wanted);
        }
        if self.engaged
            && self
                .last_sent
                .is_some_and(|sent| (sent - current).abs() > TOLERANCE)
        {
            // Someone else moved the control: the fader is no longer in
            // charge of it.
            self.engaged = false;
        }
        if !self.engaged {
            let near = (wanted - current).abs() <= TOLERANCE;
            let crossed =
                previous.is_some_and(|before| (before - current) * (wanted - current) < 0.0);
            if !(near || crossed) {
                return None;
            }
            self.engaged = true;
        }
        self.last_sent = Some(wanted);
        Some(wanted)
    }

    /// Forgets everything: the next reading must pick the control up again.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_always_follows_the_fader() {
        let mut p = Pickup::default();
        assert_eq!(p.filter(Takeover::Jump, 0.9, 0.1), Some(0.9), "leaps");
        assert_eq!(p.filter(Takeover::Jump, 0.2, 0.9), Some(0.2), "and back");
    }

    #[test]
    fn pickup_ignores_a_fader_that_is_far_from_the_control() {
        let mut p = Pickup::default();
        assert_eq!(p.filter(Takeover::Pickup, 0.9, 0.3), None, "far above");
        assert_eq!(p.filter(Takeover::Pickup, 0.8, 0.3), None, "still above");
        assert!(!p.engaged(), "not engaged yet");
    }

    #[test]
    fn pickup_engages_when_the_fader_lands_near_the_value() {
        let mut p = Pickup::default();
        assert_eq!(p.filter(Takeover::Pickup, 0.9, 0.5), None, "away");
        assert_eq!(
            p.filter(Takeover::Pickup, 0.51, 0.5),
            Some(0.51),
            "within tolerance"
        );
        assert!(p.engaged(), "engaged");
        assert_eq!(
            p.filter(Takeover::Pickup, 0.7, 0.51),
            Some(0.7),
            "now it drives"
        );
    }

    #[test]
    fn pickup_engages_when_the_fader_crosses_the_value_between_readings() {
        let mut p = Pickup::default();
        assert_eq!(p.filter(Takeover::Pickup, 0.1, 0.5), None, "below");
        // A fast flick from 0.1 to 0.9 skips the tolerance band entirely.
        assert_eq!(p.filter(Takeover::Pickup, 0.9, 0.5), Some(0.9), "crossed");
    }

    #[test]
    fn a_move_from_elsewhere_drops_the_fader_until_it_is_picked_up_again() {
        let mut p = Pickup::default();
        assert_eq!(p.filter(Takeover::Pickup, 0.5, 0.5), Some(0.5), "engaged");
        // The UI (or a preset) moves the control to 0.1; the fader is at 0.5.
        assert_eq!(p.filter(Takeover::Pickup, 0.52, 0.1), None, "dropped");
        assert!(!p.engaged(), "disengaged");
        assert_eq!(p.filter(Takeover::Pickup, 0.3, 0.1), None, "approaching");
        assert_eq!(
            p.filter(Takeover::Pickup, 0.08, 0.1),
            Some(0.08),
            "crossed back"
        );
    }

    #[test]
    fn an_engaged_fader_keeps_driving_while_the_control_follows() {
        let mut p = Pickup::default();
        assert_eq!(p.filter(Takeover::Pickup, 0.5, 0.5), Some(0.5), "engaged");
        assert_eq!(
            p.filter(Takeover::Pickup, 0.6, 0.5),
            Some(0.6),
            "still engaged: the control was at what we last wrote"
        );
    }

    #[test]
    fn reset_requires_a_fresh_pickup() {
        let mut p = Pickup::default();
        p.filter(Takeover::Pickup, 0.5, 0.5);
        p.reset();
        assert_eq!(
            p.filter(Takeover::Pickup, 0.9, 0.5),
            None,
            "must pick up again"
        );
    }
}
