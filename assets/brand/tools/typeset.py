#!/usr/bin/env python3
# ============================================================================
# typeset.py: writes the brand's type-bearing SVG sources (R-06, D256).
#
# The wordmark and the lockups set "STROBE" and "SHADER VISUALIZER" in Space
# Grotesk (docs/branding.md section 4), converted to outlines, so every SVG
# here renders the same everywhere with no font installed: `cargo xtask
# brand` renders without system fonts on purpose. The font is not in the
# repository. Fetch the OFL-licensed files once, then run this:
#
#   curl -sSfLO https://cdn.jsdelivr.net/npm/@fontsource/space-grotesk@5.0.18/files/space-grotesk-latin-500-normal.woff
#   python3 assets/brand/tools/typeset.py space-grotesk-latin-500-normal.woff
#   cargo xtask brand
#
# Needs fontTools (`pip install fonttools`). The output is deterministic: the
# same font gives byte-identical SVGs.
# ============================================================================

import sys
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

BRAND = Path(__file__).resolve().parent.parent

# Colour tokens (assets/brand/README.md).
VOID = "#080B14"
BORDER = "#253247"
TEXT = "#F3F6FA"
MUTED = "#9AA8BC"

SPECTRUM = """    <linearGradient id="spectrum" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#00E5FF"/>
      <stop offset=".35" stop-color="#3382F6"/>
      <stop offset=".68" stop-color="#8A55F7"/>
      <stop offset="1" stop-color="#EC4B99"/>
    </linearGradient>"""

# The production ribbon: the application icon's heavy ribbon and its two
# contours, without the tile. Its own coordinates span about 76..398 by
# 82..402; RIBBON_BOX is that box.
RIBBON_BOX = (76.0, 82.0, 322.0, 320.0)
RIBBON = """<g fill="none" stroke="{stroke}" stroke-linecap="round">
    <g stroke-width="5" opacity=".55">
      <path d="M136 283 C112 193 181 120 252 136 C313 150 363 213 345 269 C326 320 258 366 201 340 C153 318 132 275 169 235 C209 191 272 193 308 229 C335 255 318 292 283 307 C241 325 202 300 207 266 C212 237 248 222 272 242"/>
      <path d="M100 256 C76 158 160 82 255 103 C331 121 398 200 375 277 C352 341 264 402 183 365 C124 337 98 271 145 216"/>
    </g>
    <path d="M117 269 C93 175 166 99 252 119 C318 135 381 205 360 273 C339 334 260 386 192 355 C136 330 115 273 157 226 C202 176 279 180 322 223 C355 258 333 304 290 322 C241 343 190 307 197 264 C204 221 251 207 282 232 C306 252 298 283 274 295" stroke-width="24"/>
  </g>"""


class Type:
    """One font, set as outlines."""

    def __init__(self, path):
        self.font = TTFont(path)
        self.glyphs = self.font.getGlyphSet()
        self.cmap = self.font.getBestCmap()
        self.upem = self.font["head"].unitsPerEm
        self.cap = self.font["OS/2"].sCapHeight

    def width(self, text, size, tracking):
        """The set width of `text` at `size` px with `tracking` em between
        letters (none after the last)."""
        scale = size / self.upem
        advance = sum(self.glyphs[self.cmap[ord(c)]].width for c in text) * scale
        return advance + tracking * size * (len(text) - 1)

    def path(self, text, size, tracking, x, baseline):
        """One SVG path `d` for `text`, its baseline at `baseline`, starting
        at `x`, in a y-down coordinate system."""
        scale = size / self.upem
        pen = SVGPathPen(self.glyphs)
        for c in text:
            glyph = self.glyphs[self.cmap[ord(c)]]
            # Font units are y-up: flip, then place.
            transform = TransformPen(pen, (scale, 0, 0, -scale, x, baseline))
            glyph.draw(transform)
            x += glyph.width * scale + tracking * size
        return pen.getCommands()


def number(value):
    return f"{value:.2f}".rstrip("0").rstrip(".")


def ribbon(x, y, height, stroke="url(#spectrum)"):
    """The ribbon scaled to `height`, its box's top-left at (x, y)."""
    bx, by, bw, bh = RIBBON_BOX
    s = height / bh
    return (
        f'<g transform="translate({number(x)} {number(y)}) scale({number(s)}) '
        f'translate({number(-bx)} {number(-by)})">\n  '
        + RIBBON.format(stroke=stroke)
        + "\n</g>"
    ), bw * s


def svg(width, height, title, desc, body, defs=SPECTRUM, extra=""):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{number(width)}" height="{number(height)}" '
        f'viewBox="0 0 {number(width)} {number(height)}" role="img" aria-labelledby="title desc"{extra}>\n'
        f"  <title id=\"title\">{title}</title>\n"
        f"  <desc id=\"desc\">{desc}</desc>\n"
        f"  <defs>\n{defs}\n  </defs>\n"
        f"{body}\n</svg>\n"
    )


# The type, in px at the wordmark's size. The cap height of the wordmark is
# the unit everything else is measured in (the guide's clear space is "the
# height of the wordmark's S").
WORD_SIZE = 100.0
WORD_TRACKING = 0.14
DESC_RATIO = 0.22
DESC_TRACKING = 0.32


def wordmark(t):
    width = t.width("STROBE", WORD_SIZE, WORD_TRACKING)
    cap = t.cap * WORD_SIZE / t.upem
    pad = cap
    d = t.path("STROBE", WORD_SIZE, WORD_TRACKING, pad, pad + cap)
    body = f'  <path fill="{TEXT}" d="{d}"/>'
    return svg(
        width + 2 * pad,
        cap + 2 * pad,
        "STROBE wordmark",
        "STROBE in Space Grotesk Medium, tracked, as outlines; for dark backgrounds.",
        body,
        defs="",
    )


def lockup_horizontal(t):
    cap = t.cap * WORD_SIZE / t.upem
    pad = cap
    desc_size = WORD_SIZE * DESC_RATIO
    desc_cap = t.cap * desc_size / t.upem
    # The ribbon is as tall as the wordmark and the descriptor together,
    # with the gap between them.
    gap = cap * 0.55
    block = cap + gap + desc_cap
    mark_h = block * 1.5
    mark, mark_w = ribbon(pad, pad, mark_h)
    text_x = pad + mark_w + cap * 0.6
    top = pad + (mark_h - block) / 2
    word = t.path("STROBE", WORD_SIZE, WORD_TRACKING, text_x, top + cap)
    desc = t.path("SHADER VISUALIZER", desc_size, DESC_TRACKING, text_x, top + cap + gap + desc_cap)
    width = text_x + max(
        t.width("STROBE", WORD_SIZE, WORD_TRACKING),
        t.width("SHADER VISUALIZER", desc_size, DESC_TRACKING),
    ) + pad
    body = (
        f"{mark}\n"
        f'  <path fill="{TEXT}" d="{word}"/>\n'
        f'  <path fill="{MUTED}" d="{desc}"/>'
    )
    return svg(
        width,
        mark_h + 2 * pad,
        "STROBE horizontal lockup",
        "The Parametric Flow ribbon on the left, STROBE and SHADER VISUALIZER on the right; for dark backgrounds.",
        body,
    ), (width, mark_h + 2 * pad, body)


def lockup_stacked(t):
    cap = t.cap * WORD_SIZE / t.upem
    pad = cap
    desc_size = WORD_SIZE * DESC_RATIO
    desc_cap = t.cap * desc_size / t.upem
    word_w = t.width("STROBE", WORD_SIZE, WORD_TRACKING)
    desc_w = t.width("SHADER VISUALIZER", desc_size, DESC_TRACKING)
    width = max(word_w, desc_w) + 2 * pad
    mark_h = cap * 2.6
    _, mark_w = ribbon(0, 0, mark_h)
    mark, _ = ribbon((width - mark_w) / 2, pad, mark_h)
    word_top = pad + mark_h + cap * 0.7
    word = t.path("STROBE", WORD_SIZE, WORD_TRACKING, (width - word_w) / 2, word_top + cap)
    desc_base = word_top + cap + cap * 0.6 + desc_cap
    desc = t.path("SHADER VISUALIZER", desc_size, DESC_TRACKING, (width - desc_w) / 2, desc_base)
    body = (
        f"{mark}\n"
        f'  <path fill="{TEXT}" d="{word}"/>\n'
        f'  <path fill="{MUTED}" d="{desc}"/>'
    )
    return svg(
        width,
        desc_base + pad,
        "STROBE stacked lockup",
        "The Parametric Flow ribbon above STROBE, with SHADER VISUALIZER below; for dark backgrounds.",
        body,
    )


def mark_mono():
    mark, mark_w = ribbon(0, 0, 320.0, stroke="currentColor")
    return svg(
        mark_w,
        320.0,
        "STROBE mark, one colour",
        "The Parametric Flow ribbon in a single colour: currentColor, near-black unless a page sets it.",
        mark,
        defs="",
        extra=f' color="{VOID}"',
    )


def banner(horizontal):
    """README banner, 1280x320: the horizontal lockup on a quiet dark field
    with one restrained spectral glow behind the mark (guide section 6)."""
    lw, lh, body = horizontal
    width, height = 1280.0, 320.0
    scale = (height * 0.62) / lh
    x = (width - lw * scale) / 2
    y = (height - lh * scale) / 2
    defs = SPECTRUM + """
    <radialGradient id="glow" cx=".5" cy=".5" r=".5">
      <stop offset="0" stop-color="#3382F6" stop-opacity=".22"/>
      <stop offset="1" stop-color="#3382F6" stop-opacity="0"/>
    </radialGradient>"""
    glow_r = lh * scale * 0.75
    glow_x = x + lh * scale * 0.42
    content = (
        f'  <rect width="{number(width)}" height="{number(height)}" fill="{VOID}"/>\n'
        f'  <circle cx="{number(glow_x)}" cy="{number(height / 2)}" r="{number(glow_r)}" fill="url(#glow)"/>\n'
        f'  <rect x=".5" y=".5" width="{number(width - 1)}" height="{number(height - 1)}" fill="none" stroke="{BORDER}"/>\n'
        f'  <g transform="translate({number(x)} {number(y)}) scale({number(scale)})">\n{body}\n  </g>'
    )
    return svg(width, height, "STROBE banner", "The horizontal lockup on a dark field, for the README.", content, defs=defs)


def social(t):
    """Social preview, 1280x640: one real render filling the frame, the
    compact mark and the wordmark on a quiet band (guide section 6). This
    SVG is the overlay only: `cargo xtask brand` draws it over
    `social-render.png`, a frame of a library scene from `strobe-bench`."""
    width, height = 1280.0, 640.0
    cap = t.cap * 72 / t.upem
    band_h = cap * 3.2
    band_y = height - band_h
    mark, mark_w = ribbon(56, band_y + (band_h - cap * 2.0) / 2, cap * 2.0)
    word = t.path("STROBE", 72, WORD_TRACKING, 56 + mark_w + cap * 0.5, band_y + (band_h + cap) / 2)
    defs = SPECTRUM + """
    <linearGradient id="fade" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#080B14" stop-opacity="0"/>
      <stop offset=".45" stop-color="#080B14" stop-opacity=".82"/>
      <stop offset="1" stop-color="#080B14" stop-opacity=".94"/>
    </linearGradient>"""
    content = (
        f'  <rect y="{number(band_y - band_h)}" width="{number(width)}" height="{number(band_h * 2)}" fill="url(#fade)"/>\n'
        f"{mark}\n"
        f'  <path fill="{TEXT}" d="{word}"/>'
    )
    return svg(
        width,
        height,
        "STROBE social preview overlay",
        "The mark and wordmark on a fading band, drawn over social-render.png by cargo xtask brand.",
        content,
        defs=defs,
    )


def main():
    if len(sys.argv) < 2:
        sys.exit("usage: typeset.py <space-grotesk-500.woff|ttf>")
    t = Type(sys.argv[1])
    (BRAND / "wordmark.svg").write_text(wordmark(t))
    horizontal, parts = lockup_horizontal(t)
    (BRAND / "lockup-horizontal.svg").write_text(horizontal)
    (BRAND / "lockup-stacked.svg").write_text(lockup_stacked(t))
    (BRAND / "mark-mono.svg").write_text(mark_mono())
    (BRAND / "banner.svg").write_text(banner(parts))
    (BRAND / "social-preview.svg").write_text(social(t))


if __name__ == "__main__":
    main()
