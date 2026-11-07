/*
 * The template library (D560): the taste of From image, written by hand and
 * shipped as data. A template is a picture in GLSL and the controls that
 * belong to it; what it draws comes from its own rules, never from the
 * picture's pixels. What the picture gives it is colour and a few measured
 * defaults, so its breadth is the ceiling on what the tool can make.
 *
 * Every template is original to this repository. The marks and layouts follow
 * the grammar the hand-made scenes share (clock, ink, five layouts), not any
 * one scene's drawing. New templates must compile in both dialects and pass
 * the gates in tests/e2e/play-from-image.spec.ts.
 */

import type { Layout, Motif } from "@/lib/play/image/reading";

export type Control = {
  NAME: string;
  TYPE: string;
  MIN?: number;
  MAX?: number;
  DEFAULT?: number;
  PARAMS?: number;
  LABELS?: string[];
  VALUES?: number[];
  DEF_COLOR?: number[];
  DESCRIPTION?: string;
  UI_GROUP?: string;
};

/** What the measures set, each on 0 to 1 unless it says otherwise. */
export type Knobs = {
  /** How much is drawn. */
  density: number;
  /** How heavy a stroke is. */
  weight: number;
  /** How unsteady a stroke is. */
  wobble: number;
  /** Repeats around a centre, 3 to 16. */
  order: number;
  /** Rows of a lattice, 3 to 12. */
  rows: number;
  /** A direction, radians. */
  tilt: number;
};

export type Template = {
  id: string;
  title: string;
  /** The layouts it answers; the first is its home. */
  layouts: Layout[];
  motifs: Motif[];
  /** Only for its motifs: it says something a picture has to have asked for. */
  only?: boolean;
  /** The verb on its one bang, and what pressing it does. */
  bang: { name: string; description: string };
  controls: (knobs: Knobs) => Control[];
  /** Defines `vec3 picture(vec2 p)`. */
  glsl: string;
};

function slider(
  NAME: string,
  MIN: number,
  MAX: number,
  DEFAULT: number,
  UI_GROUP: string,
  DESCRIPTION: string,
): Control {
  const value = Math.round(Math.min(MAX, Math.max(MIN, DEFAULT)) * 1000) / 1000;
  return { NAME, TYPE: "slider smooth", MIN, MAX, DEFAULT: value, PARAMS: 0.03, UI_GROUP, DESCRIPTION };
}

function counts(NAME: string, values: number[], near: number, DESCRIPTION: string): Control {
  let best = 0;
  values.forEach((value, index) => {
    if (Math.abs(value - near) < Math.abs((values[best] ?? 0) - near)) best = index;
  });
  return {
    NAME,
    TYPE: "dropdown",
    LABELS: values.map(String),
    VALUES: values,
    DEFAULT: best,
    UI_GROUP: "shape",
    DESCRIPTION,
  };
}

const lerp = (from: number, to: number, at: number) => from + (to - from) * Math.min(1, Math.max(0, at));

export const TEMPLATES: Template[] = [
  {
    id: "wheel",
    title: "Wheel",
    layouts: ["radial"],
    motifs: ["glyphs", "geometric", "cosmic"],
    bang: { name: "rewrite", description: "Writes every sign anew, band by band." },
    controls: (k) => [
      counts("rings", [3, 4, 5, 6, 7], lerp(3, 7, k.density), "Bands from the core to the rim; they keep going past it to fill the frame."),
      // Near-square cells want about 2.7 signs around per band out.
      counts("spokes", [8, 10, 12, 14, 16, 20, 24], lerp(3, 7, k.density) * 2.7, "Signs around each band."),
      slider("spin", 0, 3, 1, "motion", "How fast the bands turn against each other. At zero they hold still."),
      slider("write", 0, 2, 1, "motion", "How often signs rewrite themselves on the beat. At zero they stay as they are."),
      slider("weight", 0.3, 2, lerp(0.7, 1.5, k.weight), "shape", "Width of the strokes."),
      slider("wobble", 0, 1, k.wobble, "shape", "How unsteady the hand is. At zero every line is ruled."),
    ],
    glsl: `vec3 picture(vec2 p) {
    gCol = groundAt(p);
    float r = max(length(p), 1e-3);
    float a = atan(p.y, p.x);
    const float INNER = 0.055;
    const float RIM = 0.56;
    float span = log(RIM / INNER) / float(rings);
    float lr = log(r / INNER) / span;
    float band = floor(lr);
    float f = fract(lr) - 0.5;
    float dScale = r * span * 0.5;
    if (lr > 0.0) {
        float dir = mod(band, 2.0) * 2.0 - 1.0;
        float n = float(spokes);
        float ang = a / K_TAU + 0.5 + dir * gFlow * spin * 0.012 * (1.0 + 0.2 * band);
        float cell = floor(fract(ang) * n);
        float u = fract(fract(ang) * n) - 0.5;
        float cellId = cell + band * 37.0 + SEED;
        float g = gBeat * write * 0.22 + _hash12(vec2(cellId, 1.3));
        float gen = floor(g);
        float ph = fract(g);
        float wr = smoothstep(0.0, 0.1, ph) * smoothstep(1.0, 0.9, ph);
        vec2 q = vec2(u * K_TAU * r / max(n * r * span, 1e-3) * 2.0, f * 2.0) * 1.28;
        q += (vec2(vn(q * 2.5 + cellId), vn(q * 2.5 - cellId)) - 0.5) * 0.14 * wobble;
        float id = cellId + gen * 11.0;
        float d = sign4(q, id, 0.045 * weight) + (1.0 - wr) * 0.5;
        gHaloMask = smoothstep(0.5, 0.3, abs(f)) * smoothstep(0.5, 0.3, abs(u));
        float pick = _hash12(vec2(id, 8.1));
        vec3 c = pick < 0.14 ? accent_color : pick < 0.42 ? second_color : ink_color;
        float fade = mix(1.0, 0.5, smoothstep(RIM, RIM * 1.5, r));
        ink(d * dScale / 1.28, c * fade * (1.0 + 0.5 * gHigh * step(0.86, pick)), 0.8);
        gHaloMask = 1.0;
        ink((abs(abs(f) - 0.5) - 0.012 * weight) * r * span, second_color * 0.6 * fade, 0.25);
    }
    float petals = max(abs(cos(a * float(spokes) * 0.5 + gSway)), 0.2);
    ink(r - INNER * (0.55 + 0.45 * petals) * (1.0 + 0.2 * gBass), accent_color, 1.2);
    ink(abs(r - INNER) - 0.0016 * weight, ink_color, 0.6);
    return gCol;
}
`,
  },
  {
    id: "kaleido",
    title: "Kaleidoscope",
    layouts: ["radial", "field"],
    motifs: ["cosmic", "geometric", "fire"],
    bang: { name: "fold", description: "Turns the whole pattern on by a step." },
    controls: (k) => [
      counts("facets", [3, 4, 5, 6, 8, 10, 12], k.order, "How many times the pattern repeats around the centre."),
      slider("zoom", 0.6, 5, lerp(1.4, 3.2, k.density), "shape", "How close the pattern is."),
      slider("warp", 0, 3, lerp(0.6, 1.6, k.wobble), "shape", "How far the pattern folds in on itself. At zero it is plain cloud."),
      slider("lines", 0, 1, lerp(0.2, 0.8, k.weight), "shape", "Strength of the drawn contour lines."),
      slider("drift", 0, 2, 1, "motion", "How fast the pattern flows through the mirrors."),
    ],
    glsl: `vec3 picture(vec2 p) {
    float wedge = K_TAU / float(facets);
    float a = atan(p.y, p.x) + gBeat * 0.05;
    float r = length(p);
    a = abs(mod(a, wedge) - 0.5 * wedge);
    vec2 q = vec2(cos(a), sin(a)) * r * zoom;
    float t = gFlow * drift * 0.05;
    vec2 w = vec2(fbm4(q + vec2(t, SEED)), fbm4(q + vec2(SEED, -t) + 4.3));
    float f = fbm4(q + w * warp * 1.6 + vec2(0.0, t));
    // The bright colours are thin ridges, not fills: a flat bright area
    // reads as a cartoon and floods a dark room.
    vec3 c = mix(ground_color, second_color * 0.75, smoothstep(0.34, 0.72, f));
    c = mix(c, ink_color * 0.9, exp(-abs(f - 0.66) * 26.0));
    c = mix(c, accent_color, smoothstep(0.8, 0.95, f + 0.1 * length(w)) * 0.8);
    c *= 0.5 + 0.7 * smoothstep(0.2, 0.85, f);
    float band = abs(fract(f * 9.0 + gFlow * 0.02) - 0.5);
    c = mix(c, ink_color, lines * 0.45 * smoothstep(0.34, 0.5, f) * (1.0 - smoothstep(0.02, 0.07, band)));
    gHalo = accent_color * smoothstep(0.62, 0.9, f) * glow * (0.14 + 0.28 * gBass);
    return c;
}
`,
  },
  {
    id: "totem",
    title: "Totem",
    layouts: ["axial"],
    motifs: ["glyphs", "geometric", "organic"],
    bang: { name: "climb", description: "Sends a step of the stack upward." },
    controls: (k) => [
      counts("slots", [4, 5, 6, 7, 8, 10], lerp(4, 10, k.density), "Figures stacked up the axis."),
      counts("columns", [1, 3, 5, 7], lerp(3, 7, k.density), "Stacks across the frame; the outer ones are dimmer."),
      slider("sway", 0, 1, 0.5, "motion", "How far the stack leans with the mids. At zero it stands straight."),
      slider("rise", 0, 2, 1, "motion", "How fast the figures climb. At zero they stay put."),
      slider("weight", 0.4, 2, lerp(0.7, 1.5, k.weight), "shape", "Width of the outlines."),
    ],
    glsl: `float figure(vec2 q, float kind, float id) {
    q.x = abs(q.x);
    float w = 0.03 * weight;
    if (kind < 1.0) return min(abs(length(q) - 0.55) - w, length(q) - 0.14);
    if (kind < 2.0) return abs(sdDiamond(q, 0.72)) - w;
    if (kind < 3.0) {
        float d = abs(q.y - 0.4 + q.x * 0.8) - w;
        return min(d, abs(q.y + 0.1 + q.x * 0.8) - w) + max(q.x - 0.72, 0.0);
    }
    if (kind < 4.0) {
        float crown = abs(fract(q.x * 2.2 + 0.25) - 0.5) * 2.0;
        float top = q.y + 0.3 - (1.0 - crown) * 0.75;
        return abs(max(top, max(-q.y - 0.35, q.x - 0.72))) - w;
    }
    if (kind < 5.0) return abs(sdBox(q, vec2(0.62, 0.42))) - w;
    return sign4(q * 1.2, id, 0.05 * weight) / 1.2;
}

vec3 picture(vec2 p) {
    gCol = groundAt(p);
    float n = float(columns);
    float spacing = max(RENDERSIZE.x / RENDERSIZE.y / n, 0.34);
    float col = clamp(floor(p.x / spacing + 0.5), -floor(n * 0.5), floor(n * 0.5));
    float dim = 1.0 / (1.0 + 0.7 * abs(col));
    vec2 q = p - vec2(col * spacing, 0.0);
    q.x += sin(p.y * 3.0 + gSway * 1.2 + col) * 0.03 * sway;
    float count = float(slots);
    float size = 1.0 / count;
    float y = q.y + 0.5 + gBeat * rise * 0.04 + (col + 2.0) * 0.17;
    float slot = floor(y / size);
    float v = fract(y / size) - 0.5;
    float id = slot + abs(col) * 13.0 + SEED;
    float wave = sin(slot * 1.1 - gBeat * 1.6);
    float grow = 1.0 + 0.08 * gBass * wave;
    float span = min(spacing * 0.46, size * (0.8 + 0.6 * _hash12(vec2(id, 2.0))));
    vec2 local = vec2(q.x / span, v * size / (size * 0.46)) / grow;
    float kind = floor(_hash12(vec2(id, 5.5)) * 6.0);
    float d = figure(local, kind, id) * min(span, size * 0.46) * grow;
    float pick = _hash12(vec2(id, 9.0));
    vec3 c = pick < 0.2 ? accent_color : pick < 0.55 ? second_color : ink_color;
    gHaloMask = smoothstep(0.5, 0.34, abs(v)) * smoothstep(0.5, 0.36, abs(q.x) / spacing);
    ink(d, c * dim, dim);
    float rail = abs(abs(q.x) - span * 1.04);
    if (spacing > span * 2.3) ink(rail - 0.0012 * weight, second_color * 0.3 * dim, 0.0);
    return gCol;
}
`,
  },
  {
    id: "wall",
    title: "Sign wall",
    layouts: ["grid"],
    motifs: ["glyphs", "geometric", "city"],
    bang: { name: "rewrite", description: "Writes every sign anew, row by row." },
    controls: (k) => [
      counts("rows", [3, 4, 5, 6, 7, 8, 10], k.rows, "Rows of signs; columns follow the frame's shape."),
      slider("write", 0, 2, 1, "motion", "How often signs rewrite themselves on the beat. At zero they stay as they are."),
      slider("weight", 0.3, 2, lerp(0.7, 1.5, k.weight), "shape", "Width of the pen."),
      slider("wobble", 0, 1, k.wobble, "shape", "How unsteady the hand is. At zero every line is ruled."),
      slider("frames", 0, 1, 0.35, "shape", "Strength of the line around each tile."),
    ],
    glsl: `vec3 picture(vec2 p) {
    gCol = groundAt(p);
    float size = 1.0 / float(rows);
    vec2 g = p / size + 0.5 * vec2(RENDERSIZE.x / RENDERSIZE.y * float(rows), float(rows));
    vec2 cell = floor(g);
    vec2 f = fract(g) * 2.0 - 1.0;
    float cellId = cell.x * 17.0 + cell.y * 131.0 + SEED;
    float h = _hash12(vec2(cellId, 4.2));
    float sweep = cell.y * 0.07 + h * 0.5;
    float g2 = gBeat * write * 0.2 + sweep;
    float gen = floor(g2);
    float ph = fract(g2);
    float wr = smoothstep(0.0, 0.08, ph) * smoothstep(1.0, 0.94, ph);
    vec2 q = f * 1.22;
    q += (vec2(vn(q * 2.2 + cellId), vn(q * 2.2 - cellId)) - 0.5) * 0.16 * wobble;
    float id = cellId + gen * 7.0;
    float d = (sign4(q, id, 0.05 * weight) + (1.0 - wr) * 0.5) * size * 0.5 / 1.22;
    float pick = _hash12(vec2(id, 6.0));
    vec3 c = pick < 0.12 ? accent_color : pick < 0.36 ? second_color : ink_color;
    gHaloMask = smoothstep(1.0, 0.68, max(abs(f.x), abs(f.y)));
    ink(d, c * (1.0 + 0.6 * gHigh * step(0.9, pick)), 0.7);
    float edge = (abs(sdBox(f, vec2(0.93))) - 0.006) * size * 0.5;
    ink(edge, second_color * 0.5 * frames, 0.0);
    return gCol;
}
`,
  },
  {
    id: "serpent",
    title: "Serpent",
    layouts: ["path"],
    motifs: ["organic", "glyphs", "water"],
    bang: { name: "slither", description: "Sends a surge along the line." },
    controls: (k) => [
      counts("strands", [1, 2, 3, 4, 5], lerp(1, 4, k.density), "Lines winding across the frame."),
      slider("girth", 0.3, 2, lerp(0.7, 1.4, k.weight), "shape", "Thickness of each line."),
      slider("coil", 0.3, 3, lerp(0.8, 1.8, k.density), "shape", "How tightly it winds."),
      slider("scales", 0, 1, 0.7, "shape", "Strength of the marks along the body. At zero it is a plain line."),
      slider("slide", 0, 3, 1, "motion", "How fast it moves along its length. At zero it stays put."),
    ],
    glsl: `float spine(float x, float id) {
    return 0.26 * sin(x * 3.2 * coil + gSway * 0.7 + id * 2.1)
         + 0.12 * sin(x * 7.1 * coil - gSway * 0.5 + id * 5.3);
}

vec3 picture(vec2 p) {
    gCol = groundAt(p);
    float n = float(strands);
    for (float i = 0.0; i < n; i += 1.0) {
        float depth = n > 1.0 ? i / (n - 1.0) : 1.0;
        float y0 = n > 1.0 ? (depth - 0.5) * 0.5 : 0.0;
        float id = i + SEED;
        float h = y0 + spine(p.x, id);
        float e = 0.01;
        float slope = (y0 + spine(p.x + e, id) - h) / e;
        float stretch = sqrt(1.0 + slope * slope);
        float cross = (p.y - h) / stretch;
        float wide = 0.05 * girth * (0.55 + 0.45 * depth) * (1.0 + 0.16 * gBass);
        vec3 body = mix(second_color, ink_color, depth);
        ink(abs(cross) - wide, body, 0.7);
        float s = p.x * stretch * 22.0 / girth - gBeat * slide * 0.5 - gFlow * slide * 0.25;
        vec2 cellQ = vec2(fract(s) - 0.5, cross / wide);
        float rowId = floor(s) + id * 9.0;
        float kind = _hash12(vec2(rowId, 1.0));
        float mark = kind < 0.5 ? abs(cellQ.y) * 0.6 + abs(cellQ.x) * 1.2 - 0.34
                                : length(cellQ * vec2(1.2, 0.7)) - 0.28;
        mark = max(mark, abs(cross / wide) - 0.8);
        vec3 tone = _hash12(vec2(rowId, 3.0)) < 0.2 ? accent_color : ground_color;
        ink(mark * wide + (1.0 - scales) * wide, tone * (1.0 + 0.6 * gHigh), 0.0);
        ink(abs(abs(cross) - wide) - gPx * 0.8, ground_color * 0.6, 0.0);
    }
    return gCol;
}
`,
  },
  {
    id: "ridges",
    title: "Ridges",
    layouts: ["layers"],
    motifs: ["sky", "organic", "city"],
    bang: { name: "dawn", description: "Moves the sun on through its arc." },
    controls: (k) => [
      counts("ranges", [3, 4, 5, 6, 7], lerp(3, 7, k.density), "Ridges from far to near."),
      slider("rough", 0, 1, lerp(0.3, 0.8, k.wobble), "shape", "How jagged the ridges are. At zero they are smooth swells."),
      slider("drift", 0, 2, 1, "motion", "How fast the land slides past. At zero it holds still."),
      slider("sun", 0, 1.5, 0.8, "shape", "Size of the sun. At zero there is none."),
      slider("stars", 0, 1, 0.5, "shape", "How many stars show above the land."),
    ],
    glsl: `vec3 picture(vec2 p) {
    float sky = smoothstep(-0.5, 0.6, p.y);
    gCol = mix(second_color * 0.9, ground_color, sky);
    vec2 cell = floor(p * 90.0);
    float star = _hash12(cell + SEED);
    if (star > 1.0 - 0.05 * stars) {
        float tw = 0.5 + 0.5 * sin(gFlow * 2.0 + star * 90.0);
        ink(length(fract(p * 90.0) - 0.5) / 90.0 - 0.0012 - 0.0012 * gHigh, ink_color * tw, 0.3);
    }
    float arc = gBeat * 0.03;
    vec2 at = vec2(0.42 * sin(arc), 0.1 + 0.22 * cos(arc));
    float radius = 0.11 * sun * (1.0 + 0.1 * gBass);
    if (sun > 0.01) {
        gHalo = accent_color * exp(-max(length(p - at) - radius, 0.0) * 5.0) * glow * 0.35;
        ink(length(p - at) - radius, accent_color, 0.0);
    }
    float n = float(ranges);
    for (float i = 0.0; i < n; i += 1.0) {
        float near = (i + 1.0) / n;
        float x = p.x * (1.2 + near * 1.1) + gFlow * drift * 0.02 * (0.3 + near * near * 2.0) + i * 7.7 + SEED;
        float h = mix(vn(vec2(x * 1.4, i)), fbm4(vec2(x * 1.6, i * 3.1)), rough);
        h = -0.42 + near * -0.04 + (1.0 - near) * 0.42 + h * (0.22 + 0.14 * (1.0 - near));
        vec3 tone = mix(mix(second_color, ground_color, 0.35), ground_color * 0.3, near);
        ink((p.y - h) * 0.92, tone, 0.0);
        float rim = abs(p.y - h) - gPx * (0.8 + near);
        ink(rim, mix(ink_color, accent_color, 0.3) * (0.28 + 0.22 * (1.0 - near)), 0.25);
    }
    return gCol;
}
`,
  },
  {
    id: "marble",
    title: "Marble",
    layouts: ["field"],
    motifs: ["organic", "water", "cosmic"],
    bang: { name: "stir", description: "Turns the whole flow over." },
    controls: (k) => [
      slider("zoom", 0.5, 5, lerp(1.2, 3, k.density), "shape", "How close the pattern is."),
      slider("warp", 0, 3, lerp(0.8, 2, k.wobble), "shape", "How far the paint folds into itself. At zero it is plain cloud."),
      slider("veins", 0, 1, lerp(0.2, 0.7, k.weight), "shape", "Strength of the bright veins."),
      slider("drift", 0, 2, 1, "motion", "How fast the paint moves. At zero it holds still."),
    ],
    glsl: `vec3 picture(vec2 p) {
    vec2 q = turn(TILT + gBeat * 0.03) * p * zoom;
    float t = gFlow * drift * 0.035;
    vec2 a = vec2(fbm4(q + vec2(SEED, t)), fbm4(q + vec2(5.2, SEED - t)));
    vec2 b = vec2(fbm4(q + warp * a + vec2(1.7, 9.2 + t)), fbm4(q + warp * a + vec2(8.3, 2.8)));
    float f = fbm4(q + warp * b);
    vec3 c = mix(ground_color, second_color * 0.8, smoothstep(0.34, 0.72, f));
    c = mix(c, accent_color, smoothstep(0.66, 0.82, b.x) * 0.7);
    float vein = 1.0 - smoothstep(0.0, 0.045, abs(f - 0.5));
    c = mix(c, ink_color, vein * veins * (0.7 + 0.5 * gHigh));
    gHalo = ink_color * vein * veins * glow * (0.06 + 0.2 * gBass);
    return c * (0.5 + 0.8 * f);
}
`,
  },
  {
    id: "contour",
    title: "Contours",
    layouts: ["field", "layers"],
    motifs: ["sky", "organic", "water"],
    bang: { name: "shift", description: "Moves the height lines on a step." },
    controls: (k) => [
      slider("zoom", 0.5, 4, lerp(1, 2.6, k.density), "shape", "How close the map is."),
      counts("levels", [6, 8, 10, 14, 18, 24], lerp(6, 24, k.density), "Height lines from the lowest to the highest."),
      slider("weight", 0.4, 3, lerp(0.8, 1.8, k.weight), "shape", "Width of the lines."),
      slider("fill", 0, 1, 0.6, "shape", "How strongly each height is tinted. At zero it is lines alone."),
      slider("drift", 0, 2, 1, "motion", "How fast the land changes shape. At zero it holds still."),
    ],
    glsl: `float land(vec2 q) {
    return fbm4(q + vec2(SEED, gFlow * drift * 0.012));
}

vec3 picture(vec2 p) {
    vec2 q = p * zoom;
    float h = land(q);
    float e = 0.004;
    vec2 grad = vec2(land(q + vec2(e, 0.0)) - h, land(q + vec2(0.0, e)) - h) / e * zoom;
    float n = float(levels);
    float v = h * n + gBeat * 0.1;
    float band = abs(fract(v) - 0.5);
    float dist = (0.5 - band) / max(length(grad) * n, 0.2);
    vec3 tint = mix(ground_color, second_color, smoothstep(0.2, 0.8, h));
    gCol = mix(ground_color, tint, fill);
    gCol = mix(gCol, accent_color, fill * 0.65 * smoothstep(0.7, 0.8, h));
    float major = step(mod(floor(v + 0.5), 4.0), 0.5);
    float wide = gPx * weight * (0.7 + 0.8 * major);
    ink(dist - wide, mix(second_color, ink_color, 0.45 + 0.55 * major) * (1.0 + 0.4 * gHigh * major), 0.35 * major);
    return gCol;
}
`,
  },
  {
    id: "glass",
    title: "Stained glass",
    layouts: ["field", "grid"],
    motifs: ["geometric", "city", "fire"],
    bang: { name: "shatter", description: "Moves every pane on a step." },
    controls: (k) => [
      slider("panes", 2, 14, lerp(4, 11, k.density), "shape", "How many panes across the frame's height."),
      slider("lead", 0, 2, lerp(0.7, 1.4, k.weight), "shape", "Width of the lines between panes. At zero the panes touch."),
      slider("wander", 0, 1, 0.5, "motion", "How far the panes drift. At zero they hold still."),
      slider("sparkle", 0, 1, 0.6, "motion", "How much single panes catch the highs."),
    ],
    glsl: `vec3 picture(vec2 p) {
    vec2 q = p * panes + SEED;
    vec2 base = floor(q);
    vec2 f = fract(q);
    float near = 8.0;
    float second = 8.0;
    vec2 nearest = vec2(0.0);
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 cell = vec2(float(x), float(y));
            vec2 h = _hash22(base + cell);
            vec2 at = cell + 0.5 + 0.42 * sin(h * K_TAU + (gFlow * 0.2 + gBeat * 0.2) * wander) ;
            float d = length(at - f);
            if (d < near) {
                second = near;
                near = d;
                nearest = base + cell;
            } else if (d < second) {
                second = d;
            }
        }
    }
    float h = _hash12(nearest);
    float h2 = _hash12(nearest + 3.1);
    vec3 pane = h < 0.4 ? second_color : h < 0.55 ? ink_color : h < 0.68 ? accent_color : ground_color * 1.6;
    pane *= 0.3 + 0.65 * h2 * h2;
    float light = 0.75 + 0.35 * fbm4(p * 3.0 + vec2(gFlow * 0.04, 0.0));
    float twinkle = step(0.86, _hash12(nearest + 7.0)) * gHigh * sparkle;
    gCol = pane * light * (1.0 + 0.8 * twinkle) * (1.0 - 0.45 * smoothstep(0.1, 0.55, near));
    ink((second - near) * 0.5 / panes - 0.0018 * lead, ground_color * 0.25, 0.0);
    gHalo = pane * twinkle * glow * 0.4;
    return gCol;
}
`,
  },
  {
    id: "tide",
    title: "Tide",
    layouts: ["layers", "field"],
    motifs: ["water", "sky"],
    bang: { name: "swell", description: "Sends a swell through every band." },
    controls: (k) => [
      counts("bands", [5, 7, 9, 12, 16], lerp(5, 16, k.density), "Waves from the far ones to the near ones."),
      slider("height", 0, 2, 1, "shape", "How tall the waves are. At zero the bands lie flat."),
      slider("crest", 0, 2, lerp(0.7, 1.4, k.weight), "shape", "Width of the line along each wave's top."),
      slider("roll", 0, 3, 1, "motion", "How fast the waves roll. At zero they hold still."),
    ],
    glsl: `vec3 picture(vec2 p) {
    gCol = mix(ground_color, second_color * 0.5, smoothstep(0.5, -0.1, p.y) * 0.4);
    float n = float(bands);
    for (float i = 0.0; i < n; i += 1.0) {
        float near = (i + 1.0) / n;
        float base = 0.42 - near * 1.0;
        float t = gFlow * roll * (0.12 + 0.3 * near) + i * 2.4 + SEED;
        float x = p.x * (2.4 + 3.0 * (1.0 - near));
        float wave = sin(x + t) * 0.5 + sin(x * 2.3 - t * 1.3 + i) * 0.22 + (vn(vec2(x * 1.3 + t * 0.3, i)) - 0.5) * 0.7;
        float amp = 0.035 * height * (0.5 + near) * (1.0 + 0.5 * gBass * sin(i - gBeat));
        float y = base + wave * amp;
        vec3 tone = mix(second_color, ground_color * 0.55, near);
        tone = mix(tone, accent_color, 0.12 * step(0.75, _hash12(vec2(i, SEED))));
        ink((p.y - y) * 0.95, tone, 0.0);
        ink(abs(p.y - y) - gPx * (0.6 + 1.6 * near) * crest, mix(second_color, ink_color, 0.5 + 0.5 * near) * (1.0 + 0.5 * gHigh), 0.3);
    }
    return gCol;
}
`,
  },
  {
    id: "orbs",
    title: "Drift",
    layouts: ["field"],
    motifs: ["cosmic", "sky", "water"],
    bang: { name: "scatter", description: "Moves every light on a step." },
    controls: (k) => [
      slider("amount", 0.2, 1, lerp(0.4, 0.95, k.density), "shape", "How many lights show."),
      slider("size", 0.3, 2, lerp(0.7, 1.3, k.weight), "shape", "How big the lights are."),
      slider("ringed", 0, 1, 0.4, "shape", "How many lights are rings rather than discs."),
      slider("drift", 0, 2, 1, "motion", "How fast they float. At zero they hold still."),
    ],
    glsl: `vec3 picture(vec2 p) {
    gCol = groundAt(p);
    for (float layer = 0.0; layer < 4.0; layer += 1.0) {
        float scale = 3.0 + layer * 2.2;
        vec2 q = p * scale + vec2(layer * 3.7 + SEED, gFlow * drift * 0.05 * (1.0 + 0.5 * layer));
        q.x += sin(q.y * 0.6 + gSway + layer) * 0.2;
        vec2 cell = floor(q);
        float id = cell.x * 13.0 + cell.y * 57.0 + layer * 31.0 + floor(gBeat * 0.05 + layer) * 3.0;
        vec2 f = fract(q) - 0.5;
        if (_hash12(vec2(id, 1.0)) > amount) continue;
        vec2 at = (_hash22(vec2(id, 2.0)) - 0.5) * 0.45;
        float radius = (0.06 + 0.18 * _hash12(vec2(id, 3.0))) * size * (1.0 + 0.16 * gBass);
        float h = _hash12(vec2(id, 4.0));
        vec3 c = h < 0.2 ? accent_color : h < 0.55 ? second_color : ink_color;
        c *= 0.45 + 0.55 * (layer / 3.0);
        c *= 1.0 + 0.7 * gHigh * step(0.8, _hash12(vec2(id, 5.0)));
        float d = length(f - at) - radius;
        if (_hash12(vec2(id, 6.0)) < ringed) d = abs(d + 0.012) - 0.012;
        // A light's halo fades before its cell ends, or it shows as a square.
        gHaloMask = smoothstep(0.5, 0.3, max(abs(f.x), abs(f.y)));
        ink(d / scale, c, 0.5);
    }
    return gCol;
}
`,
  },
  {
    id: "rays",
    title: "Sunburst",
    layouts: ["radial"],
    motifs: ["fire", "sky", "cosmic"],
    bang: { name: "burst", description: "Moves the rays on a step." },
    controls: (k) => [
      counts("beams", [5, 7, 9, 12, 16, 24], k.order, "Rays around the centre."),
      slider("taper", 0, 1, lerp(0.2, 0.7, k.wobble), "shape", "How much each ray narrows as it leaves the centre."),
      slider("rings", 0, 1, lerp(0.2, 0.8, k.density), "shape", "Strength of the rings flowing out. At zero there are none."),
      slider("flow", 0, 2, 1, "motion", "How fast the rings flow out. They only ever go one way."),
    ],
    glsl: `vec3 picture(vec2 p) {
    float r = max(length(p), 1e-3);
    float a = atan(p.y, p.x);
    float n = float(beams);
    float t = gSway * 0.06 + gBeat * 0.04;
    float fan = abs(fract((a + t) / K_TAU * n) - 0.5) * 2.0;
    float id = floor((a + t) / K_TAU * n);
    float length2 = 0.5 + 0.5 * _hash12(vec2(mod(id, n), SEED));
    float edge = mix(0.75, 0.75 - 0.6 * taper, smoothstep(0.0, 0.9, r));
    float ray = smoothstep(edge + 0.02, edge - 0.02, fan);
    gCol = mix(ground_color, second_color * (0.55 + 0.45 * length2), ray * smoothstep(1.3, 0.05, r / (0.4 + length2)));
    float wave = log(r) * 3.0 - gFlow * flow * 0.35;
    float ringBand = abs(fract(wave) - 0.5);
    gCol = mix(gCol, ink_color * (0.6 + 0.4 * ray), rings * (1.0 - smoothstep(0.03, 0.09, ringBand)) * smoothstep(0.02, 0.2, r));
    float radius = 0.08 + 0.015 * gBass;
    gHalo = accent_color * exp(-max(r - radius, 0.0) * 6.0) * glow * (0.22 + 0.2 * gBass);
    ink(r - radius, accent_color, 0.0);
    ink(abs(r - radius * 1.5) - 0.002, ink_color, 0.3);
    return gCol;
}
`,
  },
  {
    id: "flame",
    title: "Flame",
    layouts: ["field"],
    motifs: ["fire"],
    only: true,
    bang: { name: "flare", description: "Throws the flames up a step." },
    controls: (k) => [
      slider("zoom", 0.6, 4, lerp(1.2, 2.6, k.density), "shape", "How close the flames are."),
      slider("reach", 0.2, 1.5, 0.9, "shape", "How high the flames climb."),
      slider("lick", 0, 2, lerp(0.6, 1.4, k.wobble), "shape", "How much the flames twist. At zero they rise straight."),
      slider("rate", 0, 3, 1, "motion", "How fast they rise. At zero they hold still."),
    ],
    glsl: `vec3 picture(vec2 p) {
    float t = gFlow * rate * 0.32 + gBeat * 0.25;
    vec2 q = vec2(p.x * zoom, p.y * zoom * 0.55 - t);
    q += lick * 0.5 * vec2(fbm4(q * 1.1 + SEED), fbm4(q * 1.1 + 7.1)) - 0.25 * lick;
    float f = fbm4(q * 2.0);
    float top = (p.y + 0.55) / reach * (0.9 - 0.1 * gBass);
    float heat = clamp(f * 1.5 - top * 1.05 + 0.1, 0.0, 1.0);
    vec3 c = mix(ground_color, second_color * 0.8, smoothstep(0.1, 0.42, heat));
    c = mix(c, accent_color, smoothstep(0.34, 0.86, heat));
    c = mix(c, ink_color, smoothstep(0.8, 1.0, heat) * 0.8);
    c *= 0.45 + 0.75 * fbm4(q * 5.0 + 3.0);
    gHalo = accent_color * smoothstep(0.25, 0.8, heat) * glow * (0.08 + 0.18 * gBass);
    return c;
}
`,
  },
];
