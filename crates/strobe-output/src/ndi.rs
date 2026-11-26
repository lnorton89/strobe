// ============================================================================
// Module:       strobe_output::ndi
// Description:  The NDI sender: the picture, packed to UYVY on the GPU and
//               read back, submitted asynchronously through an NDI 6 runtime
//               the crate loads at runtime.
//
// Dependencies: crate::sink, crate::texture, crate::frame (Pixels, Layout);
//               grafton-ndi (the `ndi` feature)
// ============================================================================

//! NDI: the interoperable path (D570, D573).
//!
//! NDI sends the picture over the network to any receiver (NDI Studio
//! Monitor, OBS with DistroAV, Resolume, a hardware decoder), so unlike Spout
//! it needs the pixels on the CPU: a readback and a send. That cost is kept
//! off the frame the way the NDI SDK's own guidance says to:
//!
//! - **UYVY, converted on the GPU.** The host packs the picture to UYVY
//!   4:2:2 in a shader before reading it back (`strobe_gfx::share`), so half
//!   of RGBA's bytes cross the bus and the SDK converts nothing. RGBA is
//!   still accepted, for a host without the stage.
//! - **Asynchronous.** `submit_video_copy` copies the frame into buffers the
//!   sender owns and returns; NDI compresses and sends on its own threads.
//!   The copy is a memcpy, allocation-free once two frames exist.
//! - **Nobody watching, nothing done.** [`NdiSender::receivers`] is NDI's
//!   connection count, which the host polls to skip the readback entirely
//!   while no receiver is connected. NDI hands a new receiver the last frame
//!   submitted, so nothing is lost by stopping.
//!
//! **The runtime.** `grafton-ndi` loads an NDI 6 runtime at runtime: beside
//! the program, in `NDI_RUNTIME_DIR_V6`, or on the loader's path. So this
//! compiles with no SDK, and a machine without NDI is told where to get it.
//! NDI's licence forbids shipping NDI Tools, and asks for a link to
//! ndi.video wherever NDI is chosen; the failure links to the tools page.
//!
//! **Clock.** The show's frame is the clock (`clock_video(false)`): NDI sends
//! when Strobe renders, never sleeping the render thread, and the host caps
//! the rate when the operator asks for one.

use crate::frame::Pixels;
use crate::sink::{Problem, Sink};

/// Where a person gets the NDI runtime: NDI Tools, which installs it. The
/// licence allows linking to it, not shipping it.
pub const TOOLS_URL: &str = "https://ndi.video/tools/";

/// The trademark line NDI's licence asks for wherever trademarks are
/// credited (the About pane, the docs).
pub const TRADEMARK: &str = "NDI\u{00AE} is a registered trademark of Vizrt NDI AB.";

/// What an NDI sender is opened with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NdiOptions {
    /// The name receivers list.
    pub name: String,
    /// NDI groups, comma-separated as NDI takes them; empty for the default
    /// group, which every receiver sees.
    pub groups: String,
    /// The frame rate receivers are told, in frames a second (0 for 60).
    pub fps: u32,
}

/// An NDI sender, named as a receiver lists it.
#[cfg(feature = "ndi")]
pub struct NdiSender {
    name: String,
    frame_rate: grafton_ndi::FrameRate,
    sender: grafton_ndi::Sender,
    /// The runtime handle the sender was made from, kept alive with it.
    _ndi: grafton_ndi::Ndi,
}

#[cfg(feature = "ndi")]
impl std::fmt::Debug for NdiSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NdiSender")
            .field("name", &self.name)
            .finish()
    }
}

#[cfg(feature = "ndi")]
impl NdiSender {
    /// Creates a sender on an NDI 6 runtime.
    ///
    /// # Errors
    ///
    /// No NDI runtime to load, or NDI refusing the sender (a name in use on
    /// this machine, or no network).
    pub fn open(options: &NdiOptions) -> Result<Self, Problem> {
        let name = options.name.trim();
        if name.is_empty() {
            return Err(Problem::new(
                "an NDI sender needs a name; that is what a receiver lists",
            ));
        }
        let ndi = grafton_ndi::Ndi::new().map_err(|error| {
            // The loader's error lists every folder it tried, which belongs
            // in the log, not on a Settings row.
            log::info!("NDI runtime not loaded: {error}");
            let found = crate::probe::ndi_runtime();
            Problem::new(match found {
                Some(path) => format!(
                    "the NDI runtime at {} would not load; reinstall NDI Tools from {TOOLS_URL} \
                     (the log has the loader's reason)",
                    path.display()
                ),
                None => format!(
                    "NDI output needs the NDI 6 runtime, which is not installed; install NDI \
                     Tools from {TOOLS_URL}"
                ),
            })
        })?;
        let mut sender_options = grafton_ndi::SenderOptions::new(name).clock_video(false);
        let groups: Vec<&str> = options
            .groups
            .split(',')
            .map(str::trim)
            .filter(|group| !group.is_empty())
            .collect();
        if !groups.is_empty() {
            sender_options = sender_options.groups(groups);
        }
        let sender = grafton_ndi::Sender::new(&ndi, &sender_options)
            .map_err(|error| Problem::new(format!("NDI refused the sender `{name}`: {error}")))?;
        let fps = if options.fps == 0 { 60 } else { options.fps };
        let frame_rate = grafton_ndi::FrameRate::new(fps, 1)
            .map_err(|error| Problem::new(format!("NDI refused {fps} fps: {error}")))?;
        Ok(Self {
            name: name.to_owned(),
            frame_rate,
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
    fn send(&mut self, pixels: Pixels<'_>) -> Result<(), Problem> {
        if !pixels.is_whole() {
            return Err(Problem::new("the picture is not a whole frame"));
        }
        if pixels.width == 0 || pixels.height == 0 {
            return Ok(());
        }
        let format = match pixels.layout {
            crate::frame::Layout::Rgba => grafton_ndi::PixelFormat::Rgba,
            crate::frame::Layout::Uyvy => grafton_ndi::PixelFormat::Uyvy,
        };
        let frame = grafton_ndi::BorrowedVideoFrame::builder(
            pixels.data,
            pixels.width,
            pixels.height,
            format,
        )
        .line_stride_bytes(pixels.layout.row_bytes(pixels.width))
        .frame_rate(self.frame_rate)
        .build()
        .map_err(|rejected| {
            Problem::new(format!(
                "NDI refused a {}x{} frame: {rejected}",
                pixels.width, pixels.height
            ))
        })?;
        self.sender
            .submit_video_copy(&frame)
            .map_err(|error| Problem::new(format!("NDI could not send the frame: {error}")))
    }

    fn send_texture(&mut self, _texture: crate::texture::SharedTexture) -> Result<(), Problem> {
        Err(Problem::new(
            "an NDI sender sends frames; it cannot share the renderer's texture",
        ))
    }

    fn receivers(&self) -> Option<u32> {
        self.sender.connection_count().ok()
    }

    fn close(&mut self) {
        // The SDK may still read the last asynchronous frame; waiting for it
        // here keeps the buffers alive until it has.
        self.sender.flush_video();
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
    pub fn open(_options: &NdiOptions) -> Result<Self, Problem> {
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
    fn send(&mut self, _pixels: Pixels<'_>) -> Result<(), Problem> {
        Err(Problem::new(
            "this build of Strobe was made without NDI output",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unnamed_sender_is_refused_before_any_runtime_is_loaded() {
        let options = NdiOptions {
            name: "  ".to_owned(),
            ..NdiOptions::default()
        };
        let refused = NdiSender::open(&options).err();
        assert!(
            refused.is_some_and(
                |problem| problem.note.contains("name") || problem.note.contains("without NDI")
            ),
            "a blank name is refused, or the build says it has no NDI"
        );
    }

    #[test]
    fn the_licence_lines_name_the_trademark_and_link_the_tools() {
        assert!(TRADEMARK.contains("Vizrt NDI AB"), "the owner is named");
        assert!(
            TOOLS_URL.starts_with("https://ndi.video"),
            "the link is NDI's"
        );
    }
}
