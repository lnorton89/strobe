/*
 * Shared scenes as files in R2, named by their content (D527).
 *
 * The /play page's Share uploads a scene (its title and files as JSON); the
 * Worker checks it, writes it once under a name taken from the SHA-256 of
 * its canonical form, and answers with a short link. The same scene always
 * gets the same name, so sharing it twice stores nothing new, and an object
 * is never changed or deleted by the site: there are no accounts, no tables,
 * nothing to migrate. Removing a reported scene is an operator's
 * `wrangler r2 object delete` (site/README.md).
 *
 * Everything here is a pure function over its input except `storeScene` and
 * `loadScene`, which take the bucket as an argument, so the rules are
 * tested without Cloudflare (tests/unit/scene-store.test.ts).
 */

import { validFileName } from "../src/lib/play/files";

/** Largest upload accepted, in bytes: far above any real scene's text. */
export const MAX_BYTES = 256 * 1024;
/** Most files one scene may carry. */
export const MAX_FILES = 16;
/** Characters of the hash a link carries: 50 bits, ample for content names. */
export const ID_LENGTH = 10;
/** A longer name, used only if a short one is ever taken by other content. */
export const LONG_ID_LENGTH = 16;

/** The links' alphabet: lowercase base32 without 0, 1, o or l. */
const ALPHABET = "23456789abcdefghijkmnpqrstuvwxyz";

export type StoredScene = { title: string; files: Record<string, string> };

/** The minimal R2 surface the store uses, so tests can pass a fake. */
export interface Bucket {
  head(key: string): Promise<{ customMetadata?: Record<string, string> } | null>;
  get(key: string): Promise<{ text(): Promise<string> } | null>;
  put(
    key: string,
    value: string,
    options: {
      httpMetadata: { contentType: string };
      customMetadata: Record<string, string>;
    },
  ): Promise<unknown>;
}

/**
 * A scene from an untrusted upload, or the reason it is refused: a JSON
 * object with an optional title and a `files` map of scene file names to
 * text, `main.glsl` among them.
 */
export function validateScene(value: unknown): StoredScene | string {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return "expected a JSON object";
  }
  const { title, files } = value as { title?: unknown; files?: unknown };
  if (files === null || typeof files !== "object" || Array.isArray(files)) {
    return "expected `files`, an object of file name to text";
  }
  const entries = Object.entries(files as Record<string, unknown>);
  if (entries.length === 0 || entries.length > MAX_FILES) {
    return `a scene has 1 to ${MAX_FILES} files`;
  }
  const clean: Record<string, string> = {};
  for (const [name, text] of entries) {
    if (!validFileName(name)) return `${name}: not a scene file name`;
    if (typeof text !== "string") return `${name}: not text`;
    clean[name] = text;
  }
  if (clean["main.glsl"] === undefined) return "a scene needs main.glsl";
  const name = typeof title === "string" ? title.trim().slice(0, 80) : "";
  return { title: name || "untitled", files: clean };
}

/** The scene as one string, the same for the same scene however it arrived. */
export function canonical(scene: StoredScene): string {
  const files = Object.fromEntries(
    Object.keys(scene.files)
      .sort()
      .map((name) => [name, scene.files[name]]),
  );
  return JSON.stringify({ title: scene.title, files });
}

/** The full SHA-256 of `text`, in the links' alphabet (5 bits a character). */
export async function contentHash(text: string): Promise<string> {
  const digest = new Uint8Array(
    await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)),
  );
  let bits = 0;
  let value = 0;
  let out = "";
  for (const byte of digest) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += ALPHABET[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  return out;
}

/** Whether `id` could be a scene's name: guards the bucket from odd keys. */
export function isSceneId(id: string): boolean {
  return new RegExp(`^[${ALPHABET}]{${ID_LENGTH},${LONG_ID_LENGTH}}$`).test(id);
}

const key = (id: string) => `scenes/${id}.json`;

/**
 * Stores `scene` under its content name and returns that name. An object
 * already there with the same content is left as it is; one with other
 * content (a 50-bit collision) moves this scene to the longer name.
 */
export async function storeScene(bucket: Bucket, scene: StoredScene): Promise<string> {
  const text = canonical(scene);
  const hash = await contentHash(text);
  for (const length of [ID_LENGTH, LONG_ID_LENGTH]) {
    const id = hash.slice(0, length);
    const existing = await bucket.head(key(id));
    if (existing?.customMetadata?.hash === hash) return id;
    if (existing) continue;
    await bucket.put(key(id), text, {
      httpMetadata: { contentType: "application/json" },
      customMetadata: { hash },
    });
    return id;
  }
  throw new Error("no free name for this scene");
}

/** The stored scene's JSON text, or undefined for no such scene. */
export async function loadScene(bucket: Bucket, id: string): Promise<string | undefined> {
  if (!isSceneId(id)) return undefined;
  const object = await bucket.get(key(id));
  return object ? object.text() : undefined;
}
