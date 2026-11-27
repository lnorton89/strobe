// ============================================================================
// Module:       strobe_output::texture
// Description:  A borrowed picture: the GL texture a sender may share without
//               reading it back, described the way a texture-sharing library
//               takes it.
//
// Dependencies: none
// ============================================================================

//! A texture lent to a sender, not given to it.
//!
//! Spout publishes the picture by sharing the GPU texture itself: nothing is
//! read back, nothing is encoded, and the cost of sending is the interop copy
//! the library makes (D570). To do that a sender needs three facts — the
//! texture's handle, its target and its size — and nothing else, and it needs
//! them **only while the call runs**: the handle belongs to the renderer's
//! owning type and is deleted when that target goes, on the same thread and
//! with the same GL context current.
//!
//! So this is a plain copyable description with no lifetime, and the rule is
//! the caller's: it is made and used inside the frame, and stored nowhere.
//! [`SharedTexture::id`] is a GL name, not an address; it means nothing on a
//! different context.

/// The GL target of a texture a sender may share: `GL_TEXTURE_2D`.
pub const TEXTURE_2D: u32 = 0x0DE1;

/// A picture to share as a texture: the GL name, its target and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedTexture {
    /// The GL texture name, valid while the renderer's target lives and the
    /// frame's context is current.
    pub id: u32,
    /// [`TEXTURE_2D`], the only target Strobe draws into.
    pub target: u32,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
}

impl SharedTexture {
    /// A 2D texture of `width` x `height`, named `id`.
    pub fn new(id: u32, width: u32, height: u32) -> Self {
        Self {
            id,
            target: TEXTURE_2D,
            width,
            height,
        }
    }

    /// Whether it names anything: a lost or unbuilt target has no handle.
    pub fn is_whole(&self) -> bool {
        self.id != 0 && self.width > 0 && self.height > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_texture_is_2d_and_needs_a_name_and_a_size() {
        let texture = SharedTexture::new(7, 1920, 1080);
        assert_eq!(texture.target, TEXTURE_2D, "Strobe draws into 2D targets");
        assert!(texture.is_whole(), "a name and a size are enough");
        assert!(!SharedTexture::new(0, 1920, 1080).is_whole(), "no name");
        assert!(!SharedTexture::new(7, 0, 1080).is_whole(), "no width");
        assert!(!SharedTexture::new(7, 1920, 0).is_whole(), "no height");
    }
}
