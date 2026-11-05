import { expect, test } from "@playwright/test";
import { ROUTES } from "./routes";

/*
 * A page load is quiet: nothing 404s, nothing throws, nothing goes to a font
 * service.
 *
 * This is what caught every page logging eight 404s, for the `<Link>` prefetches
 * Next asks for at names a static export does not write
 * (scripts/flatten-segments.mjs). GitHub is stubbed so the test depends on no
 * network and no rate limit.
 */

for (const route of [...ROUTES, { name: "play", path: "/play/" }]) {
  test(`${route.name} loads with no errors and nothing from a font service`, async ({ page }) => {
    const problems: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") problems.push(`console: ${message.text()}`);
    });
    page.on("pageerror", (error) => problems.push(`exception: ${String(error)}`));
    page.on("response", (response) => {
      if (response.status() >= 400) problems.push(`${response.status()} ${response.url()}`);
    });
    page.on("request", (request) => {
      if (/fonts\.(googleapis|gstatic)\.com/.test(request.url())) {
        problems.push(`font service: ${request.url()}`);
      }
    });
    await page.route("https://api.github.com/**", (rule) =>
      rule.fulfill({
        status: 200,
        contentType: "application/json",
        headers: { "access-control-allow-origin": "*" },
        body: "{}",
      }),
    );

    await page.goto(route.path);
    await page.waitForLoadState("networkidle");

    expect(problems).toEqual([]);
  });
}

test("the fonts are served from this origin, under their licences", async ({ page, request }) => {
  const origins = new Set<string>();
  page.on("response", (response) => {
    if (/\.woff2?(\?|$)/.test(response.url())) origins.add(new URL(response.url()).origin);
  });
  await page.goto("/");
  await page.evaluate(() => document.fonts.ready);
  expect([...origins]).toEqual(["http://localhost:4173"]);

  for (const font of ["inter", "jetbrains-mono", "space-grotesk"]) {
    const licence = await request.get(`/generated/fonts/${font}-LICENSE.txt`);
    expect(licence.status(), font).toBe(200);
    expect(await licence.text()).toContain("SIL Open Font License");
  }
});
