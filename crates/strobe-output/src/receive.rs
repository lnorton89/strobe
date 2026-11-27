// ============================================================================
// Module:       strobe_output::receive
// Description:  What a Spout or NDI receiver hands back: one picture's size
//               and byte order, in the caller's reused buffer.
//
// Dependencies: std
// ============================================================================

//! The shape of a received picture (D575).
//!
//! A receiver writes into a buffer the caller owns and reuses, so steady
//! state allocates nothing; what comes back is only what the caller needs to
//! read it. Rows are always top first and tightly packed, four bytes a pixel.

/// The order of a received pixel's four bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// R G B A.
    Rgba,
    /// B G R A: NDI's own order.
    Bgra,
}

/// One received picture, in the caller's buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Received {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The byte order.
    pub order: Order,
}

impl Received {
    /// The bytes the picture takes in the buffer.
    pub fn len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Whether it has no pixels.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
