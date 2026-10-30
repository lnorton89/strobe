import { describe, expect, it } from "vitest";
import { UNKNOWN_RELEASE, parseRelease, releaseLabel } from "../../src/lib/release";

/*
 * The release the site names comes from GitHub's latest release (lib/release.ts),
 * never a constant. These pin how the API's payload is read and what the site
 * says when GitHub gives nothing usable.
 */

describe("the latest release", () => {
  it("reads the tag, the page and the date from GitHub's payload", () => {
    const release = parseRelease({
      tag_name: "v0.2.0",
      html_url: "https://github.com/lnorton89/strobe/releases/tag/v0.2.0",
      published_at: "2026-10-26T22:57:44Z",
      prerelease: false,
    });
    expect(release).toEqual({
      tag: "v0.2.0",
      url: "https://github.com/lnorton89/strobe/releases/tag/v0.2.0",
      published: "2026-10-26T22:57:44Z",
      prerelease: false,
    });
    expect(release && releaseLabel(release)).toBe("v0.2.0");
  });

  it("adds the v a bare tag lacks", () => {
    expect(parseRelease({ tag_name: "1.0.0" })?.tag).toBe("v1.0.0");
  });

  it("refuses a payload with no version tag", () => {
    expect(parseRelease({ message: "Not Found" })).toBeUndefined();
    expect(parseRelease({ tag_name: "nightly" })).toBeUndefined();
    expect(parseRelease(null)).toBeUndefined();
  });

  it("names no number when GitHub could not be asked", () => {
    expect(releaseLabel(UNKNOWN_RELEASE)).toBe("Latest release");
    expect(UNKNOWN_RELEASE.url).toMatch(/\/releases$/);
  });
});
