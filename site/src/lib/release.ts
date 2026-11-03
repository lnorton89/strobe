/*
 * The current release, read from GitHub at build time.
 *
 * GitHub's Releases page is where a release exists: the tag, the zip and the
 * notes are published there by release.yml (CONTRIBUTING.md). The site used to
 * carry the version as a constant, which went stale the day v0.2.0 shipped,
 * so every place that names the release reads it from here instead.
 *
 * Fetched once per build (the promise is shared by every page). With GitHub
 * unreachable the build still succeeds and the site names no number, only
 * "the latest release" and a link to the Releases page: a wrong number is
 * worse than none.
 *
 * This module is imported by server components only.
 */

import { RELEASES_URL } from "./site";
import { REPO } from "./github";

export type Release = {
  /** The tag, e.g. `v0.2.0`; undefined when GitHub could not be asked. */
  tag?: string;
  /** The release's page, or the Releases page without one. */
  url: string;
  /** ISO date the release was published, when known. */
  published?: string;
  /** GitHub's pre-release flag. */
  prerelease: boolean;
};

const ENDPOINT = `https://api.github.com/repos/${REPO}/releases/latest`;

/** Turns the API's payload into a Release. Exported for testing. */
export function parseRelease(payload: unknown): Release | undefined {
  if (payload === null || typeof payload !== "object") return undefined;
  const { tag_name, html_url, published_at, prerelease } = payload as Record<string, unknown>;
  if (typeof tag_name !== "string" || !/^v?\d+\.\d+\.\d+/.test(tag_name)) return undefined;
  return {
    tag: tag_name.startsWith("v") ? tag_name : `v${tag_name}`,
    url: typeof html_url === "string" ? html_url : RELEASES_URL,
    published: typeof published_at === "string" ? published_at : undefined,
    prerelease: prerelease === true,
  };
}

/** What the site says with no answer from GitHub. */
export const UNKNOWN_RELEASE: Release = { url: RELEASES_URL, prerelease: false };

let pending: Promise<Release> | undefined;

/** The latest published release, fetched once per build. */
export function latestRelease(): Promise<Release> {
  pending ??= (async () => {
    try {
      const headers: Record<string, string> = { Accept: "application/vnd.github+json" };
      // A token raises the build's rate limit when one is in the environment.
      const token = process.env.GITHUB_TOKEN;
      if (token) headers.Authorization = `Bearer ${token}`;
      const response = await fetch(ENDPOINT, { headers, signal: AbortSignal.timeout(10_000) });
      if (!response.ok) throw new Error(`GitHub answered ${response.status}`);
      const release = parseRelease(await response.json());
      if (!release) throw new Error("GitHub's latest release has no version tag");
      return release;
    } catch (error) {
      console.warn(`release: could not read the latest release (${String(error)}); naming none`);
      return UNKNOWN_RELEASE;
    }
  })();
  return pending;
}

/** `v0.2.0`, or "Latest release" when GitHub could not be asked. */
export function releaseLabel(release: Release): string {
  return release.tag ?? "Latest release";
}
