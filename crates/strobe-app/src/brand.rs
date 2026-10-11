// ============================================================================
// Module:       strobe_app::brand
// Description:  The window and taskbar icon, decoded from the brand PNG that
//               `cargo xtask brand` renders from assets/brand/icon.svg.
//
// Dependencies: image (PNG decode), winit (Icon)
// ============================================================================

//! The application icon (`docs/branding.md`, sections 2 and 10).
//!
//! The PNG is compiled in, so the icon cannot go missing beside the
//! executable. 48 px is the size Windows asks for at 150% scaling in the
//! taskbar and Alt-Tab, the T480's usual setting; Windows scales it for the
//! other places it shows a window icon. It is drawn from the full icon
//! source rather than the simplified one, which is only for 32 px and below.
//!
//! Decoding happens once per window pair (at launch, and when a lost
//! context rebuilds the windows), never per frame. A decode failure costs
//! the icon, not the launch: the window opens with the platform default.

use winit::window::Icon;

/// The icon PNG, as `cargo xtask brand` writes it.
const ICON_PNG: &[u8] = include_bytes!("../../../assets/brand/icon-48.png");

/// The window icon, or `None` (the platform default) if it does not decode.
pub fn window_icon() -> Option<Icon> {
    match decode() {
        Ok(icon) => Some(icon),
        Err(error) => {
            log::warn!("brand: window icon unavailable, using the default: {error}");
            None
        }
    }
}

fn decode() -> anyhow::Result<Icon> {
    let image =
        image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_in_icon_decodes_at_48_px() -> anyhow::Result<()> {
        let image =
            image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)?.into_rgba8();
        assert_eq!(image.dimensions(), (48, 48), "icon-48.png is 48x48");
        decode()?;
        Ok(())
    }

    #[test]
    fn the_icon_has_a_transparent_corner_and_an_opaque_tile() -> anyhow::Result<()> {
        let image =
            image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)?.into_rgba8();
        let corner = image.get_pixel(0, 0).0[3];
        let centre = image.get_pixel(24, 24).0[3];
        assert_eq!(corner, 0, "the rounded tile leaves the corner transparent");
        assert_eq!(centre, 255, "the tile is opaque behind the mark");
        Ok(())
    }
}
