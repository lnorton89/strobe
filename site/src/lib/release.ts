/*
 * The current release: what GitHub publishes, and how the site names it (D526,
 * D557). This is the one place a version, tag or asset name is worked out.
 *
 * Two lookups feed it, because the site is a static export deployed by hand:
 *
 * - **Once per build**, scripts/sync-release.mts asks GitHub, then falls back to
 *   the newest tag in the checkout, then to naming no number. It writes the
 *   answer to one file that every page reads (lib/release-build.ts), so the
 *   build's workers cannot disagree and every page works with no JavaScript.
 * - **Once per visit in the browser**, components/Release.tsx asks GitHub again
 *   and replaces what the build wrote. A release published after the last
 *   deploy therefore shows up with no code edit and no deploy.
 *
 * Asset names carry the version, so a stable `/releases/latest/download/...`
 * URL cannot name the zip. The asset is read from the release instead, with
 * `/releases/latest` as the link when nothing is known.
 *
 * Everything here is a pure function over a payload apart from the fetch. URLs
 * are built from a checked tag and asset name, never read from a payload: the
 * browser's copy passes through sessionStorage, which anything on this origin
 * can write, and these strings become hrefs.
 */

import { CACHE_TTL_MS, type CacheStorage } from "./github";
import { LATEST_RELEASE_URL, RELEASES_URL, REPO } from "./site";

export type ReleaseAsset = {
  /** The file name. */
  name: string;
  /** Its download URL. */
  url: string;
  /** Its size in bytes, when GitHub gave one. */
  bytes?: number;
};

export type Release = {
  /** The tag; undefined when neither GitHub nor the checkout could say. */
  tag?: string;
  /** The release's page, or the latest-release redirect without a tag. */
  url: string;
  /** The UTC date it was published, as `YYYY-MM-DD`, when known. */
  published?: string;
  /** The Windows zip, when GitHub listed one. */
  windows?: ReleaseAsset;
  /** The zip's `.sha256` file, when GitHub listed one. */
  checksum?: ReleaseAsset;
};

/**
 * The fields of GitHub's release payload the site reads. The same shape is what
 * the build's file and the browser's cache store, so all three go through
 * `parseRelease`.
 */
export type ReleasePayload = {
  tag_name: string;
  published_at?: string;
  assets?: { name: string; size?: number }[];
};

/** The release workflow's tags: `vMAJOR.MINOR.PATCH`. */
export const RELEASE_TAG = /^v?\d+\.\d+\.\d+$/;

/** `strobe-<tag>-x86_64-pc-windows-msvc.zip`; the `-symbols` zip does not match. */
const WINDOWS_ZIP = /^strobe-v?\d+\.\d+\.\d+-x86_64-pc-windows-msvc\.zip$/;

/** Anything a release can attach, as a URL path segment needing no escaping. */
const ASSET_NAME = /^[\w.-]+$/;

/** What the site says with no answer from anywhere: no number, a link that works. */
export const UNKNOWN_RELEASE: Release = { url: LATEST_RELEASE_URL };

export function releaseEndpoint(api = "https://api.github.com"): string {
  return `${api}/repos/${REPO}/releases/latest`;
}

function asset(tag: string, name: string, size: unknown): ReleaseAsset {
  return {
    name,
    url: `${RELEASES_URL}/download/${tag}/${name}`,
    ...(typeof size === "number" && size > 0 ? { bytes: size } : {}),
  };
}

/** Keeps the fields `parseRelease` reads, or nothing for a payload with no tag. */
export function compactRelease(payload: unknown): ReleasePayload | undefined {
  if (payload === null || typeof payload !== "object") return undefined;
  const { tag_name, published_at, assets } = payload as Record<string, unknown>;
  if (typeof tag_name !== "string" || !RELEASE_TAG.test(tag_name)) return undefined;
  const files = Array.isArray(assets)
    ? assets.flatMap((entry: unknown) => {
        if (entry === null || typeof entry !== "object") return [];
        const { name, size } = entry as Record<string, unknown>;
        if (typeof name !== "string" || !ASSET_NAME.test(name)) return [];
        return [typeof size === "number" ? { name, size } : { name }];
      })
    : [];
  return {
    tag_name,
    ...(typeof published_at === "string" ? { published_at } : {}),
    assets: files,
  };
}

/** Turns a payload into a Release, or undefined when it names no version. */
export function parseRelease(payload: unknown): Release | undefined {
  const compact = compactRelease(payload);
  if (!compact) return undefined;
  const raw = compact.tag_name;
  const zip = compact.assets?.find((file) => WINDOWS_ZIP.test(file.name));
  const checksum = zip && compact.assets?.find((file) => file.name === `${zip.name}.sha256`);
  const date = compact.published_at ? new Date(compact.published_at) : undefined;
  return {
    tag: raw.startsWith("v") ? raw : `v${raw}`,
    url: `${RELEASES_URL}/tag/${raw}`,
    ...(date && !Number.isNaN(date.getTime())
      ? { published: date.toISOString().slice(0, 10) }
      : {}),
    ...(zip ? { windows: asset(raw, zip.name, zip.size) } : {}),
    ...(checksum ? { checksum: asset(raw, checksum.name, checksum.size) } : {}),
  };
}

/** A way of finding the release; it may throw, or answer nothing. */
export type ReleaseSource = {
  name: string;
  read: () => unknown | Promise<unknown>;
};

/**
 * Tries each source in order and returns the first usable answer, naming the
 * source. Undefined means none had one; the build still succeeds, and the site
 * names no number, since a wrong number is worse than none.
 */
export async function resolveRelease(
  sources: readonly ReleaseSource[],
  warn: (message: string) => void = () => {},
): Promise<{ source: string; payload: ReleasePayload } | undefined> {
  for (const source of sources) {
    try {
      const payload = compactRelease(await source.read());
      if (payload) return { source: source.name, payload };
      warn(`${source.name} named no version`);
    } catch (error) {
      warn(`${source.name}: ${String(error)}`);
    }
  }
  return undefined;
}

/**
 * Asks GitHub from the browser. Never throws: every failure is "keep what the
 * build wrote", so there is nothing an error channel would do. Only
 * CORS-safelisted headers, so the request is not preflighted.
 */
export async function fetchLatestRelease(options: {
  signal?: AbortSignal;
  fetchImpl?: typeof fetch;
} = {}): Promise<ReleasePayload | undefined> {
  const { signal, fetchImpl = fetch } = options;
  try {
    const response = await fetchImpl(releaseEndpoint(), {
      headers: { Accept: "application/vnd.github+json" },
      signal,
    });
    if (!response.ok) return undefined;
    return compactRelease(await response.json());
  } catch {
    return undefined;
  }
}

/** sessionStorage key. Versioned, so a shape change cannot read old entries. */
export const RELEASE_CACHE_KEY = "strobe:release:v1";

/** Reads the cached payload, or undefined when there is none or it has expired. */
export function readCachedRelease(
  storage: CacheStorage | undefined,
  now: number,
): ReleasePayload | undefined {
  if (!storage) return undefined;
  try {
    const raw = storage.getItem(RELEASE_CACHE_KEY);
    if (!raw) return undefined;
    const entry = JSON.parse(raw) as { at?: unknown; payload?: unknown };
    if (typeof entry?.at !== "number" || now - entry.at > CACHE_TTL_MS) return undefined;
    return compactRelease(entry.payload);
  } catch {
    return undefined;
  }
}

/** Stores the payload. A storage that refuses to write is not an error here. */
export function writeCachedRelease(
  storage: CacheStorage | undefined,
  payload: ReleasePayload,
  now: number,
): void {
  if (!storage) return;
  try {
    storage.setItem(RELEASE_CACHE_KEY, JSON.stringify({ at: now, payload }));
  } catch {
    // Private-mode Safari throws on write. It costs a request per page, no more.
  }
}

/** The tag, or "Latest release" when nothing could say. */
export function releaseLabel(release: Release): string {
  return release.tag ?? "Latest release";
}

/** `18.6 MB`, in decimal megabytes as a browser's download list shows them. */
export function formatBytes(bytes: number): string {
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}
