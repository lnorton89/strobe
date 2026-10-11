// ============================================================================
// Module:       strobe_scene::meta
// Description:  The meta controls: output-wide settings (master, brightness,
//               hue, mirror...) as an ordinary control bank, so a script's
//               meta/ writes take the path every other control write takes.
//
// Dependencies: crate::json (RawControl), crate::control (ControlDef, ControlBank)
// ============================================================================

//! Meta controls.
//!
//! In Synesthesia the meta controls sit beside a scene's own controls and
//! apply to the whole output rather than to one scene: master level,
//! invert, brightness, gamma, contrast, hue, saturation, vertical and
//! horizontal mirror, reactivity and audio speed (`docs/PLAN.md` §1.6). A
//! script reaches them by a `meta/` prefix or `bank: "meta"` on
//! `setControl` and its relatives.
//!
//! Strobe models them as a second [`ControlBank`] built from definitions
//! that go through the same validation as a scene's `CONTROLS`
//! ([`ControlDef::from_raw`]). That is deliberate: clamping, toggles
//! snapping to either end, normalised writes, `randomize` and `default`
//! are then the one implementation, and a script write to a meta control
//! travels the same queued-command path as a write to a scene control.
//!
//! **Who reads them.** [`MetaValues::read`] snapshots the bank into a plain
//! `Copy` struct each frame without allocating. The renderer's output post
//! stage (`strobe_gfx::post`, D180) applies master, invert, brightness,
//! gamma, contrast, hue, saturation and the two mirrors; the audio path
//! applies reactivity and audio speed (D184). See D160, D180 and Q21 for
//! what is unverified (the ranges below are Strobe's choice, not
//! Synesthesia's).

use crate::control::{ControlBank, ControlDef};
use crate::error::SceneError;
use crate::json::RawControl;

/// The meta controls, as `scene.json`-style control objects.
///
/// Names are the Synesthesia UI labels with spaces turned into `_`; the
/// script API's matching ignores case, spaces, `_` and `-`, so
/// `meta/Vertical Mirror` and `meta/vertical_mirror` reach the same one.
/// Ranges and defaults are Strobe's: neutral at the default (a master of 1,
/// brightness 0, gamma, contrast and saturation 1, no hue shift).
const DEFINITIONS: &str = r#"[
    { "NAME": "master", "TYPE": "slider", "MIN": 0.0, "MAX": 1.0, "DEFAULT": 1.0 },
    { "NAME": "invert", "TYPE": "toggle", "DEFAULT": 0, "UI_GROUP": "color" },
    { "NAME": "brightness", "TYPE": "slider", "MIN": -1.0, "MAX": 1.0, "DEFAULT": 0.0, "UI_GROUP": "color" },
    { "NAME": "gamma", "TYPE": "slider", "MIN": 0.2, "MAX": 3.0, "DEFAULT": 1.0, "UI_GROUP": "color" },
    { "NAME": "contrast", "TYPE": "slider", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 1.0, "UI_GROUP": "color" },
    { "NAME": "hue", "TYPE": "slider", "MIN": 0.0, "MAX": 1.0, "DEFAULT": 0.0, "UI_GROUP": "color" },
    { "NAME": "saturation", "TYPE": "slider", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 1.0, "UI_GROUP": "color" },
    { "NAME": "vertical_mirror", "TYPE": "toggle", "DEFAULT": 0, "UI_GROUP": "mirror" },
    { "NAME": "horizontal_mirror", "TYPE": "toggle", "DEFAULT": 0, "UI_GROUP": "mirror" },
    { "NAME": "reactivity", "TYPE": "slider", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 1.0, "UI_GROUP": "audio" },
    { "NAME": "audio_speed", "TYPE": "slider", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 1.0, "UI_GROUP": "audio" }
]"#;

/// Positions in [`DEFINITIONS`], which is a fixed list; a test pins each to
/// its name so reading by index is safe and needs no per-frame name lookup.
mod slot {
    pub(super) const MASTER: usize = 0;
    pub(super) const INVERT: usize = 1;
    pub(super) const BRIGHTNESS: usize = 2;
    pub(super) const GAMMA: usize = 3;
    pub(super) const CONTRAST: usize = 4;
    pub(super) const HUE: usize = 5;
    pub(super) const SATURATION: usize = 6;
    pub(super) const VERTICAL_MIRROR: usize = 7;
    pub(super) const HORIZONTAL_MIRROR: usize = 8;
    pub(super) const REACTIVITY: usize = 9;
    pub(super) const AUDIO_SPEED: usize = 10;
}

/// How far from its neutral value a number may be and still count as
/// neutral: far below one 8-bit step, so a slider dragged back to its
/// default by hand skips the post stage.
const NEUTRAL_EPSILON: f32 = 1.0e-4;

/// The meta controls' current values, as plain numbers.
///
/// `Default` is neutral: nothing the post stage would change. Toggles are
/// `bool`; `hue` is a turn (`0..1`, so `1.0` is back where it started).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetaValues {
    /// Output level: `1` full, `0` black.
    pub master: f32,
    /// Photographic negative.
    pub invert: bool,
    /// Added to every channel before contrast and gamma, `-1..1`.
    pub brightness: f32,
    /// Display gamma: the picture is raised to `1 / gamma`.
    pub gamma: f32,
    /// Scale about mid-grey, `0..2`.
    pub contrast: f32,
    /// Hue rotation in turns.
    pub hue: f32,
    /// `0` grey, `1` as drawn, `2` doubled.
    pub saturation: f32,
    /// Mirror top to bottom: the lower half is reflected onto the upper.
    pub vertical_mirror: bool,
    /// Mirror left to right: the left half is reflected onto the right.
    pub horizontal_mirror: bool,
    /// Gain on audio levels and hits (applied by the audio path).
    pub reactivity: f32,
    /// Gain on audio clocks (applied by the audio path).
    pub audio_speed: f32,
}

impl Default for MetaValues {
    fn default() -> Self {
        Self {
            master: 1.0,
            invert: false,
            brightness: 0.0,
            gamma: 1.0,
            contrast: 1.0,
            hue: 0.0,
            saturation: 1.0,
            vertical_mirror: false,
            horizontal_mirror: false,
            reactivity: 1.0,
            audio_speed: 1.0,
        }
    }
}

impl MetaValues {
    /// Reads the current (smoothed) output of a bank made by [`bank`].
    /// Allocates nothing. A bank of any other shape reads as neutral.
    pub fn read(bank: &ControlBank) -> Self {
        let value = |index: usize, fallback: f32| {
            bank.output(index)
                .map(|values| values[0])
                .filter(|v| v.is_finite())
                .unwrap_or(fallback)
        };
        let flag = |index: usize| value(index, 0.0) >= 0.5;
        let neutral = Self::default();
        Self {
            master: value(slot::MASTER, neutral.master),
            invert: flag(slot::INVERT),
            brightness: value(slot::BRIGHTNESS, neutral.brightness),
            gamma: value(slot::GAMMA, neutral.gamma),
            contrast: value(slot::CONTRAST, neutral.contrast),
            hue: value(slot::HUE, neutral.hue),
            saturation: value(slot::SATURATION, neutral.saturation),
            vertical_mirror: flag(slot::VERTICAL_MIRROR),
            horizontal_mirror: flag(slot::HORIZONTAL_MIRROR),
            reactivity: value(slot::REACTIVITY, neutral.reactivity),
            audio_speed: value(slot::AUDIO_SPEED, neutral.audio_speed),
        }
    }

    /// Whether the output post stage would leave the picture untouched.
    /// Reactivity and audio speed do not count: they are not pixels.
    pub fn post_is_neutral(&self) -> bool {
        let near = |a: f32, b: f32| (a - b).abs() <= NEUTRAL_EPSILON;
        near(self.master, 1.0)
            && !self.invert
            && near(self.brightness, 0.0)
            && near(self.gamma, 1.0)
            && near(self.contrast, 1.0)
            && (near(self.hue, 0.0) || near(self.hue, 1.0))
            && near(self.saturation, 1.0)
            && !self.vertical_mirror
            && !self.horizontal_mirror
    }
}

/// The validated meta control definitions, in a fixed order.
pub fn controls() -> Result<Vec<ControlDef>, SceneError> {
    let raw: Vec<RawControl> = serde_json::from_str(DEFINITIONS)?;
    raw.iter().map(ControlDef::from_raw).collect()
}

/// A bank of the meta controls, each at its default.
pub fn bank() -> Result<ControlBank, SceneError> {
    ControlBank::new(controls()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    #[test]
    fn the_definitions_validate_and_names_are_distinct() -> Result<()> {
        let bank = bank()?;
        assert_eq!(bank.defs().len(), 11, "the documented set");
        for name in [
            "Master",
            "Vertical Mirror",
            "horizontal-mirror",
            "AUDIO_SPEED",
        ] {
            assert!(bank.find(name).is_some(), "`{name}` should match");
        }
        Ok(())
    }

    #[test]
    fn the_slots_match_the_names() -> Result<()> {
        let bank = bank()?;
        for (slot, name) in [
            (slot::MASTER, "master"),
            (slot::INVERT, "invert"),
            (slot::BRIGHTNESS, "brightness"),
            (slot::GAMMA, "gamma"),
            (slot::CONTRAST, "contrast"),
            (slot::HUE, "hue"),
            (slot::SATURATION, "saturation"),
            (slot::VERTICAL_MIRROR, "vertical_mirror"),
            (slot::HORIZONTAL_MIRROR, "horizontal_mirror"),
            (slot::REACTIVITY, "reactivity"),
            (slot::AUDIO_SPEED, "audio_speed"),
        ] {
            assert_eq!(bank.find(name), Some(slot), "`{name}` sits at slot {slot}");
        }
        Ok(())
    }

    #[test]
    fn a_fresh_bank_reads_as_the_neutral_values() -> Result<()> {
        let values = MetaValues::read(&bank()?);
        assert_eq!(
            values,
            MetaValues::default(),
            "defaults are the neutral set"
        );
        assert!(
            values.post_is_neutral(),
            "and the post stage has nothing to do"
        );
        Ok(())
    }

    #[test]
    fn a_written_value_is_read_back_and_ends_neutrality() -> Result<()> {
        let mut bank = bank()?;
        for (name, value) in [("hue", 0.25), ("vertical_mirror", 1.0), ("master", 0.5)] {
            let index = bank.find(name).ok_or_else(|| anyhow!("no `{name}`"))?;
            bank.set(index, None, &[value]);
        }
        bank.tick(1.0);
        let values = MetaValues::read(&bank);
        assert_eq!(values.hue, 0.25, "hue");
        assert!(values.vertical_mirror, "mirror toggled on");
        assert!(!values.horizontal_mirror, "the other mirror is untouched");
        assert_eq!(values.master, 0.5, "master");
        assert!(!values.post_is_neutral(), "something changed");
        Ok(())
    }

    #[test]
    fn a_whole_turn_of_hue_and_a_tiny_slip_are_neutral() {
        let neutral = MetaValues::default();
        let turn = MetaValues {
            hue: 1.0,
            ..neutral
        };
        let slip = MetaValues {
            gamma: 1.00001,
            ..neutral
        };
        let audio = MetaValues {
            reactivity: 2.0,
            audio_speed: 0.0,
            ..neutral
        };
        assert!(turn.post_is_neutral(), "a full turn is no rotation");
        assert!(slip.post_is_neutral(), "below one 8-bit step");
        assert!(audio.post_is_neutral(), "audio gains are not pixels");
    }

    #[test]
    fn every_control_starts_neutral() -> Result<()> {
        let bank = bank()?;
        let output = |name: &str| -> Result<f32> {
            let index = bank.find(name).ok_or_else(|| anyhow!("no `{name}`"))?;
            Ok(bank.output(index).map_or(f32::NAN, |values| values[0]))
        };
        assert_eq!(output("master")?, 1.0, "master is fully open");
        assert_eq!(output("brightness")?, 0.0, "no brightness shift");
        assert_eq!(output("invert")?, 0.0, "not inverted");
        assert_eq!(output("gamma")?, 1.0, "neutral gamma");
        Ok(())
    }
}
