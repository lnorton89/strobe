// ============================================================================
// Module:       strobe_app (library crate root)
// Description:  The desktop app's parts: window and frame loop, scene
//               session, control-window UI, theme, and the CLI.
//
// Dependencies: winit, egui, strobe-gfx, strobe-scene, strobe-diag
// ============================================================================

//! The `strobe` desktop app, as a library so its logic is testable and
//! every item can be `pub` without being dead.
//!
//! - [`app`]: the winit application and frame loop.
//! - [`perf`]: GPU timing and adaptive resolution.
//! - [`script`], [`mouse`]: a scene's `script.js` at run time, and the
//!   pointer uniforms it and the shaders read.
//! - [`audio`]: live capture or synthetic audio, and its settings.
//! - [`session`]: the one active scene and the render-scale arithmetic.
//! - [`ui`]: the control window's panels, as a function of a model.
//! - [`theme`]: the look, in one place.
//! - [`import`]: Shadertoy import on a worker thread, the panel's state, and
//!   the browser listener's handler.
//! - [`library`], [`console`], [`stats`]: the data the UI shows. The console
//!   is a view of `strobe-diag`'s log ring.
//! - [`failures`], [`dialog`], [`gldebug`], [`diagnose`]: how errors,
//!   crashes and driver messages reach a person (all through `strobe-diag`).
//! - [`recovery`], [`fault`]: surviving a lost GL context (the policy, as a
//!   pure state machine) and `STROBE_FAULT`, which injects one to test it.
//! - [`localtime`]: local wall-clock time for `strobe_Date`.
//! - [`screenshot`]: the `--screenshot` verification mode.
//! - [`cli`]: the command line.

pub mod app;
pub mod audio;
pub mod cli;
pub mod color;
pub mod commands;
pub mod console;
pub mod diagnose;
pub mod dialog;
pub mod failures;
pub mod fault;
pub mod gldebug;
pub mod icon;
pub mod import;
pub mod library;
pub mod localtime;
pub mod motion;
pub mod mouse;
pub mod palette;
pub mod perf;
pub mod prefs;
pub mod recovery;
pub mod screenshot;
pub mod script;
pub mod session;
pub mod stats;
pub mod theme;
pub mod ui;
pub mod workspace;
