import { describe, expect, it } from "vitest";
import {
  type Bucket,
  ID_LENGTH,
  LONG_ID_LENGTH,
  canonical,
  contentHash,
  isSceneId,
  loadScene,
  storeScene,
  validateScene,
} from "../../worker/scene-store";
import { SHARE_IMAGE_BYTES, sharedIdFromPath } from "../../src/lib/play/files";

/*
 * Short share links (D527): what the Worker accepts, how a scene is named,
 * and that the same scene is stored once. The bucket is an in-memory fake
 * with R2's head/get/put shape.
 */

function fakeBucket() {
  const objects = new Map<string, { text: string; meta: Record<string, string> }>();
  const bucket: Bucket = {
    async head(key) {
      const object = objects.get(key);
      return object ? { customMetadata: object.meta } : null;
    },
    async get(key) {
      const object = objects.get(key);
      return object ? { text: async () => object.text } : null;
    },
    async put(key, value, options) {
      objects.set(key, { text: value, meta: options.customMetadata });
    },
  };
  return { bucket, objects };
}

const SCENE = { title: "rings", files: { "scene.json": "{}", "main.glsl": "vec4 renderMain(){}" } };

describe("shared scenes", () => {
  it("accepts a scene and refuses what is not one", () => {
    expect(validateScene(SCENE)).toEqual(SCENE);
    expect(validateScene({ files: { "main.glsl": "x" } })).toEqual({
      title: "untitled",
      files: { "main.glsl": "x" },
    });
    expect(validateScene([])).toMatch(/object/);
    expect(validateScene({ files: { "scene.json": "{}" } })).toMatch(/main\.glsl/);
    expect(validateScene({ files: { "main.glsl": "x", "../evil.js": "y" } })).toMatch(/not a scene file name/);
    expect(validateScene({ files: { "main.glsl": 3 } })).toMatch(/not text/);
    const many = Object.fromEntries(Array.from({ length: 17 }, (_, i) => [`p${i}.glsl`, ""]));
    expect(validateScene({ files: { ...many, "main.glsl": "" } })).toMatch(/1 to 16/);
  });

  it("accepts a scene's small pictures and refuses large or misnamed ones", () => {
    const images = { "mark.png": "iVBORw0KGgo=" };
    expect(validateScene({ ...SCENE, images })).toEqual({ ...SCENE, images });
    expect(validateScene({ ...SCENE, images: {} })).toEqual(SCENE);
    expect(validateScene({ ...SCENE, images: { "../x.png": "AAAA" } })).toMatch(/not a picture name/);
    expect(validateScene({ ...SCENE, images: { "x.png": 3 } })).toMatch(/not base64/);
    const big = "A".repeat(Math.ceil(((SHARE_IMAGE_BYTES + 1024) * 4) / 3));
    expect(validateScene({ ...SCENE, images: { "x.png": big } })).toMatch(/KB in all/);
    // A scene without pictures keeps the name it had before pictures existed.
    expect(canonical({ ...SCENE, images: {} })).toBe(canonical(SCENE));
    expect(canonical({ ...SCENE, images })).not.toBe(canonical(SCENE));
  });

  it("names a scene by its content, whatever order its files came in", async () => {
    const reordered = { title: "rings", files: { "main.glsl": "vec4 renderMain(){}", "scene.json": "{}" } };
    expect(canonical(reordered)).toBe(canonical(SCENE));
    const hash = await contentHash(canonical(SCENE));
    // 256 bits make 51 whole five-bit characters.
    expect(hash).toHaveLength(51);
    expect(isSceneId(hash.slice(0, ID_LENGTH))).toBe(true);
    expect(isSceneId("../../etc")).toBe(false);
    expect(isSceneId("short")).toBe(false);
  });

  it("stores a scene once and reads it back", async () => {
    const { bucket, objects } = fakeBucket();
    const id = await storeScene(bucket, SCENE);
    expect(id).toHaveLength(ID_LENGTH);
    expect(await storeScene(bucket, { ...SCENE })).toBe(id);
    expect(objects.size).toBe(1);
    expect(JSON.parse((await loadScene(bucket, id)) ?? "null")).toEqual(SCENE);
    expect(await loadScene(bucket, "zzzzzzzzzz")).toBeUndefined();
  });

  it("moves to the longer name when the short one holds other content", async () => {
    const { bucket, objects } = fakeBucket();
    const hash = await contentHash(canonical(SCENE));
    objects.set(`scenes/${hash.slice(0, ID_LENGTH)}.json`, { text: "{}", meta: { hash: "other" } });
    const id = await storeScene(bucket, SCENE);
    expect(id).toBe(hash.slice(0, LONG_ID_LENGTH));
  });

  it("reads a short link's id from the page's path", () => {
    expect(sharedIdFromPath("/s/k3f9x2a7qm")).toBe("k3f9x2a7qm");
    expect(sharedIdFromPath("/s/k3f9x2a7qm/")).toBe("k3f9x2a7qm");
    expect(sharedIdFromPath("/play/")).toBeUndefined();
    expect(sharedIdFromPath("/s/../etc")).toBeUndefined();
  });
});
