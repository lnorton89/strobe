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
//!
//! The `native` feature (on by default) adds the context providers and the
//! [`offscreen`] worker. Without it the crate builds for `wasm32`, where
//! `gl::Gl::from_webgl2` wraps a browser canvas's context (D524).

pub mod adaptive;
pub mod assets;
pub mod caps;
pub mod compositor;
pub mod gl;
mod media;
#[cfg(feature = "native")]
pub mod offscreen;
pub mod player;
pub mod post;
pub mod readback;
pub mod skip;
pub mod timing;

pub use assets::{AssetProblem, MediaRequest};
pub use caps::{Caps, FloatPrecision};
pub use compositor::{Blend, Compositor, CompositorError};
#[cfg(feature = "native")]
pub use gl::context::{Api, Headless};
#[cfg(feature = "native")]
pub use gl::window::{Backend, GlChoice, SwapWindow, WindowError, WindowedContext, with_icons};
pub use media::MEDIA_TYPE_IMAGE;
pub use player::{AudioTextures, FrameInputs, Mouse, Options, PlayerError, ScenePlayer};
pub use post::{Post, PostError};
pub use timing::GpuTimer;
