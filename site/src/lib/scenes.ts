import { existsSync } from "node:fs";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

/*
 * The scene library, read from the repository's scenes/ folder at build time.
 *
 * Nothing about a scene is written into the site: title, description, credit,
 * tags and the counts all come from its scene.json, and the picture is its
 * own thumbnail, re-encoded into public/generated/scenes/ by
 * scripts/sync-assets.mjs. Adding a scene to the repository adds it here on
 * the next build; a scene.json that does not parse fails the build, naming
 * the folder, rather than dropping the scene silently.
 *
 * This module is imported by server components only.
 */

const SCENES_DIR = path.join(process.cwd(), "..", "scenes");

export type SceneSummary = {
  /** The folder name without `.synScene`, e.g. `05-domain-warp`. */
  slug: string;
  title: string;
  description: string;
  credit?: string;
  license?: string;
  tags: string[];
  controls: number;
  /** Buffer passes; the implicit final pass is not counted. */
  passes: number;
  hasScript: boolean;
  /** The benchmark tier the description states, if it states one. */
  tier?: number;
  /** Site path of the thumbnail, or undefined when the scene has none. */
  thumbnail?: string;
};

type SceneJson = {
  TITLE?: unknown;
  DESCRIPTION?: unknown;
  CREDIT?: unknown;
  LICENSE?: unknown;
  TAGS?: unknown;
  CONTROLS?: unknown;
  PASSES?: unknown;
  IMAGE_PATH?: unknown;
};

const text = (value: unknown): string | undefined =>
  typeof value === "string" && value.trim().length > 0 ? value.trim() : undefined;

/** "Benchmark tier 3: heavy ALU" → 3. Only the scenes' own wording is read. */
export function parseTier(description: string): number | undefined {
  const match = /\btier (\d)\b/i.exec(description);
  return match ? Number(match[1]) : undefined;
}

/** Turns one parsed scene.json into a summary. Exported for testing. */
export function summariseScene(
  slug: string,
  json: SceneJson,
  hasScript: boolean,
): SceneSummary {
  const description = text(json.DESCRIPTION) ?? "";
  const image = text(json.IMAGE_PATH);
  const extension = image ? path.extname(image).slice(1).toLowerCase() : undefined;
  // sync-assets.mjs re-encodes every jpg or png thumbnail to WebP.
  return {
    slug,
    title: text(json.TITLE) ?? slug,
    description,
    credit: text(json.CREDIT),
    license: text(json.LICENSE),
    tags: Array.isArray(json.TAGS)
      ? json.TAGS.filter((tag): tag is string => typeof tag === "string")
      : [],
    controls: Array.isArray(json.CONTROLS) ? json.CONTROLS.length : 0,
    passes: Array.isArray(json.PASSES) ? json.PASSES.length : 0,
    hasScript,
    tier: parseTier(description),
    thumbnail:
      extension === "jpg" || extension === "png"
        ? `/generated/scenes/${slug}.webp`
        : undefined,
  };
}

/** Every scene in scenes/, in folder order (which is the library's order). */
export async function readScenes(): Promise<SceneSummary[]> {
  const entries = await readdir(SCENES_DIR, { withFileTypes: true });
  const folders = entries
    .filter((entry) => entry.isDirectory() && entry.name.endsWith(".synScene"))
    .map((entry) => entry.name)
    .sort();

  return Promise.all(
    folders.map(async (folder) => {
      const directory = path.join(SCENES_DIR, folder);
      const raw = await readFile(path.join(directory, "scene.json"), "utf8");
      let json: SceneJson;
      try {
        json = JSON.parse(raw) as SceneJson;
      } catch (error) {
        throw new Error(`scenes/${folder}/scene.json does not parse: ${String(error)}`);
      }
      return summariseScene(
        folder.replace(/\.synScene$/, ""),
        json,
        existsSync(path.join(directory, "script.js")),
      );
    }),
  );
}
