// ============================================================================
// Module:       xtask::brand
// Description:  `cargo xtask brand`: renders the application icon PNGs, the
//               Windows .ico, the banner, the lockups and the social
//               preview from the vector sources in assets/brand.
//
// Dependencies: anyhow, resvg (usvg + tiny-skia), std::fs
// ============================================================================

//! `cargo xtask brand [--check]`.
//!
//! The brand guide (`docs/branding.md`, section 8) makes the SVG the
//! source of truth and every raster a derivative of it, so nobody edits a
//! PNG by hand and no size drifts from the others. This command is that
//! rule as code:
//!
//! - sizes up to [`SMALL_MAX`] come from `icon-small.svg`, the simplified
//!   ribbon (no contours, innermost turn dropped, heavier stroke), because
//!   the guide says a shrunken master turns to mush below 32 px;
//! - larger sizes come from `icon.svg`;
//! - `icon.ico` packs the Windows sizes as PNG entries, which every Windows
//!   since Vista reads;
//! - the banner, the social preview and the two lockups are rendered at
//!   their own aspect ratio ([`PICTURES`]). Their type is outlines already
//!   (`assets/brand/tools/typeset.py`), so no font is read here either.
//!
//! `--check` renders everything in memory and fails if a committed file is
//! missing or differs, so a source edited without regenerating is a CI
//! failure rather than a stale icon. The output is deterministic for a
//! given `Cargo.lock`: no text, no system fonts, nothing read from the
//! machine.

use anyhow::{Context as _, Result, anyhow, bail};
use std::fs;
use std::path::Path;

/// Where the sources and the outputs live, relative to the workspace root.
const DIR: &str = "assets/brand";

/// Every PNG size written, in pixels. 192 and 512 are the two a web app
/// manifest names, for the remote and projector pages (D517).
const SIZES: [u32; 9] = [16, 24, 32, 48, 64, 128, 192, 256, 512];

/// The largest size drawn from the simplified source.
const SMALL_MAX: u32 = 32;

/// The sizes packed into `icon.ico`: what Windows asks for at 100% to 200%
/// scaling, plus the 256 Explorer uses for large icons.
const ICO_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];

/// Non-square pictures rendered at their own aspect ratio (D256): source,
/// output and width in pixels. The README banner and the social preview at
/// the sizes GitHub uses, each lockup for places that take no SVG, and the
/// full-bleed maskable icon a phone's launcher crops to its own shape
/// (D517).
const PICTURES: [(&str, &str, u32); 4] = [
    ("banner.svg", "banner.png", 1280),
    ("lockup-horizontal.svg", "lockup-horizontal.png", 960),
    ("lockup-stacked.svg", "lockup-stacked.png", 640),
    ("icon-maskable.svg", "icon-maskable-512.png", 512),
];

/// The social preview: an SVG overlay drawn over a real render, a frame of
/// a library scene from `strobe-bench` (the guide wants real output, not a
/// mockup). Decoding the render here keeps resvg free of raster decoders.
const SOCIAL: (&str, &str, &str) = (
    "social-preview.svg",
    "social-render.png",
    "social-preview.png",
);

/// Renders every derivative and writes it, or with `check`, compares it to
/// what is committed.
pub(crate) fn run(check: bool) -> Result<()> {
    let dir = Path::new(DIR);
    let small = load(&dir.join("icon-small.svg"))?;
    let large = load(&dir.join("icon.svg"))?;

    let mut outputs: Vec<(String, Vec<u8>)> = Vec::new();
    let mut ico_entries: Vec<(u32, Vec<u8>)> = Vec::new();
    for size in SIZES {
        let tree = if size <= SMALL_MAX { &small } else { &large };
        let png = render(tree, size)?;
        if ICO_SIZES.contains(&size) {
            ico_entries.push((size, png.clone()));
        }
        outputs.push((format!("icon-{size}.png"), png));
    }
    outputs.push(("icon.ico".to_string(), ico(&ico_entries)?));
    for (source, output, width) in PICTURES {
        let tree = load(&dir.join(source))?;
        outputs.push((output.to_string(), render_wide(&tree, width)?));
    }
    let (overlay, render, output) = SOCIAL;
    outputs.push((
        output.to_string(),
        render_over(&load(&dir.join(overlay))?, &dir.join(render))?,
    ));

    let mut stale = Vec::new();
    for (name, bytes) in &outputs {
        let path = dir.join(name);
        if check {
            let committed = fs::read(&path).unwrap_or_default();
            if committed != *bytes {
                stale.push(name.as_str());
            }
        } else {
            fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
            println!("wrote {}", path.display());
        }
    }
    if !stale.is_empty() {
        bail!(
            "{} out of date with the SVG sources: {}. Run `cargo xtask brand` and commit the result.",
            DIR,
            stale.join(", ")
        );
    }
    if check {
        println!("brand: {} files match their sources", outputs.len());
    }
    Ok(())
}

/// Parses one SVG source.
fn load(path: &Path) -> Result<resvg::usvg::Tree> {
    let data = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .with_context(|| format!("parsing {}", path.display()))
}

/// Renders a tree to a `size` x `size` PNG with a transparent background.
fn render(tree: &resvg::usvg::Tree, size: u32) -> Result<Vec<u8>> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)
        .ok_or_else(|| anyhow!("could not allocate a {size}x{size} pixmap"))?;
    let source = tree.size();
    #[expect(
        clippy::cast_precision_loss,
        reason = "icon sizes are at most 512, exact in f32"
    )]
    let side = size as f32;
    let transform =
        resvg::tiny_skia::Transform::from_scale(side / source.width(), side / source.height());
    resvg::render(tree, transform, &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .with_context(|| format!("encoding the {size} px PNG"))
}

/// Renders a tree `width` pixels wide at its own aspect ratio, with a
/// transparent background wherever the source draws nothing.
fn render_wide(tree: &resvg::usvg::Tree, width: u32) -> Result<Vec<u8>> {
    let source = tree.size();
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "picture sizes are a few thousand pixels at most, exact in f32"
    )]
    let (scale, height) = {
        let scale = width as f32 / source.width();
        (scale, (source.height() * scale).round().max(1.0) as u32)
    };
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow!("could not allocate a {width}x{height} pixmap"))?;
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .with_context(|| format!("encoding the {width}x{height} PNG"))
}

/// Draws `overlay` over the PNG at `background`, at the background's size,
/// which must be the overlay's own.
fn render_over(overlay: &resvg::usvg::Tree, background: &Path) -> Result<Vec<u8>> {
    let data = fs::read(background).with_context(|| format!("reading {}", background.display()))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::decode_png(&data)
        .with_context(|| format!("decoding {}", background.display()))?;
    let size = overlay.size();
    #[expect(
        clippy::cast_precision_loss,
        reason = "picture sizes are a few thousand pixels at most, exact in f32"
    )]
    let (width, height) = (pixmap.width() as f32, pixmap.height() as f32);
    if (width - size.width()).abs() > 0.5 || (height - size.height()).abs() > 0.5 {
        bail!(
            "{} is {width}x{height}, but the overlay is {}x{}",
            background.display(),
            size.width(),
            size.height()
        );
    }
    resvg::render(
        overlay,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .with_context(|| format!("encoding over {}", background.display()))
}

/// Packs PNG images into an ICO container.
///
/// The format is a 6-byte header, a 16-byte directory entry per image,
/// then the image data. A PNG entry stores width and height 0 for 256.
fn ico(entries: &[(u32, Vec<u8>)]) -> Result<Vec<u8>> {
    const HEADER: usize = 6;
    const ENTRY: usize = 16;
    let count = u16::try_from(entries.len()).context("too many ICO entries")?;
    let mut out = Vec::new();
    out.extend_from_slice(&0_u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1_u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = HEADER + ENTRY * entries.len();
    for (size, png) in entries {
        let side = if *size >= 256 {
            0
        } else {
            u8::try_from(*size).context("ICO side")?
        };
        out.push(side); // width
        out.push(side); // height
        out.push(0); // palette size
        out.push(0); // reserved
        out.extend_from_slice(&1_u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32_u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&u32::try_from(png.len()).context("ICO size")?.to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset).context("ICO offset")?.to_le_bytes());
        offset += png.len();
    }
    for (_, png) in entries {
        out.extend_from_slice(png);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_directory_points_at_each_image() -> Result<()> {
        let a = vec![1_u8, 2, 3];
        let b = vec![4_u8; 5];
        let out = ico(&[(16, a.clone()), (256, b.clone())])?;
        assert_eq!(&out[0..6], &[0, 0, 1, 0, 2, 0], "header");
        assert_eq!(out[6], 16, "first entry width");
        assert_eq!(out[22], 0, "256 is stored as 0");
        let first = u32::from_le_bytes([out[18], out[19], out[20], out[21]]);
        assert_eq!(first, 38, "first image follows header and two entries");
        let second = u32::from_le_bytes([out[34], out[35], out[36], out[37]]);
        assert_eq!(second, 41, "second image follows the first");
        assert_eq!(&out[38..41], a.as_slice(), "first image data");
        assert_eq!(&out[41..], b.as_slice(), "second image data");
        Ok(())
    }
}
