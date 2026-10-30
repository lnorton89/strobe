/*
 * The website's Worker (D525, D527): the static export as assets, plus the
 * three routes that make short share links for /play.
 *
 *   POST /api/share      a scene's JSON in, `{ id, url }` out
 *   GET  /api/scene/:id  the stored scene's JSON, cached for good (it never changes)
 *   GET  /s/:id          the /play page, which loads that scene
 *
 * wrangler.jsonc runs this script first only for /api/* and /s/*; every
 * other request is a static file served without it.
 */

import { type Bucket, MAX_BYTES, isSceneId, loadScene, storeScene, validateScene } from "./scene-store";

type Env = {
  ASSETS: { fetch(request: Request | URL | string): Promise<Response> };
  SCENES: Bucket;
  SHARE_LIMIT?: { limit(options: { key: string }): Promise<{ success: boolean }> };
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
  const declared = Number(request.headers.get("Content-Length") ?? "0");
  if (declared > MAX_BYTES) return json({ error: "the scene is too large to share" }, 413);
  const body = await request.text();
  if (body.length > MAX_BYTES) return json({ error: "the scene is too large to share" }, 413);
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    return json({ error: "not JSON" }, 400);
  }
  const scene = validateScene(parsed);
  if (typeof scene === "string") return json({ error: scene }, 400);
  const id = await storeScene(env.SCENES, scene);
  return json({ id, url: new URL(`/s/${id}`, request.url).toString() }, 201);
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
    const fetched = /^\/api\/scene\/([^/]+)$/.exec(path);
    if (fetched?.[1] !== undefined && request.method === "GET") return scene(fetched[1], env);
    const link = /^\/s\/([^/]+)\/?$/.exec(path);
    if (link?.[1] !== undefined && isSceneId(link[1])) {
      // The /play page itself; it reads the id from the address.
      return env.ASSETS.fetch(new URL("/play/", url));
    }
    return env.ASSETS.fetch(request);
  },
};

export default worker;
