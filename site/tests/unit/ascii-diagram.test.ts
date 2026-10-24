import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isAsciiDiagram, parseFlow } from "@/lib/ascii-diagram";

/*
 * The renderer decides from the characters of a fenced block, never from its
 * text, so that an edit to docs/ARCHITECTURE.md changes what the figure says rather
 * than whether it appears. These cover both directions of that: what counts as
 * a diagram, and what the fallbacks are when one cannot be redrawn.
 */

const REPO = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "..",
);

const FLOW = `module_loader::load
      │
      ▼
resolver::resolve
      │
      ▼
codegen::compile_and_link`;

describe("isAsciiDiagram", () => {
  it("recognises a block drawn with box characters", () => {
    expect(isAsciiDiagram(FLOW)).toBe(true);
  });

  it("recognises a boxed figure it cannot redraw", () => {
    expect(isAsciiDiagram("┌────┐\n│ hi │\n└────┘")).toBe(true);
  });

  it("leaves a shell transcript alone", () => {
    expect(isAsciiDiagram("$ cargo build --release -p strobe-app\n    Finished `release` profile")).toBe(false);
  });

  it("leaves source alone, including source full of punctuation", () => {
    const source = `uniform float bass_kick;
vec4 renderMain() {
  return vec4(abs(uv.x - 0.5) > 0.2 || bass_kick > 0.5 ? 1.0 : 0.0);
}`;
    expect(isAsciiDiagram(source)).toBe(false);
  });
});

describe("parseFlow", () => {
  it("lifts the labels of a single-column flow, in order", () => {
    expect(parseFlow(FLOW)).toEqual([
      "module_loader::load",
      "resolver::resolve",
      "codegen::compile_and_link",
    ]);
  });

  it("reads an ASCII flow drawn without the Unicode set", () => {
    expect(parseFlow("lex\n |\n v\nparse")).toEqual(["lex", "parse"]);
  });

  it("declines a figure whose labels sit inside drawn boxes", () => {
    expect(parseFlow("┌──────┐\n│ lex  │\n└──────┘")).toBeNull();
  });

  it("declines two labels with no connector between them", () => {
    expect(parseFlow("lex\nparse")).toBeNull();
  });

  it("declines a figure with only one label", () => {
    expect(parseFlow("lex\n │\n ▼")).toBeNull();
  });
});

describe("docs/ARCHITECTURE.md, as it stands today", () => {
  /*
   * A canary, not a specification. The document draws its crate tree and its
   * dependency graph in box characters; neither is a single-column flow, so
   * both must reach the page as the author's own drawing rather than being
   * "redrawn" into boxes that lose their branches.
   */
  const document = readFileSync(path.join(REPO, "docs", "ARCHITECTURE.md"), "utf8");
  const blocks = [...document.matchAll(/```[\w-]*\n([\s\S]*?)```/g)].map(
    (match) => match[1],
  );

  it("still contains a block the renderer treats as a figure", () => {
    expect(blocks.some((block) => isAsciiDiagram(block))).toBe(true);
  });

  it("keeps a branching figure as drawn rather than flattening it", () => {
    const branching = blocks.filter(
      (block) => isAsciiDiagram(block) && /[├┬┴┼]/u.test(block),
    );
    expect(branching.length, "no branching figure found").toBeGreaterThan(0);
    for (const block of branching) {
      expect(parseFlow(block), "a branching figure was flattened").toBeNull();
    }
  });
});
