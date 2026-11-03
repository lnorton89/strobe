import { describe, expect, it } from "vitest";
import { EXPORTED_ROUTES, isActiveRoute, NAV } from "@/lib/site";

describe("isActiveRoute", () => {
  it("matches the home route only exactly", () => {
    expect(isActiveRoute("/", "/")).toBe(true);
    expect(isActiveRoute("/guide/", "/")).toBe(false);
  });

  it("matches a route with or without its trailing slash", () => {
    // `trailingSlash: true` means the browser reports "/guide/", but a
    // link or test may use either form.
    expect(isActiveRoute("/guide/", "/guide/")).toBe(true);
    expect(isActiveRoute("/guide", "/guide/")).toBe(true);
    expect(isActiveRoute("/guide/", "/guide")).toBe(true);
  });

  it("treats a nested route as part of its family", () => {
    expect(isActiveRoute("/reference/cli/", "/reference/")).toBe(true);
  });

  it("does not match a route that merely shares a prefix", () => {
    expect(isActiveRoute("/references/", "/reference/")).toBe(false);
    expect(isActiveRoute("/install-notes/", "/install/")).toBe(false);
  });
});

describe("navigation", () => {
  it("exposes every primary route in the complete export registry", () => {
    expect(EXPORTED_ROUTES[0]).toBe("/");
    for (const item of NAV) expect(EXPORTED_ROUTES).toContain(item.href);
  });

  it("uses trailing slashes throughout, matching the export shape", () => {
    for (const item of NAV) {
      expect(item.href.endsWith("/")).toBe(true);
    }
  });

  it("gives every nav item a label and a blurb", () => {
    for (const item of NAV) {
      expect(item.label.length).toBeGreaterThan(0);
      expect(item.blurb.length).toBeGreaterThan(0);
    }
  });
});
