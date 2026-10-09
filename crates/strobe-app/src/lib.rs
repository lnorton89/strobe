// ============================================================================
// Module:       strobe_app (library crate root)
// Description:  The desktop app's parts: window and frame loop, scene
//               session, control-window UI, theme, and the CLI.
//
// Dependencies: winit, egui, strobe-gfx, strobe-scene
// ============================================================================

//! The `strobe` desktop app, as a library so its logic is testable and
//! every item can be `pub` without being dead.
//!
//! - [`app`]: the winit application and frame loop.
//! - [`session`]: the one active scene and the render-scale arithmetic.
//! - [`ui`]: the control window's panels, as a function of a model.
//! - [`theme`]: the look, in one place.
//! - [`library`], [`console`], [`stats`]: the data the UI shows.
//! - [`screenshot`]: the `--screenshot` verification mode.
//! - [`cli`], [`logging`]: the command line and the log.

pub mod app;
pub mod cli;
pub mod console;
pub mod library;
pub mod logging;
pub mod screenshot;
pub mod session;
pub mod stats;
pub mod theme;
pub mod ui;
