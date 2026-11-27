// ============================================================================
// Module:       strobe_output::pixel
// Description:  The two layouts a shared output wants, BGRA (NDI, Spout) and
//               UYVY 4:2:2 (NDI's default), packed from the frame's RGBA8.
//
// Dependencies: crate::Frame
// ============================================================================

//! Packing the picture for a sender.
//!
//! Strobe's picture is RGBA8 with the top row first, which is what a GL
//! readback of the output gives once the rows are turned over. Both ways of
//! sharing it want something else:
//!
//! - **Spout** shares the GL texture itself, so nothing here runs for it
//!   (D570). This module is for the routes that need pixels on the CPU.
//! - **NDI** takes BGRA or UYVY. UYVY is half the bytes (2 per pixel against
//!   4), which at 1080p60 is 249 MB/s instead of 498 MB/s, so it is the
//!   format to offer first; BGRA is the lossless one to fall back to.
//!
//! Conversion is BT.709, limited range (16-235 luma, 16-240 chroma), which
//! is what NDI's video frames declare, and 4:2:2 takes one chroma pair for
//! each pair of pixels, averaged so a hard colour edge does not crawl.
//! Everything is integer arithmetic into a caller's reused buffer: no
//! allocation, no `glGet*`, nothing that waits.

use crate::Frame;

/// Bytes a BGRA picture of `width` x `height` needs.
pub fn bgra_len(width: u32, height: u32) -> usize {
    (width as usize) * (height as usize) * 4
}

/// Bytes a UYVY picture of `width` x `height` needs at `stride` bytes a row.
///
/// `stride` is the sender's row pitch, which may be padded: NDI takes the
/// stride it is given and most senders use `width * 2`, the default here.
/// A stride too small for the picture is raised to what it needs, which for
/// an odd width is one pixel more: 4:2:2 takes a pixel pair, so a 3-pixel
/// row is 8 bytes, not 6.
pub fn uyvy_len(width: u32, height: u32, stride: usize) -> usize {
    let needed = width.div_ceil(2) as usize * 4;
    stride.max(needed) * (height as usize)
}

/// Packs `frame` as BGRA with a reused `out`.
///
/// BGRA is RGBA with the red and blue bytes swapped, which is the byte
/// order a Windows DIB and NDI's BGRA frame both want. The alpha byte is
/// copied rather than forced: `Alpha Channel` (D248) has nowhere to go
/// until an output carries it, and this is that output's first reader.
///
/// Returns whether `frame`'s buffer held one byte per channel per pixel.
pub fn to_bgra(frame: &Frame, out: &mut Vec<u8>) -> bool {
    let needed = bgra_len(frame.width, frame.height);
    let Some(source) = frame.rgba.get(..needed) else {
        return false;
    };
    out.clear();
    out.reserve(needed);
    let (texels, _) = source.as_chunks::<4>();
    for texel in texels {
        out.extend_from_slice(&[texel[2], texel[1], texel[0], texel[3]]);
    }
    true
}

/// Packs `frame` as UYVY 4:2:2 with a reused `out`, `stride` bytes a row.
///
/// Every pair of pixels becomes `U Y0 V Y1`. The chroma is the pair's mean
/// colour, so a pair that spans an edge lands between the two rather than
/// taking the left pixel's. An odd width repeats the last pixel for the
/// last chroma sample. Padded bytes at the end of a row are zeroed, so a
/// sender never puts a previous frame's tail on the wire.
///
/// Returns whether `frame`'s buffer held one byte per channel per pixel.
pub fn to_uyvy(frame: &Frame, out: &mut Vec<u8>, stride: usize) -> bool {
    let width = frame.width as usize;
    let height = frame.height as usize;
    let needed = (width * height).checked_mul(4);
    let Some(source) = needed.and_then(|needed| frame.rgba.get(..needed)) else {
        return false;
    };
    // 4:2:2 is packed in pixel pairs, so an odd width's last row byte is
    // the repeated pixel's, not padding.
    let row_bytes = stride.max(width.div_ceil(2) * 4);
    out.clear();
    out.resize(row_bytes * height, 0);
    if width == 0 {
        return true;
    }
    let pairs = width.div_ceil(2);
    for row in 0..height {
        let source_row = &source[row * width * 4..(row + 1) * width * 4];
        let out_row = &mut out[row * row_bytes..(row + 1) * row_bytes];
        for pair in 0..pairs {
            let left = pair * 2;
            let right = (left + 1).min(width - 1);
            let a = &source_row[left * 4..left * 4 + 4];
            let b = &source_row[right * 4..right * 4 + 4];
            // The pair's mean colour, in 1/2 steps, so no rounding drift.
            let sum = |offset: usize| u32::from(a[offset]) + u32::from(b[offset]);
            let (cb, cr) = chroma(
                sum(0) as f32 / 2.0,
                sum(1) as f32 / 2.0,
                sum(2) as f32 / 2.0,
            );
            let y0 = luma(a[0], a[1], a[2]);
            let y1 = luma(b[0], b[1], b[2]);
            let at = pair * 4;
            out_row[at] = cb;
            out_row[at + 1] = y0;
            out_row[at + 2] = cr;
            out_row[at + 3] = y1;
        }
    }
    true
}

/// BT.709 limited-range luma of one RGB triple: 16 is black, 235 white.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    let value = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b);
    (16.0 + value * 219.0 / 255.0).round().clamp(16.0, 235.0) as u8
}

/// BT.709 limited-range chroma of one RGB triple: 128 is neutral, 16-240
/// the range.
fn chroma(r: f32, g: f32, b: f32) -> (u8, u8) {
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let cb = 128.0 + (b - y) * 224.0 / 255.0 / 1.8556;
    let cr = 128.0 + (r - y) * 224.0 / 255.0 / 1.5748;
    (
        cb.round().clamp(16.0, 240.0) as u8,
        cr.round().clamp(16.0, 240.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// One uniform picture, top row first.
    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
        Frame {
            width,
            height,
            rgba: rgba.repeat(width as usize * height as usize),
        }
    }

    /// The UYVY bytes of a 2x1 picture of `rgba`, as `(U, Y0, V, Y1)`.
    fn pair_of(rgba: [u8; 4]) -> Result<[u8; 4]> {
        let frame = solid(2, 1, rgba);
        let mut out = Vec::new();
        assert!(to_uyvy(&frame, &mut out, 4), "a 2x1 picture packs");
        let bytes: [u8; 4] = out
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("UYVY of 2 pixels is 4 bytes, got {}", out.len()))?;
        Ok(bytes)
    }

    #[test]
    fn bgra_swaps_red_and_blue_and_keeps_alpha() -> Result<()> {
        let frame = Frame {
            width: 2,
            height: 1,
            rgba: vec![10, 20, 30, 40, 50, 60, 70, 80],
        };
        let mut out = Vec::new();
        assert!(to_bgra(&frame, &mut out), "two pixels pack");
        assert_eq!(
            out,
            vec![30, 20, 10, 40, 70, 60, 50, 80],
            "BGRA keeps every channel, red and blue swapped"
        );
        Ok(())
    }

    #[test]
    fn a_short_frame_is_refused_rather_than_packed() {
        let frame = Frame {
            width: 4,
            height: 4,
            rgba: vec![0; 8],
        };
        let mut out = vec![1];
        assert!(!to_bgra(&frame, &mut out), "8 bytes cannot fill 4x4 RGBA");
        assert!(!to_uyvy(&frame, &mut out, 8), "nor can it fill UYVY");
    }

    #[test]
    fn uyvy_is_bt709_limited_range_for_the_primaries() -> Result<()> {
        // Values from the BT.709 equations, 8-bit limited range: red and
        // blue are the extremes of Cr and Cb.
        let cases: [([u8; 4], u8, u8, u8); 3] = [
            ([255, 0, 0, 255], 63, 102, 240),
            ([0, 255, 0, 255], 173, 42, 26),
            ([0, 0, 255, 255], 32, 240, 118),
        ];
        for (rgba, y, cb, cr) in cases {
            let [u, y0, v, y1] = pair_of(rgba)?;
            assert!(
                y0.abs_diff(y) <= 1 && y1.abs_diff(y) <= 1,
                "{rgba:?}: luma {y0}/{y1} should be {y}"
            );
            assert!(u.abs_diff(cb) <= 1, "{rgba:?}: Cb {u} should be {cb}");
            assert!(v.abs_diff(cr) <= 1, "{rgba:?}: Cr {v} should be {cr}");
        }
        Ok(())
    }

    #[test]
    fn black_and_white_sit_at_the_ends_of_the_range() -> Result<()> {
        let black = pair_of([0, 0, 0, 255])?;
        assert_eq!(black, [128, 16, 128, 16], "black is 16, neutral chroma");
        let white = pair_of([255, 255, 255, 255])?;
        assert_eq!(white, [128, 235, 128, 235], "white is 235");
        Ok(())
    }

    #[test]
    fn a_pair_takes_the_mean_of_its_two_pixels() -> Result<()> {
        let frame = Frame {
            width: 2,
            height: 1,
            rgba: vec![0, 0, 0, 255, 255, 255, 255, 255],
        };
        let mut out = Vec::new();
        assert!(to_uyvy(&frame, &mut out, 4), "two pixels pack");
        assert_eq!(out[1], 16, "the first pixel's own luma is kept");
        assert_eq!(out[3], 235, "and the second's");
        assert!(
            out[0].abs_diff(128) <= 1 && out[2].abs_diff(128) <= 1,
            "grey's chroma is neutral, even across a black-to-white edge: {out:?}"
        );
        Ok(())
    }

    #[test]
    fn an_odd_width_repeats_its_last_pixel() -> Result<()> {
        let frame = solid(3, 1, [255, 0, 0, 255]);
        let mut out = Vec::new();
        assert!(to_uyvy(&frame, &mut out, 6), "three pixels pack");
        assert_eq!(out.len(), 8, "two pairs, the second half repeated");
        assert_eq!(out[4], out[0], "the repeated pair keeps its chroma");
        assert_eq!(out[7], out[3], "and its luma");
        Ok(())
    }

    #[test]
    fn a_padded_stride_zeroes_the_tail_of_every_row() -> Result<()> {
        let frame = solid(2, 2, [255, 0, 0, 255]);
        let mut out = Vec::new();
        assert!(to_uyvy(&frame, &mut out, 8), "two pixels pack into 8 bytes");
        assert_eq!(out.len(), 16, "two rows of the padded stride");
        assert_eq!(&out[4..8], &[0, 0, 0, 0], "the first row's tail is zero");
        assert_eq!(&out[12..16], &[0, 0, 0, 0], "and the second's");
        assert_eq!(uyvy_len(2, 2, 4), 8, "an unpadded stride is the minimum");
        assert_eq!(uyvy_len(2, 2, 8), 16, "a padded one is kept");
        Ok(())
    }

    #[test]
    fn a_frame_packs_into_a_buffer_that_grows_and_shrinks_with_it() -> Result<()> {
        let mut out = vec![9; 999];
        assert!(
            to_bgra(&solid(1, 1, [1, 2, 3, 4]), &mut out),
            "one pixel packs"
        );
        assert_eq!(out, vec![3, 2, 1, 4], "the stale bytes are gone");
        assert_eq!(
            bgra_len(1920, 1080),
            1920 * 1080 * 4,
            "1080p BGRA is four bytes a pixel"
        );
        assert_eq!(
            uyvy_len(1920, 1080, 0),
            1920 * 1080 * 2,
            "1080p UYVY is two, the whole reason to prefer it"
        );
        Ok(())
    }

    #[test]
    fn an_empty_picture_packs_to_nothing() -> Result<()> {
        let empty = Frame {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
        let mut out = vec![7];
        assert!(to_bgra(&empty, &mut out), "nothing to pack is not an error");
        assert!(out.is_empty(), "and it leaves an empty buffer");
        assert!(to_uyvy(&empty, &mut out, 0), "likewise");
        assert!(out.is_empty(), "still empty");
        Ok(())
    }
}
