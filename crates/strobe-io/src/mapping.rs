// ============================================================================
// Module:       strobe_io::mapping
// Description:  A mapping: one source, one target, and how the one becomes
//               the other (range, curve, inversion, button mode, takeover).
//
// Dependencies: crate::{event, target, curve, takeover}; serde
// ============================================================================

//! The mapping model (UI_DESIGN section 15).
//!
//! A [`Mapping`] says: *when this source speaks, drive that target like
//! this*. The "like this" is:
//!
//! - **`range`**, the part of the target's range the source may reach, as
//!   fractions `[low, high]` of it. `[0, 1]` is all of it; `[0.2, 0.8]`
//!   keeps a fader out of the extremes; `[1, 0]` is the same as `invert`.
//! - **`curve`** (linear, log, exp) and **`invert`**, applied in that order
//!   to the source's travel before the range: `value = low + curve(travel)
//!   * (high - low)`, with travel reversed first when inverted.
//! - **`button`**: what a button does to a control that is not itself a
//!   button. [`ButtonMode::Toggle`] flips on each press (a pad switching a
//!   toggle on and off, or a slider between its ends); [`ButtonMode::Momentary`]
//!   follows the button, on while it is held.
//! - **`takeover`**: [`Takeover::Pickup`] for a fader, so it does not make
//!   the control jump (see [`crate::takeover`]).
//! - **`threshold`**: where an analog source counts as "pressed" when it
//!   drives a button-like target (a command, a bang, a toggle). It releases
//!   [`HYSTERESIS`] below, so a shaky fader at the line does not machine-gun.
//!
//! A mapping is plain data in `show-control.json`. Its runtime state (the
//! pickup, whether it is held down) is `#[serde(skip)]` and is not part of
//! equality.

use crate::curve::Curve;
use crate::event::{Signal, Source};
use crate::takeover::{Pickup, Takeover};
use crate::target::Target;
use serde::{Deserialize, Serialize};

/// How far below the threshold an analog source must fall to release.
pub const HYSTERESIS: f32 = 0.05;

/// What a button does to a control that is not itself a button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonMode {
    /// Each press flips the control.
    #[default]
    Toggle,
    /// The control follows the button: on while held.
    Momentary,
}

impl ButtonMode {
    /// The mode's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Toggle => "Toggle",
            Self::Momentary => "Momentary",
        }
    }
}

/// What a mapping remembers between messages.
#[derive(Clone, Copy, Debug, Default)]
pub struct Runtime {
    /// The source is currently down.
    pub down: bool,
    /// A toggled button's latched state, for a mapping to a slider.
    pub latched: bool,
    /// Soft takeover.
    pub pickup: Pickup,
}

/// What a signal did to a button-like state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    /// The source is down after this message.
    pub down: bool,
    /// It was up and is now down.
    pub pressed: bool,
    /// It was down and is now up.
    pub released: bool,
}

impl Runtime {
    /// Folds a signal into the held state and reports the change.
    ///
    /// A button press while the source is already down is a press again
    /// (D371): a program change and an OSC trigger (no argument, `N`, `I`,
    /// a lone `T`) have no release at all, and a note off can be lost (a
    /// full queue, a pulled cable). Waiting for a release that never comes
    /// would make every later press do nothing.
    pub fn edge(&mut self, signal: Signal, threshold: f32) -> Edge {
        let was = self.down;
        let (now, again) = match signal {
            Signal::Button(down) => (down, down && was),
            Signal::Analog(x) => {
                let now = if was {
                    x >= threshold - HYSTERESIS
                } else {
                    x >= threshold
                };
                (now, false)
            }
        };
        self.down = now;
        Edge {
            down: now,
            pressed: (now && !was) || again,
            released: was && !now,
        }
    }
}

/// One source driving one target.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mapping {
    /// An optional label the operator gave it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// What speaks.
    pub source: Source,
    /// What is driven.
    pub target: Target,
    /// The reachable part of the target's range, `[low, high]` as fractions.
    #[serde(default = "full_range")]
    pub range: [f32; 2],
    /// The response curve.
    #[serde(default)]
    pub curve: Curve,
    /// Reverse the travel.
    #[serde(default)]
    pub invert: bool,
    /// What a button does to a non-button control.
    #[serde(default)]
    pub button: ButtonMode,
    /// What a fader does about being somewhere else.
    #[serde(default)]
    pub takeover: Takeover,
    /// Where an analog source counts as pressed.
    #[serde(default = "half")]
    pub threshold: f32,
    /// Switched on.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Runtime state; not saved.
    #[serde(skip)]
    pub runtime: Runtime,
}

fn full_range() -> [f32; 2] {
    [0.0, 1.0]
}

fn half() -> f32 {
    0.5
}

fn yes() -> bool {
    true
}

impl PartialEq for Mapping {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.source == other.source
            && self.target == other.target
            && self.range == other.range
            && self.curve == other.curve
            && self.invert == other.invert
            && self.button == other.button
            && self.takeover == other.takeover
            && self.threshold == other.threshold
            && self.enabled == other.enabled
    }
}

impl Mapping {
    /// A mapping with every option at its default.
    pub fn new(source: Source, target: Target) -> Self {
        Self {
            name: String::new(),
            source,
            target,
            range: full_range(),
            curve: Curve::Linear,
            invert: false,
            button: ButtonMode::Toggle,
            takeover: Takeover::Pickup,
            threshold: half(),
            enabled: true,
            runtime: Runtime::default(),
        }
    }

    /// The shaping options, by value, so they can be used while the runtime
    /// state is borrowed mutably.
    pub fn cfg(&self) -> Cfg {
        Cfg {
            range: self.range,
            curve: self.curve,
            invert: self.invert,
            button: self.button,
            takeover: self.takeover,
            threshold: self.threshold,
            stateless: false,
        }
    }

    /// Where `travel` (`0..=1`, the source's position) puts the target, as a
    /// fraction `0..=1` of the target's range. `outer` narrows the range
    /// further (a macro's), as `[low, high]` fractions.
    pub fn shape(&self, travel: f32, outer: Option<[f32; 2]>) -> f32 {
        self.cfg().shape(travel, outer)
    }

    /// A one-line summary for a row: `CC 21 ch 1 -> speed`.
    pub fn summary(&self) -> String {
        format!("{} -> {}", self.source.label(), self.target.label())
    }
}

/// A mapping's shaping options, copied out of it.
#[derive(Clone, Copy, Debug)]
pub struct Cfg {
    /// See [`Mapping::range`].
    pub range: [f32; 2],
    /// See [`Mapping::curve`].
    pub curve: Curve,
    /// See [`Mapping::invert`].
    pub invert: bool,
    /// See [`Mapping::button`].
    pub button: ButtonMode,
    /// See [`Mapping::takeover`].
    pub takeover: Takeover,
    /// See [`Mapping::threshold`].
    pub threshold: f32,
    /// No memory between messages: a toggle follows the level of every
    /// message instead of waiting for an edge. The built-in OSC addresses
    /// work this way, since they have no mapping to remember anything.
    pub stateless: bool,
}

impl Default for Cfg {
    fn default() -> Self {
        Mapping::new(
            Source::Osc {
                address: String::new(),
            },
            Target::command(""),
        )
        .cfg()
    }
}

impl Cfg {
    /// See [`Mapping::shape`].
    pub fn shape(&self, travel: f32, outer: Option<[f32; 2]>) -> f32 {
        let travel = if travel.is_nan() {
            0.0
        } else {
            travel.clamp(0.0, 1.0)
        };
        let travel = if self.invert { 1.0 - travel } else { travel };
        let y = self.curve.apply(travel);
        let [mut low, mut high] = self.range.map(clamp01);
        if let Some([outer_low, outer_high]) = outer.map(|r| r.map(clamp01)) {
            let span = outer_high - outer_low;
            low = outer_low + low * span;
            high = outer_low + high * span;
        }
        clamp01(low + y * (high - low))
    }

    /// The travel that [`Self::shape`] maps to `value`, clamped to the
    /// reachable range: what a fader must show for a control at `value`
    /// (MIDI feedback, D257). `None` when the range is empty, so every
    /// travel gives the same value.
    pub fn unshape(&self, value: f32, outer: Option<[f32; 2]>) -> Option<f32> {
        let [mut low, mut high] = self.range.map(clamp01);
        if let Some([outer_low, outer_high]) = outer.map(|r| r.map(clamp01)) {
            let span = outer_high - outer_low;
            low = outer_low + low * span;
            high = outer_low + high * span;
        }
        if (high - low).abs() < f32::EPSILON {
            return None;
        }
        let y = clamp01((clamp01(value) - low) / (high - low));
        let travel = self.curve.inverse(y);
        Some(if self.invert { 1.0 - travel } else { travel })
    }
}

fn clamp01(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// The index of an enabled mapping other than `except` whose source
/// overlaps `source`, if there is one.
pub fn find_conflict(
    mappings: &[Mapping],
    source: &Source,
    except: Option<usize>,
) -> Option<usize> {
    mappings
        .iter()
        .enumerate()
        .find(|(i, m)| Some(*i) != except && m.enabled && m.source.overlaps(source))
        .map(|(i, _)| i)
}

/// For each mapping, whether another enabled mapping answers to an
/// overlapping source (so both show the Conflict state).
pub fn conflicting(mappings: &[Mapping]) -> Vec<bool> {
    mappings
        .iter()
        .enumerate()
        .map(|(i, m)| m.enabled && find_conflict(mappings, &m.source, Some(i)).is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(device: Option<&str>, controller: u8) -> Source {
        Source::MidiCc {
            device: device.map(str::to_string),
            channel: 1,
            controller,
        }
    }

    fn mapping(controller: u8) -> Mapping {
        Mapping::new(cc(None, controller), Target::control("speed"))
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn the_default_mapping_is_the_identity() {
        let m = mapping(1);
        for x in [0.0, 0.25, 0.5, 1.0] {
            assert!(near(m.shape(x, None), x), "{x}");
        }
    }

    #[test]
    fn range_keeps_the_fader_inside_part_of_the_control() {
        let mut m = mapping(1);
        m.range = [0.2, 0.8];
        assert!(near(m.shape(0.0, None), 0.2), "bottom");
        assert!(near(m.shape(1.0, None), 0.8), "top");
        assert!(near(m.shape(0.5, None), 0.5), "middle");
    }

    #[test]
    fn a_reversed_range_is_an_inversion() {
        let mut m = mapping(1);
        m.range = [1.0, 0.0];
        assert!(near(m.shape(0.0, None), 1.0), "bottom is full");
        assert!(near(m.shape(1.0, None), 0.0), "top is empty");
        let mut n = mapping(1);
        n.invert = true;
        for x in [0.0, 0.3, 0.7, 1.0] {
            assert!(
                near(m.shape(x, None), n.shape(x, None)),
                "same as invert at {x}"
            );
        }
    }

    #[test]
    fn invert_runs_before_the_curve() {
        let mut m = mapping(1);
        m.invert = true;
        m.curve = Curve::Exp;
        // Travel 0.9 inverted is 0.1; exp(0.1) is small.
        assert!(
            near(m.shape(0.9, None), Curve::Exp.apply(0.1)),
            "invert then curve"
        );
    }

    #[test]
    fn unshape_finds_the_travel_that_shape_turned_into_a_value() {
        for curve in Curve::ALL {
            for invert in [false, true] {
                for range in [[0.0, 1.0], [0.2, 0.7], [0.9, 0.1]] {
                    for outer in [None, Some([0.25, 0.75])] {
                        let mut m = mapping(1);
                        m.curve = curve;
                        m.invert = invert;
                        m.range = range;
                        let cfg = m.cfg();
                        for step in 0..=10 {
                            let travel = step as f32 / 10.0;
                            let value = cfg.shape(travel, outer);
                            let back = cfg.unshape(value, outer).unwrap_or(f32::NAN);
                            assert!(
                                (back - travel).abs() < 1e-3,
                                "{curve:?} invert {invert} {range:?} {outer:?}: {travel} -> {value} -> {back}"
                            );
                        }
                    }
                }
            }
        }
        let mut flat = mapping(1);
        flat.range = [0.4, 0.4];
        assert_eq!(
            flat.cfg().unshape(0.4, None),
            None,
            "an empty range has no travel"
        );
        let m = mapping(1);
        assert_eq!(
            m.cfg().unshape(1.5, None),
            Some(1.0),
            "a value beyond the range shows the end of the travel"
        );
    }

    #[test]
    fn a_macros_range_narrows_the_mappings_range() {
        let mut m = mapping(1);
        m.range = [0.5, 1.0];
        let outer = Some([0.0, 0.5]);
        assert!(
            near(m.shape(0.0, outer), 0.25),
            "bottom: half of the lower half"
        );
        assert!(near(m.shape(1.0, outer), 0.5), "top: the macro's top");
    }

    #[test]
    fn nonsense_never_leaves_zero_to_one() {
        let mut m = mapping(1);
        m.range = [-4.0, 9.0];
        assert!(near(m.shape(f32::NAN, None), 0.0), "NaN travel");
        for x in [-1.0, 0.5, 2.0] {
            let y = m.shape(x, None);
            assert!((0.0..=1.0).contains(&y), "{x} -> {y}");
        }
        m.range = [f32::NAN, f32::NAN];
        assert!((0.0..=1.0).contains(&m.shape(0.5, None)), "NaN range");
    }

    #[test]
    fn an_analog_source_presses_at_the_threshold_and_releases_below_it() {
        let mut rt = Runtime::default();
        let step = |rt: &mut Runtime, x: f32| rt.edge(Signal::Analog(x), 0.5);
        assert!(!step(&mut rt, 0.4).pressed, "below");
        let up = step(&mut rt, 0.5);
        assert!(up.pressed && up.down, "at the line");
        assert!(
            !step(&mut rt, 0.52).pressed,
            "still held is not a new press"
        );
        let wobble = step(&mut rt, 0.47);
        assert!(
            wobble.down && !wobble.released,
            "inside the hysteresis band it stays down"
        );
        let off = step(&mut rt, 0.40);
        assert!(off.released && !off.down, "released below the band");
        assert!(step(&mut rt, 0.6).pressed, "and can press again");
    }

    #[test]
    fn a_button_source_reports_its_own_edges() {
        let mut rt = Runtime::default();
        assert!(rt.edge(Signal::Button(true), 0.5).pressed, "down");
        let again = rt.edge(Signal::Button(true), 0.5);
        assert!(
            again.pressed && again.down && !again.released,
            "a press with no release between is a press again (D371)"
        );
        assert!(rt.edge(Signal::Button(false), 0.5).released, "up");
        assert!(
            !rt.edge(Signal::Button(false), 0.5).released,
            "a second release is nothing"
        );
    }

    #[test]
    fn sources_conflict_when_one_message_would_fire_both() {
        let list = vec![
            mapping(1),
            Mapping::new(cc(Some("pad"), 1), Target::control("zoom")),
            mapping(2),
        ];
        assert_eq!(
            find_conflict(&list, &cc(Some("pad"), 1), None),
            Some(0),
            "the any-device mapping answers to pad's CC 1"
        );
        assert_eq!(
            find_conflict(&list, &cc(Some("pad"), 1), Some(0)),
            Some(1),
            "except skips itself"
        );
        assert_eq!(
            find_conflict(&list, &cc(None, 3), None),
            None,
            "CC 3 is free"
        );
        assert_eq!(
            conflicting(&list),
            vec![true, true, false],
            "the two that share CC 1"
        );
    }

    #[test]
    fn a_disabled_mapping_conflicts_with_nothing() {
        let mut list = vec![mapping(1), mapping(1)];
        list[1].enabled = false;
        assert_eq!(conflicting(&list), vec![false, false], "one is off");
        assert_eq!(
            find_conflict(&list, &cc(None, 1), None),
            Some(0),
            "only the live one"
        );
    }

    #[test]
    fn a_mapping_round_trips_and_a_minimal_file_gets_defaults() -> Result<(), serde_json::Error> {
        let mut m = mapping(21);
        m.range = [0.1, 0.9];
        m.curve = Curve::Log;
        m.invert = true;
        m.button = ButtonMode::Momentary;
        m.takeover = Takeover::Jump;
        m.name = "Master fader".into();
        let text = serde_json::to_string(&m)?;
        assert_eq!(serde_json::from_str::<Mapping>(&text)?, m, "round trip");
        let minimal = r#"{ "source": { "kind": "osc", "address": "/x" },
                           "target": { "kind": "meta", "name": "hue" } }"#;
        let parsed: Mapping = serde_json::from_str(minimal)?;
        assert_eq!(parsed.range, [0.0, 1.0], "full range");
        assert_eq!(parsed.curve, Curve::Linear, "linear");
        assert!(parsed.enabled && !parsed.invert, "on, not inverted");
        assert!(near(parsed.threshold, 0.5), "threshold");
        Ok(())
    }
}
