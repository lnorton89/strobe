/*
 * Builds the /play page's engine: crates/strobe-web compiled to wasm, with
 * its JavaScript bindings, into public/generated/engine/ (D524).
 *
 * The engine is the app's own scene player, so it is built from the
 * repository on every site build rather than committed: a committed .wasm
 * would be a second copy of the renderer that drifts from the crates.
 *
 * Needs Rust's wasm32-unknown-unknown target and wasm-bindgen-cli at the
 * version Cargo.lock pins; when either is missing it says how to get it and
 * fails, so a build cannot ship a /play page with no engine behind it.
 *
 * Runs after sync-assets (which empties public/generated/) in predev and
 * prebuild, and alone as `npm run engine`.
 */

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import path from "node:path";

const SITE = process.cwd();
const REPO = path.join(SITE, "..");
const OUT = path.join(SITE, "public", "generated", "engine");
const PROFILE = "release-web";
const WASM = path.join(
  REPO,
  "target",
  "wasm32-unknown-unknown",
  PROFILE,
  "strobe_web.wasm",
);

function run(command, args) {
  execFileSync(command, args, { cwd: REPO, stdio: "inherit" });
}

function fail(message) {
  console.error(`build-engine: ${message}`);
  process.exit(1);
}

/** The wasm-bindgen version Cargo.lock pins; the CLI must match it. */
function pinnedBindgen() {
  const lock = readFileSync(path.join(REPO, "Cargo.lock"), "utf8");
  const match = /name = "wasm-bindgen"\nversion = "([^"]+)"/.exec(lock);
  return match?.[1];
}

const version = pinnedBindgen();
let installed;
try {
  installed = execFileSync("wasm-bindgen", ["--version"], { encoding: "utf8" }).trim();
} catch {
  fail(
    `wasm-bindgen is not installed. Run:\n  rustup target add wasm32-unknown-unknown\n  cargo install wasm-bindgen-cli --version ${version} --locked`,
  );
}
if (version && !installed.endsWith(version)) {
  fail(
    `${installed} does not match the wasm-bindgen ${version} in Cargo.lock. Run:\n  cargo install wasm-bindgen-cli --version ${version} --locked`,
  );
}

run("cargo", [
  "build",
  "-p",
  "strobe-web",
  "--target",
  "wasm32-unknown-unknown",
  "--profile",
  PROFILE,
]);
if (!existsSync(WASM)) fail(`${path.relative(REPO, WASM)} was not built`);

mkdirSync(OUT, { recursive: true });
run("wasm-bindgen", ["--target", "web", "--no-typescript", "--out-dir", OUT, WASM]);
