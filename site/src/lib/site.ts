/*
 * Single source of truth for the site's identity strings and navigation.
 *
 * The positioning line and the descriptor are the brand guide's own copy
 * (docs/branding.md, "Brand at a glance" and section 9), not rewritten here.
 */

export const REPO_URL = "https://github.com/lnorton89/strobe";

/** The branch the site links into and the activity feed reads. */
export const DEFAULT_BRANCH = "master";

export const RELEASES_URL = `${REPO_URL}/releases`;

/**
 * The deployed origin, used as the metadata base for canonical and social
 * URLs.
 *
 * Netlify exposes the production origin as URL during a build, so the deployed
 * site picks it up without anything being hard-coded. A local build, a
 * Playwright run and a Lighthouse run all fall back to the port the test
 * server uses, which keeps canonical and og:image URLs resolvable there too.
 */
export const SITE_URL =
  process.env.NEXT_PUBLIC_SITE_URL ?? process.env.URL ?? "http://localhost:4173";

/** The guide's descriptor, and the page title's second half. */
export const TAGLINE = "Real-time shader visualization for live shows.";

/**
 * The site's meta description.
 *
 * The guide's positioning statement names what Strobe turns into what; this
 * adds the two facts a visitor needs to tell whether it is for them: it plays
 * Synesthesia scenes, and it runs on old hardware.
 */
export const DESCRIPTION =
  "Strobe is an open-source VJ engine that plays audio-reactive GLSL scenes, imports Shadertoy shaders and runs long playlists — built to stay up on old laptops, projectors and LED walls.";

/** The guide's positioning statement, verbatim. */
export const POSITIONING =
  "STROBE turns real-time signals and shader code into expressive visual systems for screens, projectors, and live environments.";

/** The metadata strip under the hero. */
export const BADGES = ["MIT", "Rust", "OpenGL 3.3"] as const;
/**
 * What a release is fit for, beside its number. The number itself is never
 * written here: it is read from GitHub's latest release at build time
 * (lib/release.ts), so the site cannot fall behind a release.
 */
export const RELEASE_STAGE = "testing";

export type NavItem = {
  href: string;
  label: string;
  /** Shown in the mobile menu under the label. */
  blurb: string;
  /** The icon that stands for this section, used wherever it appears. */
  icon: IconName;
};

/**
 * Icons are named rather than imported here, because this module is read by
 * the sitemap and the metadata — neither of which should pull in components.
 */
export type IconName =
  | "download"
  | "guide"
  | "scenes"
  | "play"
  | "compatibility"
  | "architecture"
  | "roadmap";

/** Getting it first, then using it, then what it is made of and where it is going. */
export const NAV: readonly NavItem[] = [
  {
    href: "/download/",
    label: "Download",
    blurb: "Windows builds, Linux, and building from source.",
    icon: "download",
  },
  {
    href: "/guide/",
    label: "Guide",
    blurb: "Scenes, audio, playlists, controllers and remotes.",
    icon: "guide",
  },
  {
    href: "/scenes/",
    label: "Scenes",
    blurb: "The original scene library that ships with Strobe.",
    icon: "scenes",
  },
  {
    href: "/play/",
    label: "Play",
    blurb: "Write and run Strobe scenes in the browser.",
    icon: "play",
  },
  {
    href: "/compatibility/",
    label: "Compatibility",
    blurb: "Which machines, drivers and scenes are verified.",
    icon: "compatibility",
  },
  {
    href: "/architecture/",
    label: "Architecture",
    blurb: "Fifteen crates, one frame loop, one GPU boundary.",
    icon: "architecture",
  },
  {
    href: "/roadmap/",
    label: "Roadmap",
    blurb: "What is built, what is verified, what is next.",
    icon: "roadmap",
  },
] as const;

/** Every route the export produces, for the sitemap and the link checker. */
export const EXPORTED_ROUTES = [
  "/",
  "/download/",
  "/guide/",
  "/scenes/",
  "/play/",
  "/compatibility/",
  "/architecture/",
  "/roadmap/",
] as const;

/** Backwards-compatible route registry name used by export checks. */
export const ROUTES = EXPORTED_ROUTES;

/** Marks the active nav item, treating nested routes as part of their family. */
export function isActiveRoute(pathname: string, href: string): boolean {
  const normalise = (value: string) =>
    value.endsWith("/") && value !== "/" ? value.slice(0, -1) : value;
  const current = normalise(pathname);
  const target = normalise(href);
  if (target === "/") return current === "/";
  return current === target || current.startsWith(`${target}/`);
}
