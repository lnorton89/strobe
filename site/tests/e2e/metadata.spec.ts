import { expect, test } from "@playwright/test";
import { ROUTES } from "./routes";

/*
 * Deployment-shape checks.
 *
 * The site ships as a static export, so whatever the build wrote is exactly
 * what gets served. The social cards are files Metaplate rendered into public/
 * (D557), named `.jpg` so any static host serves them with the right type.
 */

/** A baseline JPEG's dimensions, from its start-of-frame marker. */
function jpegSize(bytes: Buffer): { width: number; height: number } | undefined {
  for (let at = 2; at + 9 < bytes.length; ) {
    const marker = bytes[at + 1];
    if (marker >= 0xc0 && marker <= 0xc2) {
      return { height: bytes.readUInt16BE(at + 5), width: bytes.readUInt16BE(at + 7) };
    }
    at += 2 + bytes.readUInt16BE(at + 2);
  }
  return undefined;
}

for (const route of ROUTES) {
  test(`${route.name} social image is a real 1200x630 JPEG`, async ({ request }) => {
    const response = await request.get(route.og);
    expect(response.status()).toBe(200);
    expect(response.headers()["content-type"]).toContain("image/jpeg");

    const body = await response.body();
    expect(body.subarray(0, 2)).toEqual(Buffer.from([0xff, 0xd8]));
    expect(jpegSize(body)).toEqual({ width: 1200, height: 630 });
  });
}

test("every page keeps the layout's Open Graph fields", async ({ page }) => {
  for (const route of ROUTES) {
    await page.goto(route.path);
    await expect(page.locator('meta[property="og:site_name"]'), route.name).toHaveAttribute(
      "content",
      "Strobe",
    );
    await expect(page.locator('meta[property="og:type"]'), route.name).toHaveAttribute(
      "content",
      "website",
    );
    await expect(page.locator('meta[property="og:url"]'), route.name).toHaveAttribute(
      "content",
      new RegExp(`${route.path}$`),
    );
    await expect(page.locator('meta[property="og:image:alt"]'), route.name).toHaveAttribute(
      "content",
      /Strobe/,
    );
  }
});

test("every page declares a social image with its dimensions", async ({ page }) => {
  for (const route of ROUTES) {
    await page.goto(route.path);

    const ogImage = await page
      .locator('meta[property="og:image"]')
      .getAttribute("content");
    expect(ogImage, `${route.name} has no og:image`).toContain(route.og);

    await expect(page.locator('meta[property="og:image:width"]')).toHaveAttribute(
      "content",
      "1200",
    );
    await expect(page.locator('meta[property="og:image:height"]')).toHaveAttribute(
      "content",
      "630",
    );
    await expect(page.locator('meta[name="twitter:card"]')).toHaveAttribute(
      "content",
      "summary_large_image",
    );
  }
});

test("every page has a title, description and canonical link", async ({ page }) => {
  for (const route of ROUTES) {
    await page.goto(route.path);
    await expect(page).toHaveTitle(/Strobe/);

    const description = await page
      .locator('meta[name="description"]')
      .getAttribute("content");
    expect(description?.length ?? 0).toBeGreaterThan(50);

    const canonical = await page.locator('link[rel="canonical"]').getAttribute("href");
    expect(canonical, `${route.name} canonical`).toContain(route.path);
  }
});

test("the favicon is an SVG served with the right type", async ({ page, request }) => {
  await page.goto("/");
  const href = await page.locator('link[rel="icon"]').first().getAttribute("href");
  expect(href).toMatch(/\.svg/);

  const response = await request.get(href!);
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toContain("image/svg+xml");
});

test("the sitemap lists every route", async ({ request }) => {
  const response = await request.get("/sitemap.xml");
  expect(response.status()).toBe(200);
  const xml = await response.text();

  for (const route of ROUTES) {
    expect(xml, `sitemap is missing ${route.path}`).toContain(`${route.path}</loc>`);
  }
});

test("robots.txt points at the sitemap", async ({ request }) => {
  const response = await request.get("/robots.txt");
  expect(response.status()).toBe(200);
  const body = await response.text();
  expect(body).toContain("Allow: /");
  expect(body).toContain("sitemap.xml");
});

