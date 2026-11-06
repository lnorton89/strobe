import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

/*
 * Guards the deploy configuration, which the Playwright run cannot reach: it is
 * served by `serve`, which knows nothing about netlify.toml.
 */

const CONFIG = readFileSync(path.join(process.cwd(), "netlify.toml"), "utf8");

/** Returns the values block for a `[[headers]]` rule matching `forPath`. */
function headerValues(forPath: string): string | undefined {
  const rules = CONFIG.split("[[headers]]").slice(1);
  const rule = rules.find((entry) =>
    new RegExp(`for\\s*=\\s*"${forPath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"`).test(
      entry,
    ),
  );
  return rule;
}

describe("netlify.toml", () => {
  it("publishes the static export", () => {
    expect(CONFIG).toMatch(/publish\s*=\s*"out"/);
    expect(CONFIG).toMatch(/command\s*=\s*"npm run build"/);
  });

  it("pins a Node version, so the deploy matches local builds", () => {
    expect(CONFIG).toMatch(/NODE_VERSION\s*=\s*"22"/);
  });

  it("skips the Next runtime plugin, which a static export cannot use", () => {
    /*
     * Netlify auto-installs @netlify/plugin-nextjs on any project with Next in
     * its dependencies. That runtime exists to stand up a server — SSR
     * handlers, the ISR cache, the image CDN — and this site is
     * `output: "export"`, so there is no server build for its onPostBuild step
     * to publish. It failed a real deploy with "Failed publishing static
     * content" before this was set.
     *
     * This is not optional configuration: `npm run deploy` is
     * `netlify deploy --prod`, and Netlify CLI 27 always builds before
     * publishing, so every deploy goes through the path this flag disarms.
     */
    const environment = CONFIG.split("[build.environment]")[1]?.split("\n[")[0];
    expect(environment, "no [build.environment] section").toBeDefined();
    expect(environment).toMatch(/NETLIFY_NEXT_PLUGIN_SKIP\s*=\s*"true"/);
  });

  it("sends the security headers a static site should", () => {
    const rule = headerValues("/*");
    expect(rule).toMatch(/X-Content-Type-Options\s*=\s*"nosniff"/);
    expect(rule).toMatch(/Referrer-Policy/);
    expect(rule).toMatch(/X-Frame-Options/);
  });

  it("caches fingerprinted build assets immutably", () => {
    const rule = headerValues("/_next/static/*");
    expect(rule).toMatch(/immutable/);
  });

  it("does not configure a build hook, since deploys are manual", () => {
    // A [[plugins]] or build hook here would reintroduce deploy-on-push.
    expect(CONFIG).not.toMatch(/\[\[plugins\]\]/);
  });
});
