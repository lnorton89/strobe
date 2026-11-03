/*
 * Values that were previously spelled out at each use site.
 *
 * Colours live in globals.css as CSS custom properties and are referenced
 * through Tailwind tokens everywhere they can be. The hexes below exist only
 * for the one place CSS cannot reach: Satori, which renders the social images
 * outside a browser and so has no custom properties.
 *
 * Keeping them in one table means the palette has exactly two definitions —
 * this file and globals.css — and tests/unit/palette.test.ts checks that they
 * agree.
 */

/** The brand palette (docs/branding.md section 3), for contexts with no CSS. */
export const BRAND = {
  ground: "#080B14",
  panel: "#101725",
  hairline: "#253247",
  secondary: "#9AA8BC",
  label: "#8A98AE",
  text: "#F3F6FA",
  cyan: "#00E5FF",
  blue: "#3382F6",
  violet: "#8A55F7",
  magenta: "#EC4B99",
} as const;

export type BrandPalette = Record<keyof typeof BRAND, string>;

/** The primary gradient's stops, as the guide gives them. */
export const SPECTRUM = [
  { offset: 0, color: BRAND.cyan },
  { offset: 0.35, color: BRAND.blue },
  { offset: 0.68, color: BRAND.violet },
  { offset: 1, color: BRAND.magenta },
] as const;

/* ---------------------------------------------------------------- layout -- */

/** The page gutter and maximum content width, shared by every section. */
export const CONTAINER = "mx-auto max-w-[1400px] px-6 sm:px-10";

/* ------------------------------------------------------------------ type -- */

/** Icon sizes: 24 for cards, 20 for section rules, 16 inline. */
export const ICON = {
  section: 20,
  card: 24,
  inline: 16,
  small: 13,
} as const;

/* --------------------------------------------------------------- social -- */

/** Social images are the standard 1.91:1 card. */
export const OG_SIZE = { width: 1200, height: 630 } as const;
export const OG_CONTENT_TYPE = "image/png";
