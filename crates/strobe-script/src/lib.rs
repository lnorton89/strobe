// ============================================================================
// Module:       strobe_script (library crate root)
// Description:  The QuickJS host for the Synesthesia JavaScript API: one
//               sandboxed interpreter per scene, run once per frame.
//
// Dependencies: rquickjs, thiserror; strobe-scene (controls, names, audio)
// ============================================================================

//! The Synesthesia JavaScript API, hosted.
//!
//! A scene may ship a `script.js`. Synesthesia runs it every frame before
//! the shader: the script reads the frame's uniforms as globals, may move
//! controls, and publishes new uniforms with `setUniform`. This crate is
//! that host, and nothing else: it produces *values* (uniforms, control
//! writes, console lines) and leaves applying them to the app, so it never
//! touches GL, audio capture, or a clock. `dt` and every input are passed
//! in.
//!
//! The entry point is [`ScriptHost`], one per scene instance:
//!
//! ```text
//! ScriptHost::new(source, &controls, &meta, Budget::default())  evaluate, setup()
//! ScriptHost::frame(dt, &inputs, &mut controls, &mut meta)     once per frame
//!     write globals → dispatch events → update(dt) → promise jobs → harvest
//! ```
//!
//! ## Containment
//!
//! A script is third-party code running on the render thread in front of
//! an audience, so the host assumes it will misbehave.
//!
//! - Each host owns its own interpreter, capped at 64 MiB and a 256 KiB
//!   stack, so a runaway scene cannot starve another, and runaway recursion
//!   throws inside the script before it reaches the end of a Windows main
//!   thread's 1 MiB stack (D321).
//! - Every frame runs under a deadline (4 ms by default). `while (true) {}`
//!   is interrupted, the frame returns, and the last good uniforms stay in
//!   force.
//! - An exception or an interrupt is a *fault*: it is logged with its line
//!   (once per distinct message), reported in [`FrameOutput::status`]
//!   every time, and the frame's uniforms are
//!   discarded. Three faulted frames in a row disable the script until the
//!   host is rebuilt, which is what a reload does.
//!
//! The crate contains no `unsafe`. Native functions therefore cannot hold
//! the caller's `&mut ControlBank`; they queue their writes and the host
//! applies them in order at the end of the frame, so a `setControl` still
//! lands in the same frame.
//!
//! [`ScriptHost`] is deliberately not `Send`: the interpreter and its
//! shared state are single-threaded. Build and drive it on the render
//! thread.

mod budget;
mod error;
mod events;
mod host;
mod inputs;
mod natives;
mod output;
mod shared;
mod uniforms;

#[cfg(test)]
mod conformance;

pub use budget::Budget;
pub use error::ScriptError;
pub use host::ScriptHost;
pub use inputs::{FINAL_PASS_BYTES, FINAL_PASS_HEIGHT, FINAL_PASS_WIDTH, ScriptInputs};
pub use output::{ConsoleLevel, ConsoleLine, FrameOutput, ScriptStatus, ScriptUniform};
