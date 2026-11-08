/*
 * Puts the in-browser CLIP's files in this site's R2 bucket (D560), which the
 * Worker serves at /api/models/. Run it once, and again only for a new model:
 *
 *   npm run upload:clip
 *
 * Fetches each file from Hugging Face first, then uploads it as the account in
 * the same private credentials file deploy-cloudflare.mjs uses.
 */

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const WRANGLER = "wrangler@4.156.0";
const BUCKET = "strobe-models";
const SOURCE = "https://huggingface.co";
const REVISION = "main";
const FILES = [
  "Xenova/clip-vit-base-patch32/config.json",
  "Xenova/clip-vit-base-patch32/preprocessor_config.json",
  "Xenova/clip-vit-base-patch32/onnx/vision_model_quantized.onnx",
];

const FILE =
  process.env.STROBE_CLOUDFLARE_ENV ?? path.join(os.homedir(), ".config", "strobe", "cloudflare.env");
if (!existsSync(FILE)) {
  console.error(`upload-clip-model: no credentials file at ${FILE}`);
  process.exit(1);
}
const credentials = {};
for (const line of readFileSync(FILE, "utf8").split(/\r?\n/)) {
  const match = /^\s*(CLOUDFLARE_ACCOUNT_ID|CLOUDFLARE_API_TOKEN)\s*=\s*(.+?)\s*$/.exec(line);
  if (match) credentials[match[1]] = match[2].replace(/^["']|["']$/g, "");
}
const env = { ...process.env, ...credentials };
const npx = process.platform === "win32" ? "npx.cmd" : "npx";
function wrangler(args, quiet = false) {
  return spawnSync(npx, ["--yes", WRANGLER, ...args], {
    env,
    stdio: quiet ? "pipe" : "inherit",
    encoding: "utf8",
    shell: process.platform === "win32",
  });
}

const buckets = wrangler(["r2", "bucket", "list"], true);
if (buckets.status !== 0) {
  console.error(`upload-clip-model: could not list buckets:\n${buckets.stdout ?? ""}${buckets.stderr ?? ""}`);
  process.exit(1);
}
if (!new RegExp(`\\b${BUCKET}\\b`).test(buckets.stdout ?? "")) {
  if (wrangler(["r2", "bucket", "create", BUCKET]).status !== 0) process.exit(1);
}

const scratch = mkdtempSync(path.join(os.tmpdir(), "strobe-clip-"));
for (const name of FILES) {
  const [owner, model, ...rest] = name.split("/");
  const url = `${SOURCE}/${owner}/${model}/resolve/${REVISION}/${rest.join("/")}`;
  const response = await fetch(url);
  if (!response.ok) {
    console.error(`upload-clip-model: ${url} answered ${response.status}`);
    process.exit(1);
  }
  const local = path.join(scratch, ...name.split("/"));
  mkdirSync(path.dirname(local), { recursive: true });
  writeFileSync(local, Buffer.from(await response.arrayBuffer()));
  const put = wrangler(["r2", "object", "put", `${BUCKET}/${name}`, "--file", local, "--remote"]);
  if (put.status !== 0) process.exit(1);
}
console.log(`upload-clip-model: ${FILES.length} files in ${BUCKET}`);
