// ============================================================================
// Module:       strobe_app::session
// Description:  The one active scene: its controls, its compiled player, and
//               the per-frame step that feeds audio and controls into it.
//
// Dependencies: strobe-scene (SceneFolder, ControlBank, synthetic audio);
//               strobe-gfx (ScenePlayer, Gl, Caps)
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
//! The values come from [`strobe_scene::synthetic::fill`] until the audio
//! crate lands. The buffer is allocated once at load; a frame writes into
//! it in place.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use strobe_gfx::gl::Gl;
use strobe_gfx::{Caps, FrameInputs, Mouse, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;

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
    transitions: Vec<f32>,
    audio: Vec<f32>,
    frame: u64,
    time: f32,
}

impl Session {
    /// Reads, validates and compiles the scene in `dir`, rendering at
    /// `render_size`. The scene clock starts at `start` seconds.
    ///
    /// The error text is ready for the console: shader errors already
    /// point at `main.glsl:N`.
    pub fn load(
        gl: &Gl,
        caps: &Caps,
        dir: &Path,
        name: &str,
        render_size: [i32; 2],
        start: f32,
    ) -> Result<Self, String> {
        let scene = SceneFolder::load(dir).map_err(|e| e.to_string())?;
        let controls = ControlBank::new(scene.def.controls.clone()).map_err(|e| e.to_string())?;
        let transitions = scene
            .def
            .transition_uniforms
            .iter()
            .map(|t| t.initial)
            .collect();
        let player = ScenePlayer::new(gl, caps, &scene, render_size, Options::default())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            name: name.to_string(),
            dir: dir.to_path_buf(),
            title: scene.def.title.clone(),
            controls,
            player,
            transitions,
            audio: vec![0.0; SCALARS.len()],
            frame: 0,
            time: start,
        })
    }

    /// Advances the scene by `dt` seconds and renders one frame into the
    /// player's output.
    pub fn advance(&mut self, dt: f32) {
        self.time += dt;
        strobe_scene::synthetic::fill(self.time, &mut self.audio);
        self.controls.tick(dt);
        self.player.render(&FrameInputs {
            time: self.time,
            frame: self.frame,
            mouse: Mouse::default(),
            audio: &self.audio,
            controls: &self.controls,
            transitions: &self.transitions,
        });
        self.frame += 1;
    }
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
