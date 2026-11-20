import { describe, expect, it } from "vitest";
import worker from "../../worker/index";

/*
 * The Worker's share routes together (D527, D565), against an in-memory
 * bucket and a one-page site: a scene shared with a card gets a link whose
 * page names it and shows its picture, and the picture is served.
 */

const PLAY = [
  "<title>Play · Strobe</title>",
  '<meta property="og:title" content="Write a Strobe scene in your browser."/>',
  '<meta property="og:url" content="https://strobe.example/play/"/>',
  '<meta property="og:image" content="https://strobe.example/play/og-image.jpg"/>',
  '<meta name="twitter:image" content="https://strobe.example/play/og-image.jpg"/>',
].join("");

function fakeEnv() {
  const objects = new Map<string, { bytes: Uint8Array; meta: Record<string, string> }>();
  const encode = (value: string | Uint8Array) =>
    typeof value === "string" ? new TextEncoder().encode(value) : value;
  const SCENES = {
    async head(key: string) {
      const object = objects.get(key);
      return object ? { customMetadata: object.meta } : null;
    },
    async get(key: string) {
      const object = objects.get(key);
      if (!object) return null;
      return {
        text: async () => new TextDecoder().decode(object.bytes),
        arrayBuffer: async () => object.bytes.slice().buffer,
      };
    },
    async put(
      key: string,
      value: string | Uint8Array,
      options: { customMetadata?: Record<string, string> },
    ) {
      objects.set(key, { bytes: encode(value), meta: options.customMetadata ?? {} });
    },
  };
  const ASSETS = {
    async fetch(request: Request | URL | string) {
      const url = new URL(request instanceof Request ? request.url : request.toString());
      if (url.pathname === "/play/") {
        return new Response(PLAY, { headers: { "Content-Type": "text/html; charset=utf-8" } });
      }
      return new Response("missing", { status: 404 });
    },
  };
  return { env: { SCENES, ASSETS }, objects };
}

const JPEG = [0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4, 0xff, 0xd9];
const CARD = btoa(String.fromCharCode(...JPEG));
const SCENE = { title: "rings", files: { "main.glsl": "vec4 renderMain(){}" } };

function post(body: unknown): Request {
  return new Request("https://strobe.example/api/share", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

describe("the Worker's share routes", () => {
  it("gives a scene shared with a card a page and a picture of its own", async () => {
    const { env, objects } = fakeEnv();
    const shared = await worker.fetch(post({ ...SCENE, card: CARD }), env);
    expect(shared.status).toBe(201);
    const { id, url } = (await shared.json()) as { id: string; url: string };
    expect(url).toBe(`https://strobe.example/s/${id}`);
    expect([...objects.keys()].sort()).toEqual([`cards/${id}.jpg`, `scenes/${id}.json`]);

    const page = await worker.fetch(new Request(url), env);
    const html = await page.text();
    expect(page.headers.get("Content-Type")).toContain("text/html");
    expect(html).toContain('<meta property="og:title" content="rings · a Strobe scene"/>');
    expect(html).toContain(`<meta property="og:url" content="${url}"/>`);
    expect(html).toContain(`<meta property="og:image" content="https://strobe.example/api/card/${id}.jpg"/>`);
    expect(html).toContain(`<meta name="twitter:image" content="https://strobe.example/api/card/${id}.jpg"/>`);

    const picture = await worker.fetch(new Request(`https://strobe.example/api/card/${id}.jpg`), env);
    expect(picture.status).toBe(200);
    expect(picture.headers.get("Content-Type")).toBe("image/jpeg");
    expect(new Uint8Array(await picture.arrayBuffer())).toEqual(Uint8Array.from(JPEG));
  });

  it("names a scene shared without a card the same, with or without one", async () => {
    const { env } = fakeEnv();
    const plain = (await (await worker.fetch(post(SCENE), env)).json()) as { id: string; url: string };
    const html = await (await worker.fetch(new Request(plain.url), env)).text();
    expect(html).toContain('<meta property="og:title" content="rings · a Strobe scene"/>');
    expect(html).toContain('<meta property="og:image" content="https://strobe.example/play/og-image.jpg"/>');
    expect((await worker.fetch(new Request(`https://strobe.example/api/card/${plain.id}.jpg`), env)).status).toBe(404);

    // The card is not part of the scene: sharing it again with one keeps
    // the name and fills the picture in.
    const again = (await (await worker.fetch(post({ ...SCENE, card: CARD }), env)).json()) as { id: string };
    expect(again.id).toBe(plain.id);
    expect((await worker.fetch(new Request(`https://strobe.example/api/card/${plain.id}.jpg`), env)).status).toBe(200);
  });

  it("shares a scene whose card is not a picture, without the card", async () => {
    const { env, objects } = fakeEnv();
    const shared = await worker.fetch(post({ ...SCENE, card: "bm90IGEganBlZw==" }), env);
    expect(shared.status).toBe(201);
    expect([...objects.keys()].every((key) => key.startsWith("scenes/"))).toBe(true);
  });

  it("serves the plain page for a link to no scene", async () => {
    const { env } = fakeEnv();
    const page = await worker.fetch(new Request("https://strobe.example/s/abcdefghjk"), env);
    expect(await page.text()).toBe(PLAY);
  });
});
