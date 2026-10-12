// ============================================================================
// Module:       strobe_app::corpus
// Description:  The scene-corpus compatibility harness: what is measured per
//               scene and dialect, how a message becomes a cause category,
//               and how a run becomes report.json and report.md.
//
// Dependencies: serde; strobe-gfx (Gl, Caps); submodules cause, capture,
//               run, report, driver, cli
// ============================================================================

//! Compatibility measurement over a folder of scenes (Stage 0 of
//! `docs/GFX_API_RESEARCH.md`, decision D195).
//!
//! `cargo xtask corpus -- --library <dir>` loads every scene in `<dir>` the
//! way the app does, **script included**, renders it on a headless context
//! in both GLSL dialects, and writes `report.json` (one [`Record`] per scene
//! and dialect) and `report.md`. It exists to answer one question with
//! evidence instead of anecdote: *how much of a real scene library does
//! Strobe load, compile and draw, on which driver, and why not the rest?*
//!
//! ## Why it lives in strobe-app
//!
//! A scene's `script.js` can create uniforms with `setUniform`, and the
//! shader that uses them only compiles because the script ran first (D62,
//! D161). `strobe-bench` loads scenes without scripts, so it reports those
//! scenes as failing for a reason that is its own artefact. The one place
//! that does the whole load is [`crate::session::Session::load`]; the
//! harness calls it rather than copying it, so the report can never describe
//! a different pipeline from the one that plays. `strobe-script` and
//! `strobe-app` are also the two crates the MSVC cross-clippy cannot build
//! (CLAUDE.md), and `xtask` must stay buildable without a C compiler for
//! MSVC, so the harness is a binary of this crate and `cargo xtask corpus`
//! shells out to cargo for it, as `cargo xtask bench` does.
//!
//! ## Shape
//!
//! - [`run::run_scene`] does one scene on a context that is already
//!   current. It is plain code the integration test calls directly.
//! - [`driver`] runs every scene in a **child process** of the same
//!   executable (`strobe-corpus --worker`). A hang is killed at a budget
//!   (`--timeout`), and a crash in a driver (an llvmpipe segfault, an abort)
//!   becomes a `gl_error` record instead of ending the run. A GL context
//!   cannot be abandoned from another thread, so a thread-based watchdog
//!   could report a hang but never recover from it.
//! - [`cause`] turns a driver or loader message into a category.
//! - [`report`] writes the two files, and [`cli`] holds the flags.
//!
//! The corpus itself may be third-party and unlicensed (the owner's private
//! one is). Nothing here copies a scene; the report names scenes and quotes
//! one error line each, and belongs under `target/`.

pub mod capture;
pub mod cause;
pub mod cli;
pub mod driver;
pub mod report;
pub mod run;

use serde::{Deserialize, Serialize};

/// Which GLSL dialect a record was measured in.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    /// Desktop OpenGL 3.3 core, `#version 330 core`.
    Desktop,
    /// OpenGL ES 3.0, the dialect the ANGLE path uses.
    Gles,
}

impl Dialect {
    /// Both dialects, in report order.
    pub const ALL: [Self; 2] = [Self::Desktop, Self::Gles];

    /// The name used in the files and on the command line.
    pub fn label(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Gles => "gles",
        }
    }

    /// The headless context this dialect needs.
    pub fn api(self) -> strobe_gfx::Api {
        match self {
            Self::Desktop => strobe_gfx::Api::DesktopCore33,
            Self::Gles => strobe_gfx::Api::Gles30,
        }
    }
}

/// How far a scene got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Loaded, compiled and drew its frames with no GL error.
    Ok,
    /// `scene.json` or another file in the folder could not be read or is
    /// invalid.
    ParseError,
    /// `script.js` failed to start, or faulted while it ran.
    ScriptError,
    /// A pass program did not compile or link, or a GL resource was refused.
    CompileError,
    /// Drawing raised a GL error, or the worker died in the driver.
    GlError,
    /// The scene used up its time budget.
    Timeout,
}

impl Status {
    /// Every status, in report order.
    pub const ALL: [Self; 6] = [
        Self::Ok,
        Self::ParseError,
        Self::ScriptError,
        Self::CompileError,
        Self::GlError,
        Self::Timeout,
    ];

    /// The name used in the files.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ParseError => "parse_error",
            Self::ScriptError => "script_error",
            Self::CompileError => "compile_error",
            Self::GlError => "gl_error",
            Self::Timeout => "timeout",
        }
    }
}

/// A problem that did not stop the scene: it loaded and drew, but
/// something it asked for was missing. Reported so it is not lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The first line of the message.
    pub message: String,
    /// Its cause label, see [`cause::categorise`].
    pub cause: String,
}

/// One scene measured in one dialect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The scene's folder name, without `.synScene`.
    pub scene: String,
    /// The dialect it was measured in.
    pub dialect: Dialect,
    /// How far it got.
    pub status: Status,
    /// The first error line, when `status` is not `ok`.
    pub error: Option<String>,
    /// The cause label of `error`: `undeclared:<ident>`,
    /// `no_function:<name>`, `unknown_control_type:<type>`,
    /// `implicit_conversion`, `json`, `image_decode`, `preprocessor` or
    /// `other`.
    pub cause: Option<String>,
    /// A second problem behind the first, such as the compile error that
    /// follows a script that did not run.
    pub also: Option<String>,
    /// Whether the folder has a `script.js`.
    pub has_script: bool,
    /// How many uniforms the script created and the shaders declared.
    pub script_uniforms: usize,
    /// Milliseconds from the start of `Session::load` (read, script,
    /// compile, link) to a finished GL queue.
    pub compile_ms: Option<f64>,
    /// Milliseconds for the first frame, which is where a driver that
    /// defers its compile pays for it.
    pub first_frame_ms: Option<f64>,
    /// Mean milliseconds per frame after the first (the first when only one
    /// was drawn), each ended with `glFinish` so it is the GPU's time.
    pub frame_ms: Option<f64>,
    /// Whether every pixel of the final frame is black.
    pub all_black: Option<bool>,
    /// Whether every pixel of the final frame is white.
    pub all_white: Option<bool>,
    /// An FNV-1a hash of the final frame's RGB, to compare dialects and
    /// runs without storing images.
    pub checksum: Option<String>,
    /// Problems that did not stop the scene (a missing image, a script's
    /// console errors).
    pub notes: Vec<Note>,
}

impl Record {
    /// A record with nothing measured yet: status `ok`, every figure empty.
    pub fn new(scene: &str, dialect: Dialect) -> Self {
        Self {
            scene: scene.to_string(),
            dialect,
            status: Status::Ok,
            error: None,
            cause: None,
            also: None,
            has_script: false,
            script_uniforms: 0,
            compile_ms: None,
            first_frame_ms: None,
            frame_ms: None,
            all_black: None,
            all_white: None,
            checksum: None,
            notes: Vec::new(),
        }
    }

    /// A record for a scene that got no further than `status`, with
    /// `message` as its error and a cause derived from it.
    pub fn failed(scene: &str, dialect: Dialect, status: Status, message: &str) -> Self {
        let cause = match status {
            Status::Timeout | Status::Ok => None,
            Status::ParseError | Status::ScriptError | Status::CompileError | Status::GlError => {
                Some(cause::categorise(message))
            }
        };
        Self {
            status,
            error: Some(cause::first_error_line(message)),
            cause,
            ..Self::new(scene, dialect)
        }
    }
}

/// What the machine and driver were, so a result can be repeated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Driver {
    /// The dialect measured.
    pub dialect: Dialect,
    /// `GL_RENDERER`.
    pub renderer: String,
    /// `GL_VERSION`.
    pub version: String,
    /// `GL_VENDOR`.
    pub vendor: String,
    /// Where the context came from (headless EGL device, a window).
    pub context: String,
}

/// What one worker process hands back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerResult {
    /// The scene's measurements.
    pub record: Record,
    /// The driver it ran on.
    pub driver: Driver,
}

/// The knobs that decide what a run measures.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Frames of script `update` and of rendering per scene.
    pub frames: u32,
    /// Render size, `[width, height]`.
    pub size: [i32; 2],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            frames: 3,
            size: [160, 90],
        }
    }
}
