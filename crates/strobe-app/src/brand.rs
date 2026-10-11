// ============================================================================
// Module:       strobe_app::brand
// Description:  The window and taskbar icons, decoded from the brand PNGs
//               that `cargo xtask brand` renders from assets/brand.
//
// Dependencies: image (PNG decode), winit (Icon), strobe-gfx (with_icons)
// ============================================================================

//! The application icons (`docs/branding.md`, sections 2 and 10).
//!
//! The PNGs are compiled in, so they cannot go missing beside the
//! executable. Windows reads two icons per window: the big one (taskbar and
//! Alt-Tab) is the 48 px render of the full icon, which is what it asks for
//! at 150% scaling, the T480's usual setting; the small one (a decorated
//! window's title bar) is the 24 px render of the simplified icon.
//! [`strobe_gfx::with_icons`] puts each where it belongs; elsewhere the big
//! one serves for both. T480 Task 15 found the big icon unset when only
//! winit's `with_window_icon` was used, which sets the small one alone.
//!
//! Decoding happens once per window pair (at launch, and when a lost
//! context rebuilds the windows), never per frame. A decode failure costs
//! the icon, not the launch: the window opens with the platform default.

use winit::window::{Icon, WindowAttributes};

/// The taskbar and Alt-Tab icon, as `cargo xtask brand` writes it.
const BIG_PNG: &[u8] = include_bytes!("../../../assets/brand/icon-48.png");

/// The title-bar icon, from the simplified source.
const SMALL_PNG: &[u8] = include_bytes!("../../../assets/brand/icon-24.png");

/// Gives a window the Strobe icons.
#[must_use]
pub fn with_icons(attributes: WindowAttributes) -> WindowAttributes {
    strobe_gfx::with_icons(attributes, icon(SMALL_PNG), icon(BIG_PNG))
}

/// One icon, or `None` (the platform default) if it does not decode.
fn icon(png: &[u8]) -> Option<Icon> {
    match decode(png) {
        Ok(icon) => Some(icon),
        Err(error) => {
            log::warn!("brand: window icon unavailable, using the default: {error}");
            None
        }
    }
}

fn decode(png: &[u8]) -> anyhow::Result<Icon> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_in_icons_decode_at_their_sizes() -> anyhow::Result<()> {
        for (png, side) in [(BIG_PNG, 48), (SMALL_PNG, 24)] {
            let image =
                image::load_from_memory_with_format(png, image::ImageFormat::Png)?.into_rgba8();
            assert_eq!(image.dimensions(), (side, side), "icon is {side}x{side}");
            decode(png)?;
        }
        Ok(())
    }

    #[test]
    fn the_icon_has_a_transparent_corner_and_an_opaque_tile() -> anyhow::Result<()> {
        let image =
            image::load_from_memory_with_format(BIG_PNG, image::ImageFormat::Png)?.into_rgba8();
        let corner = image.get_pixel(0, 0).0[3];
        let centre = image.get_pixel(24, 24).0[3];
        assert_eq!(corner, 0, "the rounded tile leaves the corner transparent");
        assert_eq!(centre, 255, "the tile is opaque behind the mark");
        Ok(())
    }
}
