// ============================================================================
// Module:       strobe_output::ndi_receive
// Description:  NDI input: the sources NDI finds on the network, and one
//               source's video read into a reused buffer as RGBA or BGRA.
//
// Dependencies: crate::receive (Received, Order), crate::sink (Problem),
//               crate::ndi (the runtime's loading); grafton-ndi (`ndi`)
// ============================================================================

//! NDI input (D575): another program's picture, over the network, behind
//! `syn_Media`.
//!
//! **Finding.** NDI discovers sources in the background once a finder
//! exists, so [`NdiFinder`] is made once and kept: a listing reads what it
//! has found so far, and the first listing waits a moment for the first
//! answers. A source is named as NDI lists it, `MACHINE (Name)`.
//!
//! **Receiving.** A receiver asks NDI for RGBX/RGBA, so NDI decodes to the
//! layout `syn_Media` uploads and nothing is converted here beyond the
//! capture thread's turning the rows over. Each frame is copied out of NDI's
//! buffer into the caller's reused one (rows can be padded), and NDI's is
//! freed at once. A source with alpha sends BGRA in some SDK versions, which
//! is accepted too.
//!
//! **Licence.** Only the runtime is used, loaded at run time; a machine
//! without it is told where to get NDI Tools ([`crate::ndi::TOOLS_URL`]).

use crate::receive::Received;
use crate::sink::Problem;
use std::time::Duration;

/// NDI's discovery, kept for the process.
#[cfg(feature = "ndi")]
pub struct NdiFinder {
    finder: grafton_ndi::Finder,
    /// The runtime, kept alive with the finder (dropped after it).
    ndi: grafton_ndi::Ndi,
}

#[cfg(feature = "ndi")]
impl std::fmt::Debug for NdiFinder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NdiFinder").finish_non_exhaustive()
    }
}

#[cfg(feature = "ndi")]
impl NdiFinder {
    /// Loads the runtime and starts discovery.
    ///
    /// # Errors
    ///
    /// No NDI runtime, or NDI refusing to search.
    pub fn new() -> Result<Self, Problem> {
        let ndi = crate::ndi::runtime("NDI input")?;
        let options = grafton_ndi::FinderOptions::new().show_local_sources(true);
        let finder = grafton_ndi::Finder::new(&ndi, &options)
            .map_err(|error| Problem::new(format!("NDI could not search the network: {error}")))?;
        Ok(Self { finder, ndi })
    }

    /// The sources found so far, by name, waiting up to `wait` for the first
    /// answers when none has come yet.
    ///
    /// # Errors
    ///
    /// NDI's own error.
    pub fn sources(&self, wait: Duration) -> Result<Vec<String>, Problem> {
        let mut found = self
            .finder
            .current_sources()
            .map_err(|error| Problem::new(format!("NDI could not list sources: {error}")))?;
        if found.is_empty() && !wait.is_zero() {
            found = self
                .finder
                .find_sources(wait)
                .map_err(|error| Problem::new(format!("NDI could not list sources: {error}")))?;
        }
        Ok(found
            .iter()
            .map(|source| source.name().to_owned())
            .collect())
    }
}

/// A receiver connected to one NDI source.
#[cfg(feature = "ndi")]
pub struct NdiReceiver {
    receiver: grafton_ndi::Receiver,
    /// Whether a frame has ever come, for the failure's words.
    seen: bool,
}

#[cfg(feature = "ndi")]
impl std::fmt::Debug for NdiReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NdiReceiver")
            .field("seen", &self.seen)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "ndi")]
impl NdiReceiver {
    /// Connects to the source called `name`, as `finder` lists it.
    ///
    /// # Errors
    ///
    /// No such source, or NDI refusing the receiver.
    pub fn open(finder: &NdiFinder, name: &str) -> Result<Self, Problem> {
        let listed = finder
            .finder
            .current_sources()
            .map_err(|error| Problem::new(format!("NDI could not list sources: {error}")))?;
        let source = match listed.into_iter().find(|source| source.name() == name) {
            Some(source) => source,
            None => grafton_ndi::Source::new(name)
                .map_err(|error| Problem::new(format!("not an NDI source name: {error}")))?,
        };
        let options = grafton_ndi::ReceiverOptions::new(source)
            .color_format(grafton_ndi::ReceiverColorFormat::RgbxRgba)
            .bandwidth(grafton_ndi::ReceiverBandwidth::Highest)
            .allow_video_fields(false)
            .name("Strobe");
        let receiver = grafton_ndi::Receiver::new(&finder.ndi, &options)
            .map_err(|error| Problem::new(format!("NDI refused the receiver: {error}")))?;
        Ok(Self {
            receiver,
            seen: false,
        })
    }

    /// Waits up to `timeout` for the next frame and copies it into `pixels`,
    /// top row first, tightly packed. `Ok(None)` when none came in time but
    /// the source is still connected.
    ///
    /// # Errors
    ///
    /// The source went away, or sent a layout Strobe cannot read.
    pub fn receive(
        &mut self,
        pixels: &mut Vec<u8>,
        timeout: Duration,
    ) -> Result<Option<Received>, Problem> {
        let frame = self
            .receiver
            .capture_video_ref(timeout)
            .map_err(|error| Problem::new(format!("the source stopped sending: {error}")))?;
        let Some(frame) = frame else {
            if self.seen && !self.receiver.is_connected() {
                return Err(Problem::new(
                    "the source is no longer connected (end of stream)",
                ));
            }
            return Ok(None);
        };
        // NDI's format enum is non-exhaustive, so the two accepted layouts
        // are tested and everything else refused by name.
        let format = frame.pixel_format();
        let order = if matches!(
            format,
            grafton_ndi::PixelFormat::Rgba | grafton_ndi::PixelFormat::Rgbx
        ) {
            crate::receive::Order::Rgba
        } else if matches!(
            format,
            grafton_ndi::PixelFormat::Bgra | grafton_ndi::PixelFormat::Bgrx
        ) {
            crate::receive::Order::Bgra
        } else {
            return Err(Problem::new(format!(
                "the source sends {format:?}, which Strobe cannot read"
            )));
        };
        let (width, height) = (frame.width(), frame.height());
        let row = width as usize * 4;
        let plane = frame
            .plane(0)
            .ok_or_else(|| Problem::new("the source sent a frame with no picture"))?;
        pixels.resize(row * height as usize, 0);
        for (index, out) in pixels.chunks_exact_mut(row).enumerate() {
            let line = plane
                .row(index)
                .and_then(|line| line.get(..row))
                .ok_or_else(|| Problem::new("the source sent a frame shorter than its size"))?;
            out.copy_from_slice(line);
        }
        self.seen = true;
        Ok(Some(Received {
            width,
            height,
            order,
        }))
    }
}

/// A stand-in when the crate was built without the `ndi` feature.
#[cfg(not(feature = "ndi"))]
#[derive(Debug)]
pub struct NdiFinder;

#[cfg(not(feature = "ndi"))]
fn without() -> Problem {
    Problem::new("this build of Strobe was made without NDI (the `ndi` feature is off)")
}

#[cfg(not(feature = "ndi"))]
impl NdiFinder {
    /// Always fails: this build has no NDI in it.
    pub fn new() -> Result<Self, Problem> {
        Err(without())
    }

    /// Never reached.
    pub fn sources(&self, _wait: Duration) -> Result<Vec<String>, Problem> {
        Err(without())
    }
}

/// A stand-in when the crate was built without the `ndi` feature.
#[cfg(not(feature = "ndi"))]
#[derive(Debug)]
pub struct NdiReceiver;

#[cfg(not(feature = "ndi"))]
impl NdiReceiver {
    /// Always fails: this build has no NDI in it.
    pub fn open(_finder: &NdiFinder, _name: &str) -> Result<Self, Problem> {
        Err(without())
    }

    /// Never reached.
    pub fn receive(
        &mut self,
        _pixels: &mut Vec<u8>,
        _timeout: Duration,
    ) -> Result<Option<Received>, Problem> {
        Err(without())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finder_starts_or_says_where_to_get_ndi() {
        // On a machine with the runtime this searches; without it (CI, most
        // laptops) the one line names NDI Tools or the missing feature.
        if let Err(problem) = NdiFinder::new() {
            assert!(
                problem.note.contains("ndi.video") || problem.note.contains("without NDI"),
                "the reason says what to do: {problem}"
            );
        }
    }
}
