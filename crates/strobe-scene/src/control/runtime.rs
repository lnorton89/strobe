// ============================================================================
// Module:       strobe_scene::control::runtime
// Description:  Per-frame control state: targets set by the UI, MIDI or a
//               script, and the outputs smoothing, speed and bangs produce.
//
// Dependencies: crate::control::{def, kind}, crate::names
// ============================================================================

//! The control runtime.
//!
//! Every control has a *target* — what the UI, a MIDI knob or a script
//! last asked for — and an *output*, which is what the shader sees. For a
//! plain slider they are the same number. For everything else the output
//! is a function of the target and time, advanced once per frame by
//! [`ControlBank::tick`]:
//!
//! - **smooth** eases the output toward the target. Synesthesia documents
//!   this as "roughly `1 / PARAMS` frames to transition", which is a
//!   per-frame exponential step of `PARAMS`. Stepping per *frame* makes the
//!   feel depend on frame rate — a scene tuned at 60 fps would be twice as
//!   sluggish on a laptop holding 30 — so the step here is per sixtieth of
//!   a second: `alpha = 1 - (1 - PARAMS)^(dt * 60)`. At 60 fps that is
//!   exactly `PARAMS`; at any other rate it covers the same ground in the
//!   same wall-clock time.
//! - **speed** integrates: the target is a rate per second.
//! - **bang** outputs `MAX` for exactly one frame per press, then `MIN`;
//!   presses between frames are counted, never lost. `bang smooth` runs an
//!   attack/decay envelope; `bang counter` adds one per press.
//!
//! Nothing here allocates per frame; [`ControlBank::tick`] walks two
//! fixed-size vectors.

use crate::control::def::{ControlDef, MAX_DIMENSIONS, Values};
use crate::control::kind::{Bang, ControlKind, Mode};
use crate::error::SceneError;
use std::collections::HashMap;

/// The live state of one control.
#[derive(Debug, Clone, PartialEq)]
struct State {
    /// What was last asked for, in raw units (dropdown: the index).
    target: Values,
    /// What the shader sees.
    output: Values,
    /// Bang presses not yet consumed by a tick.
    presses: u32,
    /// Bang counter total.
    count: f32,
    /// `bang smooth`: still rising toward the peak.
    attacking: bool,
    /// `traveler`: the rate, easing toward the target.
    rate: Values,
    /// A glide a macro asked for (D227): the output eases to the target
    /// with this time constant, in seconds, whatever the control's own
    /// smoothing, until another write or arrival ends it.
    glide: Option<f32>,
}

/// A control's value as the shader sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UniformValue {
    /// `float`.
    Float(f32),
    /// `vec2`.
    Vec2([f32; 2]),
    /// `vec3`.
    Vec3([f32; 3]),
}

/// Every control of one scene instance, by index, by exact name and by
/// normalised name.
#[derive(Debug, Clone)]
pub struct ControlBank {
    defs: Vec<ControlDef>,
    states: Vec<State>,
    by_name: HashMap<String, usize>,
    by_exact: HashMap<String, usize>,
}

impl ControlBank {
    /// Builds the bank from validated definitions, each at its default.
    ///
    /// Fails only on two controls with exactly the same name. Two names
    /// that normalise to the same key (`in_or_out` and `inOrOut`) are legal
    /// GLSL and a real Synesthesia scene has them (D196): [`Self::find`]
    /// prefers the exact name, and the normalised key reaches the first.
    pub fn new(defs: Vec<ControlDef>) -> Result<Self, SceneError> {
        let mut by_name = HashMap::with_capacity(defs.len());
        let mut by_exact = HashMap::with_capacity(defs.len());
        for (index, def) in defs.iter().enumerate() {
            if by_exact.insert(def.name.clone(), index).is_some() {
                return Err(SceneError::NameCollision {
                    first: def.name.clone(),
                    second: def.name.clone(),
                });
            }
            by_name
                .entry(crate::names::normalize(&def.name))
                .or_insert(index);
        }
        let states = defs.iter().map(initial_state).collect();
        Ok(Self {
            defs,
            states,
            by_name,
            by_exact,
        })
    }

    /// The definitions, in declaration order.
    pub fn defs(&self) -> &[ControlDef] {
        &self.defs
    }

    /// The index of the control `name` matches, by the JavaScript API's rule.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_exact
            .get(name)
            .or_else(|| self.by_name.get(&crate::names::normalize(name)))
            .copied()
    }

    /// The target of control `index` (what the UI should show).
    pub fn target(&self, index: usize) -> Option<&Values> {
        self.states.get(index).map(|state| &state.target)
    }

    /// The output of control `index` (what the shader sees).
    pub fn output(&self, index: usize) -> Option<&Values> {
        self.states.get(index).map(|state| &state.output)
    }

    /// Sets control `index`'s target in raw units, clamped into range.
    ///
    /// `dimension` limits the write to one component; otherwise `values`
    /// fills components from the first. For a toggle, a value nearer `MAX`
    /// is on; for a bang, any write is a press; for a dropdown the value is
    /// an option index.
    pub fn set(&mut self, index: usize, dimension: Option<usize>, values: &[f32]) {
        let (Some(def), Some(state)) = (self.defs.get(index), self.states.get_mut(index)) else {
            return;
        };
        state.glide = None;
        if let ControlKind::Bang(_) = def.kind {
            state.presses = state.presses.saturating_add(1);
            return;
        }
        let dims = def.dims();
        let slots: Vec<usize> = match dimension {
            Some(d) if d < dims => vec![d],
            Some(_) => return,
            None => (0..dims.min(values.len())).collect(),
        };
        for (slot, value) in slots.into_iter().zip(values.iter()) {
            let value = match def.kind {
                ControlKind::Toggle { .. } => {
                    if def.normalize(slot, *value) >= 0.5 {
                        def.max[slot]
                    } else {
                        def.min[slot]
                    }
                }
                ControlKind::Dropdown { .. } => value.round().clamp(0.0, def.max[slot]),
                ControlKind::Slider(_)
                | ControlKind::Knob(_)
                | ControlKind::Xy(_)
                | ControlKind::Color { .. }
                | ControlKind::Bang(_) => def.clamp(slot, *value),
            };
            if let Some(target) = state.target.get_mut(slot) {
                *target = value;
            }
        }
    }

    /// As [`Self::set`], with values in `0..1` across each component's range.
    pub fn set_normalized(&mut self, index: usize, dimension: Option<usize>, values: &[f32]) {
        let Some(def) = self.defs.get(index) else {
            return;
        };
        let start = dimension.unwrap_or(0);
        let raw: Vec<f32> = values
            .iter()
            .enumerate()
            .map(|(offset, t)| {
                let slot = start + offset;
                match def.kind {
                    ControlKind::Dropdown { .. } => (t.clamp(0.0, 1.0) * def.max[0]).round(),
                    ControlKind::Slider(_)
                    | ControlKind::Knob(_)
                    | ControlKind::Toggle { .. }
                    | ControlKind::Bang(_)
                    | ControlKind::Xy(_)
                    | ControlKind::Color { .. } => def.denormalize(slot, t.clamp(0.0, 1.0)),
                }
            })
            .collect();
        self.set(index, dimension, &raw);
    }

    /// As [`Self::set_normalized`], with the output gliding to the new
    /// target over about `seconds` (a time constant), even on a control
    /// that would snap (D227). The glide lasts until the output arrives or
    /// anything else writes the control. A speed or traveler control, a bang
    /// and a dropdown take the write as usual.
    pub fn set_normalized_gliding(
        &mut self,
        index: usize,
        dimension: Option<usize>,
        values: &[f32],
        seconds: f32,
    ) {
        self.set_normalized(index, dimension, values);
        let (Some(def), Some(state)) = (self.defs.get(index), self.states.get_mut(index)) else {
            return;
        };
        let glides = matches!(
            def.kind,
            ControlKind::Slider(Mode::Plain | Mode::Smooth)
                | ControlKind::Knob(Mode::Plain | Mode::Smooth)
                | ControlKind::Xy(Mode::Plain | Mode::Smooth)
                | ControlKind::Color { .. }
        );
        if glides && seconds.is_finite() && seconds > 0.0 {
            state.glide = Some(seconds);
        }
    }

    /// Presses a bang, or flips a toggle.
    pub fn press(&mut self, index: usize) {
        let Some(def) = self.defs.get(index) else {
            return;
        };
        match def.kind {
            ControlKind::Toggle { .. } => {
                let on = self
                    .states
                    .get(index)
                    .is_some_and(|state| def.normalize(0, state.target[0]) >= 0.5);
                let next = if on { def.min[0] } else { def.max[0] };
                self.set(index, Some(0), &[next]);
            }
            ControlKind::Bang(_) => self.set(index, None, &[]),
            ControlKind::Slider(_)
            | ControlKind::Knob(_)
            | ControlKind::Xy(_)
            | ControlKind::Color { .. }
            | ControlKind::Dropdown { .. } => {}
        }
    }

    /// Points control `index`'s target back at its default.
    ///
    /// Unlike [`Self::reset`] this does not touch the output, so a smoothed
    /// control eases home instead of jumping, and a `speed` control keeps
    /// the position it has integrated to. A bang has no default to return
    /// to and is left alone.
    pub fn set_default(&mut self, index: usize) {
        let Some(def) = self.defs.get(index) else {
            return;
        };
        if matches!(def.kind, ControlKind::Bang(_)) {
            return;
        }
        let default = def.default;
        self.set(index, None, &default[..def.dims()]);
    }

    /// Sets control `index`'s target to a random point in its range.
    ///
    /// `unit` supplies the randomness, one call per component, each in
    /// `0..1`; the bank owns no generator so that a seeded script host
    /// stays reproducible. A bang is left alone: a random press is not a
    /// value.
    pub fn randomize(&mut self, index: usize, mut unit: impl FnMut() -> f32) {
        let Some(def) = self.defs.get(index) else {
            return;
        };
        if matches!(def.kind, ControlKind::Bang(_)) {
            return;
        }
        let dims = def.dims();
        let mut positions = [0.0_f32; MAX_DIMENSIONS];
        for position in positions.iter_mut().take(dims) {
            *position = unit();
        }
        self.set_normalized(index, None, &positions[..dims]);
    }

    /// Returns control `index` to its default.
    pub fn reset(&mut self, index: usize) {
        if let (Some(def), Some(state)) = (self.defs.get(index), self.states.get_mut(index)) {
            *state = initial_state(def);
        }
    }

    /// Takes `from`'s state (target and output, so nothing re-smooths) for
    /// every control still at its default target here that `from` has with
    /// an identical definition. A control already moved here keeps its
    /// value. Returns how many were taken.
    ///
    /// This is the meta hand-over (D207): a scene going live keeps the
    /// output-wide settings the operator had, except what its own preset,
    /// cue or script set. Called once per scene change, never per frame.
    pub fn carry_unchanged_from(&mut self, from: &ControlBank) -> usize {
        let mut taken = 0;
        for (def, state) in self.defs.iter().zip(self.states.iter_mut()) {
            if state.target != initial_state(def).target {
                continue;
            }
            let theirs = from
                .by_exact
                .get(&def.name)
                .filter(|&&i| from.defs.get(i) == Some(def))
                .and_then(|&i| from.states.get(i));
            if let Some(theirs) = theirs
                && theirs != state
            {
                *state = theirs.clone();
                taken += 1;
            }
        }
        taken
    }

    /// Advances every output by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        self.tick_at(dt, 1.0);
    }

    /// Advances every output by `dt` seconds, with `speed` controls
    /// integrating at `rate` times their value instead of once (D246). A
    /// Shadertoy import passes its clock's rate, so a speed control that
    /// replaced an `iTime * k` term keeps following `st_speed` and
    /// `st_audio_time` as `iTime` does; everything else passes 1. Smoothing
    /// and glides stay on real time. A rate that is not a finite number
    /// counts as 1, and a negative one as 0.
    pub fn tick_at(&mut self, dt: f32, rate: f32) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let rate = if rate.is_finite() { rate.max(0.0) } else { 1.0 };
        for (def, state) in self.defs.iter().zip(self.states.iter_mut()) {
            step(def, state, dt, rate);
        }
    }

    /// Every control's uniform name and current output.
    pub fn uniforms(&self) -> impl Iterator<Item = (&str, UniformValue)> {
        self.defs
            .iter()
            .zip(self.states.iter())
            .map(|(def, state)| {
                let [a, b, c] = state.output;
                let value = match def.dims() {
                    2 => UniformValue::Vec2([a, b]),
                    3 => UniformValue::Vec3([a, b, c]),
                    _ => UniformValue::Float(a),
                };
                (def.name.as_str(), value)
            })
    }
}

/// A control's state at its default.
fn initial_state(def: &ControlDef) -> State {
    let mut output = def.default;
    match def.kind {
        ControlKind::Dropdown { .. } => output[0] = dropdown_value(def, def.default[0]),
        ControlKind::Bang(Bang::Plain | Bang::Smooth) => output = def.min,
        ControlKind::Bang(Bang::Counter | Bang::CounterSmooth) => output = [0.0; MAX_DIMENSIONS],
        ControlKind::Slider(Mode::Speed | Mode::Traveler)
        | ControlKind::Knob(Mode::Speed | Mode::Traveler)
        | ControlKind::Xy(Mode::Speed | Mode::Traveler) => {
            output = [0.0; MAX_DIMENSIONS];
        }
        ControlKind::Slider(Mode::Plain | Mode::Smooth)
        | ControlKind::Knob(Mode::Plain | Mode::Smooth)
        | ControlKind::Xy(Mode::Plain | Mode::Smooth)
        | ControlKind::Toggle { .. }
        | ControlKind::Color { .. } => {}
    }
    State {
        target: def.default,
        output,
        presses: 0,
        count: 0.0,
        attacking: false,
        rate: def.default,
        glide: None,
    }
}

/// The value a dropdown index selects.
fn dropdown_value(def: &ControlDef, index: f32) -> f32 {
    let i = index.round().max(0.0) as usize;
    def.values.get(i).copied().unwrap_or(0.0)
}

/// The smoothing step for this frame: `PARAMS` per sixtieth of a second.
fn alpha(params: f32, dt: f32) -> f32 {
    1.0 - (1.0 - params.clamp(0.0, 1.0)).powf(dt * 60.0)
}

/// Advances one control by `dt`.
fn step(def: &ControlDef, state: &mut State, dt: f32, rate: f32) {
    let dims = def.dims();
    if let Some(seconds) = state.glide {
        // A macro's glide (D227): exponential, frame-rate independent, and
        // over once the output is within a ten-thousandth of the range.
        let a = 1.0 - (-dt / seconds).exp();
        let mut arrived = true;
        for i in 0..dims {
            state.output[i] += (state.target[i] - state.output[i]) * a;
            let span = (def.max[i] - def.min[i]).abs().max(f32::EPSILON);
            arrived &= (state.target[i] - state.output[i]).abs() <= span * 1e-4;
        }
        if arrived {
            state.output = state.target;
            state.glide = None;
        }
        return;
    }
    let a = alpha(def.params, dt);
    let ease = |output: &mut Values, target: &Values| {
        for i in 0..dims {
            output[i] += (target[i] - output[i]) * a;
        }
    };
    match def.kind {
        ControlKind::Slider(Mode::Plain)
        | ControlKind::Knob(Mode::Plain)
        | ControlKind::Xy(Mode::Plain)
        | ControlKind::Toggle { smooth: false }
        | ControlKind::Color { smooth: false } => state.output = state.target,
        ControlKind::Slider(Mode::Smooth)
        | ControlKind::Knob(Mode::Smooth)
        | ControlKind::Xy(Mode::Smooth)
        | ControlKind::Toggle { smooth: true }
        | ControlKind::Color { smooth: true } => ease(&mut state.output, &state.target),
        ControlKind::Slider(Mode::Speed)
        | ControlKind::Knob(Mode::Speed)
        | ControlKind::Xy(Mode::Speed) => {
            for i in 0..dims {
                state.output[i] += state.target[i] * dt * rate;
            }
        }
        ControlKind::Slider(Mode::Traveler)
        | ControlKind::Knob(Mode::Traveler)
        | ControlKind::Xy(Mode::Traveler) => {
            ease(&mut state.rate, &state.target);
            for i in 0..dims {
                state.output[i] += state.rate[i] * dt;
            }
        }
        ControlKind::Dropdown { smooth } => {
            let value = dropdown_value(def, state.target[0]);
            if smooth {
                state.output[0] += (value - state.output[0]) * a;
            } else {
                state.output[0] = value;
            }
        }
        ControlKind::Bang(Bang::Plain) => {
            state.output[0] = if state.presses > 0 {
                def.max[0]
            } else {
                def.min[0]
            };
            state.presses = 0;
        }
        ControlKind::Bang(Bang::Smooth) => {
            if state.presses > 0 {
                state.attacking = true;
                state.presses = 0;
            }
            // The envelope runs in normalised units and is mapped to the
            // range on the way out, so an inverted range still rises.
            let mut t = def.normalize(0, state.output[0]);
            if state.attacking {
                t += (1.0 - t) * a.max(0.25);
                if t > 0.98 {
                    t = 1.0;
                    state.attacking = false;
                }
            } else {
                t -= t * a;
            }
            state.output[0] = def.denormalize(0, t);
        }
        ControlKind::Bang(Bang::Counter) => {
            state.count += state.presses as f32;
            state.presses = 0;
            state.output[0] = state.count;
        }
        ControlKind::Bang(Bang::CounterSmooth) => {
            state.count += state.presses as f32;
            state.presses = 0;
            state.output[0] += (state.count - state.output[0]) * a;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::RawControl;
    use anyhow::Result;

    fn bank(controls: &[&str]) -> Result<ControlBank> {
        let mut defs = Vec::new();
        for json in controls {
            let raw: RawControl = serde_json::from_str(json)?;
            defs.push(ControlDef::from_raw(&raw)?);
        }
        Ok(ControlBank::new(defs)?)
    }

    fn out(bank: &ControlBank, index: usize) -> f32 {
        bank.output(index).map_or(f32::NAN, |values| values[0])
    }

    #[test]
    fn smoothing_takes_the_same_wall_time_at_any_frame_rate() -> Result<()> {
        let json = r#"{ "NAME": "s", "TYPE": "slider smooth", "PARAMS": 0.1 }"#;
        let mut fast = bank(&[json])?;
        let mut slow = bank(&[json])?;
        fast.set(0, None, &[1.0]);
        slow.set(0, None, &[1.0]);
        for _ in 0..60 {
            fast.tick(1.0 / 60.0);
        }
        for _ in 0..30 {
            slow.tick(1.0 / 30.0);
        }
        assert!(
            (out(&fast, 0) - out(&slow, 0)).abs() < 1e-4,
            "one second at 60 fps ({}) and at 30 fps ({}) should agree",
            out(&fast, 0),
            out(&slow, 0)
        );
        Ok(())
    }

    #[test]
    fn at_sixty_fps_one_frame_steps_by_params() -> Result<()> {
        let mut b = bank(&[r#"{ "NAME": "s", "TYPE": "slider smooth", "PARAMS": 0.1 }"#])?;
        b.set(0, None, &[1.0]);
        b.tick(1.0 / 60.0);
        assert!((out(&b, 0) - 0.1).abs() < 1e-5, "got {}", out(&b, 0));
        Ok(())
    }

    #[test]
    fn speed_integrates_the_target_per_second() -> Result<()> {
        let mut b = bank(&[r#"{ "NAME": "s", "TYPE": "slider speed", "MIN": -2, "MAX": 2 }"#])?;
        b.set(0, None, &[2.0]);
        for _ in 0..30 {
            b.tick(1.0 / 60.0);
        }
        assert!(
            (out(&b, 0) - 1.0).abs() < 1e-4,
            "half a second at 2/s, got {}",
            out(&b, 0)
        );
        Ok(())
    }

    #[test]
    fn a_rate_scales_speed_controls_and_nothing_else() -> Result<()> {
        let mut b = bank(&[
            r#"{ "NAME": "s", "TYPE": "slider speed", "MIN": -2, "MAX": 2 }"#,
            r#"{ "NAME": "m", "TYPE": "slider smooth", "MIN": 0, "MAX": 1 }"#,
        ])?;
        b.set(0, None, &[2.0]);
        b.set(1, None, &[1.0]);
        let mut plain = b.clone();
        for _ in 0..30 {
            b.tick_at(1.0 / 60.0, 1.5);
            plain.tick(1.0 / 60.0);
        }
        assert!(
            (out(&b, 0) - 1.5).abs() < 1e-4,
            "half a second at 2/s and rate 1.5, got {}",
            out(&b, 0)
        );
        assert!(
            (out(&b, 1) - out(&plain, 1)).abs() < 1e-6,
            "smoothing stays on real time"
        );
        let mut odd = b.clone();
        let before = out(&odd, 0);
        odd.tick_at(1.0 / 60.0, -3.0);
        assert!(
            (out(&odd, 0) - before).abs() < 1e-6,
            "a negative rate holds still"
        );
        odd.tick_at(1.0 / 60.0, f32::NAN);
        assert!(
            (out(&odd, 0) - before - 2.0 / 60.0).abs() < 1e-4,
            "a rate that is not a number counts as 1"
        );
        Ok(())
    }

    #[test]
    fn a_traveler_accelerates_toward_its_speed_and_integrates_it() -> Result<()> {
        // D197: `slider traveler` is a speed whose rate eases with PARAMS.
        let mut b = bank(&[
            r#"{ "NAME": "t", "TYPE": "slider traveler", "MIN": -2, "MAX": 2, "DEFAULT": 0, "PARAMS": 0.1 }"#,
        ])?;
        b.set(0, None, &[2.0]);
        b.tick(1.0 / 60.0);
        let first = out(&b, 0);
        assert!(
            first > 0.0 && first < 2.0 / 60.0,
            "the first frame moves, but slower than full speed: {first}"
        );
        for _ in 0..600 {
            b.tick(1.0 / 60.0);
        }
        let before = out(&b, 0);
        b.tick(1.0 / 60.0);
        let step = out(&b, 0) - before;
        assert!(
            (step - 2.0 / 60.0).abs() < 1e-3,
            "after ten seconds it travels at the full 2/s: {step}"
        );
        assert!(
            ControlKind::parse("t", "toggle traveler").is_err(),
            "only sliders, knobs and xy pads travel"
        );
        Ok(())
    }

    #[test]
    fn a_bang_is_max_for_exactly_one_frame() -> Result<()> {
        let mut b = bank(&[r#"{ "NAME": "b", "TYPE": "bang" }"#])?;
        b.press(0);
        b.tick(1.0 / 60.0);
        assert_eq!(out(&b, 0), 1.0, "the frame after the press");
        b.tick(1.0 / 60.0);
        assert_eq!(out(&b, 0), 0.0, "and back the frame after");
        Ok(())
    }

    #[test]
    fn counter_presses_between_frames_are_not_lost() -> Result<()> {
        let mut b = bank(&[r#"{ "NAME": "c", "TYPE": "bang counter" }"#])?;
        b.press(0);
        b.press(0);
        b.press(0);
        b.tick(1.0 / 60.0);
        assert_eq!(out(&b, 0), 3.0, "three presses in one frame count three");
        Ok(())
    }

    #[test]
    fn a_smooth_bang_rises_then_falls() -> Result<()> {
        let mut b = bank(&[r#"{ "NAME": "b", "TYPE": "bang smooth", "PARAMS": 0.2 }"#])?;
        b.press(0);
        let mut peak = 0.0_f32;
        for _ in 0..30 {
            b.tick(1.0 / 60.0);
            peak = peak.max(out(&b, 0));
        }
        assert!(
            peak > 0.95,
            "the envelope should reach its peak, got {peak}"
        );
        assert!(out(&b, 0) < 0.1, "and decay afterwards, got {}", out(&b, 0));
        Ok(())
    }

    #[test]
    fn a_toggle_snaps_to_its_ends_even_when_inverted() -> Result<()> {
        let mut b =
            bank(&[r#"{ "NAME": "t", "TYPE": "toggle", "MIN": 1, "MAX": 0, "DEFAULT": 1 }"#])?;
        b.tick(0.0);
        assert_eq!(out(&b, 0), 1.0, "default is MIN, i.e. off");
        b.press(0);
        b.tick(0.0);
        assert_eq!(out(&b, 0), 0.0, "pressed: MAX");
        Ok(())
    }

    #[test]
    fn a_dropdown_outputs_the_selected_value() -> Result<()> {
        let mut b = bank(&[
            r#"{ "NAME": "d", "TYPE": "dropdown", "LABELS": ["a","b","c"], "VALUES": [0.5, 2, 7] }"#,
        ])?;
        b.tick(0.0);
        assert_eq!(out(&b, 0), 0.5, "index 0 by default");
        b.set_normalized(0, None, &[1.0]);
        b.tick(0.0);
        assert_eq!(out(&b, 0), 7.0, "normalised 1 is the last option");
        Ok(())
    }

    #[test]
    fn names_resolve_by_the_script_api_rule() -> Result<()> {
        let b = bank(&[r#"{ "NAME": "rate_in", "TYPE": "slider" }"#])?;
        assert_eq!(b.find("Rate In"), Some(0), "spacing and case are ignored");
        assert_eq!(b.find("rate_out"), None, "a different name");
        Ok(())
    }

    #[test]
    fn names_equal_once_normalised_load_and_the_exact_name_wins() -> Result<()> {
        // D196: a real scene has `in_or_out` and `inOrOut`.
        let b = bank(&[
            r#"{ "NAME": "rate_in", "TYPE": "slider" }"#,
            r#"{ "NAME": "rateIn", "TYPE": "slider" }"#,
        ])?;
        assert_eq!(b.find("rate_in"), Some(0), "exact first");
        assert_eq!(b.find("rateIn"), Some(1), "exact second");
        assert_eq!(b.find("RATE IN"), Some(0), "normalised reaches the first");
        let duplicate = bank(&[
            r#"{ "NAME": "x", "TYPE": "slider" }"#,
            r#"{ "NAME": "x", "TYPE": "slider" }"#,
        ]);
        assert!(duplicate.is_err(), "an exact duplicate is still an error");
        Ok(())
    }

    #[test]
    fn a_partial_write_touches_one_dimension() -> Result<()> {
        let mut b = bank(&[
            r#"{ "NAME": "p", "TYPE": "xy", "MIN": -1, "MAX": 1, "DEFAULT": [0.5, 0.5] }"#,
        ])?;
        b.set(0, Some(1), &[-1.0]);
        b.tick(0.0);
        assert_eq!(
            b.output(0).map(|v| (v[0], v[1])),
            Some((0.5, -1.0)),
            "only y changed"
        );
        Ok(())
    }

    #[test]
    fn set_default_eases_home_without_jumping() -> Result<()> {
        let mut b =
            bank(&[r#"{ "NAME": "s", "TYPE": "slider smooth", "PARAMS": 0.5, "DEFAULT": 0.25 }"#])?;
        b.set(0, None, &[1.0]);
        b.tick(1.0 / 60.0);
        b.set_default(0);
        assert_eq!(
            b.target(0).map(|v| v[0]),
            Some(0.25),
            "the target is the default again"
        );
        assert!(
            out(&b, 0) > 0.5,
            "but the output has not jumped: {}",
            out(&b, 0)
        );
        Ok(())
    }

    #[test]
    fn set_default_leaves_a_bang_alone() -> Result<()> {
        let mut b = bank(&[r#"{ "NAME": "b", "TYPE": "bang" }"#])?;
        b.set_default(0);
        b.tick(1.0 / 60.0);
        assert_eq!(
            out(&b, 0),
            0.0,
            "returning a bang to default is not a press"
        );
        Ok(())
    }

    #[test]
    fn randomize_draws_one_position_per_component() -> Result<()> {
        let mut b = bank(&[
            r#"{ "NAME": "p", "TYPE": "xy", "MIN": 0, "MAX": 10 }"#,
            r#"{ "NAME": "d", "TYPE": "dropdown", "LABELS": ["a","b","c"], "VALUES": [5, 6, 7] }"#,
        ])?;
        let mut draws = [0.25_f32, 0.75].into_iter();
        b.randomize(0, || draws.next().unwrap_or(0.0));
        b.tick(0.0);
        assert_eq!(
            b.output(0).map(|v| (v[0], v[1])),
            Some((2.5, 7.5)),
            "x then y, mapped across the range"
        );
        b.randomize(1, || 1.0);
        b.tick(0.0);
        assert_eq!(out(&b, 1), 7.0, "a dropdown picks an option, here the last");
        Ok(())
    }

    #[test]
    fn randomize_leaves_a_bang_alone() -> Result<()> {
        let mut b = bank(&[r#"{ "NAME": "b", "TYPE": "bang" }"#])?;
        b.randomize(0, || 1.0);
        b.tick(1.0 / 60.0);
        assert_eq!(out(&b, 0), 0.0, "a random press is not a value");
        Ok(())
    }

    #[test]
    fn a_glide_eases_a_plain_slider_and_any_other_write_ends_it() -> Result<()> {
        let mut b = bank(&[
            r#"{ "NAME": "s", "TYPE": "slider", "MIN": 0.0, "MAX": 1.0, "DEFAULT": 0.0 }"#,
        ])?;
        b.set_normalized_gliding(0, None, &[1.0], 0.5);
        b.tick(0.5);
        let out = b.output(0).map_or(f32::NAN, |v| v[0]);
        assert!(
            (out - (1.0 - (-1.0_f32).exp())).abs() < 1e-4,
            "one time constant in: {out}"
        );
        for _ in 0..600 {
            b.tick(1.0 / 60.0);
        }
        assert_eq!(b.output(0).map(|v| v[0]), Some(1.0), "it arrives");
        b.set_normalized_gliding(0, None, &[0.0], 2.0);
        b.tick(0.1);
        b.set(0, None, &[0.25]);
        b.tick(1.0 / 60.0);
        assert_eq!(
            b.output(0).map(|v| v[0]),
            Some(0.25),
            "a plain write snaps again: the glide is over"
        );
        Ok(())
    }
}
