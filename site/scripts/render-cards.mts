/*
 * Renders the social cards into public/, before `next dev` and `next build`.
 *
 * Through Metaplate's own command (D557), which writes every card where its
 * metadata says it is, checks each file's format and size, and fails on what
 * the plate's guards catch. Called in-process because the plate is JSX and
 * Node cannot load that alone: this script runs under tsx, whose loader the
 * command's import then goes through. That also avoids `NODE_OPTIONS=`, which
 * an npm script cannot set on Windows.
 *
 * Every card, every time: eight take about two seconds, and --incremental cannot
 * see a scene's thumbnail changing, since the plate reads it. The manifest is kept
 * out of public/ so it is not published.
 */

import { runCli } from "metaplate/cli";

const code = await runCli([
  "render",
  "--out",
  "public",
  "--manifest",
  ".og-manifest.json",
  "src/lib/og/cards.render.tsx",
]);
process.exit(code);
