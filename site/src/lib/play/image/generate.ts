/*
 * From a picture's measures and a reading to candidate scenes (D560).
 *
 * The same picture and reading always give the same candidates: everything
 * varied comes from the picture's seed. A candidate is a text scene, a
 * main.glsl and a scene.json, so the editor, the controls panel, autosave, the
 * zip and sharing all treat it as any other scene. It holds no pixels of the
 * picture and no file name from it; only colours and a few numbers cross.
 */

import {
  type Lab,
  type Measures,
  type Rgb,
  type Swatch,
  chroma,
  labToRgb,
} from "@/lib/play/image/analyse";
import type { Reading } from "@/lib/play/image/reading";
import { CLOCK, FINISH, PAINT } from "@/lib/play/image/shared-glsl";
import { type Control, type Knobs, TEMPLATES, type Template } from "@/lib/play/image/templates";
import type { Scene } from "@/lib/play/files";

/** Four colours with jobs. */
export type Colours = { ground: Rgb; ink: Rgb; second: Rgb; accent: Rgb };

export type Candidate = {
  id: string;
  template: string;
  title: string;
  /** Why this one, for the tile. */
  note: string;
  colours: Colours;
  scene: Scene;
};

function round3(values: readonly number[]): number[] {
  return values.map((value) => Math.round(value * 1000) / 1000);
}

function hueGap(a: Lab, b: Lab): number {
  const gap = Math.abs(Math.atan2(a[2], a[1]) - Math.atan2(b[2], b[1]));
  return Math.min(gap, 2 * Math.PI - gap);
}

/**
 * Colours to jobs. The ground is the darkest colour with real weight, since
 * this is projected in a dark room: a pale picture still gets a dark ground
 * unless a candidate asks for a light one, its own colours kept for the marks.
 * Whatever a colour is measured as, it is then pushed far enough from the
 * ground to read, because a photographed gold is a pale tan.
 */
export function assignColours(measures: Measures, light = false): Colours {
  const heavy = measures.palette.filter((swatch) => swatch.weight > 0.04);
  const pool: Swatch[] = heavy.length > 0 ? heavy : measures.palette;
  const fallback: Swatch = { lab: [0.2, 0, 0], rgb: [0.1, 0.1, 0.1], weight: 1 };
  const sorted = [...pool].sort((a, b) => a.lab[0] - b.lab[0]);
  const groundSwatch = (light ? sorted[sorted.length - 1] : sorted[0]) ?? fallback;

  const groundLab: Lab = light
    ? [Math.max(0.82, groundSwatch.lab[0]), groundSwatch.lab[1] * 0.6, groundSwatch.lab[2] * 0.6]
    : [Math.min(0.2, groundSwatch.lab[0]), groundSwatch.lab[1] * 0.8, groundSwatch.lab[2] * 0.8];

  // The further from the ground and the more colourful, the better an ink.
  const others = measures.palette.filter((swatch) => swatch !== groundSwatch);
  const inkScore = (swatch: Swatch) =>
    Math.abs(swatch.lab[0] - groundLab[0]) + chroma(swatch.lab) * 1.2 + swatch.weight * 0.3;
  const inks = [...others].sort((a, b) => inkScore(b) - inkScore(a));
  const inkSwatch = inks[0] ?? fallback;
  // The second colour is the next that differs in hue, or failing that in tone.
  const secondSwatch =
    inks.find(
      (swatch) =>
        swatch !== inkSwatch &&
        (hueGap(swatch.lab, inkSwatch.lab) > 0.5 || Math.abs(swatch.lab[0] - inkSwatch.lab[0]) > 0.2),
    ) ??
    inks[1] ??
    inkSwatch;

  const legible = (lab: Lab, away: number): Lab => {
    const toward = light ? Math.min(lab[0], groundLab[0] - away) : Math.max(lab[0], groundLab[0] + away);
    const c = chroma(lab);
    // A faint tint in a measured colour is usually the room; a real colour is
    // brought up to something that survives a projector.
    const boost = c > 0.035 ? Math.min(1.5, Math.max(1, 0.12 / c)) : 1;
    return [Math.min(0.96, Math.max(0.05, toward)), lab[1] * boost, lab[2] * boost];
  };
  const inkLab = legible(inkSwatch.lab, 0.5);
  const secondLab = legible(secondSwatch.lab, 0.28);

  // The accent is a small saturated colour if there is one; otherwise the most
  // colourful swatch, or for a grey picture a bright tone of the ink.
  const accentSource =
    measures.accents[0]?.lab ??
    [...others].sort((a, b) => chroma(b.lab) - chroma(a.lab)).find((swatch) => chroma(swatch.lab) > 0.05)?.lab;
  const accentLab: Lab = accentSource
    ? legible(accentSource, 0.42)
    : legible([Math.min(0.95, inkLab[0] + 0.1), inkLab[1], inkLab[2]], 0.5);

  return {
    ground: labToRgb(groundLab),
    ink: labToRgb(inkLab),
    second: labToRgb(secondLab),
    accent: labToRgb(accentLab),
  };
}

/** What the measures set in a template's controls. */
export function knobsFrom(measures: Measures): Knobs {
  return {
    density: Math.min(1, measures.density * 1.6),
    weight: Math.min(1, Math.max(0, 1 - measures.fineness)),
    wobble: Math.min(1, Math.max(0.1, 1 - measures.orientation.strength * 1.5)),
    order: Math.max(3, Math.min(16, measures.radial.order)),
    rows: Math.max(3, Math.min(10, Math.max(measures.lattice.rows, 4))),
    tilt: measures.orientation.angle,
  };
}

function colourControl(NAME: string, colour: Rgb, description: string): Control {
  return {
    NAME,
    TYPE: "color smooth",
    DEF_COLOR: round3(colour),
    UI_GROUP: "color",
    DESCRIPTION: description,
  };
}

/** A template and its numbers, as the files of a scene. */
export function emitScene(
  template: Template,
  measures: Measures,
  reading: Reading,
  colours: Colours,
  seed: number,
  shift: Partial<Knobs> = {},
): Scene {
  const knobs = { ...knobsFrom(measures), ...shift };
  const pace = { calm: 0.65, driving: 1, wild: 1.4 }[reading.mood];
  const react = { calm: 0.6, driving: 0.8, wild: 1.1 }[reading.mood];
  const lightGround = colours.ground[0] + colours.ground[1] + colours.ground[2] > 1.8;

  const controls: Control[] = [
    ...template.controls(knobs),
    {
      NAME: template.bang.name,
      TYPE: "bang counter smooth",
      PARAMS: 0.04,
      UI_GROUP: "motion",
      DESCRIPTION: template.bang.description,
    },
    {
      NAME: "speed",
      TYPE: "slider smooth",
      MIN: 0,
      MAX: 3,
      DEFAULT: pace,
      PARAMS: 0.03,
      UI_GROUP: "motion",
      DESCRIPTION: "Overall pace. At zero only the kicks move it.",
    },
    {
      NAME: "reactivity",
      TYPE: "slider smooth",
      MIN: 0,
      MAX: 2,
      DEFAULT: react,
      PARAMS: 0.03,
      UI_GROUP: "audio",
      DESCRIPTION: "How much the music moves it. At zero it runs on time alone.",
    },
    {
      NAME: "glow",
      TYPE: "slider smooth",
      MIN: 0,
      MAX: 2,
      DEFAULT: lightGround ? 0.25 : 1,
      PARAMS: 0.03,
      UI_GROUP: "look",
      DESCRIPTION: "Light around the marks. At zero they are flat paint.",
    },
    {
      NAME: "brightness",
      TYPE: "slider smooth",
      MIN: 0.4,
      MAX: 1.6,
      DEFAULT: 1,
      PARAMS: 0.03,
      UI_GROUP: "look",
      DESCRIPTION: "Overall brightness.",
    },
    colourControl("ground_color", colours.ground, "What the marks are painted on."),
    colourControl("ink_color", colours.ink, "The main marks."),
    colourControl("second_color", colours.second, "The second colour."),
    colourControl("accent_color", colours.accent, "The small bright touches."),
  ];

  const header = `// ${template.title}, made by Strobe's From image: its rules are the template's,
// and only its colours and a few numbers came from the picture.
// Layout ${reading.layout}, motif ${reading.motif}, mood ${reading.mood}. Everything is yours to edit.

const float SEED = ${(seed % 997).toFixed(1)};
const float TILT = ${knobs.tilt.toFixed(3)};
`;
  const main = [
    header,
    CLOCK,
    PAINT,
    template.glsl,
    FINISH.replace("BANG", `${template.bang.name} * 1.0`),
  ].join("\n");

  const title = reading.subject
    ? `${reading.subject.replace(/^\w/, (first) => first.toUpperCase())}, ${template.title.toLowerCase()}`
    : `${template.title} from image`;
  const json = {
    TITLE: title,
    CREDIT: "Made with Strobe's From image",
    DESCRIPTION: `${template.title}, ${reading.motif} and ${reading.mood}: its colours come from a picture. Generated, and free to edit.`,
    PASSES: [
      {
        TARGET: "clock",
        WIDTH: 1,
        HEIGHT: 1,
        FLOAT: true,
        DOUBLE_BUFFER: true,
        WRAP: "clamp",
        FILTER: "nearest",
        FULL_PRECISION: true,
      },
    ],
    CONTROLS: controls,
    TAGS: ["2d", "from-image", reading.motif, reading.mood],
  };
  return {
    title,
    files: { "main.glsl": main, "scene.json": `${JSON.stringify(json, null, 2)}\n` },
  };
}

/** How well a template answers a reading; the seed breaks ties differently per picture. */
function fit(template: Template, reading: Reading, seed: number): number {
  const layout = template.layouts.indexOf(reading.layout);
  const motif = template.motifs.indexOf(reading.motif);
  if (template.only && motif < 0) return -1;
  const jitter = (((seed * 31 + template.id.length * 977 + (template.id.codePointAt(0) ?? 0) * 131) % 100) / 100) * 0.4;
  return (layout === 0 ? 4 : layout > 0 ? 3 : 0) + (motif === 0 ? 2.2 : motif > 0 ? 1.4 : 0) + jitter;
}

/**
 * Up to `count` candidates, spread apart on purpose: first what the reading
 * asks for, in two colourings where it has few answers, then the best of the
 * layouts it did not ask for, so a wrong reading still leaves something right
 * on the page.
 */
export function candidates(measures: Measures, reading: Reading, count = 9): Candidate[] {
  const seed = measures.seed;
  const base = assignColours(measures);
  const swap = (colours: Colours): Colours => ({ ...colours, ink: colours.second, second: colours.ink });
  const lightPicture = measures.lightness.mean > 0.68;

  const ranked = [...TEMPLATES]
    .filter((template) => fit(template, reading, seed) >= 0)
    .sort((a, b) => fit(b, reading, seed) - fit(a, reading, seed));
  const asked = ranked.filter((template) => template.layouts.includes(reading.layout));
  const others = ranked.filter((template) => !template.layouts.includes(reading.layout));

  const out: Candidate[] = [];
  const add = (
    template: Template,
    colours: Colours,
    note: string,
    variant: number,
    shift: Partial<Knobs> = {},
  ) => {
    if (out.length >= count) return;
    out.push({
      id: `${template.id}-${variant}`,
      template: template.id,
      title: template.title,
      note,
      colours,
      scene: emitScene(template, measures, reading, colours, seed + variant * 101, shift),
    });
  };

  // A template appears once until every template that fits has, so a gallery
  // is never three of one thing. Other layouts get at least a third of it.
  asked.slice(0, Math.min(asked.length, count - Math.ceil(count / 3))).forEach((template, index) => {
    add(template, base, "Your layout", index);
  });
  const seen = new Set<string>();
  const reserve = () => out.length < count;
  for (const template of others) {
    const home = template.layouts[0] ?? "field";
    if (seen.has(home) || !reserve()) continue;
    seen.add(home);
    add(template, base, "Another reading", 7);
  }
  for (const template of others) {
    if (!seen.has(template.id) && !out.some((item) => item.template === template.id)) {
      add(template, base, "Another reading", 7);
    }
  }
  const lead = asked[0];
  if (lead) {
    add(lead, swap(base), "Your layout, colours swapped", 5, { density: Math.min(1, knobsFrom(measures).density + 0.25) });
    if (lightPicture) add(lead, assignColours(measures, true), "Your layout, on the picture's light ground", 6);
    else add(lead, base, "Your layout, sparser", 6, { density: Math.max(0, knobsFrom(measures).density - 0.3) });
  }
  for (const template of others) add(template, swap(base), "Another reading", 8);
  return out;
}
