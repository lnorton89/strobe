import { ImageResponse } from "next/og";
import { BRAND, OG_SIZE, SPECTRUM } from "@/lib/constants";
import { loadOgFonts } from "@/lib/og-fonts";
import { BADGES, REPO_URL } from "@/lib/site";
import {
  RIBBON_CONTOURS,
  RIBBON_PATH,
  RIBBON_VIEWBOX,
} from "@/components/brand/StrobeMark";

/*
 * Social images, built to the guide's release-graphic rules
 * (docs/branding.md, sections 7 and 10): the horizontal lockup top-left on a
 * dark field, the metadata strip top-right, the title and description in the
 * lower half, and the primary gradient once, as a compact rule at the foot.
 * "Version and release title must remain more readable than decoration."
 *
 * The faces come from @fontsource at build time; see lib/og-fonts.ts for why
 * they are not vendored here.
 */

export const ogSize = OG_SIZE;

/** The ribbon, drawn with the spectrum gradient for Satori. */
function OgMark({ size }: { size: number }) {
  return (
    <svg width={size} height={size} viewBox={RIBBON_VIEWBOX}>
      <defs>
        <linearGradient id="og-spectrum" x1="0" y1="0" x2="1" y2="1">
          {SPECTRUM.map((stop) => (
            <stop key={stop.offset} offset={stop.offset} stopColor={stop.color} />
          ))}
        </linearGradient>
      </defs>
      <g fill="none" stroke="url(#og-spectrum)" strokeLinecap="round">
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

/** The horizontal lockup: the ribbon, STROBE, and the descriptor under it. */
function OgLockup({ cap }: { cap: number }) {
  return (
    <div style={{ display: "flex", alignItems: "center" }}>
      <OgMark size={cap * 1.85} />
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          marginLeft: cap * 0.42,
        }}
      >
        <span
          style={{
            fontFamily: "Space Grotesk",
            fontWeight: 500,
            fontSize: cap,
            lineHeight: 1,
            letterSpacing: cap * 0.14,
            color: BRAND.text,
          }}
        >
          STROBE
        </span>
        <span
          style={{
            fontFamily: "Space Grotesk",
            fontWeight: 500,
            fontSize: cap * 0.22,
            lineHeight: 1,
            letterSpacing: cap * 0.22 * 0.32,
            color: BRAND.secondary,
            marginTop: cap * 0.22 * 0.7,
          }}
        >
          SHADER VISUALIZER
        </span>
      </div>
    </div>
  );
}

export async function renderOgImage({
  eyebrow,
  title,
  description,
}: {
  eyebrow: string;
  title: string;
  description: string;
}) {
  const fonts = await loadOgFonts();
  const spectrum = `linear-gradient(90deg, ${SPECTRUM.map(
    (stop) => `${stop.color} ${stop.offset * 100}%`,
  ).join(", ")})`;

  return new ImageResponse(
    (
      <div
        style={{
          width: OG_SIZE.width,
          height: OG_SIZE.height,
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between",
          background: BRAND.ground,
          // One quiet spectral pool, top right, away from the type (guide
          // section 7: "restrained spectral glow").
          backgroundImage:
            "radial-gradient(circle at 96% 0%, rgba(138, 85, 247, 0.22), rgba(8, 11, 20, 0) 55%)",
          padding: "56px 72px",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <OgLockup cap={46} />
          <span
            style={{
              fontFamily: "JetBrains Mono",
              fontWeight: 500,
              fontSize: 15,
              letterSpacing: 2.4,
              textTransform: "uppercase",
              color: BRAND.label,
            }}
          >
            {BADGES.join(" · ")}
          </span>
        </div>

        <div style={{ display: "flex", flexDirection: "column" }}>
          <span
            style={{
              fontFamily: "JetBrains Mono",
              fontWeight: 500,
              fontSize: 16,
              letterSpacing: 2.4,
              textTransform: "uppercase",
              color: BRAND.cyan,
              marginBottom: 22,
            }}
          >
            {eyebrow}
          </span>
          <span
            style={{
              display: "flex",
              fontFamily: "Space Grotesk",
              fontWeight: 600,
              fontSize: 64,
              lineHeight: 1.06,
              letterSpacing: -1.6,
              color: BRAND.text,
              maxWidth: 1000,
            }}
          >
            {title}
          </span>
          <span
            style={{
              display: "flex",
              fontFamily: "Inter",
              fontWeight: 400,
              fontSize: 24,
              lineHeight: 1.45,
              color: BRAND.secondary,
              marginTop: 22,
              maxWidth: 920,
            }}
          >
            {description}
          </span>
        </div>

        <div style={{ display: "flex", flexDirection: "column" }}>
          <div style={{ display: "flex", height: 2, backgroundImage: spectrum, marginBottom: 20 }} />
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <span
              style={{
                fontFamily: "JetBrains Mono",
                fontWeight: 500,
                fontSize: 17,
                color: BRAND.text,
              }}
            >
              Real-time shader visualization
            </span>
            <span
              style={{
                fontFamily: "JetBrains Mono",
                fontWeight: 500,
                fontSize: 15,
                color: BRAND.label,
              }}
            >
              {REPO_URL.replace("https://", "")}
            </span>
          </div>
        </div>
      </div>
    ),
    { ...OG_SIZE, fonts },
  );
}
