/*
 * The frame times the home page shows, read from docs/BENCHMARKS.md at build time.
 *
 * D526 took a hand-copied stats band off the home page because such numbers drift
 * from the evidence. Reading the table keeps the one copy where it is recorded,
 * and a heading that stops matching fails the build rather than showing a stale
 * or empty table.
 */

import { readFileSync } from "node:fs";
import path from "node:path";

export type FrameTime = {
  /** The scene's folder name without `.synScene`. */
  slug: string;
  /** Mean ms per frame at 1280x720. */
  full: number;
  /** Mean ms per frame at render scale 0.5, 640x360. */
  half: number;
};

export type RenderScaleSweep = {
  date: string;
  commit: string;
  scenes: FrameTime[];
};

/** A refresh at 60 Hz, in milliseconds. */
export const FRAME_BUDGET_MS = 1000 / 60;

const HEADING =
  /^### (\d{4}-\d{2}-\d{2}): render-scale sweep, ThinkPad T480 \(Intel UHD 620\), commit (\w+)$/m;

/** Finds the T480 sweep's native table. Exported for testing. */
export function parseSweep(markdown: string): RenderScaleSweep {
  const heading = HEADING.exec(markdown);
  if (!heading) throw new Error("docs/BENCHMARKS.md has no render-scale sweep for the T480");

  const rest = markdown.slice(heading.index);
  const scenes: FrameTime[] = [];
  // The section's first table is native OpenGL; its first blank line after a
  // row ends it.
  for (const line of rest.split(/\r?\n/).slice(1)) {
    const row = /^\| ([\w-]+)(?: \*)? \| ([\d.]+) \| [\d.]+ \| ([\d.]+) \|/.exec(line);
    if (row) {
      scenes.push({ slug: row[1], full: Number(row[2]), half: Number(row[3]) });
    } else if (scenes.length > 0) {
      break;
    }
  }
  if (scenes.length === 0) throw new Error("the T480 render-scale sweep has no table");
  return { date: heading[1], commit: heading[2], scenes };
}

export function readSweep(): RenderScaleSweep {
  return parseSweep(readFileSync(path.join(process.cwd(), "..", "docs", "BENCHMARKS.md"), "utf8"));
}
