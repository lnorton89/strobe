import { describe, expect, it } from "vitest";
import { parseTier, readScenes, summariseScene } from "@/lib/scenes";

describe("parseTier", () => {
  it("reads the tier a description states", () => {
    expect(parseTier("Benchmark tier 3: heavy ALU")).toBe(3);
  });

  it("has no tier for a description that names none", () => {
    expect(parseTier("Scripted tier: a feedback pass plus JavaScript.")).toBeUndefined();
  });
});

describe("summariseScene", () => {
  it("takes every fact from scene.json", () => {
    const summary = summariseScene(
      "05-domain-warp",
      {
        TITLE: "Domain Warp",
        DESCRIPTION: "Marbled smoke. Benchmark tier 3: heavy ALU.",
        CREDIT: "Strobe",
        TAGS: ["2d", "noise", 4],
        CONTROLS: [{}, {}, {}],
        PASSES: [{}],
        IMAGE_PATH: "thumbnail.jpg",
      },
      false,
    );
    expect(summary).toEqual({
      slug: "05-domain-warp",
      title: "Domain Warp",
      description: "Marbled smoke. Benchmark tier 3: heavy ALU.",
      credit: "Strobe",
      license: undefined,
      tags: ["2d", "noise"],
      controls: 3,
      passes: 1,
      hasScript: false,
      tier: 3,
      thumbnail: "/generated/scenes/05-domain-warp.webp",
    });
  });

  it("falls back to the folder name and leaves out a thumbnail it cannot serve", () => {
    const summary = summariseScene("99-bare", { IMAGE_PATH: "thumbnail.gif" }, true);
    expect(summary.title, "title").toBe("99-bare");
    expect(summary.thumbnail, "thumbnail").toBeUndefined();
    expect(summary.hasScript, "script").toBe(true);
  });
});

describe("the repository's scenes/ folder", () => {
  it("reads every scene, each with a title and a thumbnail", async () => {
    const scenes = await readScenes();
    expect(scenes.length, "no scenes read").toBeGreaterThan(0);
    for (const scene of scenes) {
      expect(scene.title.length, `${scene.slug} has no title`).toBeGreaterThan(0);
      expect(scene.thumbnail, `${scene.slug} has no thumbnail`).toBeDefined();
    }
  });

  it("finds the scripted scene's script.js", async () => {
    const scenes = await readScenes();
    expect(
      scenes.some((scene) => scene.hasScript),
      "no scene with a script.js",
    ).toBe(true);
  });
});
