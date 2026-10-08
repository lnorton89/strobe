// ============================================================================
// Module:       strobe_app::ui::model
// Description:  What the panels read each frame, and the actions they hand
//               back to the app.
//
// Dependencies: strobe-scene (ControlBank); crate::{library, console, stats}
// ============================================================================

//! The UI's inputs and outputs.
//!
//! [`Model`] borrows from the app for one frame. Only two things in it are
//! mutable: the control bank, which is the point of the controls panel, and
//! the render-scale slider's working value, which exists so a drag does not
//! reallocate every render target on every pointer move (the app applies
//! the value when the drag ends).

use crate::console::Console;
use crate::library::Library;
use crate::stats::FrameStats;
use strobe_scene::control::ControlBank;

/// Something the UI asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Load the library entry at this index.
    Load(usize),
    /// Reload the current scene from disk.
    Reload,
    /// Apply this render scale.
    SetRenderScale(f32),
    /// Toggle borderless fullscreen on the output window.
    ToggleFullscreen,
    /// Move the output window to the monitor at this index.
    MoveToMonitor(usize),
    /// Empty the console.
    ClearConsole,
}

/// The output as an egui texture, for the preview.
#[derive(Debug, Clone, Copy)]
pub struct Preview {
    /// The registered texture.
    pub texture: egui::TextureId,
    /// Width over height of the rendered frame.
    pub aspect: f32,
    /// The render resolution, for the readout.
    pub size: [i32; 2],
}

/// Everything the panels need for one frame.
pub struct Model<'a> {
    /// The scenes on disk.
    pub library: &'a Library,
    /// The library entry currently playing.
    pub playing: Option<usize>,
    /// The playing scene's title, from its definition.
    pub scene_title: &'a str,
    /// The playing scene's controls.
    pub controls: Option<&'a mut ControlBank>,
    /// The live preview, once a scene has rendered.
    pub preview: Option<Preview>,
    /// Console lines.
    pub console: &'a Console,
    /// Frame timings.
    pub stats: &'a FrameStats,
    /// The render-scale slider's working value.
    pub render_scale: &'a mut f32,
    /// Monitor labels, for the output picker.
    pub monitors: &'a [String],
    /// The monitor the output was last sent to.
    pub output_monitor: Option<usize>,
    /// Whether the output window is fullscreen.
    pub fullscreen: bool,
    /// The GL backend, renderer and version, one line.
    pub gl_summary: &'a str,
}
