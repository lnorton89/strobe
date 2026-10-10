// ============================================================================
// Module:       strobe_script::output
// Description:  What one script frame produces: uniforms, console lines,
//               and the script's health.
//
// Dependencies: strobe_scene::assemble (ExtraType)
// ============================================================================

//! Frame output.
//!
//! Everything a script can *do* that the host cannot apply itself leaves
//! through [`FrameOutput`]. Control writes are the exception: they go
//! straight into the [`strobe_scene::control::ControlBank`]s the caller
//! passed in, the scene's own and the meta bank
//! ([`strobe_scene::meta`]), so a `meta/` write is applied the same way as
//! any other and the caller reads the result from its bank.

use std::sync::Arc;
use strobe_scene::assemble::ExtraType;

/// A uniform the script publishes, ready to upload.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptUniform {
    /// The uniform's name in the shader.
    pub name: Arc<str>,
    /// Its GLSL type, fixed by how many components the script first gave.
    pub ty: ExtraType,
    /// The value. Components past the type's width are zero.
    pub value: [f32; 4],
}

/// How loud a console line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLevel {
    /// `print`, `console.log`, `console.info`, `console.debug`.
    Log,
    /// `console.warn`.
    Warn,
    /// `console.error`, and everything the host itself reports: faults,
    /// unknown names, missing callbacks.
    Error,
}

/// One line for the developer console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleLine {
    /// Severity.
    pub level: ConsoleLevel,
    /// The text, without a trailing newline.
    pub text: String,
}

/// The script's health after a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStatus {
    /// The frame ran to completion.
    Running,
    /// The frame threw or ran out of time. Its uniforms were discarded and
    /// the last good ones stay in force. `consecutive` counts faulted
    /// frames in a row, including this one.
    Faulted {
        /// Faulted frames in a row.
        consecutive: u32,
    },
    /// Too many faults in a row: the script no longer runs, and the last
    /// good uniforms stay in force until the host is rebuilt.
    Disabled,
}

/// Everything one [`crate::ScriptHost::frame`] call produced.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameOutput {
    /// Every uniform the script has published, in the order each first
    /// appeared (so the order is stable across frames). Values are the last
    /// good ones: a faulted frame contributes nothing.
    pub uniforms: Vec<ScriptUniform>,
    /// True when a uniform became known since the last frame (or since
    /// [`crate::ScriptHost::announce_extras`]). The shader does not declare
    /// it yet: the app must recompile with
    /// [`strobe_scene::assemble::Request::extra`] set to
    /// [`Self::extras`].
    pub new_uniforms: bool,
    /// Lines for the developer console, in order.
    pub console: Vec<ConsoleLine>,
    /// Whether the script is healthy.
    pub status: ScriptStatus,
}

impl FrameOutput {
    /// The uniforms as `(name, type)` pairs, the shape
    /// [`strobe_scene::assemble::Request::extra`] takes.
    pub fn extras(&self) -> Vec<(String, ExtraType)> {
        self.uniforms
            .iter()
            .map(|uniform| (uniform.name.to_string(), uniform.ty))
            .collect()
    }
}
