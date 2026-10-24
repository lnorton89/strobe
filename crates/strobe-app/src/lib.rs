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
//! - [`ui`]: the control window's panels (Contextual Perform, D120), as a
//!   function of a model.
//! - The UI system ported from rustaman (D5, D130): [`color`] and
//!   [`palette`] (themes, with their contrast tests), [`motion`] (durations
//!   and reduced motion) and [`icon`] (geometry icons). None of them knows
//!   about egui; `ui::theme`, `ui::motion` and `ui::icon` are the bindings.
//! - [`workspace`], [`prefs`], [`commands`]: the pane model and how it fits
//!   a window, the saved UI preferences, and the keyboard command table with
//!   its focus rule. All pure, all tested without a window.
//! - [`import`]: Shadertoy import on a worker thread, the panel's state, and
//!   the browser listener's handler.
//! - [`intake`]: what a dropped file or folder is for, and copying a
//!   dropped scene folder into the library.
//! - [`library`], [`console`], [`stats`]: the data the UI shows. The console
//!   is a view of `strobe-diag`'s log ring.
//! - [`failures`], [`dialog`], [`gldebug`], [`diagnose`]: how errors,
//!   crashes and driver messages reach a person (all through `strobe-diag`).
//! - [`recovery`], [`fault`]: surviving a lost GL context (the policy, as a
//!   pure state machine) and `STROBE_FAULT`, which injects one to test it.
//! - [`localtime`]: local wall-clock time for `strobe_Date`.
//! - [`output_monitor`]: the monitor chosen for the output, followed across
//!   display changes by what it is rather than its place in the list (D400).
//! - [`foreground`]: whether the fullscreen output over the control window
//!   has the focus, so the log says when Windows may compose it (D453).
//! - [`screenshot`]: the `--screenshot` verification mode.
//! - [`brand`]: the window and taskbar icon (D190).
//! - [`venue`]: venue mode's launch-time pieces (D288), the trust file for an
//!   unattended show machine.
//! - [`trusted`]: the laptop's half of venue mode (D294), the trust-file
//!   proof, the retry schedule and the refusal texts.
//! - [`cli`]: the command line.
//! - [`corpus`]: the scene-corpus compatibility harness behind
//!   `strobe-corpus` and `cargo xtask corpus` (D195).

pub mod app;
pub mod audio;
pub mod brand;
pub mod cli;
pub mod color;
pub mod command_palette;
pub mod commands;
pub mod console;
pub mod corpus;
pub mod desktop;
pub mod diagnose;
pub mod dialog;
pub mod editor;
pub mod failures;
pub mod fault;
pub mod foreground;
pub mod generate;
pub mod gldebug;
pub mod icon;
pub mod import;
pub mod intake;
pub mod lan;
pub mod library;
pub mod localtime;
pub mod motion;
pub mod mouse;
pub mod output_monitor;
pub mod palette;
pub mod perf;
pub mod picker;
pub mod prefs;
pub mod recovery;
pub mod remote;
pub mod screenshot;
pub mod script;
pub mod session;
pub mod show;
pub mod snapshot;
pub mod stats;
pub mod trusted;
pub mod ui;
pub mod venue;
pub mod workspace;
