/*
 * The website's Worker (D525, D527): the static export as assets, plus the
 * three routes that make short share links for /play.
 *
 *   POST /api/share      a scene's JSON in, `{ id, url }` out
 *   GET  /api/scene/:id  the stored scene's JSON, cached for good (it never changes)
 *   GET  /s/:id          the /play page, which loads that scene, with the
 *                        scene's own title and picture in its social tags (D565)
 *   GET  /api/card/:id.jpg  the picture Share sent with the scene (D565)
 *   POST /api/read-image a small picture in, a reading of it out, nothing kept (D560)
 *   GET  /api/models/... the in-browser CLIP's files, from R2 (D560)
 *
 * wrangler.jsonc runs this script first only for /api/* and /s/*; every
 * other request is a static file served without it.
 */

import { type ModelBucket, serveModelFile } from "./model-store";
import { type Ai, readImage } from "./read-image";
import { type Bucket, MAX_BYTES, isSceneId, loadScene, storeScene, validateScene } from "./scene-store";
import {
  CARD_BASE64_CHARS,
  type CardBucket,
  cardPage,
  hasCard,
  loadCard,
  readCard,
  storeCard,
} from "./share-card";

type Env = {
  ASSETS: { fetch(request: Request | URL | string): Promise<Response> };
  SCENES: Bucket & CardBucket;
  SHARE_LIMIT?: { limit(options: { key: string }): Promise<{ success: boolean }> };
  READ_LIMIT?: { limit(options: { key: string }): Promise<{ success: boolean }> };
  AI?: Ai;
  MODELS?: ModelBucket;
};

function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", ...headers },
  });
}

async function share(request: Request, env: Env): Promise<Response> {
  const visitor = request.headers.get("CF-Connecting-IP") ?? "unknown";
  if (env.SHARE_LIMIT && !(await env.SHARE_LIMIT.limit({ key: visitor })).success) {
    return json({ error: "too many shares; wait a minute" }, 429);
  }
  // The upload may carry a card beside the scene (D565); the scene's own
  // limit is unchanged and is measured without it.
  const most = MAX_BYTES + CARD_BASE64_CHARS + 64;
  const declared = Number(request.headers.get("Content-Length") ?? "0");
  if (declared > most) return json({ error: "the scene is too large to share" }, 413);
  const body = await request.text();
  if (body.length > most) return json({ error: "the scene is too large to share" }, 413);
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    return json({ error: "not JSON" }, 400);
  }
  const sent = (parsed as { card?: unknown } | null)?.card;
  const sentLength = typeof sent === "string" ? sent.length : 0;
  if (body.length - sentLength > MAX_BYTES) {
    return json({ error: "the scene is too large to share" }, 413);
  }
  const scene = validateScene(parsed);
  if (typeof scene === "string") return json({ error: scene }, 400);
  const id = await storeScene(env.SCENES, scene);
  const picture = readCard(sent);
  if (picture) await storeCard(env.SCENES, id, picture);
  return json({ id, url: new URL(`/s/${id}`, request.url).toString() }, 201);
}

async function card(id: string, env: Env): Promise<Response> {
  const bytes = isSceneId(id) ? await loadCard(env.SCENES, id) : undefined;
  if (bytes === undefined) return json({ error: "no such card" }, 404);
  return new Response(bytes, {
    headers: {
      "Content-Type": "image/jpeg",
      // A scene keeps the first card it gets, so this never changes either.
      "Cache-Control": "public, max-age=31536000, immutable",
    },
  });
}

/** The /play page for the shared scene `id`, its social tags naming that scene. */
async function sharedPage(id: string, url: URL, env: Env): Promise<Response> {
  const page = await env.ASSETS.fetch(new URL("/play/", url));
  const text = await loadScene(env.SCENES, id);
  if (!page.ok || text === undefined) return page;
  let title = "untitled";
  try {
    const stored = JSON.parse(text) as { title?: unknown };
    if (typeof stored.title === "string" && stored.title) title = stored.title;
  } catch {
    return page;
  }
  const image = (await hasCard(env.SCENES, id))
    ? new URL(`/api/card/${id}.jpg`, url).toString()
    : undefined;
  const html = cardPage(await page.text(), {
    title,
    url: new URL(`/s/${id}`, url).toString(),
    image,
  });
  const headers = new Headers(page.headers);
  headers.delete("Content-Length");
  headers.delete("ETag");
  return new Response(html, { status: page.status, headers });
}

async function scene(id: string, env: Env): Promise<Response> {
  const text = await loadScene(env.SCENES, id);
  if (text === undefined) return json({ error: "no such scene" }, 404);
  return new Response(text, {
    headers: {
      "Content-Type": "application/json",
      // A name is the content's hash, so what it names never changes.
      "Cache-Control": "public, max-age=31536000, immutable",
    },
  });
}

const worker = {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname;
    if (path === "/api/share") {
      if (request.method !== "POST") return json({ error: "POST a scene" }, 405, { Allow: "POST" });
      return share(request, env);
    }
    if (path === "/api/read-image") {
      if (request.method !== "POST") return json({ error: "POST a picture" }, 405, { Allow: "POST" });
      return readImage(request, env);
    }
    const modelFile = /^\/api\/models\/(.+)$/.exec(path);
    if (modelFile?.[1] !== undefined) return serveModelFile(modelFile[1], request.method, env.MODELS);
    const fetched = /^\/api\/scene\/([^/]+)$/.exec(path);
    if (fetched?.[1] !== undefined && request.method === "GET") return scene(fetched[1], env);
    const picture = /^\/api\/card\/([^/]+)\.jpg$/.exec(path);
    if (picture?.[1] !== undefined && request.method === "GET") return card(picture[1], env);
    const link = /^\/s\/([^/]+)\/?$/.exec(path);
    if (link?.[1] !== undefined && isSceneId(link[1])) {
      // The /play page itself; it reads the id from the address.
      return sharedPage(link[1], url, env);
    }
    return env.ASSETS.fetch(request);
  },
};

export default worker;
