import { expect, test, type Page } from "@playwright/test";

/*
 * A lost WebGL context (D559, backlog W-02). WEBGL_lose_context takes the
 * context as a driver reset or a phone would, and gives it back; the page
 * builds a new engine on it and plays the scene again without a reload.
 *
 * Chromium is asked for SwiftShader, as in play.spec.ts.
 */

test.use({
  launchOptions: {
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
    ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE }
      : {}),
  },
});

test.setTimeout(120_000);

const BARS = `vec4 renderMain() {
    float w = sin(_uv.x * 12.0 + TIME * 2.0);
    vec3 c = mix(vec3(0.9, 0.2, 0.1), vec3(0.1, 0.3, 0.9), 0.5 + 0.5 * w);
    return vec4(c, 1.0);
}
`;

const CANVAS = "section[aria-label=Preview] canvas";
const compiled = (page: Page) => page.getByText(/^Compiled · /);
const waiting = (page: Page) => page.getByTestId("play-context-lost");

/** The context's own switch, kept on the page: it is the only way back. */
function loseContext(page: Page) {
  return page.evaluate((selector) => {
    const canvas = document.querySelector<HTMLCanvasElement>(selector);
    const switcher = canvas?.getContext("webgl2")?.getExtension("WEBGL_lose_context");
    if (!switcher) throw new Error("no WEBGL_lose_context here");
    (window as unknown as { contextSwitch: WEBGL_lose_context }).contextSwitch = switcher;
    switcher.loseContext();
  }, CANVAS);
}

function restoreContext(page: Page) {
  return page.evaluate(() => {
    (window as unknown as { contextSwitch: WEBGL_lose_context }).contextSwitch.restoreContext();
  });
}

/** How many of the canvas's pixels are not black, as in play.spec.ts. */
function litPixels(page: Page) {
  return page.evaluate((selector) => {
    const canvas = document.querySelector<HTMLCanvasElement>(selector);
    if (!canvas || canvas.width === 0) return 0;
    const copy = document.createElement("canvas");
    copy.width = 64;
    copy.height = 36;
    const context = copy.getContext("2d");
    if (!context) return 0;
    context.drawImage(canvas, 0, 0, copy.width, copy.height);
    const { data } = context.getImageData(0, 0, copy.width, copy.height);
    let lit = 0;
    for (let i = 0; i < data.length; i += 4) {
      if ((data[i] ?? 0) + (data[i + 1] ?? 0) + (data[i + 2] ?? 0) > 24) lit += 1;
    }
    return lit;
  }, CANVAS);
}

/** Opens a scene by putting it where the page autosaves. */
async function openStored(page: Page, files: Record<string, string>) {
  await page.addInitScript((stored) => {
    if (window.sessionStorage.getItem("seeded")) return;
    window.sessionStorage.setItem("seeded", "1");
    window.localStorage.setItem("strobe.play.scene", stored);
  }, JSON.stringify({ title: "context", files }));
  await page.goto("/play/");
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
}

test("the picture comes back with the controls as they were", async ({ page }) => {
  const problems: string[] = [];
  page.on("pageerror", (error) => problems.push(String(error)));
  await page.goto("/play/?scene=02-plasma-pulse");
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
  const slider = page.locator("section[aria-label=Preview] input[type=range]").first();
  const low = await slider.getAttribute("min");
  await slider.fill(low ?? "0");
  const moved = await slider.inputValue();

  await loseContext(page);
  await expect(waiting(page)).toBeVisible();
  await expect(page.getByRole("button", { name: /generate(d)? controls$/i })).toBeDisabled();

  await restoreContext(page);
  await expect(waiting(page)).toHaveCount(0, { timeout: 30_000 });
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
  await expect.poll(() => litPixels(page), { timeout: 15_000 }).toBeGreaterThan(100);
  await expect(slider).toHaveValue(moved);
  // The frame loop is running again, not just the one frame.
  await expect(page.getByText(/^[1-9]\d* fps$/)).toBeVisible({ timeout: 15_000 });
  expect(problems, "the page threw nothing").toEqual([]);
});

test("it comes back more than once", async ({ page }) => {
  await openStored(page, { "main.glsl": BARS });
  for (const round of [1, 2]) {
    await loseContext(page);
    await expect(waiting(page), `round ${round} waits`).toBeVisible();
    await restoreContext(page);
    await expect(waiting(page), `round ${round} recovers`).toHaveCount(0, { timeout: 30_000 });
    await expect.poll(() => litPixels(page), { timeout: 30_000 }).toBeGreaterThan(100);
  }
});

test("a broken edit keeps its error and the last good picture", async ({ page }) => {
  await openStored(page, { "main.glsl": BARS });
  const editor = page.locator(".cm-content");
  await editor.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.insertText("vec4 renderMain() {\n    return vec4(notDeclared, 1.0);\n}\n");
  await page.getByRole("button", { name: "Compile" }).click();
  const status = page.locator("pre").filter({ hasText: /main\.glsl:2\b/ });
  await expect(status).toBeVisible({ timeout: 30_000 });

  await loseContext(page);
  await expect(waiting(page)).toBeVisible();
  await restoreContext(page);
  await expect(waiting(page)).toHaveCount(0, { timeout: 30_000 });

  await expect(status).toBeVisible({ timeout: 60_000 });
  await expect(editor).toContainText("notDeclared");
  await expect.poll(() => litPixels(page), { timeout: 30_000 }).toBeGreaterThan(100);
});

test("a run in progress stops, and can run again after", async ({ page }) => {
  await openStored(page, { "main.glsl": BARS });
  const button = page.getByRole("button", { name: "Generate controls" });
  // In one task, so the context goes before the run's first step.
  await page.evaluate((selector) => {
    const generate = [...document.querySelectorAll("button")].find(
      (candidate) => candidate.textContent === "Generate controls",
    );
    const canvas = document.querySelector<HTMLCanvasElement>(selector);
    const switcher = canvas?.getContext("webgl2")?.getExtension("WEBGL_lose_context");
    if (!generate || !switcher) throw new Error("no button or no WEBGL_lose_context");
    (window as unknown as { contextSwitch: WEBGL_lose_context }).contextSwitch = switcher;
    generate.click();
    switcher.loseContext();
  }, CANVAS);
  await expect(page.getByTestId("generate-message")).toContainText("measuring stopped");
  await expect(page.getByTestId("generate-review")).toHaveCount(0);

  await restoreContext(page);
  await expect(waiting(page)).toHaveCount(0, { timeout: 30_000 });
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
  await button.click();
  const review = page.getByTestId("generate-review");
  await expect(review).toBeVisible({ timeout: 90_000 });
  await review.getByRole("button", { name: /^Add \d+ control/ }).click();
  await expect(page.getByTestId("generate-accepted")).toBeVisible({ timeout: 30_000 });
});

test("a review open at the time can still be accepted", async ({ page }) => {
  await openStored(page, { "main.glsl": BARS });
  await page.getByRole("button", { name: "Generate controls" }).click();
  const review = page.getByTestId("generate-review");
  await expect(review).toBeVisible({ timeout: 90_000 });

  await loseContext(page);
  await expect(waiting(page)).toBeVisible();
  await restoreContext(page);
  await expect(waiting(page)).toHaveCount(0, { timeout: 30_000 });

  await review.getByRole("button", { name: /^Add \d+ control/ }).click();
  await expect(page.getByTestId("generate-accepted")).toBeVisible({ timeout: 30_000 });
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
});

test("a scripted scene's script runs again", async ({ page }) => {
  await page.goto("/play/?scene=11-script-orbits");
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });

  await loseContext(page);
  await expect(waiting(page)).toBeVisible();
  await restoreContext(page);
  await expect(waiting(page)).toHaveCount(0, { timeout: 30_000 });

  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
  await expect.poll(() => litPixels(page), { timeout: 30_000 }).toBeGreaterThan(100);
  // Long enough for a dead worker to pass the script's deadline and say so.
  await page.waitForTimeout(2_000);
  await expect(page.getByText(/^script\.js:/)).toHaveCount(0);
  await expect(compiled(page)).toBeVisible();
});

test("a phone shows the whole scene name", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto("/play/?scene=12-silicon-atlas");
  const title = page.locator("#play-title");
  await expect(title).toHaveValue(/silicon/i, { timeout: 60_000 });
  const fits = await title.evaluate((input: HTMLInputElement) => input.scrollWidth <= input.clientWidth);
  expect(fits, "the name fits its field").toBe(true);
});
