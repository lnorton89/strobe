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

import { SHARE_IMAGE_BYTES, validFileName, validImageName } from "../src/lib/play/files";

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

export type StoredScene = {
  title: string;
  files: Record<string, string>;
  /** The scene's small pictures, base64 by file name (D554). */
  images?: Record<string, string>;
};

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
 * object with an optional title, a `files` map of scene file names to
 * text, `main.glsl` among them, and optionally an `images` map of picture
 * names to base64, SHARE_IMAGE_BYTES in all at most (D554).
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
  const scene: StoredScene = { title: name || "untitled", files: clean };
  const { images } = value as { images?: unknown };
  if (images === undefined) return scene;
  if (images === null || typeof images !== "object" || Array.isArray(images)) {
    return "expected `images`, an object of picture name to base64";
  }
  const pictures: Record<string, string> = {};
  let total = 0;
  for (const [picture, data] of Object.entries(images as Record<string, unknown>)) {
    if (!validImageName(picture)) return `${picture}: not a picture name`;
    if (typeof data !== "string" || !/^[A-Za-z0-9+/]*={0,2}$/.test(data)) {
      return `${picture}: not base64`;
    }
    total += Math.floor((data.length * 3) / 4);
    pictures[picture] = data;
  }
  if (total > SHARE_IMAGE_BYTES + 2 * Object.keys(pictures).length) {
    return `the pictures are over ${SHARE_IMAGE_BYTES / 1024} KB in all`;
  }
  if (Object.keys(pictures).length > 0) scene.images = pictures;
  return scene;
}

/** The scene as one string, the same for the same scene however it arrived. */
export function canonical(scene: StoredScene): string {
  const sorted = (map: Record<string, string>) =>
    Object.fromEntries(
      Object.keys(map)
        .sort()
        .map((name) => [name, map[name]]),
    );
  const files = sorted(scene.files);
  // A scene with no pictures keeps the form it had before they existed, so
  // its name does not change.
  if (!scene.images || Object.keys(scene.images).length === 0) {
    return JSON.stringify({ title: scene.title, files });
  }
  return JSON.stringify({ title: scene.title, files, images: sorted(scene.images) });
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
