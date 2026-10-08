// ============================================================================
// Module:       strobe_gfx (library crate root)
// Description:  The GL renderer: context providers, the safe GL boundary, and
//               the scene player that runs a scene's passes each frame.
//
// Dependencies: glow, glutin; strobe-scene (what to render)
// ============================================================================

//! The renderer.
//!
//! [`gl`] is the GL boundary — the only `unsafe` in the workspace — and
//! the context providers. Everything else here is safe code over it:
//! [`caps`] decides the dialect and formats a context supports, and
//! [`player`] compiles a scene and renders its passes.

pub mod caps;
pub mod gl;
pub mod player;

pub use caps::{Caps, FloatPrecision};
pub use gl::context::{Api, Headless};
pub use player::{FrameInputs, Mouse, Options, PlayerError, ScenePlayer};
