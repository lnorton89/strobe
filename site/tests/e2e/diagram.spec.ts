import { expect, test } from "@playwright/test";
import { preparePage } from "./prepare";

/*
 * docs/ARCHITECTURE.md draws the crate tree and the dependency graph as
 * untagged fenced blocks of box characters. Through the shared window chrome
 * they would read as a terminal transcript — which they are not: nothing
 * printed them, they are figures.
 *
 * These checks are about the shape of the block, never its wording, which is
 * also how the renderer decides. An edit upstream changes what the figure says
 * and none of this.
 */

test("the crate figure is a diagram, not a terminal window", async ({ page }) => {
  await page.goto("/architecture/");
  await preparePage(page);
  await page.evaluate(() => {
    document.querySelectorAll("details").forEach((node) => (node.open = true));
  });

  const diagram = page.getByRole("main").locator("figure:not([data-window])").first();
  await expect(diagram).toBeVisible();

  /*
   * No window chrome. The three things every window bar carries and this must
   * not: the mark, the three controls, and a path beside them. It is captioned
   * as what it is instead.
   */
  await expect(diagram.locator("svg")).toHaveCount(0);
  await expect(diagram.locator("div[aria-hidden='true'] > span")).toHaveCount(0);
  await expect(diagram.locator("figcaption")).toHaveText(/diagram/i);
});

test("a branching figure keeps the author's drawing, columns intact", async ({ page }) => {
  await page.goto("/architecture/");
  await preparePage(page);

  // The dependency graph branches, so it cannot be redrawn as a run of boxes;
  // it must arrive as the author drew it.
  const art = page
    .getByRole("main")
    .locator("figure:not([data-window]) pre")
    .filter({ hasText: "strobe-scene" })
    .first();
  await expect(art).toBeVisible();

  // Centring each line separately would pull a box drawing's columns apart;
  // the block is centred, its lines are not.
  await expect(art).toHaveCSS("text-align", "left");
});

test("no window on the page is titled after an unlabelled block", async ({ page }) => {
  await page.goto("/architecture/");
  await preparePage(page);
  // "Output" was the title the diagram used to be given.
  const titles = page.getByRole("main").locator("figure[data-window] span.text-center");
  for (const title of await titles.all()) {
    expect((await title.textContent())?.trim().toLowerCase()).not.toBe("output");
  }
});
