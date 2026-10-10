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
//! **What is and is not here.** The bank, the names and the values exist.
//! Whether a value *does* anything is the renderer's business, and the
//! renderer has no output post stage yet, so today the values are held and
//! a script can set them, but they do not change the picture. Nothing
//! pretends otherwise: see D160 and Q21 for what is unverified (the
//! ranges below are Strobe's choice, not Synesthesia's).

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
