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
//! [`caps`] decides the dialect and formats a context supports,
//! [`player`] compiles a scene and renders its passes, [`compositor`] mixes
//! two scenes' frames for a playlist transition, [`timing`] measures the
//! GPU without waiting for it, and [`adaptive`] turns that into a render
//! scale.

pub mod adaptive;
pub mod caps;
pub mod compositor;
pub mod gl;
pub mod offscreen;
pub mod player;
pub mod post;
pub mod readback;
pub mod timing;

pub use caps::{Caps, FloatPrecision};
pub use compositor::{Blend, Compositor, CompositorError};
pub use gl::context::{Api, Headless};
pub use gl::window::{Backend, GlChoice, SwapWindow, WindowError, WindowedContext};
pub use player::{AudioTextures, FrameInputs, Mouse, Options, PlayerError, ScenePlayer};
pub use post::{Post, PostError};
pub use timing::GpuTimer;
