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
}
