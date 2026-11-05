/*
 * Finds the current release once per build and writes it where the pages read
 * it (lib/release-build.ts). Runs with `npm run assets`, after sync-assets, which
 * empties public/generated/.
 *
 * In order: GitHub's latest published release; the newest version tag reachable
 * from HEAD, for a build with no network; nothing, in which case the site names
 * no number. A shallow clone has no tags, so it lands on the last of these, and
 * the build still succeeds: the browser asks GitHub again on every visit.
 *
 * `GITHUB_TOKEN` raises the rate limit where one is set, and `GITHUB_API_URL`
 * names another API, as it does on GitHub Actions.
 */

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { releaseEndpoint, resolveRelease } from "../src/lib/release";
import { RELEASE_FILE } from "../src/lib/release-build";

const found = await resolveRelease(
  [
    {
      name: "GitHub",
      read: async () => {
        const headers: Record<string, string> = { Accept: "application/vnd.github+json" };
        const token = process.env.GITHUB_TOKEN;
        if (token) headers.Authorization = `Bearer ${token}`;
        const response = await fetch(releaseEndpoint(process.env.GITHUB_API_URL), {
          headers,
          signal: AbortSignal.timeout(10_000),
        });
        if (!response.ok) throw new Error(`answered ${response.status}`);
        return response.json();
      },
    },
    {
      name: "git describe",
      read: () => ({
        tag_name: execFileSync(
          "git",
          ["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*"],
          { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
        ).trim(),
      }),
    },
  ],
  (message) => console.warn(`sync-release: ${message}`),
);

mkdirSync(path.dirname(RELEASE_FILE), { recursive: true });
writeFileSync(RELEASE_FILE, `${JSON.stringify(found ?? { source: "none" }, null, 2)}\n`);
console.log(
  found
    ? `sync-release: ${found.payload.tag_name}, from ${found.source}`
    : "sync-release: no release found; the build names no number",
);
