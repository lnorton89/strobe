import { describe, expect, it } from "vitest";
import {
  SHARE_IMAGE_BYTES,
  type Scene,
  base64ToBytes,
  bytesToBase64,
  decodeShare,
  encodeShare,
  fileOrder,
  forSharing,
  imageBytes,
  imageNameFor,
  parseScene,
  presetFile,
  presetName,
  readZip,
  validFileName,
  validImageName,
  validPresetName,
  writeZip,
} from "../../src/lib/play/files";

/*
 * A sandbox scene as files (D524), with the pictures and presets the app's
 * scene folders hold (D554): what a name may be, what a link and the
 * browser's storage carry, and that a zip keeps everything.
 */

/** `n` bytes of a repeating pattern, base64. */
const picture = (n: number) => bytesToBase64(Uint8Array.from({ length: n }, (_, i) => i % 251));

const SCENE: Scene = {
  title: "with pictures",
  files: {
    "main.glsl": "#include \"helpers.glsl\"\nvec4 renderMain() { return texture(mark, _uv); }\n",
    "helpers.glsl": "float k() { return 1.0; }\n",
    "scene.json": '{ "IMAGES": [ { "NAME": "mark", "PATH": "mark.png" } ] }',
    "presets/slow one.json": '{ "name": "slow one", "scene": {} }\n',
  },
  images: { "mark.png": picture(300), "big.jpg": picture(SHARE_IMAGE_BYTES) },
};

describe("scene file names", () => {
  it("allows text files, presets under presets/, and pictures by their own rule", () => {
    expect(validFileName("main.glsl")).toBe(true);
    expect(validFileName("presets/slow one.json")).toBe(true);
    expect(validFileName("presets/../x.json")).toBe(false);
    expect(validFileName("presets/.hidden.json")).toBe(false);
    expect(validFileName("lib/noise.glsl")).toBe(false);
    expect(validFileName("mark.png")).toBe(false);
    expect(validImageName("mark.png")).toBe(true);
    expect(validImageName("photo.JPEG")).toBe(true);
    expect(validImageName("../mark.png")).toBe(false);
    expect(validImageName("clip.gif")).toBe(false);
  });

  it("keeps the app's preset-name rule", () => {
    expect(validPresetName("slow one")).toBe(true);
    expect(validPresetName("Ölçü-2_b.c")).toBe(true);
    expect(validPresetName("")).toBe(false);
    expect(validPresetName(" padded")).toBe(false);
    expect(validPresetName(".dot")).toBe(false);
    expect(validPresetName("a/b")).toBe(false);
    expect(validPresetName("x".repeat(49))).toBe(false);
    expect(presetName(presetFile("slow one"))).toBe("slow one");
    expect(presetName("scene.json")).toBeUndefined();
  });

  it("names a picked picture after itself, made valid", () => {
    expect(imageNameFor("C:\\fakepath\\my photo.png")).toBe("my-photo.png");
    expect(imageNameFor(".hidden.jpg")).toBe("hidden.jpg");
    expect(imageNameFor("notes.txt")).toBeUndefined();
  });

  it("lists presets after the scene's own files", () => {
    expect(fileOrder(SCENE.files)).toEqual([
      "main.glsl",
      "scene.json",
      "helpers.glsl",
      "presets/slow one.json",
    ]);
  });
});

describe("pictures", () => {
  it("round-trip through base64 to the bytes the engine is given", () => {
    const bytes = Uint8Array.from({ length: 70_000 }, (_, i) => (i * 7) % 256);
    expect(base64ToBytes(bytesToBase64(bytes))).toEqual(bytes);
    expect(imageBytes(SCENE)["mark.png"]).toEqual(base64ToBytes(picture(300)));
  });

  it("go in a link only while small, smallest first, and say which were left out", () => {
    const { scene, dropped } = forSharing(SCENE);
    expect(Object.keys(scene.images ?? {})).toEqual(["mark.png"]);
    expect(dropped).toEqual(["big.jpg"]);
    expect(scene.files).toEqual(SCENE.files);
    const none = forSharing({ title: "t", files: { "main.glsl": "" } });
    expect(none.scene.images).toBeUndefined();
    expect(none.dropped).toEqual([]);
  });

  it("survive a share link, and a damaged one is dropped rather than trusted", async () => {
    const { scene } = forSharing(SCENE);
    expect(await decodeShare(`#${await encodeShare(scene)}`)).toEqual(scene);
    const parsed = parseScene({
      title: "x",
      files: { "main.glsl": "vec4 renderMain() { return vec4(1.0); }" },
      images: { "ok.png": "AAAA", "bad.png": "not base64!", "../up.png": "AAAA", "x.gif": "AAAA" },
    });
    expect(parsed?.images).toEqual({ "ok.png": "AAAA" });
  });
});

describe("the .synScene zip", () => {
  it("keeps every file, preset and picture, whatever their size", async () => {
    const zip = writeZip(SCENE);
    const back = await readZip(await zip.arrayBuffer());
    expect(back.title).toBe("with-pictures");
    expect(back.files).toEqual(SCENE.files);
    expect(back.images).toEqual(SCENE.images);
  });

  it("reads a zip with no pictures as before", async () => {
    const plain = { title: "plain", files: { "main.glsl": "vec4 renderMain() { return vec4(1.0); }" } };
    const back = await readZip(await writeZip(plain).arrayBuffer());
    expect(back).toEqual(plain);
  });
});
