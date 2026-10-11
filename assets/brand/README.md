# STROBE brand assets

This directory contains the initial **Parametric Flow** brand direction for STROBE.

## Current source asset

- `mark.svg` — initial vector concept for the continuous ribbon symbol.

This SVG is a design starting point, not the final production mark. Refine the silhouette and contour logic before generating release icons. In particular, make a simplified small-size variant instead of using the full-detail mark unchanged at 16 px.

## Planned exports

- `mark-mono.svg`
- `wordmark.svg`
- `lockup-horizontal.svg`
- `lockup-stacked.svg`
- `icon-16.png`, `icon-32.png`, `icon-64.png`, `icon-128.png`, `icon-256.png`, `icon-512.png`
- `icon.ico` with multiple resolutions
- `banner.png`
- `social-preview.png`

## Color tokens

| Token | Hex |
|---|---|
| Void | `#080B14` |
| Panel | `#101725` |
| Border | `#253247` |
| Text | `#F3F6FA` |
| Muted | `#9AA8BC` |
| Cyan | `#00E5FF` |
| Blue | `#3382F6` |
| Violet | `#8A55F7` |
| Magenta | `#EC4B99` |
| Slate | `#94A3B8` |

Primary gradient: cyan → blue → violet → magenta.

## Asset production rules

1. Keep vector artwork as the source of truth.
2. Generate raster exports from source rather than editing each size manually.
3. Use transparent backgrounds for standalone marks and opaque backgrounds for banners.
4. Export in sRGB.
5. Simplify contour detail at 32 px and below.
6. Test the mark on dark and light surfaces and at actual operating-system icon sizes.
7. Use real STROBE screenshots and shader renders in project documentation. Do not present concept mockups as actual application captures.

See [the full brand guidelines](../../docs/branding.md) for logo use, typography, color accessibility, product UI, voice, and implementation checklist.
