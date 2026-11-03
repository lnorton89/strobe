// ============================================================================
// Module:       strobe_capture::convert
// Description:  A camera picture (YUY2, NV12, MJPEG, grey, RGB, BGR; top row
//               first) to RGBA8 bottom row first, into a reused buffer.
//
// Dependencies: image (MJPEG); crate::format, crate::backend
// ============================================================================

//! Pixel conversion, on the capture thread (M-04 proposal, step 2).
//!
//! Every converter writes RGBA8 with the **bottom row first**, GL's order,
//! so the picture is upright in `syn_Media` without another pass, and
//! writes into the caller's buffer, which is reused from picture to
//! picture. YUV is BT.601 limited range in integer arithmetic, as USB
//! cameras send it. MJPEG decodes through `image` (pure Rust, already
//! linked), never nokhwa's mozjpeg.
//!
//! The spike measured 1.75 ms for a 720p YUY2 or NV12 picture on a Ryzen 5
//! 5600X, and 5.6-6.3 ms for MJPEG (`docs/research/m04-webcam.md`).

use crate::backend::RawFrame;
use crate::format::PixelFormat;
use std::fmt;

/// A picture that could not be converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// Fewer bytes than the size and format need.
    Short {
        /// Bytes needed.
        needed: usize,
        /// Bytes there.
        got: usize,
    },
    /// A size YUY2 or NV12 cannot have (odd width, or odd height for NV12).
    Size {
        /// Width.
        width: u32,
        /// Height.
        height: u32,
    },
    /// The JPEG did not decode, or decoded to another size.
    Jpeg(String),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short { needed, got } => {
                write!(f, "a picture of {got} bytes, {needed} needed")
            }
            Self::Size { width, height } => write!(f, "a {width}x{height} picture"),
            Self::Jpeg(why) => write!(f, "a JPEG that did not decode: {why}"),
        }
    }
}

/// Converts `frame` into `out`, resizing `out` to `width * height * 4`.
///
/// # Errors
///
/// The bytes do not fit the size and format; `out` is left as it was sized.
pub fn to_rgba(frame: &RawFrame<'_>, out: &mut Vec<u8>) -> Result<(), ConvertError> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    match frame.format {
        PixelFormat::Yuy2 => {
            if !w.is_multiple_of(2) {
                return Err(size(frame));
            }
            check(frame.data, w * h * 2)?;
            out.resize(w * h * 4, 255);
            yuy2(frame.data, w, h, out);
        }
        PixelFormat::Nv12 => {
            if !w.is_multiple_of(2) || !h.is_multiple_of(2) {
                return Err(size(frame));
            }
            check(frame.data, w * h * 3 / 2)?;
            out.resize(w * h * 4, 255);
            nv12(frame.data, w, h, out);
        }
        PixelFormat::Gray => {
            check(frame.data, w * h)?;
            out.resize(w * h * 4, 255);
            packed(frame.data, w, h, 1, out, |p, o| {
                if let (&[y], [r, g, b, _]) = (p, o) {
                    (*r, *g, *b) = (y, y, y);
                }
            });
        }
        PixelFormat::Rgb => {
            check(frame.data, w * h * 3)?;
            out.resize(w * h * 4, 255);
            packed(frame.data, w, h, 3, out, |p, o| {
                if let (&[pr, pg, pb], [r, g, b, _]) = (p, o) {
                    (*r, *g, *b) = (pr, pg, pb);
                }
            });
        }
        PixelFormat::Bgr => {
            check(frame.data, w * h * 3)?;
            out.resize(w * h * 4, 255);
            packed(frame.data, w, h, 3, out, |p, o| {
                if let (&[pb, pg, pr], [r, g, b, _]) = (p, o) {
                    (*r, *g, *b) = (pr, pg, pb);
                }
            });
        }
        PixelFormat::Mjpeg => mjpeg(frame, out)?,
    }
    Ok(())
}

fn size(frame: &RawFrame<'_>) -> ConvertError {
    ConvertError::Size {
        width: frame.width,
        height: frame.height,
    }
}

fn check(data: &[u8], needed: usize) -> Result<(), ConvertError> {
    if data.len() < needed {
        return Err(ConvertError::Short {
            needed,
            got: data.len(),
        });
    }
    Ok(())
}

/// Rows of `w` pixels of `bytes` each, top first, to RGBA bottom first.
fn packed(
    src: &[u8],
    w: usize,
    h: usize,
    bytes: usize,
    out: &mut [u8],
    pixel: impl Fn(&[u8], &mut [u8]),
) {
    for (row, line) in src.chunks_exact(w * bytes).take(h).enumerate() {
        let Some(dst) = out.get_mut(row_range(h - 1 - row, w)) else {
            return;
        };
        for (p, o) in line.chunks_exact(bytes).zip(dst.as_chunks_mut::<4>().0) {
            pixel(p, o);
        }
    }
}

fn row_range(row: usize, w: usize) -> std::ops::Range<usize> {
    row * w * 4..(row + 1) * w * 4
}

fn yuy2(src: &[u8], w: usize, h: usize, out: &mut [u8]) {
    for (row, line) in src.chunks_exact(w * 2).take(h).enumerate() {
        let Some(dst) = out.get_mut(row_range(h - 1 - row, w)) else {
            return;
        };
        for (&[y0, u, y1, v], pair) in line
            .as_chunks::<4>()
            .0
            .iter()
            .zip(dst.as_chunks_mut::<8>().0)
        {
            let (left, right) = pair.split_at_mut(4);
            yuv_pixel(y0, u, v, left);
            yuv_pixel(y1, u, v, right);
        }
    }
}

fn nv12(src: &[u8], w: usize, h: usize, out: &mut [u8]) {
    let (luma, chroma) = src.split_at(w * h);
    for row in 0..h {
        let (Some(ys), Some(uvs), Some(dst)) = (
            luma.get(row * w..(row + 1) * w),
            chroma.get((row / 2) * w..(row / 2 + 1) * w),
            out.get_mut(row_range(h - 1 - row, w)),
        ) else {
            return;
        };
        for ((&[y0, y1], &[u, v]), pair) in ys
            .as_chunks::<2>()
            .0
            .iter()
            .zip(uvs.as_chunks::<2>().0)
            .zip(dst.as_chunks_mut::<8>().0)
        {
            let (left, right) = pair.split_at_mut(4);
            yuv_pixel(y0, u, v, left);
            yuv_pixel(y1, u, v, right);
        }
    }
}

/// BT.601 limited range to RGB, 8.8 fixed point.
fn yuv_pixel(y: u8, u: u8, v: u8, out: &mut [u8]) {
    let c = (i32::from(y) - 16) * 298;
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    let clamp = |x: i32| u8::try_from(((x + 128) >> 8).clamp(0, 255)).unwrap_or(u8::MAX);
    if let [r, g, b, _] = out {
        *r = clamp(c + 409 * e);
        *g = clamp(c - 100 * d - 208 * e);
        *b = clamp(c + 516 * d);
    }
}

fn mjpeg(frame: &RawFrame<'_>, out: &mut Vec<u8>) -> Result<(), ConvertError> {
    let decoded = image::load_from_memory_with_format(frame.data, image::ImageFormat::Jpeg)
        .map_err(|error| ConvertError::Jpeg(error.to_string()))?
        .into_rgba8();
    let (w, h) = (decoded.width() as usize, decoded.height() as usize);
    if (decoded.width(), decoded.height()) != (frame.width, frame.height) {
        return Err(ConvertError::Jpeg(format!(
            "{w}x{h}, not the {}x{} the camera said",
            frame.width, frame.height
        )));
    }
    out.resize(w * h * 4, 255);
    for (row, line) in decoded.as_raw().chunks_exact(w * 4).enumerate() {
        if let Some(dst) = out.get_mut(row_range(h - 1 - row, w)) {
            dst.copy_from_slice(line);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn frame(format: PixelFormat, width: u32, height: u32, data: &[u8]) -> RawFrame<'_> {
        RawFrame {
            format,
            width,
            height,
            data,
        }
    }

    fn pixel(out: &[u8], i: usize) -> [u8; 4] {
        let mut p = [0; 4];
        if let Some(src) = out.get(i * 4..i * 4 + 4) {
            p.copy_from_slice(src);
        }
        p
    }

    #[test]
    fn yuy2_is_flipped_and_full_range() -> Result<()> {
        // 2x2: top row white, bottom row black.
        let data = [235, 128, 235, 128, 16, 128, 16, 128];
        let mut out = Vec::new();
        to_rgba(&frame(PixelFormat::Yuy2, 2, 2, &data), &mut out)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert_eq!(out.len(), 16, "RGBA for 4 pixels");
        assert_eq!(pixel(&out, 0), [0, 0, 0, 255], "bottom row first: black");
        assert_eq!(pixel(&out, 2), [255, 255, 255, 255], "then the top: white");
        Ok(())
    }

    #[test]
    fn nv12_reads_its_chroma_plane() -> Result<()> {
        // 2x2 of Y 81, U 90, V 240: BT.601 red.
        let data = [81, 81, 81, 81, 90, 240];
        let mut out = Vec::new();
        to_rgba(&frame(PixelFormat::Nv12, 2, 2, &data), &mut out)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let [r, g, b, a] = pixel(&out, 3);
        assert!(
            r > 240 && g < 20 && b < 20 && a == 255,
            "red: {r} {g} {b} {a}"
        );
        Ok(())
    }

    #[test]
    fn packed_formats_keep_their_channels() -> Result<()> {
        let mut out = Vec::new();
        to_rgba(&frame(PixelFormat::Bgr, 1, 1, &[1, 2, 3]), &mut out)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert_eq!(pixel(&out, 0), [3, 2, 1, 255], "BGR swapped");
        to_rgba(&frame(PixelFormat::Rgb, 1, 1, &[1, 2, 3]), &mut out)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert_eq!(pixel(&out, 0), [1, 2, 3, 255], "RGB as is");
        to_rgba(&frame(PixelFormat::Gray, 1, 2, &[10, 20]), &mut out)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert_eq!(pixel(&out, 0), [20, 20, 20, 255], "grey, flipped");
        Ok(())
    }

    #[test]
    fn short_or_odd_pictures_are_refused() {
        let mut out = Vec::new();
        assert_eq!(
            to_rgba(&frame(PixelFormat::Yuy2, 2, 2, &[0; 7]), &mut out),
            Err(ConvertError::Short { needed: 8, got: 7 }),
            "one byte short"
        );
        assert_eq!(
            to_rgba(&frame(PixelFormat::Nv12, 3, 2, &[0; 9]), &mut out),
            Err(ConvertError::Size {
                width: 3,
                height: 2
            }),
            "odd width"
        );
        assert!(out.is_empty(), "nothing written on refusal");
    }

    #[test]
    fn mjpeg_decodes_upright() -> Result<()> {
        // 16x16: top half white, bottom half black.
        let mut picture = image::RgbImage::new(16, 16);
        for (_, y, p) in picture.enumerate_pixels_mut() {
            *p = if y < 8 {
                image::Rgb([255, 255, 255])
            } else {
                image::Rgb([0, 0, 0])
            };
        }
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95).encode_image(&picture)?;
        let mut out = Vec::new();
        to_rgba(&frame(PixelFormat::Mjpeg, 16, 16, &jpeg), &mut out)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert_eq!(out.len(), 16 * 16 * 4, "RGBA");
        assert!(pixel(&out, 0)[0] < 30, "bottom row first: black");
        assert!(pixel(&out, 16 * 15)[0] > 225, "top row last: white");
        let wrong = to_rgba(&frame(PixelFormat::Mjpeg, 8, 8, &jpeg), &mut out);
        assert!(
            matches!(wrong, Err(ConvertError::Jpeg(_))),
            "a size the camera did not say"
        );
        Ok(())
    }
}
