// ============================================================================
// Module:       strobe_app::session
// Description:  The one active scene: its controls, its compiled player, and
//               the per-frame step that feeds audio and controls into it.
//
// Dependencies: strobe-scene (SceneFolder, ControlBank); strobe-gfx
//               (ScenePlayer, Gl, Caps); crate::{audio, script}
// ============================================================================

//! The active scene.
//!
//! A [`Session`] is everything that exists while one scene plays. Loading
//! a new scene builds a whole new session and only then replaces the old
//! one, so a scene that fails to compile leaves the show running.
//!
//! ## Render resolution
//!
//! The scene renders at the output window's size times a *render scale*.
//! That scale is the main performance lever on an old laptop, because
//! fragment cost is proportional to pixel count: half scale is a quarter
//! of the work. [`render_size`] is the one place that arithmetic lives.
//!
//! ## Audio
//!
//! Audio is the app's, not the scene's: [`Session::advance`] takes the
//! [`AudioInput`] and asks it for the frame at the scene clock's time, so
//! the same capture (or synthetic track) carries on across scene changes.
//! The scalars go to the shaders as uniforms and the spectrum and
//! level-trail texels to the player's audio textures, before the passes
//! render.
//!
//! ## Scripts
//!
//! A scene with a `script.js` gets a [`ScriptRun`] built at load. The frame
//! order is fixed (see `docs/ARCHITECTURE.md`):
//!
//! ```text
//! clock -> audio frame -> script (events, update, control writes)
//!       -> ControlBank::tick -> audio textures -> script uniforms -> render
//! ```
//!
//! **Uniforms the script creates are compiled in at load.** A shader cannot
//! use a uniform nobody declared, and a show must not compile shaders while
//! it plays, so [`Session::load`] runs the script's first frame *before*
//! building the player and declares whatever it published (in `setup()` or
//! in that first `update`) in every pass program. That frame is part of the
//! scene's life, not a probe: it runs with the real audio at the scene's
//! start and a 60 Hz step, and the scene's second frame is its next.
//! A script that publishes a uniform later (say, only once a beat is heard)
//! costs one recompile at that moment, logged, because the alternative is a
//! shader that cannot compile. A uniform whose name clashes with something
//! the scene declares is dropped with a warning.
//!
//! The previous frame's 64x36 downsample (`textureFinalPass`) is **not**
//! provided yet: it needs a GPU readback every frame, which stalls the
//! pipeline on an old laptop. `textureFinalPass` returns black.

use crate::audio::AudioInput;
use crate::failures;
use crate::script::{self, Harvest, ScriptRun, ScriptState};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_gfx::gl::Gl;
use strobe_gfx::{AudioTextures, Caps, FrameInputs, Mouse, Options, PlayerError, ScenePlayer};
use strobe_scene::assemble::ExtraType;
use strobe_scene::control::def::Values;
use strobe_scene::control::{ControlBank, ControlKind};
use strobe_scene::{SceneError, SceneFolder};
use strobe_script::ScriptInputs;

/// The step the script's load-time frame takes: one frame at 60 Hz.
const FIRST_FRAME_DT: f32 = 1.0 / 60.0;

/// The render scales the UI offers.
pub const RENDER_SCALE: RangeInclusive<f32> = 0.25..=1.0;

/// Smallest render target edge, so a minimised or tiny window still has a
/// valid framebuffer.
const MIN_EDGE: i32 = 16;

/// Clamps a requested scale into [`RENDER_SCALE`]; NaN becomes full scale.
pub fn clamp_scale(scale: f32) -> f32 {
    if scale.is_nan() {
        *RENDER_SCALE.end()
    } else {
        scale.clamp(*RENDER_SCALE.start(), *RENDER_SCALE.end())
    }
}

/// The render resolution for a `window` in pixels at `scale`, never larger
/// than `max_edge` (the context's texture limit).
pub fn render_size(window: [u32; 2], scale: f32, max_edge: i32) -> [i32; 2] {
    let scale = clamp_scale(scale);
    let edge = |pixels: u32| {
        let scaled = (pixels as f32 * scale).round() as i32;
        scaled.clamp(MIN_EDGE, max_edge.max(MIN_EDGE))
    };
    [edge(window[0]), edge(window[1])]
}

/// Why a scene could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The folder, `scene.json` or the controls were unreadable or invalid.
    #[error("{0}")]
    Scene(#[from] SceneError),
    /// A shader would not compile or link, or a GL resource was refused.
    /// Compiler messages already point at `main.glsl:N`.
    #[error("{0}")]
    Player(#[from] PlayerError),
}

/// One loaded, compiled scene.
#[derive(Debug)]
pub struct Session {
    /// The scene's folder name.
    pub name: String,
    /// The scene's folder.
    pub dir: PathBuf,
    /// The scene's design title, from `scene.json`.
    pub title: String,
    /// Live control values; the UI writes targets here.
    pub controls: ControlBank,
    /// The compiled passes and render targets.
    pub player: ScenePlayer,
    /// Kept so the player can be rebuilt with more script uniforms.
    scene: SceneFolder,
    gl: Gl,
    caps: Caps,
    script: Option<ScriptRun>,
    /// Whether a rebuild for script uniforms failed (not retried).
    rebuild_failed: bool,
    /// Scratch for the script's uniform values, in the player's order.
    extra_values: Vec<[f32; 4]>,
    transitions: Vec<f32>,
    frame: u64,
    time: f32,
}

impl Session {
    /// Reads, validates and compiles the scene in `dir`, rendering at
    /// `render_size`. The scene clock starts at `start` seconds.
    ///
    /// Shader errors already point at `main.glsl:N`; see
    /// [`crate::failures::scene_load`] for how an error is reported. A
    /// `script.js` that fails to start is reported and the scene loads
    /// without it.
    pub fn load(
        gl: &Gl,
        caps: &Caps,
        dir: &Path,
        name: &str,
        render_size: [i32; 2],
        start: f32,
        audio: &mut AudioInput,
    ) -> Result<Self, LoadError> {
        let scene = SceneFolder::load(dir)?;
        let mut controls = ControlBank::new(scene.def.controls.clone())?;
        let transitions = scene
            .def
            .transition_uniforms
            .iter()
            .map(|t| t.initial)
            .collect();

        let mut script = scene.script.as_deref().and_then(|source| {
            ScriptRun::start(source, name, &controls)
                .map_err(|error| failure::report(failures::script_load(name, &error)))
                .ok()
        });
        // The script's first frame comes before the player, so the uniforms
        // it creates are known when the shaders are compiled.
        let harvest = script.as_mut().map(|run| {
            let frame = audio.frame_at(start);
            run.frame(
                FIRST_FRAME_DT,
                &ScriptInputs {
                    time: start,
                    frame_count: 0,
                    audio: &frame.scalars,
                    ..ScriptInputs::default()
                },
                &mut controls,
            )
        });
        let extras = harvest.as_ref().map_or_else(Vec::new, |harvest| {
            script::usable_extras(&scene.def, extras_of(harvest))
        });
        if !extras.is_empty() {
            log::info!("script uniforms compiled in at load: {}", names_of(&extras));
        }
        let player =
            ScenePlayer::with_extras(gl, caps, &scene, render_size, Options::default(), &extras)?;
        let mut session = Self {
            name: name.to_string(),
            dir: dir.to_path_buf(),
            title: scene.def.title.clone(),
            controls,
            player,
            scene,
            gl: gl.clone(),
            caps: caps.clone(),
            script,
            rebuild_failed: false,
            extra_values: Vec::new(),
            transitions,
            frame: 0,
            time: start,
        };
        if let Some(harvest) = &harvest {
            session.send_script_uniforms(harvest);
        }
        Ok(session)
    }

    /// The scene clock, in seconds.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// The controls' targets (what the operator last asked for), for
    /// carrying across a rebuilt GL context. Bangs hold no value.
    pub fn control_targets(&self) -> Vec<(usize, Values)> {
        self.controls
            .defs()
            .iter()
            .enumerate()
            .filter(|(_, def)| !matches!(def.kind, ControlKind::Bang(_)))
            .filter_map(|(index, _)| self.controls.target(index).map(|value| (index, *value)))
            .collect()
    }

    /// Puts targets saved by [`Self::control_targets`] back.
    pub fn restore_control_targets(&mut self, targets: &[(usize, Values)]) {
        for (index, values) in targets {
            self.controls.set(*index, None, values);
        }
    }

    /// What the controls panel shows and edits, borrowed together (the
    /// controls mutably, the rest not): the scene's title, its controls,
    /// and how its script is doing, if it has one.
    pub fn panel_parts(&mut self) -> (&str, &mut ControlBank, Option<&ScriptState>) {
        (
            self.title.as_str(),
            &mut self.controls,
            self.script.as_ref().map(ScriptRun::state),
        )
    }

    /// Advances the scene by `dt` seconds and renders one frame into the
    /// player's output. `mouse` is the pointer over the output window and
    /// `date` is `strobe_Date` in local time (see [`crate::localtime`]).
    ///
    /// The script's `FRAMECOUNT` runs one ahead of the shader's, because
    /// its first frame already ran at load.
    pub fn advance(&mut self, dt: f32, audio: &mut AudioInput, mouse: Mouse, date: [f32; 4]) {
        self.time += dt;
        let audio = audio.frame_at(self.time);
        if let Some(run) = self.script.as_mut() {
            let harvest = run.frame(
                dt,
                &ScriptInputs {
                    time: self.time,
                    frame_count: self.frame + 1,
                    audio: &audio.scalars,
                    mouse: mouse.mouse,
                    click: mouse.click,
                    muv: mouse.muv,
                    muvc: mouse.muvc,
                    final_pass: None,
                },
                &mut self.controls,
            );
            if harvest.new_uniforms {
                self.adopt_new_uniforms(&harvest);
            }
            self.send_script_uniforms(&harvest);
        }
        self.controls.tick(dt);
        self.player.set_date(date);
        self.player.upload_audio(&AudioTextures {
            spectrum: &audio.spectrum,
            level_trail: &audio.level_trail,
            shadertoy: Some(&audio.shadertoy),
        });
        self.player.render(&FrameInputs {
            time: self.time,
            frame: self.frame,
            mouse,
            audio: &audio.scalars,
            controls: &self.controls,
            transitions: &self.transitions,
        });
        self.frame += 1;
    }

    /// Rebuilds the player when the script has published a uniform the
    /// shaders were not compiled with. Rare, logged, and never retried
    /// after a failure.
    fn adopt_new_uniforms(&mut self, harvest: &Harvest) {
        let wanted = script::usable_extras(&self.scene.def, extras_of(harvest));
        if wanted == self.player.extras() || self.rebuild_failed {
            return;
        }
        log::warn!(
            "script created uniform(s) after load ({}); recompiling this scene's shaders once",
            names_of(&wanted)
        );
        let size = self.player.render_size();
        match ScenePlayer::with_extras(
            &self.gl,
            &self.caps,
            &self.scene,
            size,
            Options::default(),
            &wanted,
        ) {
            Ok(player) => self.player = player,
            Err(error) => {
                self.rebuild_failed = true;
                failure::report(
                    Failure::new(Subsystem::Shader, "recompile")
                        .scene(&self.name)
                        .detail("script_uniforms", names_of(&wanted))
                        .message(error)
                        .action(
                            "keeping the shaders compiled at load; new script uniforms are not sent",
                        ),
                );
            }
        }
    }

    /// Hands the script's uniform values to the player, in the order the
    /// player declared them.
    fn send_script_uniforms(&mut self, harvest: &Harvest) {
        self.extra_values.clear();
        for (name, _) in self.player.extras() {
            let value = harvest
                .uniforms
                .iter()
                .find(|uniform| *uniform.name == **name)
                .map_or([0.0; 4], |uniform| uniform.value);
            self.extra_values.push(value);
        }
        self.player.set_script_uniforms(&self.extra_values);
    }
}

/// The `(name, type)` pairs of a harvest, the shape the assembler takes.
fn extras_of(harvest: &Harvest) -> Vec<(String, ExtraType)> {
    harvest
        .uniforms
        .iter()
        .map(|uniform| (uniform.name.to_string(), uniform.ty))
        .collect()
}

/// Names for a log line.
fn names_of(extras: &[(String, ExtraType)]) -> String {
    extras
        .iter()
        .map(|(name, ty)| format!("{name}: {}", ty.glsl()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_follows_the_window() {
        assert_eq!(render_size([1920, 1080], 1.0, 8192), [1920, 1080], "full");
    }

    #[test]
    fn half_scale_is_a_quarter_of_the_pixels() {
        assert_eq!(render_size([1920, 1080], 0.5, 8192), [960, 540], "half");
    }

    #[test]
    fn scale_is_clamped_and_sizes_have_a_floor_and_a_ceiling() {
        assert_eq!(
            render_size([1000, 1000], 0.0, 8192),
            [250, 250],
            "floor 0.25"
        );
        assert_eq!(render_size([0, 0], 1.0, 8192), [16, 16], "zero window");
        assert_eq!(render_size([9000, 100], 1.0, 4096), [4096, 100], "max edge");
        assert!((clamp_scale(f32::NAN) - 1.0).abs() < f32::EPSILON, "NaN");
    }
}
