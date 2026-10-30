/*
 * Copies the repository's own images into public/generated/ before a build.
 *
 * The site shows real output (docs/branding.md section 7: "real output over
 * mockups"): the bundled scenes' thumbnails, the app screenshots in
 * docs/images/, and the icon PNGs `cargo xtask brand` renders. All of them
 * already live in the repository, so they are copied rather than committed a
 * second time — a second copy is a second thing to keep in step, and the
 * PNGs are binary. public/generated/ is git-ignored.
 *
 * Scene thumbnails are re-encoded rather than copied: some are 1280x720 PNGs
 * of 600 kB shown in a card 300 px wide. They become 640 px WebP, which is
 * enough for a 2x display at the card's largest size.
 *
 * Runs as `predev` and `prebuild`. Fails loudly if a source is missing, so a
 * renamed screenshot breaks the build instead of shipping a broken image.
 */

import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import sharp from "sharp";

const SITE = process.cwd();
const REPO = path.join(SITE, "..");
const OUT = path.join(SITE, "public", "generated");

/** Brand files the layout and the 404 page reference. */
const BRAND = ["icon-small.svg", "icon-32.png", "icon-256.png"];

/**
 * Screenshots the pages show, from docs/images/perform/. Every file directly
 * in docs/images/ is copied too, to generated/docs-images/, because the
 * repository documents the site renders link their figures from there
 * (lib/repo-docs.ts rewrites those links).
 */
const SCREENSHOTS = [
  "perform/browser-thumbnails-1440x900.png",
  "perform/editor-1440x900.png",
  "perform/diagnostics-1440x900.png",
  "perform/generate-controls-1440x900.png",
];

function copy(from, to) {
  if (!existsSync(from)) {
    throw new Error(`sync-assets: ${path.relative(REPO, from)} does not exist`);
  }
  mkdirSync(path.dirname(to), { recursive: true });
  copyFileSync(from, to);
}

rmSync(OUT, { recursive: true, force: true });

for (const file of BRAND) {
  copy(path.join(REPO, "assets", "brand", file), path.join(OUT, "brand", file));
}

for (const file of SCREENSHOTS) {
  copy(path.join(REPO, "docs", "images", file), path.join(OUT, "screens", path.basename(file)));
}

const docImages = path.join(REPO, "docs", "images");
let figures = 0;
for (const entry of readdirSync(docImages, { withFileTypes: true })) {
  if (!entry.isFile()) continue;
  copy(path.join(docImages, entry.name), path.join(OUT, "docs-images", entry.name));
  figures += 1;
}

/** Width of a re-encoded thumbnail: the card's widest rendering, at 2x. */
const THUMBNAIL_WIDTH = 640;

// Every scene folder that carries a thumbnail, by the name its scene.json gives.
const scenes = path.join(REPO, "scenes");
let thumbnails = 0;
for (const entry of readdirSync(scenes, { withFileTypes: true })) {
  if (!entry.isDirectory() || !entry.name.endsWith(".synScene")) continue;
  const source = ["jpg", "png"]
    .map((ext) => path.join(scenes, entry.name, `thumbnail.${ext}`))
    .find((file) => existsSync(file));
  if (!source) continue;
  const slug = entry.name.replace(/\.synScene$/, "");
  const target = path.join(OUT, "scenes", `${slug}.webp`);
  mkdirSync(path.dirname(target), { recursive: true });
  await sharp(source)
    .resize({ width: THUMBNAIL_WIDTH, withoutEnlargement: true })
    .webp({ quality: 82 })
    .toFile(target);
  thumbnails += 1;
}

/*
 * Each scene's text files as one JSON document for the /play page to open
 * (D524): the shaders, scene.json and script.js at the folder's top level.
 * Textures and media are left out; the sandbox does not load them yet
 * (backlog W-01).
 */
const SCENE_TEXT = /\.(glsl|frag|json|js)$/i;
let playable = 0;
for (const entry of readdirSync(scenes, { withFileTypes: true })) {
  if (!entry.isDirectory() || !entry.name.endsWith(".synScene")) continue;
  const dir = path.join(scenes, entry.name);
  const files = {};
  for (const file of readdirSync(dir, { withFileTypes: true })) {
    if (file.isFile() && SCENE_TEXT.test(file.name)) {
      files[file.name] = readFileSync(path.join(dir, file.name), "utf8");
    }
  }
  if (!files["main.glsl"]) continue;
  const slug = entry.name.replace(/\.synScene$/, "");
  const target = path.join(OUT, "play", `${slug}.json`);
  mkdirSync(path.dirname(target), { recursive: true });
  writeFileSync(target, JSON.stringify({ title: slug, files }));
  playable += 1;
}

console.log(
  `sync-assets: ${BRAND.length} brand files, ${SCREENSHOTS.length} screenshots, ${figures} document figures, ${thumbnails} scene thumbnails, ${playable} playable scenes`,
);
