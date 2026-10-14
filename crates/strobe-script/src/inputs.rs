// ============================================================================
// Module:       strobe_script::inputs
// Description:  Everything the app hands the script host each frame: clock,
//               audio scalars, mouse uniforms, and the final-pass downsample.
//
// Dependencies: std only
// ============================================================================

//! Per-frame inputs.
//!
//! These are the values the script sees as globals, minus the controls
//! (those come from the [`strobe_scene::control::ControlBank`] passed to
//! `frame`). Audio arrives as a plain slice in
//! [`strobe_scene::audio::SCALARS`] order, the same indexing the analyser
//! fills, so no names cross this boundary per frame.

/// Width of the final-pass downsample behind `textureFinalPass`.
pub const FINAL_PASS_WIDTH: usize = 64;

/// Height of the final-pass downsample behind `textureFinalPass`.
pub const FINAL_PASS_HEIGHT: usize = 36;

/// Length in bytes of a downsample: `FINAL_PASS_WIDTH * FINAL_PASS_HEIGHT`
/// RGB8 pixels.
pub const FINAL_PASS_BYTES: usize = FINAL_PASS_WIDTH * FINAL_PASS_HEIGHT * 3;

/// One frame's worth of input for [`crate::ScriptHost::frame`].
#[derive(Debug, Clone, Copy)]
pub struct ScriptInputs<'a> {
    /// `TIME`: seconds since the scene started.
    pub time: f32,
    /// `FRAMECOUNT`: frames since the scene started. Kept integral until
    /// it reaches the script, because an `f32` stops counting by ones at
    /// 16 million frames, about 77 hours at 60 fps.
    pub frame_count: u64,
    /// The audio scalars in [`strobe_scene::audio::SCALARS`] order. A
    /// slice shorter than the list leaves the missing tail at its silent
    /// value ([`strobe_scene::audio::silent_value`]).
    pub audio: &'a [f32],
    /// `_mouse`: pixel position (x, y) and the pixel position of the last
    /// click (z, w).
    pub mouse: [f32; 4],
    /// `_click`: left, right, middle held (`0` or `1`), as the player feeds
    /// it. A real script tests `_click.x > 0.5` for the left button (R12).
    pub click: [f32; 3],
    /// `_muv`: mouse position in `0..1` across the output.
    pub muv: [f32; 2],
    /// `_muvc`: mouse position in centred coordinates, as `_uvc`.
    pub muvc: [f32; 2],
    /// A recent frame's final pass, downsampled to [`FINAL_PASS_WIDTH`] by
    /// [`FINAL_PASS_HEIGHT`] RGB8, rows from the bottom up (the same
    /// orientation as `_uv`). The app fills it from an asynchronous GPU
    /// readback, so it is a few frames old (D162: the picture two frames
    /// before the one just drawn), never the immediately previous frame.
    /// `None`, or a slice of any other length, makes `textureFinalPass`
    /// return black, which is also what a script sees until the first
    /// readback has landed.
    pub final_pass: Option<&'a [u8]>,
}

impl Default for ScriptInputs<'_> {
    /// A silent room, no mouse, at time zero.
    fn default() -> Self {
        Self {
            time: 0.0,
            frame_count: 0,
            audio: &[],
            mouse: [0.0; 4],
            click: [0.0; 3],
            muv: [0.0; 2],
            muvc: [0.0; 2],
            final_pass: None,
        }
    }
}
