/*
 * The social card, as a Metaplate plate (D557).
 *
 * The cards are ordinary files. `metaplate render` writes `og-image.jpg` beside
 * each route's page before the build, through ./cards.render.tsx, so a static
 * host serves them with the right type and no server ever runs the renderer.
 *
 * The layout is the guide's social preview (docs/branding.md section 7): one
 * real render, a compact mark and the wordmark, and no feature list or tiny
 * copy. Colours come from lib/constants.ts because Satori has no CSS custom
 * properties.
 *
 * This module is imported by the pages too, for `og.metadata`. That only builds
 * a URL: Metaplate loads Satori and Resvg when something renders, and
 * next.config.ts keeps them out of the bundle.
 */

import { readdirSync } from "node:fs";
import path from "node:path";
import { readImageAsset, type ImageAsset } from "metaplate/assets";
import { fontsourceFontLoader } from "metaplate/fonts";
import { createNodeOg } from "metaplate/node";
import { BRAND, OG_SIZE, SPECTRUM } from "../constants";
import { RIBBON_CONTOURS, RIBBON_PATH, RIBBON_VIEWBOX } from "../../components/brand/StrobeMark";
import type { CardCopy } from "./cards";

/** The width of the scene frame on the right of the card. */
const RENDER_WIDTH = 500;

/** The ribbon, with the spectrum gradient. */
function Mark({ size }: { size: number }) {
  return (
    <svg width={size} height={size} viewBox={RIBBON_VIEWBOX}>
      <defs>
        <linearGradient id="spectrum" x1="0" y1="0" x2="1" y2="1">
          {SPECTRUM.map((stop) => (
            <stop key={stop.offset} offset={stop.offset} stopColor={stop.color} />
          ))}
        </linearGradient>
      </defs>
      <g fill="none" stroke="url(#spectrum)" strokeLinecap="round">
        <g strokeWidth={5} opacity={0.55}>
          {RIBBON_CONTOURS.map((d) => (
            <path key={d} d={d} />
          ))}
        </g>
        <path d={RIBBON_PATH} strokeWidth={24} />
      </g>
    </svg>
  );
}

/** A library scene's own thumbnail, whatever its extension. */
function readSceneFrame(scene: string): Promise<ImageAsset> {
  // Relative to the site, where `metaplate render` and the tests both run.
  const folder = path.join("..", "scenes", `${scene}.synScene`);
  const file = readdirSync(folder).find((name) => /^thumbnail\.(png|jpe?g)$/.test(name));
  if (!file) throw new Error(`${folder} has no thumbnail for its social card`);
  return readImageAsset(path.join(folder, file));
}

const spectrum = `linear-gradient(90deg, ${SPECTRUM.map(
  (stop) => `${stop.color} ${stop.offset * 100}%`,
).join(", ")})`;

export const og = createNodeOg<CardCopy>({
  alt: (copy) => copy.alt,
  size: OG_SIZE,
  // A real file name, so a static host knows the type from the extension.
  imagePath: "og-image.jpg",
  /*
   * JPEG, because the card is half a photograph: as PNG one reached 916 kB,
   * over WhatsApp's 600 kB limit. sharp is loaded here rather than above so a
   * page that only wants a URL never does.
   */
  output: {
    format: "jpeg",
    encode: async ({ pixels, width, height }) => {
      const { default: sharp } = await import("sharp");
      return sharp(pixels, { raw: { width, height, channels: 4 } })
        .jpeg({ quality: 82, mozjpeg: true })
        .toBuffer();
    },
  },
  /*
   * Crawlers cache an image by its URL, so a changed card gets a new `?v=`.
   * Raise `revision` when the layout below changes what unchanged copy looks
   * like: the copy is all the token can see of a page.
   */
  version: "copy",
  revision: "1",
  fonts: fontsourceFontLoader([
    { font: "space-grotesk", weight: 500 },
    { font: "space-grotesk", weight: 600 },
    { font: "inter", weight: 400 },
    { font: "jetbrains-mono", weight: 500 },
  ]),
  // A card with text off the canvas or no font for it fails the build.
  guards: { missingGlyphs: "error", overflow: "error", blank: "error" },
  component: async (copy) => {
    const frame = await readSceneFrame(copy.scene);
    return (
      <div
        style={{
          display: "flex",
          width: "100%",
          height: "100%",
          background: BRAND.ground,
        }}
      >
        <div
          style={{
            boxSizing: "border-box",
            display: "flex",
            flexDirection: "column",
            justifyContent: "space-between",
            width: OG_SIZE.width - RENDER_WIDTH,
            height: "100%",
            padding: "56px 60px 52px",
          }}
        >
          <div style={{ display: "flex", alignItems: "center" }}>
            <Mark size={64} />
            <span
              style={{
                fontFamily: "Space Grotesk",
                fontWeight: 500,
                fontSize: 34,
                letterSpacing: 5,
                marginLeft: 18,
                color: BRAND.text,
              }}
            >
              STROBE
            </span>
          </div>
          <div style={{ display: "flex", flexDirection: "column" }}>
            <span
              style={{
                fontFamily: "JetBrains Mono",
                fontWeight: 500,
                fontSize: 17,
                letterSpacing: 2.4,
                textTransform: "uppercase",
                color: BRAND.cyan,
                marginBottom: 20,
              }}
            >
              {copy.eyebrow}
            </span>
            <span
              style={{
                fontFamily: "Space Grotesk",
                fontWeight: 600,
                fontSize: 54,
                lineHeight: 1.08,
                letterSpacing: -1.2,
                color: BRAND.text,
              }}
            >
              {copy.title}
            </span>
            <span
              style={{
                fontFamily: "Inter",
                fontWeight: 400,
                fontSize: 24,
                lineHeight: 1.4,
                color: BRAND.secondary,
                marginTop: 20,
              }}
            >
              {copy.description}
            </span>
          </div>
          <div style={{ display: "flex", height: 3, backgroundImage: spectrum }} />
        </div>
        {/* eslint-disable-next-line @next/next/no-img-element, jsx-a11y/alt-text -- Satori, not the DOM: the card's alt is the plate's. */}
        <img
          src={frame.src}
          width={RENDER_WIDTH}
          height={OG_SIZE.height}
          style={{ objectFit: "cover" }}
        />
      </div>
    );
  },
});
