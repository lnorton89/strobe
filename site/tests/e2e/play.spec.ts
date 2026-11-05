import { expect, test, type Page } from "@playwright/test";

/*
 * The /play smoke test (D552): the engine CI built from crates/strobe-web
 * starts, a scene compiles and draws, and a shader error is marked on its
 * line while the last good picture stays. It is what fails a pull request
 * that breaks the browser build or the sandbox (backlog W-03).
 *
 * WebGL2 needs a GPU or SwiftShader. A CI runner has no GPU, and Chromium
 * no longer falls back to SwiftShader on its own, so the suite asks for it
 * by name: the same software rasteriser on every machine, which also makes
 * the run independent of the desk's graphics driver.
 */

test.use({
  launchOptions: {
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
    ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE }
      : {}),
  },
});

/** The status line under the editor once the engine has compiled the scene. */
const compiled = (page: Page) => page.getByText(/^Compiled · /);

/**
 * How many of the canvas's pixels are not black, sampled on a grid. The
 * engine asks for `preserveDrawingBuffer`, so the last frame can be read
 * back between animation frames.
 */
function litPixels(page: Page) {
  return page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>("section[aria-label=Preview] canvas");
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
  });
}

test("a library scene compiles and draws", async ({ page }) => {
  await page.goto("/play/?scene=02-plasma-pulse");
  await expect(compiled(page)).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("#play-title")).toHaveValue(/plasma/i);
  // Its controls came back from the engine.
  await expect(page.getByRole("heading", { name: "Controls" })).toBeVisible();
  await expect.poll(() => litPixels(page), { timeout: 15_000 }).toBeGreaterThan(100);
});

test("a shader error is marked on its line and the picture stays", async ({ page }) => {
  // A fresh context has nothing stored, so the page opens the new-scene
  // starter.
  await page.goto("/play/");
  await expect(compiled(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => litPixels(page), { timeout: 15_000 }).toBeGreaterThan(100);

  const editor = page.locator(".cm-content");
  await editor.click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.insertText(
    "vec4 renderMain() {\n    return vec4(notDeclared, 1.0);\n}\n",
  );
  await page.getByRole("button", { name: "Compile" }).click();

  // The player's error names the user's file and line, the tab is marked,
  // and the editor underlines that line.
  const status = page.locator("pre").filter({ hasText: /main\.glsl:2\b/ });
  await expect(status).toBeVisible({ timeout: 15_000 });
  await expect(status).toContainText("notDeclared");
  await expect(page.getByRole("tab", { name: /main\.glsl/ })).toContainText("●");
  await expect(page.locator(".cm-lintRange-error").first()).toBeVisible();
  await expect(compiled(page)).toHaveCount(0);
  // A failed compile keeps the last good scene playing.
  expect(await litPixels(page)).toBeGreaterThan(100);
});
