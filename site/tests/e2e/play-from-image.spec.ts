import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

/*
 * From image in the sandbox (D560), on the real engine: a picture goes in, the
 * templates compile and play in ES 3.00, the gates run, one opens in the
 * editor, and on this path nothing leaves the machine. Chromium is asked for
 * SwiftShader, as in play.spec.ts, so this takes its time.
 */

test.use({
  launchOptions: {
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
    ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE }
      : {}),
  },
});

test.setTimeout(300_000);

/** A picture drawn in the page, so the test needs no image file. */
async function paintedPng(page: import("@playwright/test").Page): Promise<Buffer> {
  const url = await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = 320;
    canvas.height = 320;
    const context = canvas.getContext("2d");
    if (!context) return "";
    context.fillStyle = "#101840";
    context.fillRect(0, 0, 320, 320);
    context.strokeStyle = "#f0d060";
    context.lineWidth = 6;
    for (let i = 0; i < 8; i += 1) {
      context.beginPath();
      context.arc(160, 160, 30 + i * 16, i * 0.5, i * 0.5 + 4);
      context.stroke();
    }
    context.fillStyle = "#e04030";
    context.fillRect(150, 150, 20, 20);
    return canvas.toDataURL("image/png");
  });
  return Buffer.from(url.replace(/^data:image\/png;base64,/, ""), "base64");
}

test("a picture becomes scenes that play, and none of it leaves the machine", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(String(error)));

  await page.goto("/play/");
  await expect(page.getByText(/Compiled ·/), "the engine started").toBeVisible({ timeout: 60_000 });
  // From here on: the site header's own release check is not this feature.
  const off: string[] = [];
  page.on("request", (request) => {
    const { hostname } = new URL(request.url());
    if (hostname !== "localhost" && !request.url().startsWith("data:") && !request.url().startsWith("blob:")) {
      off.push(request.url());
    }
    if (request.method() === "POST") off.push(`POST ${request.url()}`);
  });
  await page.getByRole("button", { name: "From image" }).click();
  const dialog = page.getByRole("dialog", { name: "Make a scene from a picture" });
  await expect(dialog, "the panel opens").toBeVisible();

  await page.getByTestId("from-image-file").setInputFiles({
    name: "my secret holiday.png",
    mimeType: "image/png",
    buffer: await paintedPng(page),
  });
  await expect(dialog.getByRole("button", { name: "Wheel" }), "a reading to change").toBeVisible();
  await dialog.getByRole("button", { name: "Wheel" }).click();
  await expect(dialog.getByRole("button", { name: "Wheel" }), "the click counts").toHaveAttribute("aria-pressed", "true");

  // The corners are keyboard-operable.
  await dialog.getByRole("button", { name: /top left corner/ }).press("ArrowRight");

  await dialog.getByRole("button", { name: "Make scenes" }).click();
  const tiles = dialog.getByRole("region", { name: "Candidates" }).getByRole("button");
  await expect(tiles.first(), "a candidate plays").toBeVisible({ timeout: 120_000 });
  await expect(dialog.getByText(/screened for flashing/), "every candidate has run").toBeVisible({ timeout: 240_000 });
  const count = await tiles.count();
  expect(count, "most of the gallery survives the gates").toBeGreaterThanOrEqual(6);
  await expect(dialog.getByText(/did not compile/), "every template compiles in ES 3.00").toHaveCount(0);

  const axe = await new AxeBuilder({ page }).include("dialog").analyze();
  expect(axe.violations.map((violation) => violation.id), "the panel passes axe").toEqual([]);

  await tiles.first().click();
  await expect(dialog, "picking closes the panel").toBeHidden();
  await expect(page.getByText(/Compiled ·/), "it opens in the editor and compiles").toBeVisible({ timeout: 60_000 });
  await expect(page.getByRole("heading", { name: "Controls" }), "with its controls").toBeVisible();
  await expect(page.getByLabel("speed").first(), "the generated controls are there").toBeVisible();
  const stored = await page.evaluate(() => window.localStorage.getItem("strobe.play.scene") ?? "");
  await page.waitForFunction(() => (window.localStorage.getItem("strobe.play.scene") ?? "").includes("From image"), null, { timeout: 5000 }).catch(() => undefined);
  const after = await page.evaluate(() => window.localStorage.getItem("strobe.play.scene") ?? "");
  expect(`${stored}${after}`, "the file's name is not in the scene").not.toContain("holiday");

  expect(off, "nothing left the machine").toEqual([]);
  expect(errors, "no uncaught errors").toEqual([]);
});

test("a visitor's key goes to their provider and nowhere else", async ({ page }) => {
  const seen: { host: string; key: boolean }[] = [];
  await page.route("https://openrouter.ai/**", async (route) => {
    const headers = route.request().headers();
    seen.push({ host: "openrouter.ai", key: (headers.authorization ?? "").includes("test-key-123") });
    await route.fulfill({
      json: { data: [{ id: "test/vision", architecture: { input_modalities: ["image", "text"] } }] },
      headers: { "Access-Control-Allow-Origin": "*" },
    });
  });
  const leaked: string[] = [];
  page.on("request", (request) => {
    const text = `${request.url()} ${JSON.stringify(request.headers())} ${request.postData() ?? ""}`;
    if (text.includes("test-key-123") && !request.url().startsWith("https://openrouter.ai/")) leaked.push(request.url());
  });

  await page.goto("/play/");
  await expect(page.getByText(/Compiled ·/), "the engine started").toBeVisible({ timeout: 60_000 });
  await page.getByRole("button", { name: "From image" }).click();
  const dialog = page.getByRole("dialog", { name: "Make a scene from a picture" });
  await dialog.getByText("Use your own AI model").click();
  await expect(dialog.getByRole("button", { name: "Sign in with OpenRouter" }), "sign-in is offered").toBeVisible();
  await dialog.getByLabel("API key").fill("test-key-123");
  await dialog.getByRole("button", { name: "Load models" }).click();
  await expect(dialog.getByLabel("Model"), "the provider's models").toHaveValue("test/vision");
  expect(seen, "the key went in the header").toContainEqual({ host: "openrouter.ai", key: true });
  await dialog.getByRole("button", { name: "Forget" }).click();
  await expect(dialog.getByLabel("API key"), "forgotten").toHaveValue("");
  const stored = await page.evaluate(() => JSON.stringify({ ...window.localStorage }) + JSON.stringify({ ...window.sessionStorage }));
  expect(stored, "and gone from storage").not.toContain("test-key-123");
  expect(leaked, "the key went nowhere else").toEqual([]);
});
