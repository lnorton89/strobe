import { useId } from "react";

/*
 * The mark: the Parametric Flow ribbon (docs/branding.md, section 2).
 *
 * The path data is the application icon's, copied from
 * assets/brand/icon.svg and assets/brand/icon-small.svg rather than redrawn —
 * the guide forbids redrawing the ribbon inconsistently. Two forms, as the
 * guide asks: the heavy ribbon with its two fine contours from 32 px up, and
 * below that the simplified silhouette with no contours, the innermost turn
 * dropped and a heavier stroke ("do not merely shrink the detailed master").
 *
 * No glow is baked in; the guide forbids that too.
 */

/** The ribbon's heavy stroke, shared by the mark and the social images. */
export const RIBBON_PATH =
  "M117 269 C93 175 166 99 252 119 C318 135 381 205 360 273 C339 334 260 386 192 355 C136 330 115 273 157 226 C202 176 279 180 322 223 C355 258 333 304 290 322 C241 343 190 307 197 264 C204 221 251 207 282 232 C306 252 298 283 274 295";

/** The two fine contours either side of it. */
export const RIBBON_CONTOURS = [
  "M136 283 C112 193 181 120 252 136 C313 150 363 213 345 269 C326 320 258 366 201 340 C153 318 132 275 169 235 C209 191 272 193 308 229 C335 255 318 292 283 307 C241 325 202 300 207 266 C212 237 248 222 272 242",
  "M100 256 C76 158 160 82 255 103 C331 121 398 200 375 277 C352 341 264 402 183 365 C124 337 98 271 145 216",
] as const;

/** The small-size silhouette: the same curve, stopped before the inner turn. */
export const RIBBON_SMALL_PATH =
  "M117 269 C93 175 166 99 252 119 C318 135 381 205 360 273 C339 334 260 386 192 355 C136 330 115 273 157 226 C202 176 279 180 322 223 C355 258 333 304 290 322";

/** The framing mark-mono.svg uses: the ribbon's bounds with its stroke. */
export const RIBBON_VIEWBOX = "76 82 322 320";

/** The primary gradient's stops (guide section 3: 0, 35, 68, 100%). */
export const SPECTRUM_STOPS = [
  { offset: 0, color: "var(--cyan)" },
  { offset: 0.35, color: "var(--blue)" },
  { offset: 0.68, color: "var(--violet)" },
  { offset: 1, color: "var(--magenta)" },
] as const;

/** Below this rendered size the simplified silhouette is drawn. */
const DETAIL_FLOOR = 32;

export type MarkVariant = "spectrum" | "mono";

type StrobeMarkProps = {
  /** Rendered width and height in px. The guide's floor is 16. */
  size?: number;
  /** Full colour, or one colour (`currentColor` unless `color` is given). */
  variant?: MarkVariant;
  color?: string;
  className?: string;
  /**
   * Accessible name. Omit for decorative uses beside a text label; the mark
   * is then hidden from assistive technology.
   */
  title?: string;
};

export default function StrobeMark({
  size = 32,
  variant = "spectrum",
  color = "currentColor",
  className,
  title,
}: StrobeMarkProps) {
  // Each instance needs its own gradient id: two marks on one page sharing an
  // id would both paint with whichever definition the browser found first.
  const gradientId = `spectrum-${useId().replace(/:/g, "")}`;
  const stroke = variant === "mono" ? color : `url(#${gradientId})`;
  const detailed = size >= DETAIL_FLOOR;

  return (
    <svg
      viewBox={RIBBON_VIEWBOX}
      width={size}
      height={size}
      className={className}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable="false"
      style={{ display: "block", flex: "none" }}
    >
      {variant === "spectrum" ? (
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="1" y2="1">
            {SPECTRUM_STOPS.map((stop) => (
              <stop key={stop.offset} offset={stop.offset} stopColor={stop.color} />
            ))}
          </linearGradient>
        </defs>
      ) : null}
      <g fill="none" stroke={stroke} strokeLinecap="round">
        {detailed ? (
          <>
            <g strokeWidth={5} opacity={0.55}>
              {RIBBON_CONTOURS.map((d) => (
                <path key={d} d={d} />
              ))}
            </g>
            <path d={RIBBON_PATH} strokeWidth={24} />
          </>
        ) : (
          <path d={RIBBON_SMALL_PATH} strokeWidth={36} />
        )}
      </g>
    </svg>
  );
}
