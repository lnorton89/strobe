// ============================================================================
// Module:       strobe_app::ui
// Description:  The control window's interface: the egui host, the data the
//               panels read, the actions they return, and the panels.
//
// Dependencies: egui, egui-winit, egui_glow (via submodules)
// ============================================================================

//! The control window's UI.
//!
//! The UI is a pure function of a [`Model`] that returns [`Action`]s:
//! panels read app state through the model, write control targets through
//! the one `&mut ControlBank` it carries, and everything else they want
//! done comes back as an action for the app to carry out. Product state
//! (selection, render scale, console, fullscreen) lives in the app, never
//! in egui's widget memory, so a future workspace layout, MIDI mapping or
//! playlist can change it without going through a widget.

pub mod audio;
pub mod browser;
pub mod console;
pub mod controls;
pub mod host;
pub mod import;
pub mod layout;
pub mod model;

pub use host::UiHost;
pub use model::{Action, AudioPanel, ImportPanel, ListenerPanel, Model, PerfPanel, Preview};
