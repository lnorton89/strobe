/*
 * Copies the in-browser CLIP's library and wasm into public/generated/clip/
 * (D560), so From image serves them itself: a page that may hold a visitor's
 * API key runs no script from another site. The versions are the ones
 * package-lock.json names.
 *
 * Runs after sync-assets, which empties public/generated/, in predev and
 * prebuild. Each file is under Cloudflare's 25 MiB limit on one asset.
 */

import { copyFileSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);
const OUT = path.join(process.cwd(), "public", "generated", "clip");
mkdirSync(OUT, { recursive: true });

const transformers = path.dirname(require.resolve("@huggingface/transformers"));
const runtime = path.join(process.cwd(), "node_modules", "onnxruntime-web", "dist");
const FILES = [
  [path.join(transformers, "transformers.min.js"), "transformers.min.js"],
  [path.join(runtime, "ort-wasm-simd-threaded.jsep.mjs"), "ort-wasm-simd-threaded.jsep.mjs"],
  [path.join(runtime, "ort-wasm-simd-threaded.jsep.wasm"), "ort-wasm-simd-threaded.jsep.wasm"],
];
for (const [from, name] of FILES) copyFileSync(from, path.join(OUT, name));
console.log(`copy-clip: ${FILES.length} files in public/generated/clip`);
