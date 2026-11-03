import { readFileSync } from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { describe, expect, it } from "vitest";
import { workerMain } from "../../src/lib/play/script-worker";
import type { FromWorker, ToWorker } from "../../src/lib/play/script-protocol";

/*
 * The /play page's script host (D524), driven the way the page drives it.
 *
 * The page starts the worker from `(${workerMain})()`, so each test does
 * exactly that in a fresh VM context standing in for the worker's global
 * scope: a function that reached outside itself would fail here as it would
 * in the browser.
 */

function startWorker() {
  const sent: FromWorker[] = [];
  const context = vm.createContext({
    postMessage: (message: FromWorker) => sent.push(message),
  });
  context.self = context;
  vm.runInContext(`(${workerMain.toString()})();`, context);
  const post = (data: ToWorker) => {
    (context.onmessage as (event: { data: ToWorker }) => void)({ data });
    return sent.at(-1);
  };
  return { post };
}

const SCALARS = ["syn_BassLevel", "syn_Level"];

function frame(time: number, extra: Partial<Extract<ToWorker, { type: "frame" }>> = {}) {
  return {
    type: "frame" as const,
    dt: 1 / 60,
    time,
    frame: Math.round(time * 60),
    audio: [0.25, 0.5],
    controls: { glow: [0.4], tint: [1, 0, 0] },
    mouse: [],
    ...extra,
  };
}

const CONTROLS = [
  { name: "glow", dims: 1, group: "look" },
  { name: "tint", dims: 3, group: "look" },
];

describe("the script worker", () => {
  it("declares the uniforms setup and the first update create, and refuses later ones", () => {
    const { post } = startWorker();
    const loaded = post({
      type: "load",
      source: `
        function setup() { setUniform("pulse", 0); }
        function update(dt) {
          setUniform("pulse", syn_BassLevel * 2);
          if (TIME === 0) setUniform("spin", 1, 2);
          if (TIME > 0) setUniform("late", 1);
        }`,
      controls: CONTROLS,
      scalars: SCALARS,
    });
    expect(loaded?.type).toBe("loaded");
    if (loaded?.type !== "loaded") return;
    expect(loaded.uniforms).toEqual([
      ["pulse", 1],
      ["spin", 2],
    ]);
    const next = post(frame(1));
    expect(next?.type).toBe("frame");
    if (next?.type !== "frame") return;
    expect(next.uniforms).toEqual([0.5, 0, 0, 0, 1, 2, 0, 0]);
    expect(next.console.map((line) => line.text).join()).toContain("late");
  });

  it("reads controls and audio as globals and queues control writes", () => {
    const { post } = startWorker();
    post({
      type: "load",
      source: `
        function update() {
          setControl("glow", tint.r * 0.5);
          setControlNormalized("tint", [0, 1, 0]);
          randomizeGroup("look");
          console.log("level", syn_Level);
        }`,
      controls: CONTROLS,
      scalars: SCALARS,
    });
    const next = post(frame(1));
    if (next?.type !== "frame") throw new Error("no frame came back");
    expect(next.writes).toEqual([
      { op: "set", name: "glow", values: [0.5], normalized: false },
      { op: "set", name: "tint", values: [0, 1, 0], normalized: true },
      { op: "randomize", name: "glow" },
      { op: "randomize", name: "tint" },
    ]);
    expect(next.console).toEqual([{ level: "log", text: "level 0.5" }]);
  });

  it("fires edge events on transitions only, and while-events every frame", () => {
    const { post } = startWorker();
    post({
      type: "load",
      source: `
        var ups = 0, held = 0;
        onOffToOn("syn_BassLevel", "up");
        whileOn("syn_BassLevel", function () { held += 1; });
        function up() { ups += 1; }
        function update() { setUniform("counts", ups, held); }`,
      controls: CONTROLS,
      scalars: SCALARS,
    });
    const levels = [0.2, 0.8, 0.9, 0.1, 0.7];
    let last: FromWorker | undefined;
    for (const [i, bass] of levels.entries()) last = post(frame(i + 1, { audio: [bass, 0] }));
    if (last?.type !== "frame") throw new Error("no frame came back");
    expect(last.uniforms.slice(0, 2)).toEqual([2, 3]);
  });

  it("reports an exception with its line and disables the script after three faults", () => {
    const { post } = startWorker();
    post({
      type: "load",
      source: "function update() {\n  undefinedThing.x = 1;\n}",
      controls: CONTROLS,
      scalars: SCALARS,
    });
    const first = post(frame(1));
    if (first?.type !== "frame") throw new Error("no frame came back");
    expect(first.error).toMatch(/^script\.js/);
    post(frame(2));
    const third = post(frame(3));
    if (third?.type !== "frame") throw new Error("no frame came back");
    expect(third.console.map((line) => line.text).join()).toContain("disabled");
    const fourth = post(frame(4));
    if (fourth?.type !== "frame") throw new Error("no frame came back");
    expect(fourth.error).toBeUndefined();
  });

  it("is self-contained, since it runs from its own source", () => {
    const source = readFileSync(
      path.join(process.cwd(), "src", "lib", "play", "script-worker.ts"),
      "utf8",
    );
    const imports = source.match(/^import (?!type ).*$/gm) ?? [];
    expect(imports).toEqual([]);
  });
});
