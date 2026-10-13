// ============================================================================
// Module:       strobe_autocontrol::score
// Description:  Measures what each candidate does to the picture: renders a
//               few fixed frames headless with each value nudged and scores
//               the image difference.
//
// Dependencies: strobe-gfx (player, headless context), strobe-scene (controls, audio)
// ============================================================================

//! Scoring by measured effect.
//!
//! A pattern that *looks* like a control is not necessarily one: the `0.3`
//! in `TIME * 0.3` may sit in a branch that is never taken, or scale a
//! term that is clamped away. The only honest test is to change it and look.
//!
//! ## How
//!
//! Every candidate has already been rewritten into a uniform (see
//! [`crate::rewrite`]), so the scene is **compiled once** with all of them as
//! controls, and a probe is just a different uniform value: no recompile per
//! candidate. For the baseline and each nudge the [`Rig`]
//!
//! 1. resets history (a resize to another size and back clears every pass
//!    target to black, so feedback starts from the same place every time);
//! 2. builds a fresh `ControlBank`, so speed integrators restart at zero;
//! 3. renders [`ScoreOptions::frames`] frames at fixed times with the
//!    synthetic audio track and reads the last [`ScoreOptions::capture`].
//!
//! Everything in that list is a pure function of the scene and the options,
//! so two runs on one machine give identical numbers.
//!
//! ## The score
//!
//! The mean absolute difference per colour channel between the baseline and
//! the nudged frames, in `0..1` (0.01 is one percent of full scale, averaged
//! over every pixel). A candidate is nudged down and up ([`nudges`]) and its
//! score is the larger of the two. A nudge that turns a lit picture entirely
//! black marks the candidate [`Broken`]: a slider that can black out the
//! output is a hazard, not a control.

use crate::candidate::{Candidate, ValueType};
use strobe_gfx::gl::Gl;
use strobe_gfx::{AudioTextures, Caps, FrameInputs, Mouse, Options, PlayerError, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::{ControlBank, ControlDef};
use strobe_scene::synthetic;

/// What the renderer measures with.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct ScoreOptions {
    /// Render size. Small on purpose: this runs at import time, not per frame.
    pub size: [i32; 2],
    /// Frames rendered per probe, counting the ones before capture.
    pub frames: u32,
    /// How many of the last frames are read back and compared.
    pub capture: u32,
    /// Seconds of scene time between frames.
    pub dt: f32,
    /// The least mean difference (0..1) that counts as visible.
    pub threshold: f32,
    /// The fraction of the strongest candidate's score under which another
    /// is weak (D259), if it is also under [`WEAK_CEILING`]: next to a
    /// knob that repaints the screen, one that moves a pixel is noise on
    /// the panel.
    pub relative: f32,
}

impl Default for ScoreOptions {
    fn default() -> Self {
        Self {
            size: [160, 90],
            frames: 10,
            capture: 2,
            dt: 0.2,
            threshold: 0.003,
            relative: 0.05,
        }
    }
}

/// Why scoring could not run.
#[derive(Debug, thiserror::Error)]
pub enum ScoreError {
    /// The scene (or its rewrite) did not compile.
    #[error("{0}")]
    Player(#[from] PlayerError),
    /// The scene's controls could not form a bank.
    #[error("{0}")]
    Scene(#[from] strobe_scene::SceneError),
    /// The driver raised an error flag while rendering.
    #[error("GL error 0x{0:x}")]
    Gl(u32),
}

/// A candidate at or above this score (one percent of full scale) is never
/// weak, however strong the scene's best is: T26's tiny-effect controls
/// were all under it (D259).
pub const WEAK_CEILING: f32 = 0.01;

/// The score under which a candidate is weak next to the scene's `best`.
pub fn weak_floor(best: f32, options: &ScoreOptions) -> f32 {
    (best * options.relative).min(WEAK_CEILING)
}

/// The date every probe renders at (`strobe_Date`: year, month 0..11, day,
/// seconds since midnight), so a scene that shows the time is deterministic.
const FIXED_DATE: [f32; 4] = [2026.0, 0.0, 1.0, 43_200.0];

/// A rendered probe: the captured frames, RGBA8, top row first.
pub type Frames = Vec<Vec<u8>>;

/// A scene compiled once, ready to render probes.
pub struct Rig {
    gl: Gl,
    player: ScenePlayer,
    controls: Vec<ControlDef>,
    transitions: Vec<f32>,
    options: ScoreOptions,
}

impl Rig {
    /// Compiles `folder` and gets ready to render it at `options.size`.
    pub fn new(
        gl: &Gl,
        caps: &Caps,
        folder: &SceneFolder,
        options: ScoreOptions,
    ) -> Result<Self, ScoreError> {
        // Context creation can leave a stale error flag that is not ours.
        gl.clear_errors();
        // A script's uniforms are declared (at zero) so a scripted scene
        // compiles without running its script (D259).
        let extras = crate::script::uniforms(folder);
        let mut player =
            ScenePlayer::with_extras(gl, caps, folder, options.size, Options::default(), &extras)?;
        // The wall clock would make a scene that reads the date render
        // differently from one probe to the next.
        player.set_date(FIXED_DATE);
        Ok(Self {
            gl: gl.clone(),
            player,
            controls: folder.def.controls.clone(),
            transitions: folder
                .def
                .transition_uniforms
                .iter()
                .map(|t| t.initial)
                .collect(),
            options,
        })
    }

    /// Renders one probe: every control at its default except `overrides`
    /// (control index, raw component values).
    pub fn render(&mut self, overrides: &[(usize, Vec<f32>)]) -> Result<Frames, ScoreError> {
        let [width, height] = self.options.size;
        // Another size and back: every pass target is reallocated and
        // cleared, so history from the previous probe cannot leak in.
        self.player.resize([width + 1, height])?;
        self.player.resize([width, height])?;

        let mut bank = ControlBank::new(self.controls.clone())?;
        for (index, values) in overrides {
            bank.set(*index, None, values);
        }
        let mut audio = vec![0.0; SCALARS.len()];
        let mut spectrum = vec![[0.0_f32; 4]; 256];
        let mut trail = vec![[0.0_f32; 4]; 256];
        let mut captured = Vec::new();
        let frames = self.options.frames.max(1);
        let first_capture = frames.saturating_sub(self.options.capture.max(1));
        for frame in 0..frames {
            let time = frame as f32 * self.options.dt;
            synthetic::fill(time, &mut audio);
            synthetic::fill_textures(time, &mut spectrum, &mut trail, 4.0);
            self.player.upload_audio(&AudioTextures {
                spectrum: &spectrum,
                level_trail: &trail,
                // Resampled from the synthetic spectrum: deterministic, and
                // what an imported shader's audio channel reads without a
                // live capture (D112).
                shadertoy: None,
            });
            self.player.render(&FrameInputs {
                time,
                frame: u64::from(frame),
                mouse: Mouse::default(),
                audio: &audio,
                controls: &bank,
                transitions: &self.transitions,
            });
            if frame >= first_capture {
                captured.push(self.player.read_output());
            }
            bank.tick(self.options.dt);
        }
        match self.gl.error() {
            Some(code) => Err(ScoreError::Gl(code)),
            None => Ok(captured),
        }
    }
}

/// Mean absolute difference per colour channel between two sets of frames,
/// in `0..1`. Alpha is ignored: the output is composited opaque.
pub fn difference(a: &Frames, b: &Frames) -> f32 {
    let mut sum = 0_u64;
    let mut count = 0_u64;
    for (left, right) in a.iter().zip(b.iter()) {
        for (px_a, px_b) in left.as_chunks::<4>().0.iter().zip(right.as_chunks::<4>().0) {
            for channel in 0..3 {
                sum += u64::from(px_a[channel].abs_diff(px_b[channel]));
                count += 1;
            }
        }
    }
    if count == 0 {
        0.0
    } else {
        sum as f32 / (count as f32 * 255.0)
    }
}

/// Whether every captured pixel is black.
pub fn is_black(frames: &Frames) -> bool {
    frames.iter().all(|frame| {
        frame
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| px[..3].iter().all(|c| *c == 0))
    })
}

/// The values to try for a candidate: one lower and one higher than its
/// default, each a different value from it and inside its range.
///
/// Floats go to half and one and a half times the default; integers to
/// those rounded; a colour to half brightness and halfway to white.
pub fn nudges(candidate: &Candidate) -> Vec<Vec<f32>> {
    let (lo, hi) = {
        let a = candidate.min.first().copied().unwrap_or(0.0);
        let b = candidate.max.first().copied().unwrap_or(1.0);
        (a.min(b), a.max(b))
    };
    let default = candidate.default.clone();
    let mut out: Vec<Vec<f32>> = Vec::new();
    match candidate.value_type {
        ValueType::Float => {
            let x = default.first().copied().unwrap_or(0.0);
            out.push(vec![(x * 0.5).clamp(lo, hi)]);
            out.push(vec![(x * 1.5).clamp(lo, hi)]);
        }
        ValueType::Int | ValueType::Count => {
            let x = default.first().copied().unwrap_or(0.0);
            out.push(vec![(x * 0.5).round().clamp(lo, hi)]);
            out.push(vec![(x * 1.5).round().clamp(lo, hi)]);
        }
        ValueType::Colour => {
            out.push(default.iter().map(|c| c * 0.5).collect());
            out.push(default.iter().map(|c| c + (1.0 - c) * 0.5).collect());
        }
    }
    out.retain(|values| *values != default);
    out
}

/// What was measured for one candidate.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Measure {
    /// The larger of the two nudge differences, `0..1`.
    pub score: f32,
    /// The difference when nudged down.
    pub low: f32,
    /// The difference when nudged up.
    pub high: f32,
    /// Set when a nudge blacked out a lit picture.
    pub broken: Option<Broken>,
}

/// How a nudge ruined the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Broken {
    /// The nudged render was entirely black while the baseline was lit (what
    /// a NaN or an infinity looks like once it reaches an 8-bit output).
    Black,
}

/// Measures every probe: `(candidate, index of its control in the folder)`.
///
/// Returns one [`Measure`] per probe, in order, and the baseline frames the
/// differences were taken against.
pub fn measure(
    rig: &mut Rig,
    probes: &[(&Candidate, usize)],
) -> Result<(Vec<Measure>, Frames), ScoreError> {
    let baseline = rig.render(&[])?;
    let lit = !is_black(&baseline);
    let mut out = Vec::with_capacity(probes.len());
    for (candidate, control) in probes {
        let mut scores = Vec::new();
        let mut broken = None;
        for values in nudges(candidate) {
            let frames = rig.render(&[(*control, values)])?;
            if lit && is_black(&frames) {
                broken = Some(Broken::Black);
            }
            scores.push(difference(&baseline, &frames));
        }
        let low = scores.first().copied().unwrap_or(0.0);
        let high = scores.get(1).copied().unwrap_or(0.0);
        out.push(Measure {
            score: low.max(high),
            low,
            high,
            broken,
        });
    }
    Ok((out, baseline))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{Group, Kind, ValueType};

    fn candidate(value_type: ValueType, default: &[f32], min: f32, max: f32) -> Candidate {
        Candidate {
            id: 0,
            kind: Kind::Scale,
            group: Group::Shape,
            value_type,
            default: default.to_vec(),
            min: vec![min],
            max: vec![max],
            name: "x".to_string(),
            role: None,
            context: String::new(),
            sites: Vec::new(),
        }
    }

    #[test]
    fn floats_are_nudged_to_half_and_one_and_a_half() {
        let got = nudges(&candidate(ValueType::Float, &[4.0], 0.0, 8.0));
        assert_eq!(got, vec![vec![2.0], vec![6.0]], "half and one and a half");
    }

    #[test]
    fn a_nudge_never_leaves_the_range_or_repeats_the_default() {
        let got = nudges(&candidate(ValueType::Int, &[4.0], 1.0, 8.0));
        assert_eq!(got, vec![vec![2.0], vec![6.0]], "rounded integers");
        let narrow = nudges(&candidate(ValueType::Int, &[1.0], 1.0, 2.0));
        assert_eq!(
            narrow,
            vec![vec![2.0]],
            "half of 1 is the default again, so it is dropped"
        );
    }

    #[test]
    fn a_colour_goes_darker_and_lighter() {
        let got = nudges(&candidate(ValueType::Colour, &[0.2, 0.4, 0.8], 0.0, 1.0));
        let expected = [[0.1, 0.2, 0.4], [0.6, 0.7, 0.9]];
        for (probe, wanted) in got.iter().zip(expected) {
            for (value, want) in probe.iter().zip(wanted) {
                assert!(
                    (value - want).abs() < 1e-6,
                    "half brightness, then halfway to white: {got:?}"
                );
            }
        }
        assert_eq!(got.len(), 2, "two probes");
    }

    #[test]
    fn difference_is_a_mean_per_channel_and_ignores_alpha() {
        let a: Frames = vec![vec![0, 0, 0, 255, 255, 255, 255, 255]];
        let b: Frames = vec![vec![255, 0, 0, 0, 255, 255, 255, 0]];
        // One pixel differs by 255 in one of three channels; the other is
        // identical in rgb. 255 / (6 * 255).
        let got = difference(&a, &b);
        assert!((got - 1.0 / 6.0).abs() < 1e-6, "got {got}");
        assert_eq!(difference(&a, &a), 0.0, "identical frames");
    }

    #[test]
    fn black_means_every_colour_channel_is_zero() {
        assert!(is_black(&vec![vec![0, 0, 0, 255]]), "opaque black");
        assert!(!is_black(&vec![vec![0, 1, 0, 255]]), "a lit pixel");
    }
}
