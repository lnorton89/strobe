import Link from "next/link";
import type { ComponentType, ReactNode } from "react";

/*
 * The site's two link shapes: a call to action and an inline forward link.
 *
 * Apart from ui.tsx because client components use them too, and ui.tsx renders
 * markdown, which should stay out of the browser's bundle.
 *
 * The rules they encode are uppercase 0.1em tracking on actions and one accent
 * per view.
 */

type ButtonVariant = "primary" | "secondary" | "ghost";

const BUTTON_BASE =
  "panel-hover pressable inline-flex items-center gap-2.5 text-sm font-semibold tracking-[0.1em] uppercase";

const BUTTON_VARIANT: Record<ButtonVariant, string> = {
  primary: "bg-accent text-on-accent hover:bg-accent-deep px-7 py-3.5",
  // The fill on hover is what makes a bordered button feel like a surface
  // rather than an outline; the border going to full text colour alone read as
  // a hairline flicker.
  secondary:
    "border-hairline-strong text-text hover:border-text hover:bg-panel border px-7 py-3.5",
  ghost: "text-secondary hover:text-text hover:bg-panel px-2 py-3.5",
};

/**
 * What an icon component has to accept to be usable here.
 *
 * `onAccent` is part of it because the primary variant sets it: the icons
 * mark their meaningful part in the accent, which vanishes on an accent fill.
 */
type ActionIcon = ComponentType<{
  size?: number;
  className?: string;
  onAccent?: boolean;
}>;

type ActionProps = {
  href: string;
  children: ReactNode;
  variant?: ButtonVariant;
  icon?: ActionIcon;
  /** Set for links leaving the site; adds the usual target and rel. */
  external?: boolean;
  /** Set for a file on another origin: a plain link, in this tab. */
  file?: boolean;
  className?: string;
};

/** A call to action. Internal links route through next/link. */
export function Action({
  href,
  children,
  variant = "secondary",
  icon: Icon,
  external,
  file,
  className,
}: ActionProps) {
  const classes = `${BUTTON_BASE} ${BUTTON_VARIANT[variant]}${className ? ` ${className}` : ""}`;
  const content = (
    <>
      {/*
        Only the primary variant paints the accent, so only it needs the icon's
        accented detail moved off the accent. Deciding it here rather than at
        each call site means a caller cannot forget.
      */}
      {Icon ? <Icon size={16} onAccent={variant === "primary"} /> : null}
      {children}
    </>
  );

  if (file) {
    return (
      <a href={href} className={classes}>
        {content}
      </a>
    );
  }
  if (external) {
    return (
      <a href={href} target="_blank" rel="noopener noreferrer" className={classes}>
        {content}
      </a>
    );
  }
  return (
    <Link href={href} className={classes}>
      {content}
    </Link>
  );
}

/** An inline forward link, set in the accent. */
export function ArrowLink({
  href,
  children,
  external,
  className,
}: {
  href: string;
  children: ReactNode;
  external?: boolean;
  className?: string;
}) {
  const classes = `text-accent-text hover:text-text panel-hover group inline-flex w-fit items-center gap-1.5 text-[13px] font-semibold tracking-widest uppercase${
    className ? ` ${className}` : ""
  }`;
  /*
   * The arrow advances a couple of pixels under the cursor. It is the one
   * gesture on the site that says something about direction rather than about
   * state, which is why it earns movement where the other hovers only change
   * colour. `motion-reduce:` removes the travel outright rather than shortening
   * it.
   */
  const content = (
    <>
      {children}
      <span
        aria-hidden="true"
        className="inline-block transition-transform duration-150 ease-out group-hover:translate-x-[3px] motion-reduce:transition-none motion-reduce:group-hover:translate-x-0"
      >
        →
      </span>
    </>
  );

  if (external) {
    return (
      <a href={href} target="_blank" rel="noopener noreferrer" className={classes}>
        {content}
      </a>
    );
  }
  return (
    <Link href={href} className={classes}>
      {content}
    </Link>
  );
}
