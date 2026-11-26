// ============================================================================
// Module:       strobe_output (library crate root)
// Description:  The seam a finished picture leaves the engine through: a
//               frame, the pixel layouts a sender wants (BGRA, UYVY), one
//               destination with the show's failure policy, and a probe for
//               the optional NDI and Spout runtimes. No GL.
//
// Dependencies: log
// ============================================================================

//! Sharing the picture: NDI and Spout output (backlog M-06, D570).
//!
//! Strobe draws one finished picture a frame: after the output post stage,
//! at the scene's or the compositor's size, never letterboxed into a window.
//! This crate is what carries it to another program. It knows nothing about
//! GL (the renderer hands it pixels, or lends it a texture) and nothing about
//! the app's state; the host decides when an output is on.
//!
//! Three pieces, deliberately apart:
//!
//! - [`Frame`] is one picture, RGBA8 top row first, in a buffer the host
//!   reuses. Everything a sender sees crosses in one of these.
//! - [`pixel`] packs it for a sender that wants bytes: BGRA for NDI's
//!   lossless path and for anything that takes a DIB, UYVY 4:2:2 (BT.709,
//!   limited range) for NDI's default, at half the bandwidth. Spout needs
//!   neither: it shares the GL texture itself, which is why it costs no
//!   readback (D570).
//! - [`sink`] is the destination seam. One [`Sink`] per stream, and
//!   [`Output`] around it with the rules a show needs: a failure is said
//!   once, takes the sink down, and never waits in the frame.
//!
//! [`probe`] answers whether the optional runtime is even installed, so the
//! app can tell a person what to install instead of surfacing an SDK error
//! mid-show. Neither output needs an SDK at build time (D570): the NDI
//! runtime is loaded when an output is turned on, and Spout's library is
//! found by the loader, so this crate compiles everywhere and does nothing
//! until asked.
//!
//! The backends themselves (`grafton-ndi` for NDI, Spout's `SpoutLibrary`
//! for the texture path) land next, behind features, with the app's wiring;
//! the seam, the formats and the failure policy here are what they plug
//! into.

pub mod frame;
pub mod ndi;
pub mod pixel;
pub mod probe;
pub mod sink;
pub mod spout;
pub mod texture;

pub use frame::Frame;
pub use ndi::NdiSender;
pub use sink::{Outcome, Output, Problem, Sink, Status};
pub use spout::SpoutSender;
pub use texture::SharedTexture;
