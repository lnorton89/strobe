# STROBE brand assets

This directory holds the **Parametric Flow** brand for STROBE. The rules
for using it are in [the brand guidelines](../../docs/branding.md).

## Sources (edit these)

| File | What it is |
|---|---|
| `mark.svg` | The concept ribbon mark with its fine contours. It is a design starting point, not the final production mark. |
| `icon.svg` | The application icon for 48 px and up: the ribbon with two contours, on the dark tile. |
| `icon-small.svg` | The application icon for 32 px and below: no contours, the innermost turn dropped, a heavier stroke (guide section 2: "do not merely shrink the detailed master"). |

## Generated (never edit by hand)

`cargo xtask brand` renders these from the sources above.
`cargo xtask brand --check` fails if any of them differs from a fresh
render. Both `cargo xtask ci` and CI run the check.

- `icon-16.png`, `icon-24.png`, `icon-32.png`: from `icon-small.svg`.
- `icon-48.png`, `icon-64.png`, `icon-128.png`, `icon-256.png`, `icon-512.png`: from `icon.svg`.
- `icon.ico`: 16, 24, 32, 48, 64 and 256, stored as PNG entries.

The app compiles `icon-48.png` in as the window and taskbar icon
(`strobe_app::brand`). The title bar draws the small ribbon as vector
geometry in the theme's accent (D190).

## Still to make

- `mark-mono.svg`
- `wordmark.svg`
- `lockup-horizontal.svg`, `lockup-stacked.svg`
- `banner.png`, `social-preview.png`
- the icon embedded as a Windows `.exe` resource (with the release packaging, Q12)

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

Primary gradient: cyan → blue → violet → magenta. The app's default theme
(`strobe` in `crates/strobe-app/assets/themes.toml`) uses Void, Panel,
Border, Text, Muted and Cyan as its tokens.

## Asset production rules

1. Keep vector artwork as the source of truth.
2. Generate raster exports from source rather than editing each size manually.
3. Use transparent backgrounds for standalone marks and opaque backgrounds for banners.
4. Export in sRGB.
5. Simplify contour detail at 32 px and below.
6. Test the mark on dark and light surfaces and at actual operating-system icon sizes.
7. Use real STROBE screenshots and shader renders in project documentation. Do not present concept mockups as actual application captures.
