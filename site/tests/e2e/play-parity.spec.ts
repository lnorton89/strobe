import { readFileSync } from "node:fs";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";

/*
 * The sandbox plays what the app plays (D554, backlog W-01): an #include of
 * the scene's own file, a picture the visitor adds for IMAGES, a script that
 * reads the picture back with textureFinalPass, and presets saved and
 * applied as presets/<name>.json.
 *
 * Each test opens its scene by putting it where the page autosaves, so the
 * page starts on it as on a return visit. Chromium is asked for SwiftShader,
 * as in play.spec.ts: a CI runner has no GPU.
 */

test.use({
  launchOptions: {
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
    ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE }
      : {}),
  },
});

type StoredScene = { title: string; files: Record<string, string>; images?: Record<string, string> };

/** The corpus gate's picture fixture, Strobe's own mark (MIT). */
const MARK = path.resolve(
  __dirname,
  "../../../crates/strobe-app/tests/corpus-gate/fixtures/media-image.synScene/mark.png",
);

async function openStored(page: Page, scene: StoredScene) {
  await page.addInitScript((stored) => {
    window.localStorage.setItem("strobe.play.scene", stored);
  }, JSON.stringify(scene));
  await page.goto("/play/");
  await expect(page.getByText(/^Compiled · /)).toBeVisible({ timeout: 30_000 });
}

/** The canvas's colour at (u, v), 0..1 from the top left, as 0..255 RGB. */
function pixelAt(page: Page, u: number, v: number) {
  return page.evaluate(
    ([x, y]) => {
      const canvas = document.querySelector<HTMLCanvasElement>("section[aria-label=Preview] canvas");
      if (!canvas) return [0, 0, 0];
      const copy = document.createElement("canvas");
      copy.width = canvas.width;
      copy.height = canvas.height;
      const context = copy.getContext("2d");
      if (!context) return [0, 0, 0];
      context.drawImage(canvas, 0, 0);
      const px = Math.min(canvas.width - 1, Math.floor(x * canvas.width));
      const py = Math.min(canvas.height - 1, Math.floor(y * canvas.height));
      return Array.from(context.getImageData(px, py, 1, 1).data.slice(0, 3));
    },
    [u, v],
  );
}

const brightness = async (page: Page, u: number, v: number) =>
  (await pixelAt(page, u, v)).reduce((sum, value) => sum + value, 0);

test("#include finds the scene's own file", async ({ page }) => {
  await openStored(page, {
    title: "include",
    files: {
      "main.glsl": '#include "helpers.glsl"\n\nvec4 renderMain() {\n    return vec4(lit(), 0.0, 0.0, 1.0);\n}\n',
      "helpers.glsl": "float lit() { return 1.0; }\n",
    },
  });
  await expect.poll(() => pixelAt(page, 0.5, 0.5), { timeout: 15_000 }).toEqual([255, 0, 0]);
});

test("a picture added for IMAGES is drawn, and a missing one is named", async ({ page }) => {
  await openStored(page, {
    title: "picture",
    files: {
      "main.glsl": "vec4 renderMain() {\n    return vec4(texture(mark, _uv).rgb, 1.0);\n}\n",
      "scene.json": '{ "IMAGES": [ { "NAME": "mark", "PATH": "mark.png" } ] }',
    },
  });
  await expect(page.getByText(/reads mark\.png, which is not in the scene/)).toBeVisible();
  expect(await brightness(page, 0.5, 0.5)).toBe(0);

  await page.locator('input[type=file][accept="image/png,image/jpeg"]').setInputFiles({
    name: "mark.png",
    mimeType: "image/png",
    buffer: readFileSync(MARK),
  });
  await expect(page.getByRole("tab", { name: "mark.png" })).toBeVisible();
  await expect(page.getByText(/reads mark\.png/)).toHaveCount(0, { timeout: 15_000 });
  // The mark is drawn: somewhere across the middle row is no longer black.
  await expect
    .poll(
      async () => {
        let lit = 0;
        for (let u = 0.05; u < 1; u += 0.1) lit += (await brightness(page, u, 0.5)) > 0 ? 1 : 0;
        return lit;
      },
      { timeout: 15_000 },
    )
    .toBeGreaterThan(0);
});

test("textureFinalPass reads the picture back to the script", async ({ page }) => {
  // The left half is red; the right half shows what the script read from
  // the left half's middle, which is black until the readback arrives.
  await openStored(page, {
    title: "readback",
    files: {
      "main.glsl":
        "vec4 renderMain() {\n    return _uv.x < 0.5 ? vec4(1.0, 0.0, 0.0, 1.0) : vec4(seen, 1.0);\n}\n",
      "script.js": "function update() {\n  setUniform(\"seen\", textureFinalPass(0.25, 0.5));\n}\n",
    },
  });
  await expect.poll(() => pixelAt(page, 0.75, 0.5), { timeout: 15_000 }).toEqual([255, 0, 0]);
});

test("a preset is saved as presets/<name>.json and applied", async ({ page }) => {
  await openStored(page, {
    title: "presets",
    files: {
      "main.glsl": "vec4 renderMain() {\n    return vec4(level, 0.0, 0.0, 1.0);\n}\n",
      "scene.json":
        '{ "CONTROLS": [ { "NAME": "level", "TYPE": "slider", "MIN": 0, "MAX": 1, "DEFAULT": 0.5 } ] }',
    },
  });
  page.once("dialog", (dialog) => void dialog.accept("quiet"));
  await page.getByRole("button", { name: "Save preset" }).click();
  await expect(page.getByRole("tab", { name: "presets/quiet.json" })).toBeVisible();
  await page.getByRole("tab", { name: "presets/quiet.json" }).click();
  await expect(page.locator(".cm-content")).toContainText('"level": 0.5');

  await page.locator("#play-preset").selectOption("presets/quiet.json");
  await expect(page.getByText("Applied preset quiet")).toBeVisible();
});
