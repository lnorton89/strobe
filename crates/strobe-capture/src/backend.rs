// ============================================================================
// Module:       strobe_capture::backend
// Description:  The seam between the capture threads and a camera API: list
//               cameras, open one by name, read its raw pictures.
//
// Dependencies: crate::cause, crate::format
// ============================================================================

//! What a camera API has to offer the capture threads.
//!
//! [`Backend`] is shared between threads; a [`Stream`] is opened, read and
//! dropped on the one reader thread that opened it, which is what Media
//! Foundation's COM objects need, so it is not `Send`.

use crate::cause::Cause;
use crate::format::PixelFormat;
use std::fmt;

/// A failure from a backend, with what it most likely means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The likely cause.
    pub cause: Cause,
    /// The backend's own words.
    pub detail: String,
}

impl Problem {
    /// A problem whose cause is read from the backend's wording
    /// ([`Cause::from_text`]).
    pub fn from_text(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            cause: Cause::from_text(&detail),
            detail,
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

/// One raw picture as the camera sent it, top row first.
#[derive(Debug, Clone, Copy)]
pub struct RawFrame<'a> {
    /// How the bytes are laid out.
    pub format: PixelFormat,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The bytes.
    pub data: &'a [u8],
}

/// A camera API.
pub trait Backend: Send + Sync + 'static {
    /// The cameras it lists, by name, in its order.
    ///
    /// # Errors
    ///
    /// The API could not list.
    fn list(&self) -> Result<Vec<String>, Problem>;

    /// Opens the camera called `name` (the first, if two share it) and
    /// starts its stream.
    ///
    /// # Errors
    ///
    /// Not there, in use, blocked, or the API failed.
    fn open(&self, name: &str) -> Result<Box<dyn Stream>, Problem>;
}

/// An open camera.
pub trait Stream {
    /// The format it sends, for the log: `1280x720 YUY2 30 fps`.
    fn describe(&self) -> String;

    /// The next picture. Blocks until one comes, which may be never: the
    /// watchdog is the timeout.
    ///
    /// # Errors
    ///
    /// The camera stopped (unplugged, taken by another app).
    fn read(&mut self) -> Result<RawFrame<'_>, Problem>;
}

/// No camera API: what a system without a backend gets.
#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Unavailable;

#[cfg(not(windows))]
impl Unavailable {
    fn problem() -> Problem {
        Problem {
            cause: Cause::Unsupported,
            detail: "camera input is built for Windows only so far".to_string(),
        }
    }
}

/// Listing fails too, so a chosen camera says why rather than "not found".
#[cfg(not(windows))]
impl Backend for Unavailable {
    fn list(&self) -> Result<Vec<String>, Problem> {
        Err(Self::problem())
    }

    fn open(&self, _name: &str) -> Result<Box<dyn Stream>, Problem> {
        Err(Self::problem())
    }
}
