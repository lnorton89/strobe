// ============================================================================
// Module:       strobe_output::frame
// Description:  One finished picture on its way to a sender: RGBA8, top row
//               first, and the size it is.
//
// Dependencies: none
// ============================================================================

//! One finished picture, in the order a sender takes it.
//!
//! The app's output is pooled and owned by the renderer; a sender may keep a
//! frame past the call that handed it over (NDI's asynchronous path does),
//! so what crosses this boundary is a copy in a plain `Vec<u8>`. The buffer
//! is the host's to reuse: [`Frame::rgba`] is handed in and cleared, never
//! allocated per frame once it has grown.
//!
//! Rows run top first, which is the order every sender wants and the
//! opposite of GL's; the readback turns them over once (D570).

/// A picture to share: RGBA8, top row first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA8, top row first.
    pub rgba: Vec<u8>,
}

impl Frame {
    /// An empty frame, for a sender to be started before the first picture.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A black frame of `width` x `height`, with `rgba` reused as its buffer.
    ///
    /// A sender that starts on a black frame has something to advertise
    /// before the show asks for its first picture, so a receiver can connect
    /// to it during setup instead of finding a sender that is not there yet.
    pub fn black(width: u32, height: u32, rgba: &mut Vec<u8>) -> Self {
        let bytes = (width as usize) * (height as usize) * 4;
        rgba.clear();
        rgba.resize(bytes, 0);
        // Opaque black: an alpha of 0 would read as a transparent frame on
        // an output that carries alpha (D248).
        let (texels, _) = rgba.as_chunks_mut::<4>();
        for texel in texels {
            texel[3] = 255;
        }
        Self {
            width,
            height,
            rgba: std::mem::take(rgba),
        }
    }

    /// Whether the size and the buffer agree, so a sender may read it.
    pub fn is_whole(&self) -> bool {
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        expected.is_some_and(|bytes| self.rgba.len() == bytes)
    }

    /// The frame's size, for a sender that changed size mid-show.
    pub fn size(&self) -> [u32; 2] {
        [self.width, self.height]
    }
}

impl Frame {
    /// The frame lent as [`Pixels`], RGBA.
    pub fn pixels(&self) -> Pixels<'_> {
        Pixels {
            width: self.width,
            height: self.height,
            layout: Layout::Rgba,
            data: &self.rgba,
        }
    }
}

/// How a picture's bytes are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Four bytes a pixel: red, green, blue, alpha.
    Rgba,
    /// UYVY 4:2:2, BT.709 limited range: four bytes a pixel pair (U, Y0, V,
    /// Y1), the layout NDI prefers. The width is even.
    Uyvy,
}

impl Layout {
    /// Bytes one row of `width` pixels takes.
    pub fn row_bytes(self, width: u32) -> usize {
        match self {
            Self::Rgba => (width as usize) * 4,
            Self::Uyvy => (width as usize).div_ceil(2) * 4,
        }
    }
}

/// A picture lent to a sender for the length of one call: the bytes stay the
/// host's, rows top first. A sender that keeps a frame past the call copies
/// it (NDI's asynchronous send copies into buffers of its own).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pixels<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// How [`Self::data`] is laid out.
    pub layout: Layout,
    /// The rows, top first, with no padding between them.
    pub data: &'a [u8],
}

impl Pixels<'_> {
    /// Whether the size, the layout and the bytes agree.
    pub fn is_whole(&self) -> bool {
        let expected = self
            .layout
            .row_bytes(self.width)
            .checked_mul(self.height as usize);
        let even = self.layout != Layout::Uyvy || self.width.is_multiple_of(2);
        even && expected.is_some_and(|bytes| self.data.len() == bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_black_frame_is_opaque_and_the_right_size() {
        let mut buffer = vec![9; 4];
        let frame = Frame::black(2, 2, &mut buffer);
        assert_eq!(frame.size(), [2, 2], "the size asked for");
        assert_eq!(frame.rgba.len(), 16, "four pixels of four bytes");
        assert!(
            frame
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|texel| texel == &[0, 0, 0, 255]),
            "black with an opaque alpha, not a transparent one"
        );
        assert!(buffer.is_empty(), "the host's buffer was taken, not copied");
    }

    #[test]
    fn a_frame_must_hold_one_texel_per_pixel() {
        let mut frame = Frame {
            width: 2,
            height: 2,
            rgba: vec![0; 16],
        };
        assert!(frame.is_whole(), "16 bytes is 2x2 RGBA8");
        frame.rgba.pop();
        assert!(!frame.is_whole(), "15 bytes is not");
        let huge = Frame {
            width: u32::MAX,
            height: u32::MAX,
            rgba: Vec::new(),
        };
        assert!(!huge.is_whole(), "an impossible size is not whole");
    }

    #[test]
    fn uyvy_pixels_take_four_bytes_a_pair_and_an_even_width() {
        let data = [0_u8; 16];
        let pixels = |width, layout| Pixels {
            width,
            height: 2,
            layout,
            data: &data,
        };
        assert!(pixels(4, Layout::Uyvy).is_whole(), "4x2 UYVY is 16 bytes");
        assert!(pixels(2, Layout::Rgba).is_whole(), "2x2 RGBA is 16 bytes");
        assert!(
            !pixels(3, Layout::Uyvy).is_whole(),
            "an odd UYVY width has no whole last pair"
        );
        assert_eq!(Layout::Uyvy.row_bytes(1920), 3840, "half of RGBA's bytes");
    }
}
