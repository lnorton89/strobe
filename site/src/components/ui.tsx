import type { ReactNode } from "react";
import Markdown, { InlineMarkdown } from "@/components/Markdown";

export { Action, ArrowLink } from "@/components/Action";

/*
 * The small, repeated pieces.
 *
 * Every one of these existed as copy-pasted class strings across the pages —
 * the same button, the same panel, the same source note — which is exactly how
 * a design system drifts. They are gathered here so the brand rules they
 * encode (uppercase 0.1em tracking on actions, hairline grids, one accent per
 * view) are stated once.
 */

/* ------------------------------------------------------------------ text -- */

/** Document-scale prose, filling its column up to a readable measure. */
export function Prose({
  children,
  className,
}: {
  children: string;
  className?: string;
}) {
  return (
    <div className={`max-w-[86ch] [&_p:first-child]:mt-0 ${className ?? ""}`}>
      <Markdown>{children}</Markdown>
    </div>
  );
}

/** A page lede: markdown that inherits a larger size from this wrapper. */
export function Lede({ children }: { children: string }) {
  return (
    <div className="text-secondary text-[18px] leading-[1.55] tracking-[-0.01em] text-pretty sm:text-[21px] [&_code]:font-mono [&_code]:text-[0.9em]">
      <InlineMarkdown>{children}</InlineMarkdown>
    </div>
  );
}

/**
 * The note that says which repository file a page is rendered from.
 *
 * Repeated on the three document pages, always in the same form: an accent
 * rule, mono text, and one link back to the source.
 */
export function SourceNote({
  children,
  className,
}: {
  children: string;
  className?: string;
}) {
  return (
    <div
      className={`border-accent text-bright border-l-2 pl-6 font-mono text-[13px] leading-[1.8] [&_a]:text-accent-text [&_a]:underline [&_a]:underline-offset-[3px] [&_a]:decoration-accent-text/40 [&_a:hover]:decoration-accent-text ${
        className ?? ""
      }`}
    >
      <InlineMarkdown>{children}</InlineMarkdown>
    </div>
  );
}


/* --------------------------------------------------------------- surfaces -- */

/** A cell in one of the site's hairline grids. */
export function Panel({
  children,
  className,
  interactive,
  as: Tag = "div",
}: {
  children: ReactNode;
  className?: string;
  /** Adds the hover treatment, for cells that are links or contain one. */
  interactive?: boolean;
  as?: "div" | "li";
}) {
  return (
    <Tag
      className={`bg-panel flex flex-col ${
        interactive ? "hover:bg-panel-raised panel-interactive" : ""
      } ${className ?? ""}`}
    >
      {children}
    </Tag>
  );
}

/**
 * A bordered callout — the site's device for a statement that carries more
 * weight than the prose around it.
 */
export function Callout({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={`border-hairline bg-panel flex flex-col gap-5 border p-8 sm:p-12 ${
        className ?? ""
      }`}
    >
      {children}
    </div>
  );
}

/**
 * A list marked with a small square bullet.
 *
 * `accent` uses accent squares, for a short list that is making a point;
 * the default uses the hairline colour, for an ordinary list.
 */
export function MarkedList({
  items,
  accent,
  className,
}: {
  items: readonly string[];
  accent?: boolean;
  className?: string;
}) {
  return (
    <ul className={`flex list-none flex-col gap-2.5 pl-0 ${className ?? ""}`}>
      {items.map((item) => (
        <li
          key={item}
          className={`text-secondary relative pl-6 text-[14.5px] leading-[1.6] before:absolute before:top-[0.55em] before:left-0 before:h-[6px] before:w-[6px] before:content-[''] ${
            accent ? "before:bg-accent" : "before:bg-hairline-strong"
          }`}
        >
          {item}
        </li>
      ))}
    </ul>
  );
}

/* ------------------------------------------------------------ grid fill -- */

/** Column counts of a responsive `.rule-grid`, per breakpoint. */
export type GridColumns = { base: number; sm?: number; lg?: number; xl?: number };

/*
 * Visibility classes per breakpoint, spelled out so Tailwind finds them in the
 * source. A breakpoint the grid does not change at inherits the one below.
 */
const SHOW = { base: "flex", sm: "sm:flex", lg: "lg:flex", xl: "xl:flex" } as const;
const HIDE = { base: "hidden", sm: "sm:hidden", lg: "lg:hidden", xl: "xl:hidden" } as const;

/**
 * Empty cells that complete the last row of a `.rule-grid`.
 *
 * The grid paints its hairline as the container's background and lets it
 * show through 1px gaps, so it only reads as rules while the children cover
 * it. A last row with fewer items than columns leaves a block of flat
 * hairline grey instead. These cells cover it, at whatever column count the
 * breakpoint gives.
 */
export function GridFill({ count, columns }: { count: number; columns: GridColumns }) {
  const steps = (["base", "sm", "lg", "xl"] as const).reduce<
    { step: keyof typeof SHOW; cols: number }[]
  >((acc, step) => {
    const cols = columns[step] ?? acc[acc.length - 1]?.cols ?? 1;
    return [...acc, { step, cols }];
  }, []);
  const most = Math.max(...steps.map((entry) => entry.cols));

  return (
    <>
      {Array.from({ length: most - 1 }, (_, index) => {
        const position = index + 1;
        const classes = steps.map(({ step, cols }) => {
          const missing = (cols - (count % cols)) % cols;
          return position <= missing ? SHOW[step] : HIDE[step];
        });
        return (
          <div
            key={position}
            aria-hidden="true"
            className={`bg-ground ${classes.join(" ")}`}
          />
        );
      })}
    </>
  );
}
