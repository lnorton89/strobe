import { expect, test, type Page } from "@playwright/test";

/*
 * A new release reaches the site with no code edit and no deploy (D557).
 *
 * GitHub is stubbed with a version that does not exist, and every place the home
 * and download pages name the release has to follow it. What the build wrote is
 * the other half: with GitHub down or JavaScript off, the page keeps a working
 * link and never an invented number.
 */

const API = "https://api.github.com/repos/*/*/releases/latest";

const ZIP = "strobe-v9.9.9-x86_64-pc-windows-msvc.zip";

const NEW_RELEASE = {
  tag_name: "v9.9.9",
  published_at: "2099-01-02T03:04:05Z",
  assets: [
    { name: "strobe-v9.9.9-sbom.cdx.json", size: 1 },
    { name: "strobe-v9.9.9-x86_64-pc-windows-msvc-symbols.zip", size: 2 },
    { name: ZIP, size: 12_300_000 },
    { name: `${ZIP}.sha256`, size: 3 },
  ],
};

async function publish(page: Page, release: unknown = NEW_RELEASE) {
  await page.route(API, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "access-control-allow-origin": "*" },
      body: JSON.stringify(release),
    }),
  );
}

/** Every release version in the page's visible text. */
async function versionsOnPage(page: Page) {
  const text = await page.locator("body").innerText();
  return [...new Set(text.match(/\bv\d+\.\d+\.\d+\b/g) ?? [])];
}

test.describe("a release published after the last deploy", () => {
  test("the header and the home page follow it, and name nothing else", async ({ page }) => {
    await publish(page);
    await page.goto("/");

    const header = page.getByRole("banner").locator("[data-release]");
    await expect(header).toHaveText("v9.9.9");
    await expect(header).toHaveAttribute("href", /\/releases\/tag\/v9\.9\.9$/);
    await expect(page.getByRole("heading", { name: /v9\.9\.9 is the current release/ })).toBeVisible();
    await expect(page.getByRole("main").locator("a[data-release]")).toHaveText("v9.9.9 · testing");
    expect(await versionsOnPage(page)).toEqual(["v9.9.9"]);
  });

  test("the download page offers its zip by name, with its size and date", async ({ page }) => {
    await publish(page);
    await page.goto("/download/");

    const download = page.getByRole("link", { name: "Download v9.9.9 for Windows" });
    await expect(download).toHaveAttribute(
      "href",
      new RegExp(`/releases/download/v9\\.9\\.9/${ZIP.replaceAll(".", "\\.")}$`),
    );
    const block = page.locator("[data-release-download]");
    await expect(block).toContainText(ZIP);
    await expect(block).toContainText("12.3 MB");
    await expect(block).toContainText("published 2099-01-02");
    await expect(block.getByRole("link", { name: "SHA-256" })).toHaveAttribute(
      "href",
      new RegExp(`${ZIP.replaceAll(".", "\\.")}\\.sha256$`),
    );
    await expect(block.getByRole("link", { name: "Release notes" })).toHaveAttribute(
      "href",
      /\/releases\/tag\/v9\.9\.9$/,
    );
    expect(await versionsOnPage(page)).toEqual(["v9.9.9"]);
  });

  test("a second page in the session costs no second request", async ({ page }) => {
    let requests = 0;
    await page.route(API, (route) => {
      requests += 1;
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        headers: { "access-control-allow-origin": "*" },
        body: JSON.stringify(NEW_RELEASE),
      });
    });
    await page.goto("/");
    await expect(page.getByRole("banner").locator("[data-release]")).toHaveText("v9.9.9");
    await page.goto("/guide/");
    await expect(page.getByRole("banner").locator("[data-release]")).toHaveText("v9.9.9");
    expect(requests).toBe(1);
  });
});

test.describe("when GitHub gives nothing", () => {
  for (const [name, answer] of [
    ["is unreachable", null],
    ["rate limits the reader", { status: 403, body: '{"message":"API rate limit exceeded"}' }],
    ["answers with no version", { status: 200, body: '{"message":"Not Found"}' }],
  ] as const) {
    test(`the build's answer stays when GitHub ${name}`, async ({ page }) => {
      const built = await (await page.request.get("/generated/release.json")).json();
      await page.route(API, (route) =>
        answer ? route.fulfill({ ...answer, contentType: "application/json" }) : route.abort("failed"),
      );
      await page.goto("/download/");
      const download = page.locator("[data-release-download]").getByRole("link").first();
      await expect(download).toBeVisible();
      await expect(download).toHaveAttribute("href", /github\.com\/.+\/releases\//);
      const expected = (built as { payload?: { tag_name?: string } }).payload?.tag_name;
      await expect(page.getByRole("banner").locator("[data-release]")).toHaveText(
        expected ?? "Latest release",
      );
    });
  }

  test("with no JavaScript the page is complete, from the build", async ({ browser }) => {
    const context = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 1280, height: 900 } });
    const page = await context.newPage();
    await page.goto("/download/");
    const download = page.locator("[data-release-download]").getByRole("link").first();
    await expect(download).toHaveAttribute("href", /github\.com\/.+\/releases\//);
    await expect(page.getByRole("banner").locator("[data-release]")).toHaveAttribute(
      "href",
      /github\.com\/.+\/releases\//,
    );
    await context.close();
  });
});
