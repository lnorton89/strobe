/*
 * Deploys the static export (out/) to Cloudflare Pages with wrangler, as the
 * account named in a private credentials file — never the machine's wrangler
 * login.
 *
 * Why: this machine's `wrangler login` belongs to another account that other
 * work deploys with, and wrangler keeps one login per user profile. Logging
 * in again would sign that work out. Wrangler instead takes an API token and
 * account id from the environment, per process, ahead of its login, so this
 * script reads them from a file and hands them to its own wrangler child only.
 * The login file is never read or written.
 *
 * The file (default ~/.config/strobe/cloudflare.env, or STROBE_CLOUDFLARE_ENV)
 * holds two lines, and stays outside the repository:
 *
 *   CLOUDFLARE_ACCOUNT_ID=<the account's id>
 *   CLOUDFLARE_API_TOKEN=<a token with Account > Cloudflare Pages > Edit>
 *
 * Usage (after `npm run build`):
 *   npm run deploy:cf            production (branch main)
 *   npm run deploy:cf:preview    a preview deployment (branch preview)
 */

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const WRANGLER = "wrangler@4.156.0";
const PROJECT = process.env.STROBE_PAGES_PROJECT ?? "strobe";
const FILE =
  process.env.STROBE_CLOUDFLARE_ENV ?? path.join(os.homedir(), ".config", "strobe", "cloudflare.env");

function fail(message) {
  console.error(`deploy-cloudflare: ${message}`);
  process.exit(1);
}

if (!existsSync(FILE)) {
  fail(
    `no credentials file at ${FILE}.\nCreate it with CLOUDFLARE_ACCOUNT_ID=… and CLOUDFLARE_API_TOKEN=… (a token with Account > Cloudflare Pages > Edit) for the account to deploy to.`,
  );
}
const credentials = {};
for (const line of readFileSync(FILE, "utf8").split(/\r?\n/)) {
  const match = /^\s*(CLOUDFLARE_ACCOUNT_ID|CLOUDFLARE_API_TOKEN)\s*=\s*(.+?)\s*$/.exec(line);
  if (match) credentials[match[1]] = match[2].replace(/^["']|["']$/g, "");
}
for (const key of ["CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_API_TOKEN"]) {
  if (!credentials[key]) fail(`${FILE} has no ${key}`);
}
if (!existsSync(path.join(process.cwd(), "out", "index.html"))) {
  fail("out/ has no build; run `npm run build` first");
}

// Only the child sees the credentials; nothing is exported to this shell.
const env = { ...process.env, ...credentials };
const npx = process.platform === "win32" ? "npx.cmd" : "npx";
function wrangler(args, { quiet = false } = {}) {
  const result = spawnSync(npx, ["--yes", WRANGLER, ...args], {
    env,
    stdio: quiet ? "pipe" : "inherit",
    encoding: "utf8",
    shell: process.platform === "win32",
  });
  return result;
}

const preview = process.argv.includes("--preview");

// Say which account this is before anything is published. `whoami` names it
// only when the token may list memberships, which a Pages-only token may
// not; the id is what wrangler deploys to either way.
const who = wrangler(["whoami"], { quiet: true });
const account = /│\s*(.+?)\s*│\s*[0-9a-f]{32}\s*│/.exec(who.stdout ?? "")?.[1];
console.log(
  `deploy-cloudflare: account ${account ? `${account} (${credentials.CLOUDFLARE_ACCOUNT_ID})` : credentials.CLOUDFLARE_ACCOUNT_ID}, project ${PROJECT}`,
);

// The project is created on the first deploy; an existing one is left alone.
// `--force` makes it a Pages project: without it, wrangler 4.156 hands a new
// project to Workers static assets, which needs Workers permissions a Pages
// token does not have, and deploys nothing.
const list = wrangler(["pages", "project", "list"], { quiet: true });
if (list.status !== 0) fail(`wrangler could not use the token:\n${list.stderr || list.stdout}`);
if (!new RegExp(`\\b${PROJECT}\\b`).test(list.stdout ?? "")) {
  const made = wrangler([
    "pages",
    "project",
    "create",
    PROJECT,
    "--production-branch",
    "main",
    "--force",
  ]);
  if (made.status !== 0) fail(`could not create the Pages project ${PROJECT}`);
}

const deployed = wrangler([
  "pages",
  "deploy",
  "out",
  "--project-name",
  PROJECT,
  "--branch",
  preview ? "preview" : "main",
  "--commit-dirty=true",
]);
process.exit(deployed.status ?? 1);
