import { expect, test, type Page } from "@playwright/test";

/*
 * Generate controls in the sandbox (D558): the engine's measuring runs in the
 * page, a step per animation frame, and what it proposes changes nothing
 * until it is accepted. The scenes are opened from the library or by putting
 * them where the page autosaves, as play-parity.spec.ts does.
 *
 * Chromium is asked for SwiftShader, as in play.spec.ts: a CI runner has no
 * GPU. Software rendering makes a run slow, so these take their time.
 */

test.use({
  launchOptions: {
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
    ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE }
      : {}),
  },
});

test.setTimeout(240_000);

type StoredScene = { title: string; files: Record<string, string> };

/** Numbers worth a control and no controls yet. */
const BARS = `vec4 renderMain() {
    float w = sin(_uv.x * 12.0 + TIME * 2.0);
    vec3 c = mix(vec3(0.9, 0.2, 0.1), vec3(0.1, 0.3, 0.9), 0.5 + 0.5 * w);
    return vec4(c, 1.0);
}
`;

const compiled = (page: Page) => page.getByText(/^Compiled · /);
const generateButton = (page: Page) =>
  page.getByRole("button", { name: /^(Add generated|Generate) controls$/ });
const progress = (page: Page) => page.getByTestId("generate-progress");
const review = (page: Page) => page.getByTestId("generate-review");
const message = (page: Page) => page.getByTestId("generate-message");
const accepted = (page: Page) => page.getByTestId("generate-accepted");

async function openStored(page: Page, scene: StoredScene) {
  await page.addInitScript((stored) => {
    // Only once: a reload in a test reads what the page itself saved.
    if (window.sessionStorage.getItem("seeded")) return;
    window.sessionStorage.setItem("seeded", "1");
    window.localStorage.setItem("strobe.play.scene", stored);
  }, JSON.stringify(scene));
  await page.goto("/play/");
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
}

/** The scene as the page has autosaved it. */
const saved = (page: Page) =>
  page.evaluate(
    () =>
      JSON.parse(window.localStorage.getItem("strobe.play.scene") ?? "{}") as {
        title: string;
        files: Record<string, string>;
      },
  );

/** How many of the canvas's sampled pixels are lit, and a hash of them. */
function picture(page: Page) {
  return page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>("section[aria-label=Preview] canvas");
    if (!canvas || canvas.width === 0) return { lit: 0, hash: 0 };
    const copy = document.createElement("canvas");
    copy.width = 64;
    copy.height = 36;
    const context = copy.getContext("2d");
    if (!context) return { lit: 0, hash: 0 };
    context.drawImage(canvas, 0, 0, copy.width, copy.height);
    const { data } = context.getImageData(0, 0, copy.width, copy.height);
    let lit = 0;
    let hash = 0;
    for (let i = 0; i < data.length; i += 4) {
      const r = data[i] ?? 0;
      const g = data[i + 1] ?? 0;
      const b = data[i + 2] ?? 0;
      if (r + g + b > 24) lit += 1;
      hash = (hash * 31 + r + g * 3 + b * 7) % 1_000_003;
    }
    return { lit, hash };
  });
}

/** Records the page's animation frames until `stop` is called in the page. */
function watchFrames(page: Page) {
  return page.evaluate(() => {
    const seen = { frames: 0, worst: 0, last: performance.now(), on: true };
    (window as unknown as { __frames: typeof seen }).__frames = seen;
    const tick = (now: number) => {
      if (!seen.on) return;
      seen.frames += 1;
      seen.worst = Math.max(seen.worst, now - seen.last);
      seen.last = now;
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

const framesSeen = (page: Page) =>
  page.evaluate(() => {
    const seen = (window as unknown as { __frames: { frames: number; worst: number; on: boolean } })
      .__frames;
    seen.on = false;
    return { frames: seen.frames, worst: seen.worst };
  });

test("the library's scene with no controls is measured while it plays, and can be cancelled", async ({
  page,
}) => {
  await page.goto("/play/?scene=12-silicon-atlas");
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
  await expect(page.locator("#play-title")).toHaveValue(/silicon/i);
  await expect(page.getByText(/^No controls\./)).toBeVisible();
  await expect(generateButton(page)).toHaveText("Generate controls");
  await expect(generateButton(page)).toHaveAttribute("title", /Nothing changes until you add them/);
  const before = await saved(page);
  await expect.poll(async () => (await picture(page)).lit, { timeout: 30_000 }).toBeGreaterThan(100);

  await generateButton(page).click();
  await expect(progress(page)).toBeVisible();
  await expect(progress(page)).toContainText(/Scoring \d+ of \d+/, { timeout: 180_000 });
  // The preview is still animating under the measuring.
  const first = await picture(page);
  await expect.poll(async () => (await picture(page)).hash, { timeout: 60_000 }).not.toBe(first.hash);

  await progress(page).getByRole("button", { name: "Cancel" }).click();
  await expect(message(page)).toHaveText(/Cancelled\. Nothing changed\./);
  expect((await saved(page)).files).toEqual(before.files);
  await expect(page.getByText(/^No controls\./)).toBeVisible();
  expect((await picture(page)).lit).toBeGreaterThan(100);
});

test("a library scene with its controls baked in gets them back, keeps drawing, and can undo", async ({
  page,
}) => {
  // Measuring 12-silicon-atlas to the end takes minutes on SwiftShader, so
  // the whole flow runs here on a lighter scene with no controls: its own
  // controls become the #defines a scene without them would have.
  const response = await page.request.get("/generated/play/01-aurora-gradient.json");
  const library = (await response.json()) as StoredScene;
  const declared = JSON.parse(library.files["scene.json"] ?? "{}") as {
    CONTROLS?: { NAME: string; TYPE: string; DEFAULT?: number | number[] }[];
  };
  const defines = (declared.CONTROLS ?? [])
    .map((control) => {
      const value = control.DEFAULT ?? 0;
      const glsl = Array.isArray(value)
        ? `vec${value.length}(${value.map((v) => v.toFixed(5)).join(", ")})`
        : value.toFixed(5);
      return `#define ${control.NAME} ${glsl}\n`;
    })
    .join("");
  const { CONTROLS: _controls, ...rest } = declared;
  await openStored(page, {
    title: library.title,
    files: {
      ...library.files,
      "main.glsl": `${defines}${library.files["main.glsl"] ?? ""}`,
      "scene.json": `${JSON.stringify(rest, null, 2)}\n`,
    },
  });
  await expect(page.getByText(/^No controls\./)).toBeVisible();
  await expect(generateButton(page)).toHaveText("Generate controls");
  const before = await saved(page);
  await expect.poll(async () => (await picture(page)).lit, { timeout: 30_000 }).toBeGreaterThan(100);

  await watchFrames(page);
  await generateButton(page).click();
  await expect(review(page)).toBeVisible({ timeout: 120_000 });
  const { frames } = await framesSeen(page);
  expect(frames, "the page kept running animation frames").toBeGreaterThan(3);

  // Grouped, named and measured, and nothing has reached the scene yet.
  await expect(review(page)).toContainText("At their defaults the picture is the original's");
  await expect(
    review(page).getByRole("heading", { name: /^(Motion|Shape|Colour|Quality)$/ }).first(),
  ).toBeVisible();
  await expect(review(page)).toContainText(/\d+\.\d%/);
  expect((await saved(page)).files).toEqual(before.files);
  await expect(page.getByText(/^No controls\./)).toBeVisible();

  const name = await review(page).locator('input[id$="-name"]').first().inputValue();
  await review(page).getByRole("button", { name: /^Add \d+ controls?$/ }).click();
  await expect(accepted(page)).toContainText(/Added \d+ controls?/);
  await expect(accepted(page)).toContainText("scene.json");
  await expect(accepted(page)).toContainText("main.glsl");

  // In the controls panel, in the editor's files, and in the autosave.
  await expect(page.getByText(/^No controls\./)).toHaveCount(0, { timeout: 60_000 });
  await expect(page.locator("section[aria-label=Preview]").getByText(name).first()).toBeVisible();
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
  await expect.poll(async () => (await picture(page)).lit, { timeout: 30_000 }).toBeGreaterThan(100);
  await expect.poll(async () => (await saved(page)).files["scene.json"]).toContain(name);
  const after = await saved(page);
  expect(after.files["main.glsl"]).toContain(name);
  await page.getByRole("tab", { name: "scene.json" }).click();
  await expect(page.locator(".cm-content")).toContainText(name);

  // The way back: the files as they were, and the panel empty again.
  await accepted(page).getByRole("button", { name: "Undo" }).click();
  await expect(message(page)).toContainText("Undone");
  await expect(page.getByText(/^No controls\./)).toBeVisible({ timeout: 60_000 });
  await expect.poll(async () => (await saved(page)).files).toEqual(before.files);
  await expect(page.locator(".cm-content")).not.toContainText(name);
  await expect.poll(async () => (await picture(page)).lit, { timeout: 30_000 }).toBeGreaterThan(100);
});

test("cancel stops the run and changes nothing", async ({ page }) => {
  await openStored(page, { title: "bars", files: { "main.glsl": BARS } });
  const before = await saved(page);
  await generateButton(page).click();
  await expect(progress(page)).toBeVisible();
  await progress(page).getByRole("button", { name: "Cancel" }).click();
  await expect(message(page)).toHaveText(/Cancelled\. Nothing changed\./);
  await expect(progress(page)).toHaveCount(0);
  await expect(generateButton(page)).toBeEnabled();
  expect((await saved(page)).files).toEqual(before.files);
  await expect(page.getByText(/^No controls\./)).toBeVisible();
  const first = await picture(page);
  expect(first.lit).toBeGreaterThan(100);
  await expect.poll(async () => (await picture(page)).hash, { timeout: 15_000 }).not.toBe(first.hash);

  // It can start again, and this time finish.
  await generateButton(page).click();
  await expect(review(page)).toBeVisible({ timeout: 120_000 });
});

test("a review keeps, drops and renames, and the result survives a reload", async ({ page }) => {
  await openStored(page, { title: "bars", files: { "main.glsl": BARS } });
  await generateButton(page).click();
  await expect(review(page)).toBeVisible({ timeout: 120_000 });

  const keeps = review(page).getByRole("checkbox");
  const total = await keeps.count();
  expect(total).toBeGreaterThan(1);
  const names = review(page).locator('input[id$="-name"]');
  const dropped = await names.nth(1).inputValue();
  await keeps.nth(1).uncheck();
  await names.first().fill("my_control");
  await review(page).getByRole("button", { name: /^Add \d+ controls?$/ }).click();
  await expect(accepted(page)).toBeVisible();

  await expect(page.locator("section[aria-label=Preview]").getByText("my_control").first()).toBeVisible({
    timeout: 60_000,
  });
  const after = await saved(page);
  expect(after.files["scene.json"]).toContain("my_control");
  expect(after.files["main.glsl"]).toContain("my_control");
  expect(after.files["scene.json"]).not.toContain(`"${dropped}"`);

  // A bad name is refused where it was typed, and the review stays. A second
  // run over the result offers the rest under new names.
  await page.reload();
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
  await expect(page.locator("section[aria-label=Preview]").getByText("my_control").first()).toBeVisible();
  await expect(generateButton(page)).toHaveText("Add generated controls");
  await generateButton(page).click();
  await expect(review(page).or(message(page))).toBeVisible({ timeout: 120_000 });
  if (await review(page).isVisible()) {
    await review(page).locator('input[id$="-name"]').first().fill("not a name");
    await review(page).getByRole("button", { name: /^Add \d+ controls?$/ }).click();
    await expect(review(page)).toContainText("cannot be a control's name");
    expect((await saved(page)).files).toEqual(after.files);
  }
});

test("a range is typed as a person types it, a negative number included", async ({ page }) => {
  await openStored(page, { title: "bars", files: { "main.glsl": BARS } });
  await generateButton(page).click();
  await expect(review(page)).toBeVisible({ timeout: 120_000 });

  // A key at a time: "-" and "." alone read as empty until the digit comes.
  const low = review(page).locator('input[id$="-min"]').first();
  const high = review(page).locator('input[id$="-max"]').first();
  await low.selectText();
  await page.keyboard.type("-1", { delay: 20 });
  await expect(low).toHaveValue("-1");
  await high.selectText();
  await page.keyboard.type(".5", { delay: 20 });
  await expect(high).toHaveValue(".5");

  // Half a number changes nothing, and leaving the field shows what the row has.
  await high.selectText();
  await page.keyboard.type("-", { delay: 20 });
  await low.focus();
  await expect(high).toHaveValue("0.5");

  // The first slider's default is above that top, so the top widens to hold it (D219).
  const name = await review(page).locator('input[id$="-name"]').first().inputValue();
  await review(page).getByRole("button", { name: /^Add \d+ controls?$/ }).click();
  await expect(accepted(page)).toBeVisible();
  const scene = JSON.parse((await saved(page)).files["scene.json"] ?? "{}") as {
    CONTROLS?: { NAME: string; MIN?: number; DEFAULT?: number; MAX?: number }[];
  };
  const control = scene.CONTROLS?.find((entry) => entry.NAME === name);
  expect(control?.MIN, "the typed lowest value reaches scene.json").toBe(-1);
  expect(control?.MAX, "the top holds the default").toBe(control?.DEFAULT);
});

test("a scene with its own controls keeps them", async ({ page }) => {
  const json =
    '{ "CONTROLS": [ { "NAME": "level", "TYPE": "slider", "MIN": 0, "MAX": 1, "DEFAULT": 1 } ] }';
  const shader = BARS.replace("return vec4(c, 1.0);", "return vec4(c * level, 1.0);");
  await openStored(page, { title: "mine", files: { "main.glsl": shader, "scene.json": json } });
  await expect(generateButton(page)).toHaveText("Add generated controls");
  await generateButton(page).click();
  await expect(review(page)).toBeVisible({ timeout: 120_000 });
  await review(page).getByRole("button", { name: /^Add \d+ controls?$/ }).click();
  await expect(accepted(page)).toBeVisible();
  const after = await saved(page);
  expect(after.files["scene.json"]).toContain('"NAME": "level"');
  await expect(page.getByLabel("level")).toBeVisible({ timeout: 60_000 });
});

test("a scene that does not compile is said not to, and is left as typed", async ({ page }) => {
  await openStored(page, { title: "bars", files: { "main.glsl": BARS } });
  const broken = "vec4 renderMain() {\n    return vec4(notDeclared * 2.0, 1.0);\n}\n";
  await page.locator(".cm-content").click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.insertText(broken);
  await generateButton(page).click();
  await expect(message(page)).toContainText("No controls generated", { timeout: 60_000 });
  await expect(message(page)).toContainText("does not compile");
  await expect(message(page)).toContainText("main.glsl:2");
  await expect(review(page)).toHaveCount(0);
  await expect.poll(async () => (await saved(page)).files["main.glsl"]).toBe(broken);
});

test("a scene where nothing matters says so", async ({ page }) => {
  await openStored(page, {
    title: "still",
    files: {
      "main.glsl": `vec4 renderMain() {
    float unused = sin(_uv.x * 7.0);
    return vec4(0.5, 0.5, 0.5, 1.0 + unused * 0.0);
}
`,
    },
  });
  const before = await saved(page);
  await generateButton(page).click();
  await expect(message(page)).toContainText("No controls to offer", { timeout: 120_000 });
  await expect(message(page)).toContainText("unchanged");
  expect((await saved(page)).files).toEqual(before.files);
});

test("a script scene is measured with the script stopped, and says so", async ({ page }) => {
  await openStored(page, {
    title: "scripted",
    files: {
      "main.glsl": `vec4 renderMain() {
    float w = sin(_uv.x * 12.0 + pulse);
    return vec4(vec3(0.9, 0.2, 0.1) * (0.5 + 0.5 * w), 1.0);
}
`,
      "script.js": 'function update(dt) {\n  setUniform("pulse", TIME);\n}\n',
    },
  });
  await generateButton(page).click();
  await expect(review(page)).toBeVisible({ timeout: 120_000 });
  await expect(review(page)).toContainText("This scene has a script");
  await review(page).getByRole("button", { name: /^Add \d+ controls?$/ }).click();
  await expect(accepted(page)).toBeVisible();
  await expect(compiled(page)).toBeVisible({ timeout: 60_000 });
  expect((await saved(page)).files["script.js"]).toContain("setUniform");
});

test("typing stops a run and a review goes stale; what was typed is never lost", async ({ page }) => {
  await openStored(page, { title: "bars", files: { "main.glsl": BARS } });
  const editor = page.locator(".cm-content");

  // While it runs.
  await generateButton(page).click();
  await expect(progress(page)).toBeVisible();
  await editor.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.insertText("// typed during the run\n");
  await expect(message(page)).toContainText("You edited the scene", { timeout: 15_000 });
  await expect.poll(async () => (await saved(page)).files["main.glsl"]).toContain("typed during");

  // During the review: refused, not applied over the edit.
  await generateButton(page).click();
  await expect(review(page)).toBeVisible({ timeout: 120_000 });
  await editor.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.insertText("// typed during the review\n");
  await expect(review(page)).toContainText("has changed since it was measured");
  await expect(review(page).getByRole("button", { name: /^Add \d+ controls?$/ })).toBeDisabled();
  await review(page).getByRole("button", { name: "Discard" }).click();

  // After an accept: Undo asks before it discards an edit.
  await generateButton(page).click();
  await expect(review(page)).toBeVisible({ timeout: 120_000 });
  await review(page).getByRole("button", { name: /^Add \d+ controls?$/ }).click();
  await expect(accepted(page)).toBeVisible();
  await editor.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.insertText("// typed after\n");
  await expect.poll(async () => (await saved(page)).files["main.glsl"]).toContain("typed after");

  page.once("dialog", (dialog) => void dialog.dismiss());
  await accepted(page).getByRole("button", { name: "Undo" }).click();
  await expect(accepted(page)).toBeVisible();
  expect((await saved(page)).files["main.glsl"]).toContain("typed after");

  page.once("dialog", (dialog) => void dialog.accept());
  await accepted(page).getByRole("button", { name: "Undo" }).click();
  await expect(message(page)).toContainText("Undone");
  const restored = (await saved(page)).files["main.glsl"];
  expect(restored).toContain("typed during the review");
  expect(restored).not.toContain("typed after");
});
