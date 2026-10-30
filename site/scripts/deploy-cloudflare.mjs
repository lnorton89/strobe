/*
 * Deploys the static export (out/) to Cloudflare as a static-assets Worker
 * (wrangler.jsonc), at strobe.<account subdomain>.workers.dev, as the account
 * named in a private credentials file — never the machine's wrangler login.
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
 *   CLOUDFLARE_API_TOKEN=<a token with Account > Workers Scripts > Edit>
 *
 * Usage (it builds the site first, for the address it is served from,
 * STROBE_SITE_URL or https://strobe.lnorton89.workers.dev):
 *   npm run deploy:cf            the live site
 *   npm run deploy:cf:preview    a version with its own preview URL
 */

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const WRANGLER = "wrangler@4.156.0";
const WORKER = process.env.STROBE_WORKER_NAME ?? "strobe";
/** Where the Worker is served: its name under the account's workers.dev subdomain. */
const SITE_URL = (process.env.STROBE_SITE_URL ?? "https://strobe.lnorton89.workers.dev").replace(
  /\/$/,
  "",
);
const FILE =
  process.env.STROBE_CLOUDFLARE_ENV ?? path.join(os.homedir(), ".config", "strobe", "cloudflare.env");

function fail(message) {
  console.error(`deploy-cloudflare: ${message}`);
  process.exit(1);
}

if (!existsSync(FILE)) {
  fail(
    `no credentials file at ${FILE}.\nCreate it with CLOUDFLARE_ACCOUNT_ID=… and CLOUDFLARE_API_TOKEN=… (a token with Account > Workers Scripts > Edit) for the account to deploy to.`,
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
// The site writes its own origin into canonical links and social-card URLs
// at build time (lib/site.ts), and a plain local build names the test
// server's localhost. So the deploy builds the site itself for the address
// it will be served from, and checks the result before publishing.
const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const built = spawnSync(npm, ["run", "build"], {
  env: { ...process.env, NEXT_PUBLIC_SITE_URL: SITE_URL },
  stdio: "inherit",
  shell: process.platform === "win32",
});
if (built.status !== 0) fail("the build failed; nothing was deployed");
const index = path.join(process.cwd(), "out", "index.html");
if (!existsSync(index)) fail("the build left no out/index.html");
if (!readFileSync(index, "utf8").includes(`<link rel="canonical" href="${SITE_URL}/"`)) {
  fail(`out/index.html does not name ${SITE_URL} as its canonical address; nothing was deployed`);
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
// only when the token may list memberships, which a narrow token may
// not; the id is what wrangler deploys to either way.
const who = wrangler(["whoami"], { quiet: true });
const account = /│\s*(.+?)\s*│\s*[0-9a-f]{32}\s*│/.exec(who.stdout ?? "")?.[1];
console.log(
  `deploy-cloudflare: account ${account ? `${account} (${credentials.CLOUDFLARE_ACCOUNT_ID})` : credentials.CLOUDFLARE_ACCOUNT_ID}, worker ${WORKER}`,
);

// The bucket shared scenes are stored in (D527) is made on the first deploy;
// an existing one, and what is in it, is left alone.
const BUCKET = "strobe-scenes";
const buckets = wrangler(["r2", "bucket", "list"], { quiet: true });
if (buckets.status !== 0) {
  // wrangler writes its API errors to stdout; both streams are shown.
  fail(`wrangler could not list R2 buckets:\n${buckets.stdout ?? ""}\n${buckets.stderr ?? ""}`);
}
if (!new RegExp(`\\b${BUCKET}\\b`).test(buckets.stdout ?? "")) {
  const made = wrangler(["r2", "bucket", "create", BUCKET]);
  if (made.status !== 0) fail(`could not create the R2 bucket ${BUCKET}`);
}

// wrangler.jsonc beside package.json describes the Worker: the static export
// as assets plus the share routes, on the account's workers.dev subdomain.
// A preview uploads a version with its own preview URL and leaves the live
// site as it is.
const deployed = wrangler(
  preview ? ["versions", "upload", "--name", WORKER] : ["deploy", "--name", WORKER],
);
process.exit(deployed.status ?? 1);
