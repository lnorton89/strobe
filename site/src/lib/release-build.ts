/*
 * The build's answer about the current release, for server components.
 *
 * scripts/sync-release.mts writes it once, before `next build`, because a build
 * renders its pages in several worker processes: each one fetching for itself
 * could give some pages a version and others none when GitHub's rate limit runs
 * out part way through. Reading one file cannot disagree with itself.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { parseRelease, UNKNOWN_RELEASE, type Release } from "./release";

export const RELEASE_FILE = path.join("public", "generated", "release.json");

let cached: Release | undefined;

/** The release this build names. No file, or a bad one, means naming none. */
export function buildRelease(): Release {
  cached ??= (() => {
    try {
      const { payload } = JSON.parse(readFileSync(RELEASE_FILE, "utf8")) as { payload?: unknown };
      return parseRelease(payload) ?? UNKNOWN_RELEASE;
    } catch {
      return UNKNOWN_RELEASE;
    }
  })();
  return cached;
}
