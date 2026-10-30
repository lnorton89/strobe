import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { AA_NORMAL, contrastRatio } from "@/lib/contrast";
import { BRAND } from "@/lib/constants";

/*
 * Pins the palette against WCAG AA, in both themes.
 *
 * The brand guide fixes the colours; this file fixes which of them may carry
 * text on which surface, in both themes.
 */

/** Every `selector { … }` rule's custom properties, read from globals.css. */
function readRules(): { selector: string; props: Record<string, string> }[] {
  const css = readFileSync(
    path.join(process.cwd(), "src", "app", "globals.css"),
    "utf8",
  );
  // Comments can contain braces and hex values; strip them before parsing.
  const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");

  const rules: { selector: string; props: Record<string, string> }[] = [];
  for (const match of withoutComments.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selector = match[1].replace(/\s+/g, " ").trim();
    const props: Record<string, string> = {};
    for (const prop of match[2].matchAll(/--([\w-]+):\s*(#[0-9a-fA-F]{3,8});/g)) {
      props[prop[1]] = prop[2].toLowerCase();
    }
    if (Object.keys(props).length > 0) rules.push({ selector, props });
  }
  return rules;
}

const RULES = readRules();

/** Merges every rule whose selector satisfies `matches`, in file order. */
function tokensWhere(
  label: string,
  matches: (selector: string) => boolean,
): Record<string, string> {
  const merged: Record<string, string> = {};
  const found = RULES.filter((rule) => matches(rule.selector));
  if (found.length === 0) throw new Error(`no rule found for ${label}`);
  for (const rule of found) Object.assign(merged, rule.props);
  return merged;
}

/**
 * Code keeps its own ground in both themes, so its tokens live in one rule
 * listing all three root selectors. That rule is identified by content rather
 * than by selector text, which keeps this test working if the selector list is
 * reordered.
 */
const CODE_RULE = RULES.find((rule) => "code-ground" in rule.props);
if (!CODE_RULE) throw new Error("no rule declares --code-ground");
const SHARED = CODE_RULE.props;

/** Dark is the bare `:root` default plus the explicit dark override. */
const DARK = tokensWhere(
  "the dark theme",
  (selector) => selector === ":root" || selector === ':root[data-theme="dark"]',
);
const LIGHT = tokensWhere(
  "the light theme",
  (selector) =>
    selector === ':root[data-theme="light"]' ||
    // The media-query form carries the same values for a visitor who has made
    // no explicit choice, and must not be allowed to drift from it.
    selector === ':root:not([data-theme="dark"])',
);

const THEMES = { dark: DARK, light: LIGHT } as const;

describe.each(Object.entries(THEMES))("%s theme", (themeName, T) => {
  const SURFACES = {
    ground: T.ground,
    panel: T.panel,
    "panel-raised": T["panel-raised"],
  };

  it("declares every token the site paints with", () => {
    for (const name of [
      "ground",
      "panel",
      "panel-raised",
      "hairline",
      "hairline-strong",
      "text",
      "bright",
      "secondary",
      "label",
      "accent",
      "accent-deep",
      "accent-text",
      "on-accent",
    ]) {
      expect(T[name], `${themeName} is missing --${name}`).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  for (const token of ["text", "bright", "secondary", "label"]) {
    for (const [surfaceName, surface] of Object.entries(SURFACES)) {
      it(`--${token} clears AA on ${surfaceName}`, () => {
        const ratio = contrastRatio(T[token], surface);
        expect(
          ratio,
          `${themeName}: ${T[token]} on ${surface} is ${ratio.toFixed(2)}:1`,
        ).toBeGreaterThanOrEqual(AA_NORMAL);
      });
    }
  }

  it("--accent-text clears AA on every surface, including raised panels", () => {
    for (const [surfaceName, surface] of Object.entries(SURFACES)) {
      const ratio = contrastRatio(T["accent-text"], surface);
      expect(
        ratio,
        `${themeName}: accent-text on ${surfaceName} is ${ratio.toFixed(2)}:1`,
      ).toBeGreaterThanOrEqual(AA_NORMAL);
    }
  });

  it("--on-accent is legible on the accent fill", () => {
    // Buttons and the status badge set text directly on the accent.
    expect(contrastRatio(T["on-accent"], T.accent)).toBeGreaterThanOrEqual(AA_NORMAL);
  });

  it("keeps --label quieter than --secondary, preserving the grey hierarchy", () => {
    expect(contrastRatio(T.label, T.ground)).toBeLessThan(
      contrastRatio(T.secondary, T.ground),
    );
  });
});

describe("the brand's own values are unchanged", () => {
  it("dark uses the guide's palette (docs/branding.md section 3)", () => {
    expect(DARK.ground, "Void").toBe("#080b14");
    expect(DARK.panel, "Panel").toBe("#101725");
    expect(DARK.hairline, "Border").toBe("#253247");
    expect(DARK.text, "Text").toBe("#f3f6fa");
    expect(DARK.secondary, "Muted").toBe("#9aa8bc");
    expect(DARK.grey, "Slate").toBe("#94a3b8");
    expect(DARK.accent, "Cyan").toBe("#00e5ff");
  });

  it("keeps the gradient's four stops in both themes", () => {
    for (const theme of [DARK, LIGHT]) {
      expect(theme.cyan, "cyan").toBe("#00e5ff");
      expect(theme.blue, "blue").toBe("#3382f6");
      expect(theme.violet, "violet").toBe("#8a55f7");
      expect(theme.magenta, "magenta").toBe("#ec4b99");
    }
  });

  it("agrees with the table the social images paint from", () => {
    expect(BRAND.ground.toLowerCase(), "ground").toBe(DARK.ground);
    expect(BRAND.panel.toLowerCase(), "panel").toBe(DARK.panel);
    expect(BRAND.hairline.toLowerCase(), "hairline").toBe(DARK.hairline);
    expect(BRAND.text.toLowerCase(), "text").toBe(DARK.text);
    expect(BRAND.secondary.toLowerCase(), "secondary").toBe(DARK.secondary);
    expect(BRAND.label.toLowerCase(), "label").toBe(DARK.label);
    expect(BRAND.cyan.toLowerCase(), "cyan").toBe(DARK.cyan);
  });

  it("moves the light theme's accent off cyan, which cannot carry text on paper", () => {
    expect(
      contrastRatio(LIGHT.cyan, LIGHT.ground),
      "brand cyan on the light ground",
    ).toBeLessThan(AA_NORMAL);
    expect(LIGHT.accent, "the light accent").not.toBe(LIGHT.cyan);
  });
});

describe("the code and terminal surfaces", () => {
  // Code keeps its own ground in both themes, so these ratios hold everywhere.
  const surfaces = [SHARED["code-ground"], SHARED["code-terminal"]];

  it("keeps the code ground at Void", () => {
    expect(SHARED["code-ground"]).toBe("#080b14");
  });

  for (const token of [
    "syn-punctuation",
    "syn-comment",
    "term-prompt",
    "term-command",
    "term-flag",
    "term-output",
    "term-error",
    "term-gutter",
  ]) {
    it(`--${token} clears AA on both code surfaces`, () => {
      for (const surface of surfaces) {
        const ratio = contrastRatio(SHARED[token], surface);
        expect(
          ratio,
          `${token} ${SHARED[token]} on ${surface} is ${ratio.toFixed(2)}:1`,
        ).toBeGreaterThanOrEqual(AA_NORMAL);
      }
    });
  }

  it("keeps the command brighter than its flags, and the flags than the output", () => {
    const ground = SHARED["code-terminal"];
    const ladder = ["term-output", "term-flag", "term-command"];
    const ratios = ladder.map((token) => contrastRatio(SHARED[token], ground));
    for (let index = 1; index < ratios.length; index += 1) {
      expect(
        ratios[index],
        `${ladder[index]} must be brighter than ${ladder[index - 1]}`,
      ).toBeGreaterThan(ratios[index - 1]);
    }
  });
});
