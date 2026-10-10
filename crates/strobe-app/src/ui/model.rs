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
use crate::script::ScriptState;
use crate::stats::FrameStats;
use strobe_audio::capture::{DeviceChoice, DeviceInfo};
use strobe_scene::control::ControlBank;

/// Something the UI asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Load the library entry at this index.
    Load(usize),
    /// Reload the current scene from disk.
    Reload,
    /// Apply this render scale.
    SetRenderScale(f32),
    /// Show or hide the output window. The scene keeps rendering either way.
    SetOutputVisible(bool),
    /// Toggle borderless fullscreen on the output window (showing it first
    /// if it is hidden).
    ToggleFullscreen,
    /// Move the output window to the monitor at this index.
    MoveToMonitor(usize),
    /// Empty the console.
    ClearConsole,
    /// Capture from this audio input.
    SelectAudioInput(DeviceChoice),
    /// List the capturable audio devices again.
    RefreshAudioDevices,
    /// Set the gain on audio levels and hits.
    SetReactivity(f32),
    /// Set the gain on the audio time clocks.
    SetAudioSpeed(f32),
    /// Register a tap-tempo tap.
    TapTempo,
    /// Turn adaptive resolution on or off.
    SetAdaptive(bool),
}

/// What the performance readouts and the adaptive toggle show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerfPanel {
    /// Whether adaptive resolution is switched on.
    pub adaptive: bool,
    /// Whether the GPU can be timed on this context at all.
    pub has_timer: bool,
    /// Smoothed GPU milliseconds per frame, once measured.
    pub gpu_ms: Option<f32>,
    /// The per-frame budget in milliseconds.
    pub budget_ms: f32,
    /// The render scale in force.
    pub scale: f32,
}

/// The audio settings panel's inputs.
pub struct AudioPanel<'a> {
    /// The status line: which source is active, and on which device.
    pub source: &'a str,
    /// Whether a live capture is the active source.
    pub live: bool,
    /// Whether capture was asked for (`--audio synthetic` says no).
    pub can_capture: bool,
    /// Capturable devices, inputs and (on Windows) outputs for loopback.
    pub devices: &'a [DeviceInfo],
    /// The device chosen.
    pub choice: &'a DeviceChoice,
    /// Reactivity slider's value.
    pub reactivity: &'a mut f32,
    /// Audio-speed slider's value.
    pub speed: &'a mut f32,
    /// The tempo in the latest frame.
    pub bpm: f32,
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
    /// Whether the output window is shown.
    pub output_visible: bool,
    /// Whether the output window is fullscreen.
    pub fullscreen: bool,
    /// The GL backend, renderer and version, one line.
    pub gl_summary: &'a str,
    /// Audio source and settings.
    pub audio: AudioPanel<'a>,
    /// The playing scene's script, if it has one.
    pub script: Option<&'a ScriptState>,
    /// GPU time, budget and adaptive resolution.
    pub perf: PerfPanel,
}
