import { describe, expect, it } from "vitest";
import { FRAME_BUDGET_MS, parseSweep, readSweep } from "../../src/lib/benchmarks";

/* The home page's frame times are read from docs/BENCHMARKS.md at build time. */

const TABLE = `
### 2099-01-02: render-scale sweep, ThinkPad T480 (Intel UHD 620), commit abc1234

Native:

| Scene | 1 (1280x720) | 0.75 (960x540) | 0.5 (640x360) | Saved |
|---|---:|---:|---:|---:|
| 01-one | 4.76 | 4.04 | 3.37 | 29% |
| 11-two * | 6.81 | 5.30 | 4.61 | 32% |

ANGLE:

| Scene | 1 (1280x720) | 0.75 (960x540) | 0.5 (640x360) | Saved |
|---|---:|---:|---:|---:|
| 01-one | 9.00 | 8.00 | 7.00 | 1% |
`;

describe("the T480 sweep", () => {
  it("reads the first table, and only it, with the footnote mark dropped", () => {
    expect(parseSweep(TABLE)).toEqual({
      date: "2099-01-02",
      commit: "abc1234",
      scenes: [
        { slug: "01-one", full: 4.76, half: 3.37 },
        { slug: "11-two", full: 6.81, half: 4.61 },
      ],
    });
  });

  it("fails rather than showing nothing when the section is gone", () => {
    expect(() => parseSweep("# Benchmarks\n")).toThrow(/no render-scale sweep/);
  });

  it("finds the real table, for the real library", () => {
    const { scenes } = readSweep();
    expect(scenes.length, "the sweep should cover the library").toBeGreaterThanOrEqual(17);
    expect(scenes.every((scene) => scene.half <= scene.full + 1), "a half-scale time looks wrong").toBe(
      true,
    );
    expect(FRAME_BUDGET_MS).toBeCloseTo(16.67, 2);
  });
});
