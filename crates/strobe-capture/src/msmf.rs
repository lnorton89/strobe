// ============================================================================
// Module:       strobe_capture::msmf
// Description:  The Windows camera backend: nokhwa's Media Foundation binding,
//               listing by name, opening at the format `format::pick` chose,
//               raw reads with no decoding.
//
// Dependencies: nokhwa (input-msmf, no default features); crate::backend,
//               crate::format
// ============================================================================

//! Media Foundation through nokhwa (D528).
//!
//! Only nokhwa's safe API is used, so no `unsafe` lives here. The binding is
//! patched in `vendor/nokhwa-bindings-windows` (D529): the per-frame `IMFSample`
//! leak (nokhwa #251) is fixed, and a read that gets an error or
//! end-of-stream flag returns an error instead of spinning.
//!
//! Reads use `frame_raw`, the camera's own bytes, so nothing is decoded or
//! copied twice before [`crate::convert`].

use crate::backend::{Backend, Problem, RawFrame, Stream};
use crate::format::{Offer, PixelFormat, pick};
use nokhwa::Camera;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};
use std::time::Instant;

/// The Media Foundation backend.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MediaFoundation;

impl Backend for MediaFoundation {
    fn list(&self) -> Result<Vec<String>, Problem> {
        let started = Instant::now();
        let cameras = nokhwa::query(ApiBackend::MediaFoundation)
            .map_err(|error| Problem::from_text(error.to_string()))?;
        log::debug!(
            "camera: {} listed in {:.1} ms",
            cameras.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        Ok(cameras
            .iter()
            .map(nokhwa::utils::CameraInfo::human_name)
            .collect())
    }

    fn open(&self, name: &str) -> Result<Box<dyn Stream>, Problem> {
        let started = Instant::now();
        let cameras = nokhwa::query(ApiBackend::MediaFoundation)
            .map_err(|error| Problem::from_text(error.to_string()))?;
        let index = cameras
            .iter()
            .find(|camera| camera.human_name() == name)
            .map(|camera| camera.index().clone())
            .ok_or_else(|| Problem::from_text(format!("\"{name}\" not found")))?;
        let any = RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);
        let mut camera = Camera::with_backend(index, any, ApiBackend::MediaFoundation)
            .map_err(|error| Problem::from_text(error.to_string()))?;
        let offers: Vec<Offer> = camera
            .compatible_camera_formats()
            .map_err(|error| Problem::from_text(error.to_string()))?
            .iter()
            .map(offer)
            .collect();
        let chosen = pick(&offers).ok_or_else(|| Problem {
            cause: crate::Cause::Unreadable,
            detail: "the camera offers no format Strobe reads".to_string(),
        })?;
        let exact =
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(chosen.width, chosen.height),
                frame_format(chosen.format),
                chosen.fps,
            )));
        camera
            .set_camera_requset(exact)
            .map_err(|error| Problem::from_text(error.to_string()))?;
        camera
            .open_stream()
            .map_err(|error| Problem::from_text(error.to_string()))?;
        // What the camera settled on, which a driver may round.
        let format = camera.camera_format();
        let settled = offer(&format);
        log::info!(
            "camera \"{name}\": {settled} of {} offered, open in {:.0} ms",
            offers.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        Ok(Box::new(Open {
            camera,
            offer: settled,
            last: Vec::new(),
        }))
    }
}

/// An open camera.
struct Open {
    camera: Camera,
    offer: Offer,
    /// The last picture's bytes.
    last: Vec<u8>,
}

impl Stream for Open {
    fn describe(&self) -> String {
        self.offer.to_string()
    }

    fn read(&mut self) -> Result<RawFrame<'_>, Problem> {
        let Offer {
            width,
            height,
            format,
            ..
        } = self.offer;
        // Media Foundation's binding copies the sample out and hands the
        // copy over (owned), so this moves it, no second copy.
        self.last = self
            .camera
            .frame_raw()
            .map_err(|error| Problem::from_text(error.to_string()))?
            .into_owned();
        Ok(RawFrame {
            format,
            width,
            height,
            data: &self.last,
        })
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        if let Err(error) = self.camera.stop_stream() {
            log::debug!("camera: stopping the stream: {error}");
        }
    }
}

fn offer(format: &CameraFormat) -> Offer {
    let pixel = match format.format() {
        FrameFormat::YUYV => PixelFormat::Yuy2,
        FrameFormat::NV12 => PixelFormat::Nv12,
        FrameFormat::MJPEG => PixelFormat::Mjpeg,
        FrameFormat::GRAY => PixelFormat::Gray,
        FrameFormat::RAWRGB => PixelFormat::Rgb,
        FrameFormat::RAWBGR => PixelFormat::Bgr,
    };
    Offer {
        width: format.width(),
        height: format.height(),
        format: pixel,
        fps: format.frame_rate(),
    }
}

fn frame_format(format: PixelFormat) -> FrameFormat {
    match format {
        PixelFormat::Yuy2 => FrameFormat::YUYV,
        PixelFormat::Nv12 => FrameFormat::NV12,
        PixelFormat::Mjpeg => FrameFormat::MJPEG,
        PixelFormat::Gray => FrameFormat::GRAY,
        PixelFormat::Rgb => FrameFormat::RAWRGB,
        PixelFormat::Bgr => FrameFormat::RAWBGR,
    }
}
