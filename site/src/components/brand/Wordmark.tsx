import StrobeMark from "./StrobeMark";

/*
 * The wordmark and the horizontal lockup (docs/branding.md, section 2).
 *
 * The same setting assets/brand/tools/typeset.py outlines: STROBE in Space
 * Grotesk Medium, tracked 0.14 em, in Text; the descriptor at 0.22 of the
 * wordmark's size, tracked 0.32 em, in Muted. Set as live type here rather
 * than as the outlined SVG so it follows the theme — the outlines are painted
 * in the dark theme's Text and vanish on a light ground.
 *
 * The gradient never touches the letters (guide: "Do not apply the gradient to
 * every letter"). It lives in the mark alone.
 */

/** typeset.py's own figures, in em. */
export const WORDMARK_TRACKING = "0.14em";
export const DESCRIPTOR_RATIO = 0.22;
export const DESCRIPTOR_TRACKING = "0.32em";

/** The guide's descriptor line. */
export const DESCRIPTOR = "Shader visualizer";

type WordmarkProps = {
  /** Font size of STROBE. A number is px; a string is any CSS length. */
  size?: number | string;
  /** Draws the ribbon to the left, at about 2.6 cap heights, as the lockup does. */
  withMark?: boolean;
  /** Adds SHADER VISUALIZER under the word, as the lockups do. */
  withDescriptor?: boolean;
  className?: string;
};

export default function Wordmark({
  size = 24,
  withMark = false,
  withDescriptor = false,
  className,
}: WordmarkProps) {
  const fontSize = typeof size === "number" ? `${size}px` : size;
  return (
    <span className={`inline-flex items-center ${className ?? ""}`} style={{ fontSize }}>
      {/* The visible run is spaced capitals; the name is said once, plainly. */}
      <span className="sr-only">Strobe</span>
      {withMark ? (
        // The mark is sized in em so it scales with the word it stands beside.
        <span aria-hidden="true" style={{ width: "1.85em", height: "1.85em", marginRight: "0.42em" }}>
          <StrobeMarkEm />
        </span>
      ) : null}
      <span aria-hidden="true" className="inline-flex flex-col">
        <span
          className="text-text font-display"
          style={{
            fontWeight: 500,
            lineHeight: 1,
            letterSpacing: WORDMARK_TRACKING,
            // Tracking adds space after the last letter too; take it back so
            // the word's box ends at the E.
            marginRight: `-${WORDMARK_TRACKING}`,
            whiteSpace: "nowrap",
          }}
        >
          STROBE
        </span>
        {withDescriptor ? (
          <span
            className="text-secondary font-display"
            style={{
              fontSize: `${DESCRIPTOR_RATIO}em`,
              fontWeight: 500,
              lineHeight: 1,
              letterSpacing: DESCRIPTOR_TRACKING,
              marginTop: "0.7em",
              whiteSpace: "nowrap",
              textTransform: "uppercase",
            }}
          >
            {DESCRIPTOR}
          </span>
        ) : null}
      </span>
    </span>
  );
}

/** The mark filling an em-sized box; detail is kept, since lockups are large. */
function StrobeMarkEm() {
  return <StrobeMark size={64} className="h-full w-full" />;
}
