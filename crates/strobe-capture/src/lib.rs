// ============================================================================
// Module:       strobe_capture (library crate root)
// Description:  Webcam capture for syn_Media: listing cameras, a capture
//               thread per camera with a watchdog and reconnect by name, and
//               conversion of its pictures to RGBA. No GL.
//
// Dependencies: log, image (MJPEG); nokhwa (Media Foundation, Windows only)
// ============================================================================

//! Webcam capture (backlog M-04, D528).
//!
//! This crate owns everything about a camera that is not drawing it, the way
//! `strobe-audio` owns its cpal thread: which cameras there are, opening one
//! at 720p, a thread that reads its pictures and converts them to RGBA, a
//! watchdog for a read that never returns, and reopening the camera by name
//! when it comes back. It knows nothing of GL or of `strobe-gfx`.
//!
//! Pictures go to a [`Sink`] the host implements. The app's sink is a thin
//! wrapper over `strobe_gfx::live`'s one-picture mailbox, so neither crate
//! depends on the other and each picture is written once, into a buffer the
//! render thread handed back.
//!
//! ```text
//! start(name, sink) -> "camera" thread (supervisor: watchdog, wanted?)
//!                       -> "camera-read" thread: list, open, read, convert
//!                            sink.buffer() -> convert -> sink.put(Picture)
//!                            a failure -> sink.failed(..) once per outage
//!                            the next picture -> sink.recovered(..)
//! ```
//!
//! The threads are never joined. A Media Foundation read has no timeout and
//! can block for ever on a camera that stopped sending, so nothing waits on
//! one: the watchdog says so, leaves that reader behind, and starts another.
//! Both stop by themselves once [`Sink::wanted`] says no one shows the
//! camera any more.
//!
//! On Windows the backend is nokhwa's Media Foundation binding ([`system`]).
//! Elsewhere there is none yet: [`list`] and a started camera both say so,
//! as [`Cause::Unsupported`], the camera once. The supervisor, the conversion and the
//! failure wording are plain Rust and tested on every OS through a fake
//! [`Backend`].

mod backend;
pub mod cause;
pub mod convert;
pub mod format;
#[cfg(windows)]
mod msmf;
mod supervisor;

pub use backend::{Backend, Problem, RawFrame, Stream};
pub use cause::{Cause, Failure};
pub use format::{Offer, PixelFormat};
pub use supervisor::Timing;

use std::sync::Arc;
use std::time::Instant;

/// One converted picture.
#[derive(Debug, Default)]
pub struct Picture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8, bottom row first: GL's order, so `syn_Media` reads upright.
    pub rgba: Vec<u8>,
    /// When the reader had it, before conversion.
    pub captured: Option<Instant>,
}

/// Where a camera's pictures and news go. Called from the capture threads,
/// so it must be cheap and must never wait on the render thread.
pub trait Sink: Send + Sync + 'static {
    /// A buffer to convert the next picture into: one handed back by the
    /// render thread when there is one, so steady state allocates nothing.
    fn buffer(&self) -> Vec<u8>;
    /// The newest picture. It may replace one not yet shown.
    fn put(&self, picture: Picture);
    /// The camera stopped, or never started. Called once per outage, not
    /// once per retry; the picture should read black until the next
    /// [`Self::put`].
    fn failed(&self, failure: &Failure);
    /// A picture came again after [`Self::failed`].
    fn recovered(&self, camera: &str);
    /// Whether anything still shows this camera. When it says no, the
    /// threads stop within a tick and close the camera.
    fn wanted(&self) -> bool;
}

/// The cameras this system lists, by name, in its own order. Takes a few
/// to some tens of milliseconds, so a UI calls it off its frame thread.
///
/// # Errors
///
/// The backend's own error, worded for a person.
pub fn list() -> Result<Vec<String>, Problem> {
    system().list()
}

/// Starts showing the camera called `name` into `sink`, on the system's
/// backend with the default [`Timing`]. Returns at once: the camera opens
/// on its own thread, and a camera that is not there yet is waited for.
pub fn start(name: &str, sink: Arc<dyn Sink>) {
    start_with(system(), Timing::default(), name, sink);
}

/// [`start`] with a chosen backend and timing (tests use a fake backend
/// and short times).
pub fn start_with(backend: Arc<dyn Backend>, timing: Timing, name: &str, sink: Arc<dyn Sink>) {
    supervisor::spawn(backend, timing, name.to_string(), sink);
}

/// The platform's camera backend: Media Foundation on Windows, none
/// elsewhere yet (backlog M-04 covers Windows only).
pub fn system() -> Arc<dyn Backend> {
    #[cfg(windows)]
    {
        Arc::new(msmf::MediaFoundation)
    }
    #[cfg(not(windows))]
    {
        Arc::new(backend::Unavailable)
    }
}
