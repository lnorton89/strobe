/*
 * Puts the export's prefetch files where the browser asks for them.
 *
 * Next's client asks for a route segment's data at a dotted name,
 * `download/__next.download.__PAGE__.txt`, but `output: "export"` writes it in
 * a directory, `download/__next.download/__PAGE__.txt`. With no server to
 * translate, every `<Link>` in view logged a 404 on every page load, on every
 * static host. Each nested file is copied to the name the client uses, which
 * also makes the prefetches work.
 *
 * Runs as `postbuild`. tests/e2e/console.spec.ts fails when a page load logs an
 * error, so this comes to light if a Next upgrade changes either side.
 */

import { copyFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";

const OUT = path.join(process.cwd(), "out");

/** Copies every file under a `__next.*` directory to its flat name beside it. */
function flatten(directory, prefix) {
  for (const name of readdirSync(directory)) {
    const file = path.join(directory, name);
    if (statSync(file).isDirectory()) flatten(file, `${prefix}.${name}`);
    else copyFileSync(file, path.join(flatten.root, `${prefix}.${name}`));
  }
}

let copied = 0;

function walk(directory) {
  for (const name of readdirSync(directory)) {
    const entry = path.join(directory, name);
    if (!statSync(entry).isDirectory() || name === "_next" || name === "generated") continue;
    if (name.startsWith("__next.")) {
      flatten.root = directory;
      flatten(entry, name);
      copied += 1;
    } else {
      walk(entry);
    }
  }
}

walk(OUT);
console.log(`flatten-segments: ${copied} segment director${copied === 1 ? "y" : "ies"}`);
