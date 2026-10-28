# STROBE brand assets

This directory holds the **Parametric Flow** brand for STROBE. The rules
for using it are in [the brand guidelines](../../docs/branding.md).

## Sources (edit these)

| File | What it is |
|---|---|
| `mark.svg` | The concept ribbon mark with its fine contours. It is a design starting point, not the final production mark. |
| `icon.svg` | The application icon for 48 px and up: the ribbon with two contours, on the dark tile. |
| `icon-small.svg` | The application icon for 32 px and below: no contours, the innermost turn dropped, a heavier stroke (guide section 2: "do not merely shrink the detailed master"). |
| `icon-maskable.svg` | The application icon as a web app's maskable icon: no tile corners, the ribbon inside the central 80% circle. |
| `mark-mono.svg` | The production ribbon in one colour, `currentColor` (near-black unless a page sets it), for print and low-colour use. |
| `social-render.png` | A real frame for the social preview: `12-silicon-atlas` from `strobe-bench --size 1280x640 --frames 3 --warmup 120`. |

### Written by `tools/typeset.py` (edit the script, not these)

The type is Space Grotesk Medium (SIL Open Font License 1.1, copyright
2020 The Space Grotesk Project Authors) set as outlines, so nothing here
needs a font to render. The font itself is not in the repository; the
script's header says where to fetch it.

| File | What it is |
|---|---|
| `wordmark.svg` | STROBE, tracked, in Text (`#F3F6FA`), for dark backgrounds. |
| `lockup-horizontal.svg` | The ribbon left of STROBE and SHADER VISUALIZER. |
| `lockup-stacked.svg` | The ribbon above STROBE, SHADER VISUALIZER below. |
| `banner.svg` | The horizontal lockup on a dark field with one quiet glow, 1280x320. |
| `social-preview.svg` | The overlay for the social preview: a fading band, the ribbon and STROBE. |

The lockups use the application icon's heavy ribbon without its tile, not
the fine-contour `mark.svg`, whose 2.3 px strokes vanish at lockup sizes.

## Generated (never edit by hand)

`cargo xtask brand` renders these from the sources above.
`cargo xtask brand --check` fails if any of them differs from a fresh
render. Both `cargo xtask ci` and CI run the check.

- `icon-16.png`, `icon-24.png`, `icon-32.png`: from `icon-small.svg`.
- `icon-48.png`, `icon-64.png`, `icon-128.png`, `icon-192.png`, `icon-256.png`, `icon-512.png`: from `icon.svg`.
- `icon-maskable-512.png`: from `icon-maskable.svg`, the ribbon on a full-bleed Void square inside the central circle a phone's launcher crops to; the remote and projector pages' install icon (D517).
- `icon.ico`: 16, 24, 32, 48, 64 and 256, stored as PNG entries.
- `banner.png` (1280 wide), `lockup-horizontal.png` (960) and `lockup-stacked.png` (640): from their SVGs, at their own aspect ratio.
- `social-preview.png`: `social-preview.svg` drawn over `social-render.png`, 1280x640.

The app compiles `icon-48.png` in as the window and taskbar icon
(`strobe_app::brand`). The title bar draws the small ribbon as vector
geometry in the theme's accent (D190).

## Still to make

- A look at the icon in Explorer and on the taskbar by a person, on Windows (R-06).

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
