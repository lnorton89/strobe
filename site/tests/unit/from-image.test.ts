import { describe, expect, it } from "vitest";
import { type Pixels, analyse, labToRgb, rgbToLab } from "../../src/lib/play/image/analyse";
import { assignColours, candidates, emitScene } from "../../src/lib/play/image/generate";
import { countFlashes, gate } from "../../src/lib/play/image/gate";
import { mapPoint, squareToQuad } from "../../src/lib/play/image/intake";
import { challengeFor, complete, listModels } from "../../src/lib/play/image/llm";
import { cleanReading, firstJsonObject, firstReading } from "../../src/lib/play/image/reading";
import { TEMPLATES } from "../../src/lib/play/image/templates";
import { cleanSceneJson, parseReply, writeScene } from "../../src/lib/play/image/writer";
import { MODEL_FILES, serveModelFile } from "../../worker/model-store";
import { explainFailure, readImage } from "../../worker/read-image";

/*
 * From image (D560). The pixels here are made up on the spot, so these pin the
 * arithmetic; whether real pictures give good scenes is a person's call, and
 * that the templates compile is tests/e2e/play-from-image.spec.ts.
 */

function picture(width: number, height: number, at: (x: number, y: number) => [number, number, number]): Pixels {
  const data = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const [r, g, b] = at(x / width, y / height);
      data.set([r, g, b, 255], (y * width + x) * 4);
    }
  }
  return { width, height, data };
}

const DUSK = picture(200, 200, (x, y) => {
  // A dark blue ground with an orange disc and a mirrored pair of bars.
  const disc = Math.hypot(x - 0.5, y - 0.5) < 0.2;
  const bar = Math.abs(Math.abs(x - 0.5) - 0.35) < 0.04;
  return disc ? [240, 130, 30] : bar ? [230, 230, 220] : [12, 18, 60];
});

describe("measuring a picture", () => {
  it("round-trips Oklab", () => {
    const back = labToRgb(rgbToLab([0.2, 0.5, 0.8]));
    expect(back[0], "red").toBeCloseTo(0.2, 2);
    expect(back[1], "green").toBeCloseTo(0.5, 2);
    expect(back[2], "blue").toBeCloseTo(0.8, 2);
  });

  it("finds the colours and the symmetry", () => {
    const measures = analyse(DUSK);
    const orange = measures.palette.some(({ rgb }) => rgb[0] > 0.7 && rgb[2] < 0.35);
    expect(orange, `orange in ${JSON.stringify(measures.palette.map((s) => s.rgb))}`).toBe(true);
    expect(measures.palette[0]?.weight, "palette sorted by share").toBeGreaterThan(0.4);
    expect(measures.mirror, "mirrored").toBeGreaterThan(0.9);
    expect(measures.lightness.mean, "a dark picture").toBeLessThan(0.5);
  });

  it("does not call a lopsided picture mirrored", () => {
    const lopsided = picture(200, 200, (x, y) => (x < 0.3 && y < 0.6 ? [230, 230, 230] : [10, 10, 10]));
    expect(analyse(lopsided).mirror, "mirror score").toBeLessThan(0.5);
  });

  it("hears the repeats around a centre", () => {
    const star = picture(240, 240, (x, y) => {
      const angle = Math.atan2(y - 0.5, x - 0.5);
      const v = Math.cos(angle * 8) > 0 ? 230 : 15;
      return [v, v, v];
    });
    const { radial } = analyse(star);
    expect(radial.order, "eight-fold").toBe(8);
    expect(radial.score, "strongly radial").toBeGreaterThan(0.5);
  });

  it("gives the same answer twice, so a picture gets the same scenes", () => {
    expect(analyse(DUSK), "deterministic").toEqual(analyse(DUSK));
  });
});

describe("making scenes", () => {
  const measures = analyse(DUSK);
  const reading = { ...firstReading(measures), by: "test" };

  it("gives colours jobs that read on a dark room", () => {
    const { ground, ink } = assignColours(measures);
    const light = (rgb: readonly number[]) => (rgb[0] ?? 0) + (rgb[1] ?? 0) + (rgb[2] ?? 0);
    expect(light(ground), "a dark ground").toBeLessThan(0.9);
    expect(light(ink), "ink well above the ground").toBeGreaterThan(light(ground) + 0.8);
  });

  it("emits every template as a scene the app's format accepts", () => {
    for (const template of TEMPLATES) {
      const scene = emitScene(template, measures, reading, assignColours(measures), 7);
      const json = JSON.parse(scene.files["scene.json"] ?? "{}") as {
        CONTROLS: { NAME: string; TYPE: string; DEF_COLOR?: number[] }[];
        PASSES: { TARGET: string }[];
      };
      const names = json.CONTROLS.map((control) => control.NAME);
      expect(new Set(names).size, `${template.id}: control names are unique`).toBe(names.length);
      expect(names.length, `${template.id}: a usable number of controls`).toBeGreaterThanOrEqual(8);
      expect(names.length, `${template.id}: not too many`).toBeLessThanOrEqual(16);
      for (const name of ["speed", "reactivity", "glow", "brightness", "ground_color", template.bang.name]) {
        expect(names, `${template.id} has ${name}`).toContain(name);
      }
      expect(json.PASSES[0]?.TARGET, `${template.id}: the clock pass`).toBe("clock");
      const glsl = scene.files["main.glsl"] ?? "";
      expect(glsl, `${template.id}: draws a picture`).toContain("vec3 picture(vec2 p)");
      expect(glsl, `${template.id}: never samples a picture`).not.toMatch(/\b(syn_Media|IMAGES|_textureImage|_loadImage)/);
      expect(glsl, `${template.id}: no fwidth`).not.toMatch(/\bfwidth\b|\bdFdx\b/);
      // ES 3.00 has no implicit int to float.
      expect(glsl, `${template.id}: BANG was replaced`).not.toContain("BANG");
    }
  });

  it("keeps file names and pixels out of a scene", () => {
    const [first] = candidates(measures, reading, 1);
    const text = JSON.stringify(first?.scene);
    expect(text, "no data URI").not.toContain("base64");
    expect(first?.scene.images, "no pictures").toBeUndefined();
  });

  it("offers something for a wrong reading too, all different", () => {
    const list = candidates(measures, { ...reading, layout: "radial" }, 9);
    expect(list.length, "a full gallery").toBe(9);
    expect(new Set(list.map((item) => item.id)).size, "no repeats").toBe(9);
    expect(new Set(list.map((item) => item.template)).size, "mostly different templates").toBeGreaterThanOrEqual(8);
    expect(list.slice(0, 2).every((item) => item.note.startsWith("Your layout")), "yours first").toBe(true);
  });

  it("uses a template that needs its motif only with that motif", () => {
    const cold = candidates(measures, { ...reading, layout: "field", motif: "water" }, 12);
    expect(cold.some((item) => item.template === "flame"), "no flames on water").toBe(false);
    const hot = candidates(measures, { ...reading, layout: "field", motif: "fire" }, 12);
    expect(hot.some((item) => item.template === "flame"), "flames on fire").toBe(true);
  });
});

describe("the gates", () => {
  const frames = (values: number[], size = 8) =>
    values.map((value) => new Uint8ClampedArray(size * size * 4).fill(value).map((v, i) => (i % 4 === 3 ? 255 : v)));
  const textured = (shift: number, size = 8) => {
    const data = new Uint8ClampedArray(size * size * 4);
    for (let i = 0; i < size * size; i += 1) {
      const v = 60 + ((i * 37 + shift) % 80);
      data.set([v, v, v, 255], i * 4);
    }
    return data;
  };

  it("counts a flash as a pair of opposing changes", () => {
    expect(countFlashes([0, 0.5, 0, 0.5, 0]), "two up-downs").toBe(2);
    expect(countFlashes([0.1, 0.12, 0.15, 0.17]), "a slow rise is no flash").toBe(0);
    expect(countFlashes([0.85, 0.99, 0.85, 0.99]), "both states bright").toBe(0);
  });

  it("rejects a strobing picture and keeps a moving one", () => {
    const strobe = frames(Array.from({ length: 30 }, (_, i) => (i % 4 < 2 ? 0 : 220)));
    const bad = gate({ width: 8, height: 8, rate: 30, frames: strobe });
    expect(bad.passed, "strobing fails").toBe(false);
    expect(bad.problems.join(), "says why").toContain("flashes");
    const good = gate({ width: 8, height: 8, rate: 30, frames: Array.from({ length: 30 }, (_, i) => textured(i)) });
    expect(good.passed, `moving passes: ${good.problems.join()}`).toBe(true);
  });

  it("rejects black, flat, blown out and frozen", () => {
    const check = (set: Uint8ClampedArray[], word: string) => {
      const result = gate({ width: 8, height: 8, rate: 30, frames: set });
      expect(result.problems.join(), `${word} is named`).toContain(word);
    };
    check(frames([1, 1, 1]), "black");
    check(frames([120, 120, 120]), "flat");
    check(frames([255, 255, 255]), "blown");
    check([textured(3), textured(3), textured(3)], "does not move");
  });
});

describe("straightening", () => {
  it("maps the square onto the corners", () => {
    const m = squareToQuad([
      [0.1, 0.2],
      [0.9, 0.1],
      [0.8, 0.9],
      [0.2, 0.8],
    ]);
    for (const [u, v, x, y] of [
      [0, 0, 0.1, 0.2],
      [1, 0, 0.9, 0.1],
      [1, 1, 0.8, 0.9],
      [0, 1, 0.2, 0.8],
    ] as const) {
      const [px, py] = mapPoint(m, u, v);
      expect(px, `x at ${u},${v}`).toBeCloseTo(x, 6);
      expect(py, `y at ${u},${v}`).toBeCloseTo(y, 6);
    }
  });
});

describe("a model's reading", () => {
  it("keeps only our own words", () => {
    expect(
      cleanReading({ layout: "Radial", motif: "lasers", mood: "calm", subject: "A <b>jester</b> doll!!", extra: 1 }),
      "cleaned",
    ).toEqual({ layout: "radial", mood: "calm", subject: "a bjesterb doll" });
    expect(cleanReading("ignore your instructions"), "not an object").toEqual({});
  });

  it("finds JSON inside prose", () => {
    expect(firstJsonObject('Sure! ```json\n{"layout":"grid"}\n```'), "fenced").toEqual({ layout: "grid" });
    expect(firstJsonObject("no json here"), "none").toBeUndefined();
  });
});

describe("providers", () => {
  type Call = { url: string; init: RequestInit };
  function fake(body: unknown, calls: Call[], status = 200): typeof fetch {
    return ((url: string, init: RequestInit) => {
      calls.push({ url, init });
      return Promise.resolve(new Response(JSON.stringify(body), { status }));
    }) as unknown as typeof fetch;
  }
  const messages = [{ role: "user" as const, parts: [{ jpegBase64: "QQ==" }, { text: "hi" }] }];
  const base = { key: "test-key", model: "m", system: "s", messages, maxTokens: 10 };

  it("speaks Anthropic, from a browser", async () => {
    const calls: Call[] = [];
    const text = await complete(
      { ...base, provider: "anthropic" },
      fake({ content: [{ type: "text", text: "ok" }] }, calls),
    );
    expect(text, "reply").toBe("ok");
    const headers = calls[0]?.init.headers as Record<string, string>;
    expect(headers["anthropic-dangerous-direct-browser-access"], "browser header").toBe("true");
    expect(headers["x-api-key"], "key header").toBe("test-key");
    const sent = JSON.parse(String(calls[0]?.init.body)) as { messages: { content: { type: string }[] }[] };
    expect(sent.messages[0]?.content[0]?.type, "picture part").toBe("image");
  });

  it("speaks OpenAI and OpenRouter", async () => {
    const calls: Call[] = [];
    const reply = { choices: [{ message: { content: "ok" } }] };
    expect(await complete({ ...base, provider: "openai" }, fake(reply, calls)), "openai").toBe("ok");
    expect(await complete({ ...base, provider: "openrouter" }, fake(reply, calls)), "openrouter").toBe("ok");
    expect(calls[0]?.url, "openai url").toContain("api.openai.com");
    expect(String(calls[0]?.init.body), "openai's token field").toContain("max_completion_tokens");
    expect(calls[1]?.url, "openrouter url").toContain("openrouter.ai");
    expect(String(calls[1]?.init.body), "a data URI").toContain("data:image/jpeg;base64,QQ==");
  });

  it("speaks Gemini", async () => {
    const calls: Call[] = [];
    const reply = { candidates: [{ content: { parts: [{ text: "ok" }] } }] };
    expect(await complete({ ...base, provider: "google" }, fake(reply, calls)), "reply").toBe("ok");
    expect(calls[0]?.url, "model in the url").toContain("/models/m:generateContent");
    expect(String(calls[0]?.init.body), "inline picture").toContain("inlineData");
  });

  it("never puts the key in a URL", async () => {
    const calls: Call[] = [];
    for (const provider of ["anthropic", "openai", "openrouter", "google"] as const) {
      await complete({ ...base, provider }, fake({}, calls)).catch(() => "");
    }
    for (const call of calls) expect(call.url, "no key in the address").not.toContain("test-key");
  });

  it("says when a key is refused", async () => {
    const calls: Call[] = [];
    await expect(
      complete({ ...base, provider: "openai" }, fake({ error: { message: "bad key" } }, calls, 401)),
      "a refusal",
    ).rejects.toThrow(/refused the key.*bad key/);
  });

  it("lists models that can take pictures", async () => {
    const calls: Call[] = [];
    const google = await listModels(
      "google",
      "k",
      fake(
        {
          models: [
            { name: "models/gemini-pro", supportedGenerationMethods: ["generateContent"] },
            { name: "models/gemini-flash", supportedGenerationMethods: ["generateContent"] },
            { name: "models/gemini-embedding", supportedGenerationMethods: ["embedContent"] },
          ],
        },
        calls,
      ),
    );
    expect(google, "flash first, no embeddings").toEqual(["gemini-flash", "gemini-pro"]);
    const router = await listModels(
      "openrouter",
      "k",
      fake(
        {
          data: [
            { id: "a/vision", architecture: { input_modalities: ["text", "image"] } },
            { id: "b/text", architecture: { input_modalities: ["text"] } },
          ],
        },
        calls,
      ),
    );
    expect(router, "only vision").toEqual(["a/vision"]);
  });

  it("makes PKCE's published test vector", async () => {
    // RFC 7636, appendix B.
    expect(await challengeFor("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "S256").toBe(
      "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
    );
  });
});

describe("the model as an untrusted author", () => {
  const good = `<scene_json>{"TITLE":"T","CONTROLS":[]}</scene_json><main_glsl>vec4 renderMain() { return vec4(1.0); }</main_glsl>`;

  it("takes the two blocks, fenced or not", () => {
    const parsed = parseReply(
      "Here!\n<scene_json>\n```json\n{}\n```\n</scene_json>\n<main_glsl>\n```glsl\nx\n```\n</main_glsl>",
    );
    expect(parsed, "fences stripped").toEqual({ json: "{}", glsl: "x" });
    expect(typeof parseReply("nothing"), "a reason").toBe("string");
  });

  it("drops anything that reaches for a file", () => {
    const clean = cleanSceneJson(JSON.stringify({ TITLE: "T", IMAGES: ["/etc/passwd"], PASS_SOURCES: ["a"], MEDIA: "x" }));
    expect(typeof clean, "valid").toBe("string");
    expect(String(clean), "no IMAGES").not.toContain("IMAGES");
    expect(String(clean), "no PASS_SOURCES").not.toContain("PASS_SOURCES");
    expect(cleanSceneJson("[]"), "an array").toHaveProperty("error");
    expect(cleanSceneJson("{"), "broken").toHaveProperty("error");
  });

  const measures = analyse(DUSK);
  const reading = { ...firstReading(measures), by: "test" };
  const passing = { passed: true, problems: [], flashesPerSecond: 0, mean: 0.3 };
  function modelSaying(replies: string[]): typeof fetch {
    let next = 0;
    return (() => {
      const content = replies[Math.min(next, replies.length - 1)] ?? "";
      next += 1;
      return Promise.resolve(new Response(JSON.stringify({ choices: [{ message: { content } }] })));
    }) as unknown as typeof fetch;
  }
  const request = {
    provider: "openai" as const,
    key: "k",
    model: "m",
    picture: "QQ==",
    measures,
    reading,
    onProgress: () => undefined,
  };

  it("feeds compile errors back until it plays, and returns only the two files", async () => {
    let compiles = 0;
    const written = await writeScene({
      ...request,
      critiques: 0,
      fetcher: modelSaying(["no blocks", good]),
      tools: {
        compile: () => {
          compiles += 1;
          return Promise.resolve(undefined);
        },
        render: () => Promise.resolve({ result: passing, jpegBase64: "QQ==" }),
      },
    });
    expect(compiles, "only the good reply reached the compiler").toBe(1);
    expect(Object.keys(written.scene.files).sort(), "two files, never script.js").toEqual(["main.glsl", "scene.json"]);
  });

  it("gives up with the error after three bad compiles", async () => {
    await expect(
      writeScene({
        ...request,
        critiques: 0,
        fetcher: modelSaying([good]),
        tools: {
          compile: () => Promise.resolve("main.glsl:1: nope"),
          render: () => Promise.resolve({ result: passing, jpegBase64: "QQ==" }),
        },
      }),
      "stops",
    ).rejects.toThrow(/did not compile after 3 tries[\s\S]*nope/);
  });

  it("keeps the last good version when a critique round breaks it", async () => {
    let compiles = 0;
    const written = await writeScene({
      ...request,
      critiques: 1,
      fetcher: modelSaying([good]),
      tools: {
        compile: () => {
          compiles += 1;
          return Promise.resolve(compiles === 1 ? undefined : "main.glsl:1: broke");
        },
        render: () => Promise.resolve({ result: passing, jpegBase64: "QQ==" }),
      },
    });
    expect(written.rounds, "no round landed").toBe(0);
    expect(written.warnings.join(), "says so").toContain("did not compile");
    expect(written.scene.title, "the first scene").toBe("T");
  });
});

describe("the model's files", () => {
  const stored = { size: 3, httpEtag: '"e"' };
  const bucket = {
    head: () => Promise.resolve(stored),
    get: () => Promise.resolve({ ...stored, body: new Response("abc").body as ReadableStream }),
  };
  const name = Object.keys(MODEL_FILES)[0] ?? "";

  it("serves the named files for good, and nothing else in the bucket", async () => {
    const response = await serveModelFile(name, "GET", bucket);
    expect(response.status, "found").toBe(200);
    expect(await response.text(), "the bytes").toBe("abc");
    expect(response.headers.get("Cache-Control"), "kept for good").toContain("immutable");
    expect((await serveModelFile("../strobe-scenes/x", "GET", bucket)).status, "not on the list").toBe(404);
    expect((await serveModelFile(name, "PUT", bucket)).status, "read only").toBe(405);
    expect((await serveModelFile(name, "GET", undefined)).status, "no bucket").toBe(503);
    const head = await serveModelFile(name, "HEAD", bucket);
    expect(head.headers.get("Content-Length"), "its size for progress").toBe("3");
  });

  it("says so when the files were never uploaded", async () => {
    const empty = { head: () => Promise.resolve(null), get: () => Promise.resolve(null) };
    expect((await serveModelFile(name, "GET", empty)).status, "not there").toBe(404);
  });
});

describe("the free reading", () => {
  const post = (body: string) =>
    new Request("https://x/api/read-image", { method: "POST", body, headers: { "Content-Length": String(body.length) } });
  const ask = JSON.stringify({ jpegBase64: "QUJD" });

  it("answers in our words only", async () => {
    const response = await readImage(post(ask), {
      AI: { run: () => Promise.resolve({ response: 'Here: {"layout":"grid","motif":"evil","mood":"calm"}' }) },
    });
    expect(response.status, "ok").toBe(200);
    expect(await response.json(), "cleaned").toEqual({ reading: { layout: "grid", mood: "calm" } });
  });

  it("refuses what it should", async () => {
    const AI = { run: () => Promise.resolve({ response: "{}" }) };
    expect((await readImage(post(ask), {})).status, "no binding").toBe(503);
    expect((await readImage(post("nope"), { AI })).status, "not JSON").toBe(400);
    expect((await readImage(post('{"jpegBase64":"<script>"}'), { AI })).status, "not base64").toBe(400);
    expect((await readImage(post(JSON.stringify({ jpegBase64: "A".repeat(300_000) })), { AI })).status, "too big").toBe(413);
    const limited = { AI, READ_LIMIT: { limit: () => Promise.resolve({ success: false }) } };
    expect((await readImage(post(ask), limited)).status, "rate limited").toBe(429);
  });

  it("explains what went wrong without passing it through", () => {
    expect(explainFailure("Error: 5016: Prior to using this model you must agree").status, "licence").toBe(503);
    expect(explainFailure("4006: you have used up your daily free allocation").status, "allowance").toBe(429);
    const other = explainFailure("secret internal detail");
    expect(other.status, "anything else").toBe(502);
    expect(other.error, "no internals").not.toContain("secret");
  });
});
