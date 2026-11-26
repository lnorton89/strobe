// ============================================================================
// Module:       strobe_output::ndi
// Description:  The NDI sender: the picture read back to the CPU and sent as
//               a frame, through an NDI 6 runtime the crate loads at runtime.
//
// Dependencies: crate::sink, crate::texture, crate::Frame; grafton-ndi (the
//               `ndi` feature)
// ============================================================================

//! NDI: the interoperable path (D570).
//!
//! NDI sends the picture over the network to any receiver — NDI Studio
//! Monitor, OBS with the NDI plugin, a hardware decoder — so unlike Spout it
//! needs the pixels on the CPU: a readback, a frame and a send. That is the
//! cost this path has and Spout does not, and it is why the sender takes a
//! [`Frame`] and not a texture.
//!
//! **The runtime.** `grafton-ndi` loads an NDI 6 runtime at runtime: beside
//! the program, in `NDI_RUNTIME_DIR_V6`, or on the loader's path. So this
//! compiles with no SDK, and a machine without NDI says so through
//! [`crate::probe::ndi_runtime`] instead of failing to build.
//!
//! **Clock.** The show's frame is the clock: `clock_video(false)`, so NDI
//! sends when Strobe renders rather than at a rate of its own. A scene that
//! renders at 40 fps sends at 40 fps, which is what the projector shows.
//!
//! **Format.** Frames go as RGBA, which is what the readback already holds:
//! no conversion on the frame thread, and every NDI receiver takes it. UYVY
//! would halve the bandwidth on the wire at the cost of a conversion per
//! frame; that is the lever to pull when a venue network is the constraint
//! (M-06's measurement).

use crate::Frame;
use crate::sink::{Problem, Sink};

/// An NDI sender, named as a receiver lists it.
#[cfg(feature = "ndi")]
#[derive(Debug)]
pub struct NdiSender {
    name: String,
    sender: grafton_ndi::Sender,
    /// The runtime handle the sender was made from, kept alive with it.
    _ndi: grafton_ndi::Ndi,
}

#[cfg(feature = "ndi")]
impl NdiSender {
    /// Creates a sender called `name` on an NDI 6 runtime.
    ///
    /// # Errors
    ///
    /// No NDI runtime to load, or NDI refusing the sender (a name in use on
    /// this machine, or no network).
    pub fn open(name: &str) -> Result<Self, Problem> {
        if name.trim().is_empty() {
            return Err(Problem::new(
                "an NDI sender needs a name; that is what a receiver lists",
            ));
        }
        let ndi = grafton_ndi::Ndi::new().map_err(|error| {
            Problem::new(format!(
                "NDI output needs the NDI 6 runtime, which could not be loaded ({error}); \
                 install NDI Tools from https://ndi.video"
            ))
        })?;
        let options = grafton_ndi::SenderOptions::new(name).clock_video(false);
        let sender = grafton_ndi::Sender::new(&ndi, &options)
            .map_err(|error| Problem::new(format!("NDI refused the sender `{name}`: {error}")))?;
        Ok(Self {
            name: name.to_owned(),
            sender,
            _ndi: ndi,
        })
    }

    /// The name receivers list.
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(feature = "ndi")]
impl Sink for NdiSender {
    fn send(&mut self, frame: &Frame) -> Result<(), Problem> {
        if !frame.is_whole() {
            return Err(Problem::new("the picture is not a whole frame"));
        }
        if frame.width == 0 || frame.height == 0 {
            return Ok(());
        }
        // A frame the SDK owns: `send_video` may hold it, so the pixels are
        // copied into it rather than borrowed from the readback's buffer.
        let pixels = frame.rgba.clone();
        let built = grafton_ndi::VideoFrame::builder()
            .resolution(frame.width, frame.height)
            .pixel_format(grafton_ndi::PixelFormat::Rgba)
            .data(pixels)
            .build();
        let video = built.map_err(|rejected| {
            Problem::new(format!(
                "NDI refused a {}x{} frame: {rejected}",
                frame.width, frame.height
            ))
        })?;
        self.sender
            .send_video(&video)
            .map_err(|error| Problem::new(format!("NDI could not send the frame: {error}")))
    }

    fn send_texture(&mut self, _texture: crate::texture::SharedTexture) -> Result<(), Problem> {
        Err(Problem::new(
            "an NDI sender sends frames; it cannot share the renderer's texture",
        ))
    }
}

/// A stand-in when the crate was built without the `ndi` feature, so the
/// app's code is the same either way and the failure says why.
#[cfg(not(feature = "ndi"))]
#[derive(Debug)]
pub struct NdiSender;

#[cfg(not(feature = "ndi"))]
impl NdiSender {
    /// Always fails: this build has no NDI sender in it.
    pub fn open(_name: &str) -> Result<Self, Problem> {
        Err(Problem::new(
            "this build of Strobe was made without NDI output (the `ndi` feature is off)",
        ))
    }

    /// No name: nothing was opened.
    pub fn name(&self) -> &str {
        ""
    }
}

#[cfg(not(feature = "ndi"))]
impl Sink for NdiSender {
    fn send(&mut self, _frame: &Frame) -> Result<(), Problem> {
        Err(Problem::new(
            "this build of Strobe was made without NDI output",
        ))
    }
}
