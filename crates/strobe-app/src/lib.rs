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
//! - [`audio`]: live capture or synthetic audio, and its settings.
//! - [`session`]: the one active scene and the render-scale arithmetic.
//! - [`ui`]: the control window's panels, as a function of a model.
//! - [`theme`]: the look, in one place.
//! - [`library`], [`console`], [`stats`]: the data the UI shows. The console
//!   is a view of `strobe-diag`'s log ring.
//! - [`failures`], [`dialog`], [`gldebug`], [`diagnose`]: how errors,
//!   crashes and driver messages reach a person (all through `strobe-diag`).
//! - [`screenshot`]: the `--screenshot` verification mode.
//! - [`cli`]: the command line.

pub mod app;
pub mod audio;
pub mod cli;
pub mod console;
pub mod diagnose;
pub mod dialog;
pub mod failures;
pub mod gldebug;
pub mod library;
pub mod screenshot;
pub mod session;
pub mod stats;
pub mod theme;
pub mod ui;
