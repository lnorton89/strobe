import { describe, expect, it } from "vitest";
import { verifySocialImage } from "metaplate";
import { CARDS } from "../../src/lib/og/cards";
import { pageMetadata } from "../../src/lib/og/metadata";
import { og } from "../../src/lib/og/plate";
import { EXPORTED_ROUTES } from "../../src/lib/site";

/*
 * The social cards (D557). Rendering them here runs the plate's guards, so a
 * title that no longer fits or a character no font covers fails the unit tests,
 * as well as the build's `metaplate render`.
 */

describe("the social cards", () => {
  it("has one for every exported route", () => {
    expect(Object.keys(CARDS).sort()).toEqual([...EXPORTED_ROUTES].sort());
  });

  for (const route of EXPORTED_ROUTES) {
    it(`renders ${route} as a 1200x630 JPEG, and the guards pass`, async () => {
      const bytes = await og.render(CARDS[route]);
      const [image] = og.metadata(route, CARDS[route]).openGraph.images;
      const report = verifySocialImage(bytes, { ...image, url: `https://example.com${image.url}` }, {
        // WhatsApp's 600 KB is the limit the PNG cards went over. Next makes the URL absolute.
        targets: ["universal", "whatsapp", "linkedin", "discord", "x"],
      });
      expect(report.issues).toEqual([]);
      expect(report.actual).toMatchObject({ width: 1200, height: 630, format: "jpeg" });
    }, 30_000);
  }

  it("states the fields a page would otherwise lose to Next's shallow merge", () => {
    const metadata = pageMetadata("/download/", { title: "Download", description: "Words." });
    expect(metadata.openGraph).toMatchObject({
      siteName: "Strobe",
      url: "/download/",
      title: CARDS["/download/"].title,
      description: "Words.",
    });
    const images = metadata.openGraph?.images;
    expect(JSON.stringify(images)).toMatch(/\/download\/og-image\.jpg\?v=/);
    expect(metadata.twitter).toMatchObject({ card: "summary_large_image" });
  });

  it("gives a changed card a new URL, and an unchanged one the same", () => {
    const copy = CARDS["/guide/"];
    const url = (title: string) =>
      JSON.stringify(og.metadata("/guide/", { ...copy, title }).openGraph.images);
    expect(url(copy.title)).toBe(url(copy.title));
    expect(url(copy.title)).not.toBe(url("Something else"));
  });
});
