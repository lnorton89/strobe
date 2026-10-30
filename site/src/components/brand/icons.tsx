import type { CSSProperties, ReactNode } from "react";

/*
 * The site's icon set.
 *
 * 24 px grid, 1.6 px stroke with round caps and joins — the ribbon's own
 * line ending. The accent marks the one part of each icon that carries the
 * meaning; nothing else in the set is coloured (guide section 6: cyan for
 * the active part, not for every element). At 16 px the stroke thickens to
 * 1.8 so the figure holds together.
 */

/**
 * The colour of the part that carries the meaning.
 *
 * Indirected through a custom property so an icon can be dropped onto the
 * accent fill without every glyph in the set growing its own variant: on a
 * cyan button the cyan detail is invisible, so `onAccent` rebinds
 * this to `currentColor` — the button's own text colour, `--on-accent` — and
 * the figure goes monochrome instead of losing half of itself.
 */
const ACCENT = "var(--icon-accent, var(--accent))";

/** Set on the SVG by `onAccent`; every accented part reads it. */
const ON_ACCENT_STYLE = { "--icon-accent": "currentColor" } as CSSProperties;

type IconProps = {
  size?: number;
  className?: string;
  /** Accessible name. Omitted icons are hidden from assistive technology. */
  title?: string;
  /**
   * Set when the icon sits on the accent fill. `Action`'s primary variant
   * passes it automatically, so a caller placing an icon on a button never has
   * to think about it.
   */
  onAccent?: boolean;
};

function Icon({
  size = 24,
  className,
  title,
  onAccent,
  children,
}: IconProps & { children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      // The stroke thickens at 16 px so the figure holds together.
      strokeWidth={size <= 16 ? 1.8 : 1.6}
      className={className}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable="false"
      style={
        onAccent
          ? { display: "block", flex: "none", ...ON_ACCENT_STYLE }
          : { display: "block", flex: "none" }
      }
    >
      {title ? <title>{title}</title> : null}
      {children}
    </svg>
  );
}

export function DownloadIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M4 14.5v4.5h16v-4.5" />
      <path d="M12 3.5v11M7.5 10l4.5 4.5 4.5-4.5" stroke={ACCENT} />
    </Icon>
  );
}

export function GuideIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M12 6.5C10 5 7 4.5 3.5 5v13c3.5-.5 6.5 0 8.5 1.5 2-1.5 5-2 8.5-1.5V5C17 4.5 14 5 12 6.5z" />
      <path d="M12 6.5v13" stroke={ACCENT} />
    </Icon>
  );
}

export function ScenesIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="3.5" y="4.5" width="7.5" height="6" rx="1" />
      <rect x="13" y="4.5" width="7.5" height="6" rx="1" />
      <rect x="3.5" y="13.5" width="7.5" height="6" rx="1" />
      <rect x="13" y="13.5" width="7.5" height="6" rx="1" stroke={ACCENT} />
    </Icon>
  );
}

export function CompatibilityIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="3" y="4" width="18" height="12.5" rx="1.5" />
      <path d="M9 20h6M12 16.5V20" />
      <path d="M8.5 10.5l2.5 2.5 4.5-5" stroke={ACCENT} />
    </Icon>
  );
}

export function ArchitectureIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M12 3.5l8.5 4.5-8.5 4.5L3.5 8z" />
      <path d="M3.5 12l8.5 4.5 8.5-4.5" />
      <path d="M3.5 16l8.5 4.5 8.5-4.5" stroke={ACCENT} />
    </Icon>
  );
}

export function RoadmapIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M5 19c0-4 4-4 7-7s7-3 7-7" />
      <circle cx="5" cy="19" r="1.6" />
      <circle cx="12" cy="12" r="1.6" />
      <circle cx="19" cy="5" r="1.6" fill={ACCENT} stroke={ACCENT} />
    </Icon>
  );
}

export function PlayIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 6.5l6 4.5-6 4.5" />
      <path d="M3.5 19.5h17" />
      <path d="M15.5 15.5h5" stroke={ACCENT} />
    </Icon>
  );
}

export function AudioIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M3.5 12h1.5M7.5 8v8M11 5v14M18.5 9v6M20.5 12h0" />
      <path d="M14.5 7.5v9" stroke={ACCENT} />
    </Icon>
  );
}

export function ImportIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M14 4.5h5.5v15h-15V14" />
      <path d="M4 4l8.5 8.5M12.5 7v5.5H7" stroke={ACCENT} />
    </Icon>
  );
}

export function PlaylistIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M3.5 6h12M3.5 11h12M3.5 16h7" />
      <path d="M14.5 14.5v5l4.5-2.5z" stroke={ACCENT} />
    </Icon>
  );
}

export function ControllerIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M6 3.5v17M12 3.5v17M18 3.5v17" />
      <path d="M4 15h4M10 8h4" />
      <path d="M16 12.5h4" stroke={ACCENT} />
    </Icon>
  );
}

export function PerformIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="3" y="4" width="18" height="16" rx="1.5" />
      <path d="M3 8.5h18M14 8.5V20" />
      <path d="M6 12.5h5" stroke={ACCENT} />
    </Icon>
  );
}

export function ResilienceIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M12 3.5l7.5 3v5.5c0 4.5-3.2 7.5-7.5 8.5-4.3-1-7.5-4-7.5-8.5V6.5z" />
      <path d="M8.5 12l2.5 2.5 4.5-5" stroke={ACCENT} />
    </Icon>
  );
}

export function RemoteIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="7" y="3" width="10" height="18" rx="2" />
      <path d="M10.5 18h3" />
      <path d="M10 8.5c1.2-1.2 2.8-1.2 4 0M8.5 6.5c2-2 5-2 7 0" stroke={ACCENT} />
    </Icon>
  );
}

export function ShellIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="3" y="4.5" width="18" height="15" rx="1.5" />
      <path d="M7 10l3 2.5L7 15" />
      <path d="M12.5 15H17" stroke={ACCENT} />
    </Icon>
  );
}

/**
 * GitHub's own glyph. Not part of the brand set — it is a third-party mark and
 * deliberately drawn with its real curves rather than forced onto the 34°
 * grid, which would misrepresent someone else's logo.
 */
export function GitHubIcon({ size = 16, className, title }: IconProps) {
  return (
    <svg
      viewBox="0 0 16 16"
      width={size}
      height={size}
      fill="currentColor"
      className={className}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable="false"
      style={{ display: "block", flex: "none" }}
    >
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
    </svg>
  );
}

/**
 * The icon that stands for each navigable section.
 *
 * Named in lib/site.ts and resolved here, so the footer, the mobile menu and a
 * page's own header all mark a section with the same figure.
 */
export const NAV_ICONS = {
  download: DownloadIcon,
  guide: GuideIcon,
  scenes: ScenesIcon,
  play: PlayIcon,
  compatibility: CompatibilityIcon,
  architecture: ArchitectureIcon,
  roadmap: RoadmapIcon,
} as const;
