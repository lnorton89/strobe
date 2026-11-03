// ============================================================================
// Module:       strobe_capture::format
// Description:  Camera pixel formats and the choice among a camera's offers:
//               720p30 uncompressed first, MJPEG only when 720p is offered no
//               other way.
//
// Dependencies: std
// ============================================================================

//! Which of a camera's formats to ask for (M-04 proposal, step 2).
//!
//! Uncompressed YUY2 or NV12 costs 1.75 ms a 720p picture to convert on the
//! capture thread; MJPEG costs three times that through `image`. So
//! [`pick`] wants 1280x720 at 30 fps, uncompressed, and takes MJPEG only
//! when 720p is offered no other way. Without 720p it takes the size
//! nearest to it, uncompressed before compressed at that size. Among those,
//! the frame rate nearest 30, higher before lower.

use std::fmt;

/// How a camera lays out a picture's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// Packed 4:2:2, `Y0 U Y1 V`.
    Yuy2,
    /// A Y plane, then interleaved U and V at half size each way.
    Nv12,
    /// One JPEG per picture.
    Mjpeg,
    /// One byte of luma per pixel.
    Gray,
    /// Three bytes per pixel, R G B.
    Rgb,
    /// Three bytes per pixel, B G R.
    Bgr,
}

impl PixelFormat {
    /// Whether the camera compresses it.
    pub fn compressed(self) -> bool {
        match self {
            Self::Mjpeg => true,
            Self::Yuy2 | Self::Nv12 | Self::Gray | Self::Rgb | Self::Bgr => false,
        }
    }
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Yuy2 => "YUY2",
            Self::Nv12 => "NV12",
            Self::Mjpeg => "MJPEG",
            Self::Gray => "GRAY",
            Self::Rgb => "RGB",
            Self::Bgr => "BGR",
        })
    }
}

/// One format a camera offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel layout.
    pub format: PixelFormat,
    /// Frames per second.
    pub fps: u32,
}

impl fmt::Display for Offer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}x{} {} {} fps",
            self.width, self.height, self.format, self.fps
        )
    }
}

/// The size asked for.
pub const WANTED_SIZE: (u32, u32) = (1280, 720);
/// The frame rate asked for.
pub const WANTED_FPS: u32 = 30;

/// The offer to ask for, or `None` when there is none.
pub fn pick(offers: &[Offer]) -> Option<Offer> {
    offers.iter().copied().min_by_key(|offer| rank(*offer))
}

/// Smaller is better: how far the size is from 720p (and whether it is
/// 720p at all), compressed after uncompressed at a size, then how far the
/// rate is from 30 with higher rates first.
fn rank(offer: Offer) -> (bool, u64, bool, u32, bool) {
    let (w, h) = WANTED_SIZE;
    let exact = (offer.width, offer.height) == (w, h);
    let area = u64::from(offer.width) * u64::from(offer.height);
    let size_gap = area.abs_diff(u64::from(w) * u64::from(h));
    (
        !exact,
        size_gap,
        offer.format.compressed(),
        offer.fps.abs_diff(WANTED_FPS),
        offer.fps < WANTED_FPS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(width: u32, height: u32, format: PixelFormat, fps: u32) -> Offer {
        Offer {
            width,
            height,
            format,
            fps,
        }
    }

    #[test]
    fn uncompressed_720p30_wins() {
        let offers = [
            offer(1920, 1080, PixelFormat::Mjpeg, 30),
            offer(1280, 720, PixelFormat::Mjpeg, 30),
            offer(1280, 720, PixelFormat::Yuy2, 30),
            offer(640, 480, PixelFormat::Yuy2, 30),
        ];
        assert_eq!(
            pick(&offers),
            Some(offer(1280, 720, PixelFormat::Yuy2, 30)),
            "720p YUY2"
        );
    }

    #[test]
    fn mjpeg_only_when_720p_is_offered_no_other_way() {
        let offers = [
            offer(1280, 720, PixelFormat::Mjpeg, 30),
            offer(640, 480, PixelFormat::Yuy2, 30),
        ];
        assert_eq!(
            pick(&offers),
            Some(offer(1280, 720, PixelFormat::Mjpeg, 30)),
            "720p even compressed beats a smaller picture"
        );
    }

    #[test]
    fn without_720p_the_nearest_size() {
        let offers = [
            offer(1920, 1080, PixelFormat::Yuy2, 30),
            offer(640, 480, PixelFormat::Nv12, 30),
            offer(320, 240, PixelFormat::Yuy2, 30),
        ];
        assert_eq!(
            pick(&offers),
            Some(offer(640, 480, PixelFormat::Nv12, 30)),
            "640x480 is nearer 720p's area than 1080p"
        );
    }

    #[test]
    fn the_rate_nearest_30_higher_first() {
        let offers = [
            offer(1280, 720, PixelFormat::Yuy2, 10),
            offer(1280, 720, PixelFormat::Yuy2, 25),
            offer(1280, 720, PixelFormat::Yuy2, 35),
            offer(1280, 720, PixelFormat::Yuy2, 60),
        ];
        assert_eq!(
            pick(&offers).map(|o| o.fps),
            Some(35),
            "35 and 25 are as near; the higher wins"
        );
        assert_eq!(pick(&[]), None, "nothing offered");
    }
}
