import { describe, expect, it } from "vitest";
import type { Scene } from "../../src/lib/play/files";
import {
  type GenerateProposal,
  type KeptControl,
  changedFiles,
  choicesFrom,
  effectLabel,
  failureMessage,
  groupRows,
  nextStepAt,
  nothingToOffer,
  progressFraction,
  progressLabel,
  reviewRows,
  sameSources,
  undoGenerate,
  usesIncludes,
} from "../../src/lib/play/generate";

/*
 * Generate controls in the sandbox (D558), the page's half: what it says
 * while it runs, how often it asks for a step, what a review sends to the
 * engine, and that an undo never loses what the person typed without asking.
 */

const control = (over: Partial<KeptControl>): KeptControl => ({
  candidate: 0,
  name: "zoom",
  group: "Shape",
  kind: "coordinate scale",
  default: [3],
  min: [0],
  max: [6],
  score: 0.1,
  context: "uv * 3.0",
  ...over,
});

const proposal = (kept: KeptControl[], candidates = kept.length): GenerateProposal => ({
  state: "done",
  scene: "bars",
  candidates,
  fidelity: 0,
  kept,
  suggestions: [],
});

const scene = (files: Record<string, string>): Scene => ({ title: "bars", files });

describe("progress", () => {
  it("names what is happening and counts candidates from one", () => {
    const at = (phase: "preparing" | "scoring" | "checking", done = 0, total = 0) =>
      ({ state: "running", phase, done, total }) as const;
    expect(progressLabel(at("preparing"))).toBe("Compiling the scene");
    expect(progressLabel(at("scoring", 2, 12))).toBe("Scoring 3 of 12");
    expect(progressLabel(at("scoring", 12, 12))).toBe("Scoring 12 of 12");
    expect(progressLabel(at("checking"))).toBe("Checking the result");
    expect(progressFraction(at("preparing"))).toBe(0);
    expect(progressFraction(at("scoring", 3, 12))).toBe(0.25);
    expect(progressFraction(at("scoring", 0, 0))).toBe(0);
    expect(progressFraction(at("checking"))).toBe(1);
  });

  it("leaves the page at least half the time, however slow a step is", () => {
    for (const cost of [0, 2, 16, 250, 4000]) {
      const started = 1000;
      const end = started + cost;
      const next = nextStepAt(started, cost);
      expect(next - end).toBeGreaterThanOrEqual(cost);
    }
  });
});

describe("the review", () => {
  it("starts with everything kept under the engine's names", () => {
    const rows = reviewRows(proposal([control({}), control({ candidate: 4, name: "tint" })]));
    expect(rows.map((row) => [row.keep, row.name, row.min, row.max])).toEqual([
      [true, "zoom", 0, 6],
      [true, "tint", 0, 6],
    ]);
  });

  it("sends the kept rows only, trimmed", () => {
    const rows = reviewRows(proposal([control({}), control({ candidate: 4, name: "tint" })]));
    rows[0] = { ...rows[0]!, name: " wave " };
    rows[1] = { ...rows[1]!, keep: false };
    expect(choicesFrom(rows)).toEqual([
      { candidate: 0, name: "wave", context: "uv * 3.0", min: 0, max: 6 },
    ]);
  });

  it("widens a slider's range to hold its default, so the picture stays as measured", () => {
    const rows = reviewRows(proposal([control({ default: [3] })]));
    rows[0] = { ...rows[0]!, min: 4, max: 5 };
    expect(choicesFrom(rows)[0]).toMatchObject({ min: 3, max: 5 });
    rows[0] = { ...rows[0]!, min: 1, max: 2 };
    expect(choicesFrom(rows)[0]).toMatchObject({ min: 1, max: 3 });
  });

  it("leaves a colour's range alone", () => {
    const colour = control({ default: [0.9, 0.2, 0.1], min: [0, 0, 0], max: [1, 1, 1] });
    expect(choicesFrom(reviewRows(proposal([colour])))[0]).toMatchObject({ min: 0, max: 1 });
  });

  it("groups as the app lists them, strongest first in each", () => {
    const rows = reviewRows(
      proposal([
        control({ candidate: 1, name: "a", group: "Colour", score: 0.5 }),
        control({ candidate: 2, name: "b", group: "Motion", score: 0.4 }),
        control({ candidate: 3, name: "c", group: "Colour", score: 0.3 }),
        control({ candidate: 4, name: "d", group: "Quality", score: 0.2 }),
        control({ candidate: 5, name: "e", group: "Shape", score: 0.1 }),
      ]),
    );
    const grouped = groupRows(rows);
    expect(grouped.map(([group]) => group)).toEqual(["Motion", "Shape", "Colour", "Quality"]);
    const colour = grouped.find(([group]) => group === "Colour")?.[1] ?? [];
    expect(colour.map(({ row, index }) => [row.name, index])).toEqual([
      ["a", 0],
      ["c", 2],
    ]);
  });

  it("prints an effect as a percentage of full scale", () => {
    expect(effectLabel(0.0824)).toBe("8.2%");
    expect(effectLabel(null)).toBe("not measured");
  });
});

describe("what changed", () => {
  it("counts the code, not the title or a preset", () => {
    const a = scene({ "main.glsl": "x", "presets/a.json": "{}" });
    expect(sameSources(a, { ...a, title: "other" })).toBe(true);
    expect(sameSources(a, scene({ "main.glsl": "x" }))).toBe(true);
    expect(sameSources(a, scene({ "main.glsl": "y" }))).toBe(false);
    expect(sameSources(a, scene({ "main.glsl": "x", "scene.json": "{}" }))).toBe(false);
  });

  it("names the files a generation changed", () => {
    const before = scene({ "main.glsl": "1.0", "helpers.glsl": "h" });
    const after = scene({ "main.glsl": "zoom", "helpers.glsl": "h", "scene.json": "{}" });
    expect(changedFiles(before, after)).toEqual(["main.glsl", "scene.json"]);
  });
});

describe("undo", () => {
  const before = scene({ "main.glsl": "1.0", "helpers.glsl": "h" });
  const after = scene({ "main.glsl": "zoom", "helpers.glsl": "h", "scene.json": "{}" });

  it("restores the files, removing one the generation added", () => {
    const { scene: restored, lost } = undoGenerate(after, before, after);
    expect(restored.files).toEqual(before.files);
    expect(lost).toEqual([]);
  });

  it("keeps what came later in files it did not touch", () => {
    const current = scene({
      ...after.files,
      "helpers.glsl": "typed since",
      "presets/quiet.json": "{}",
    });
    const { scene: restored, lost } = undoGenerate(current, before, after);
    expect(restored.files["helpers.glsl"]).toBe("typed since");
    expect(restored.files["presets/quiet.json"]).toBe("{}");
    expect(restored.files["main.glsl"]).toBe("1.0");
    expect(lost).toEqual([]);
  });

  it("names a changed file edited since, so the page can ask first", () => {
    const current = scene({ ...after.files, "main.glsl": "zoom // typed since" });
    expect(undoGenerate(current, before, after).lost).toEqual(["main.glsl"]);
  });

  it("keeps the title the person gave it since", () => {
    const current: Scene = { ...after, title: "renamed" };
    expect(undoGenerate(current, before, after).scene.title).toBe("renamed");
  });
});

describe("messages", () => {
  it("says why there is nothing to offer", () => {
    expect(nothingToOffer(proposal([], 0))).toContain("no number in this scene's code");
    expect(nothingToOffer(proposal([], 1))).toContain("1 number measured");
    expect(nothingToOffer(proposal([], 7))).toContain("7 numbers measured");
    expect(nothingToOffer(proposal([], 7))).toContain("unchanged");
  });

  it("names a compile failure, and a script's part in one", () => {
    const plain = scene({ "main.glsl": "x" });
    const error = "the original scene does not compile: main.glsl:2: nope";
    expect(failureMessage(error, plain)).toBe(`No controls generated: ${error}`);
    const scripted = scene({ "main.glsl": "x", "script.js": "" });
    expect(failureMessage(error, scripted)).toContain("The script does not run");
  });

  it("notices #include in code and nowhere else", () => {
    expect(usesIncludes(scene({ "main.glsl": '#include "a.glsl"\nvoid f();' }))).toBe(true);
    expect(usesIncludes(scene({ "main.glsl": '  # include "a.glsl"' }))).toBe(true);
    expect(usesIncludes(scene({ "main.glsl": '// #include "a.glsl"' }))).toBe(false);
    expect(usesIncludes(scene({ "main.glsl": "x", "script.js": '#include "a"' }))).toBe(false);
  });
});
