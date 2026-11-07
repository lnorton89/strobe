/*
 * What a picture measures (D560): the classical half of /play's From image.
 *
 * Plain arithmetic on an RGBA array, so it runs in the page, in a test and in
 * a build script alike, and a picture never leaves the machine for it. Each
 * measure is here because it held up on real paintings and photographs:
 * colour after a levels stretch, a continuous mirror score, polar periodicity,
 * tone, orientation and density. Layout and element counts did not hold up,
 * so `guessLayout` is only a first suggestion a person can overrule with one
 * click, and nothing here claims to know what the picture is of.
 */

export type Pixels = {
  width: number;
  height: number;
  /** RGBA, a byte per channel, rows from the top. */
  data: Uint8ClampedArray | Uint8Array;
};

export type Rgb = [number, number, number];
export type Lab = [number, number, number];

export type Swatch = {
  /** sRGB, 0 to 1. */
  rgb: Rgb;
  lab: Lab;
  /** The share of the picture nearest this colour. */
  weight: number;
};

export type Measures = {
  /** Most of the picture first. */
  palette: Swatch[];
  /** Small saturated colours the clusters would average away. */
  accents: Swatch[];
  /** Oklab lightness: mean and the 5th and 95th percentiles. */
  lightness: { mean: number; low: number; high: number };
  /** Hasler and Suesstrunk's colourfulness, on 0 to 1. */
  colourfulness: number;
  /** Left against right, 0 to 1. Above about 0.6 it reads as mirrored. */
  mirror: number;
  /** How strongly the picture repeats around its centre, and how many times. */
  radial: { score: number; order: number };
  /** How strongly the picture repeats on a lattice, and its cells. */
  lattice: { score: number; columns: number; rows: number };
  /** Dominant edge direction in radians, and how much edges agree on it. */
  orientation: { angle: number; strength: number };
  /** The share of pixels on an edge. */
  density: number;
  /** Mean of the spectrum: 0 is all broad shapes, 1 all fine detail. */
  fineness: number;
  /** How much the top of the picture differs from the bottom, 0 to 1. */
  horizon: number;
  /** Width over height of what was measured. */
  aspect: number;
  /** A small stable number for this picture, so a re-run gives the same result. */
  seed: number;
};

/* ---- colour ------------------------------------------------------------ */

function toLinear(c: number): number {
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function toGamma(c: number): number {
  const v = Math.min(1, Math.max(0, c));
  return v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055;
}

/** sRGB to Oklab (Ottosson). */
export function rgbToLab([r, g, b]: Rgb): Lab {
  const lr = toLinear(r);
  const lg = toLinear(g);
  const lb = toLinear(b);
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

export function labToRgb([L, a, b]: Lab): Rgb {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    toGamma(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    toGamma(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    toGamma(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
}

export function chroma(lab: Lab): number {
  return Math.hypot(lab[1], lab[2]);
}

function labDistance(p: Lab, q: Lab): number {
  return (p[0] - q[0]) ** 2 + (p[1] - q[1]) ** 2 + (p[2] - q[2]) ** 2;
}

/* ---- resampling --------------------------------------------------------- */

/** An area-averaged copy no longer than `longest` on its long side. */
export function shrink(source: Pixels, longest: number): Pixels {
  const scale = Math.min(1, longest / Math.max(source.width, source.height));
  const width = Math.max(1, Math.round(source.width * scale));
  const height = Math.max(1, Math.round(source.height * scale));
  const data = new Uint8ClampedArray(width * height * 4);
  const sums = new Float64Array(width * height * 5);
  for (let y = 0; y < source.height; y += 1) {
    const ty = Math.min(height - 1, Math.floor((y * height) / source.height));
    for (let x = 0; x < source.width; x += 1) {
      const tx = Math.min(width - 1, Math.floor((x * width) / source.width));
      const from = (y * source.width + x) * 4;
      const to = (ty * width + tx) * 5;
      for (let c = 0; c < 4; c += 1) sums[to + c] = (sums[to + c] ?? 0) + (source.data[from + c] ?? 0);
      sums[to + 4] = (sums[to + 4] ?? 0) + 1;
    }
  }
  for (let i = 0; i < width * height; i += 1) {
    const count = sums[i * 5 + 4] || 1;
    for (let c = 0; c < 4; c += 1) data[i * 4 + c] = (sums[i * 5 + c] ?? 0) / count;
  }
  return { width, height, data };
}

/* ---- palette ------------------------------------------------------------ */

/**
 * The picture's colours with its levels stretched, since a photo of a
 * painting is lit grey: stretching beat changing the clusterer on real photos
 * (58 RGB units from the colours a person chose, against 84 without it). All
 * three channels share one stretch: a stretch for each takes out the light in
 * the room, and with it the colour of a picture that really is blue, whose
 * pale lavender came out cream.
 */
function stretchedLabs(image: Pixels): Lab[] {
  const count = image.width * image.height;
  const lows: number[] = [255, 255, 255];
  const highs: number[] = [0, 0, 0];
  const histogram = [new Uint32Array(256), new Uint32Array(256), new Uint32Array(256)];
  for (let i = 0; i < count; i += 1) {
    for (let c = 0; c < 3; c += 1) {
      const bins = histogram[c];
      const value = image.data[i * 4 + c] ?? 0;
      if (bins) bins[value] = (bins[value] ?? 0) + 1;
    }
  }
  for (let c = 0; c < 3; c += 1) {
    const bins = histogram[c] ?? new Uint32Array(256);
    let seen = 0;
    let low = 0;
    while (low < 255 && (seen += bins[low] ?? 0) < count * 0.01) low += 1;
    seen = 0;
    let high = 255;
    while (high > 0 && (seen += bins[high] ?? 0) < count * 0.01) high -= 1;
    lows[c] = low;
    highs[c] = Math.max(low + 1, high);
  }
  const low = Math.min(...lows);
  const high = Math.max(low + 1, ...highs);
  const labs: Lab[] = [];
  for (let i = 0; i < count; i += 1) {
    const rgb = [0, 1, 2].map((c) =>
      Math.min(1, Math.max(0, ((image.data[i * 4 + c] ?? 0) - low) / (high - low))),
    ) as Rgb;
    labs.push(rgbToLab(rgb));
  }
  return labs;
}

/** K-means in Oklab, seeded farthest-first so the same picture gives the same colours. */
function clusters(labs: readonly Lab[], k: number): Swatch[] {
  const first = labs[0];
  if (!first) return [];
  const mean: Lab = [0, 0, 0];
  for (const lab of labs) {
    mean[0] += lab[0] / labs.length;
    mean[1] += lab[1] / labs.length;
    mean[2] += lab[2] / labs.length;
  }
  const centres: Lab[] = [];
  let next = first;
  let far = -1;
  for (const lab of labs) {
    const d = labDistance(lab, mean);
    if (d > far) {
      far = d;
      next = lab;
    }
  }
  centres.push([...next]);
  while (centres.length < k) {
    far = -1;
    for (const lab of labs) {
      let nearest = Infinity;
      for (const centre of centres) nearest = Math.min(nearest, labDistance(lab, centre));
      if (nearest > far) {
        far = nearest;
        next = lab;
      }
    }
    if (far <= 1e-6) break;
    centres.push([...next]);
  }
  const counts = new Array<number>(centres.length).fill(0);
  for (let round = 0; round < 12; round += 1) {
    const sums = centres.map((): Lab => [0, 0, 0]);
    counts.fill(0);
    for (const lab of labs) {
      let best = 0;
      let nearest = Infinity;
      centres.forEach((centre, index) => {
        const d = labDistance(lab, centre);
        if (d < nearest) {
          nearest = d;
          best = index;
        }
      });
      const sum = sums[best];
      if (sum) {
        sum[0] += lab[0];
        sum[1] += lab[1];
        sum[2] += lab[2];
      }
      counts[best] = (counts[best] ?? 0) + 1;
    }
    centres.forEach((centre, index) => {
      const n = counts[index] ?? 0;
      const sum = sums[index];
      if (n > 0 && sum) {
        centre[0] = sum[0] / n;
        centre[1] = sum[1] / n;
        centre[2] = sum[2] / n;
      }
    });
  }
  return centres
    .map((lab, index) => ({
      lab,
      rgb: labToRgb(lab),
      weight: (counts[index] ?? 0) / labs.length,
    }))
    .filter((swatch) => swatch.weight > 0)
    .sort((a, b) => b.weight - a.weight);
}

/**
 * Peaks of the hue histogram over saturated pixels, for what k-means loses: a
 * red eye, a green line. Each is made vivid, because a small mark's measured
 * colour is mostly the paint under it mixed with the light in the room.
 */
function hueAccents(labs: readonly Lab[], palette: readonly Swatch[]): Swatch[] {
  const BINS = 36;
  const bins = new Float64Array(BINS);
  const sums = Array.from({ length: BINS }, () => ({ l: 0, c: 0, n: 0 }));
  for (const lab of labs) {
    const c = chroma(lab);
    if (c < 0.06) continue;
    const bin = Math.floor(((Math.atan2(lab[2], lab[1]) + Math.PI) / (2 * Math.PI)) * BINS) % BINS;
    bins[bin] = (bins[bin] ?? 0) + c;
    const sum = sums[bin];
    if (sum) {
      sum.l += lab[0];
      sum.c += c;
      sum.n += 1;
    }
  }
  const out: Swatch[] = [];
  for (let i = 0; i < BINS; i += 1) {
    const value = bins[i] ?? 0;
    const sum = sums[i];
    if (!sum || sum.n === 0) continue;
    if (value < (bins[(i + 1) % BINS] ?? 0) || value < (bins[(i + BINS - 1) % BINS] ?? 0)) continue;
    const angle = ((i + 0.5) / BINS) * 2 * Math.PI - Math.PI;
    const c = Math.min(0.2, (sum.c / sum.n) * 1.25);
    const lab: Lab = [Math.max(0.45, Math.min(0.85, sum.l / sum.n)), Math.cos(angle) * c, Math.sin(angle) * c];
    // Already a palette colour: nothing has been lost.
    const hueTaken = palette.some((swatch) => {
      if (chroma(swatch.lab) < 0.05) return false;
      const other = Math.atan2(swatch.lab[2], swatch.lab[1]);
      const gap = Math.abs(((angle - other + 3 * Math.PI) % (2 * Math.PI)) - Math.PI);
      return gap < 0.45 && swatch.weight > 0.03;
    });
    if (hueTaken) continue;
    out.push({ lab, rgb: labToRgb(lab), weight: sum.n / labs.length });
  }
  return out.sort((a, b) => b.weight - a.weight).slice(0, 2);
}

/* ---- structure ---------------------------------------------------------- */

/** Luminance as floats, row after row. */
function luminance(image: Pixels): Float32Array {
  const out = new Float32Array(image.width * image.height);
  for (let i = 0; i < out.length; i += 1) {
    out[i] =
      (0.2126 * (image.data[i * 4] ?? 0) +
        0.7152 * (image.data[i * 4 + 1] ?? 0) +
        0.0722 * (image.data[i * 4 + 2] ?? 0)) /
      255;
  }
  return out;
}

/** Pearson correlation, 0 for something with no variance. */
function correlation(a: readonly number[], b: readonly number[]): number {
  const n = Math.min(a.length, b.length);
  if (n < 2) return 0;
  let ma = 0;
  let mb = 0;
  for (let i = 0; i < n; i += 1) {
    ma += a[i] ?? 0;
    mb += b[i] ?? 0;
  }
  ma /= n;
  mb /= n;
  let num = 0;
  let da = 0;
  let db = 0;
  for (let i = 0; i < n; i += 1) {
    const x = (a[i] ?? 0) - ma;
    const y = (b[i] ?? 0) - mb;
    num += x * y;
    da += x * x;
    db += y * y;
  }
  return da > 1e-9 && db > 1e-9 ? num / Math.sqrt(da * db) : 0;
}

function mirrorScore(values: Float32Array, width: number, height: number): number {
  let best = 0;
  // The axis is searched near the middle: a photographed panel is rarely centred.
  const reach = Math.round(width * 0.06);
  for (let shift = -reach; shift <= reach; shift += 1) {
    const axis = (width - 1) / 2 + shift;
    const span = Math.floor(Math.min(axis, width - 1 - axis));
    if (span < width * 0.3) continue;
    const left: number[] = [];
    const right: number[] = [];
    for (let y = 0; y < height; y += 1) {
      for (let d = 1; d <= span; d += 1) {
        left.push(values[y * width + Math.round(axis - d)] ?? 0);
        right.push(values[y * width + Math.round(axis + d)] ?? 0);
      }
    }
    best = Math.max(best, correlation(left, right));
  }
  return Math.max(0, best);
}

/** A slow DFT's magnitude at `order` of `signal`, as a share of its energy. */
function harmonic(signal: readonly number[], order: number): number {
  let re = 0;
  let im = 0;
  let energy = 0;
  let mean = 0;
  for (const v of signal) mean += v / signal.length;
  signal.forEach((value, index) => {
    const phase = (2 * Math.PI * order * index) / signal.length;
    const v = value - mean;
    re += v * Math.cos(phase);
    im += v * Math.sin(phase);
    energy += v * v;
  });
  return energy > 1e-9 ? (2 * (re * re + im * im)) / (signal.length * energy) : 0;
}

function radialScore(
  values: Float32Array,
  width: number,
  height: number,
): { score: number; order: number } {
  const ANGLES = 180;
  const RINGS = 6;
  const cx = (width - 1) / 2;
  const cy = (height - 1) / 2;
  const reach = Math.min(cx, cy) * 0.92;
  const scores = new Map<number, number>();
  for (let ring = 1; ring <= RINGS; ring += 1) {
    const radius = (ring / RINGS) * reach;
    const signal: number[] = [];
    for (let a = 0; a < ANGLES; a += 1) {
      const angle = (a / ANGLES) * 2 * Math.PI;
      const x = Math.round(cx + Math.cos(angle) * radius);
      const y = Math.round(cy + Math.sin(angle) * radius);
      signal.push(values[y * width + x] ?? 0);
    }
    for (let order = 3; order <= 16; order += 1) {
      scores.set(order, (scores.get(order) ?? 0) + harmonic(signal, order) / RINGS);
    }
  }
  let score = 0;
  let order = 6;
  for (const [candidate, value] of scores) {
    if (value > score) {
      score = value;
      order = candidate;
    }
  }
  return { score: Math.min(1, score * 2.5), order };
}

/** The strongest autocorrelation peak along each axis of the edge picture. */
function latticeScore(
  values: Float32Array,
  width: number,
  height: number,
): { score: number; columns: number; rows: number } {
  const profile = (length: number, cross: number, at: (i: number, j: number) => number) => {
    const out: number[] = [];
    for (let i = 0; i < length; i += 1) {
      let sum = 0;
      for (let j = 0; j < cross; j += 1) sum += at(i, j);
      out.push(sum / cross);
    }
    return out;
  };
  const peak = (signal: number[]) => {
    let best = 0;
    let bestOrder = 1;
    for (let order = 2; order <= 12; order += 1) {
      const lag = Math.round(signal.length / order);
      if (lag < 3) break;
      const r = correlation(signal.slice(0, -lag), signal.slice(lag));
      if (r > best) {
        best = r;
        bestOrder = order;
      }
    }
    return { score: best, order: bestOrder };
  };
  const across = peak(profile(width, height, (x, y) => values[y * width + x] ?? 0));
  const down = peak(profile(height, width, (y, x) => values[y * width + x] ?? 0));
  return {
    score: Math.sqrt(Math.max(0, across.score) * Math.max(0, down.score)),
    columns: across.order,
    rows: down.order,
  };
}

/* ---- everything --------------------------------------------------------- */

/** Measures a picture of any size. Costs a few tens of milliseconds. */
export function analyse(source: Pixels): Measures {
  const colour = shrink(source, 96);
  const labs = stretchedLabs(colour);
  const palette = clusters(labs, 6);
  const accents = hueAccents(labs, palette);
  const lights = labs.map((lab) => lab[0]).sort((a, b) => a - b);
  const percentile = (p: number) => lights[Math.floor(p * (lights.length - 1))] ?? 0;
  const lightness = {
    mean: lights.reduce((sum, v) => sum + v, 0) / Math.max(1, lights.length),
    low: percentile(0.05),
    high: percentile(0.95),
  };

  // Hasler and Suesstrunk, on the bytes as published.
  let rgMean = 0;
  let ybMean = 0;
  let rgSq = 0;
  let ybSq = 0;
  const pixels = colour.width * colour.height;
  for (let i = 0; i < pixels; i += 1) {
    const r = colour.data[i * 4] ?? 0;
    const g = colour.data[i * 4 + 1] ?? 0;
    const b = colour.data[i * 4 + 2] ?? 0;
    const rg = r - g;
    const yb = 0.5 * (r + g) - b;
    rgMean += rg / pixels;
    ybMean += yb / pixels;
    rgSq += (rg * rg) / pixels;
    ybSq += (yb * yb) / pixels;
  }
  const hasler =
    Math.sqrt(Math.max(0, rgSq - rgMean * rgMean) + Math.max(0, ybSq - ybMean * ybMean)) +
    0.3 * Math.hypot(rgMean, ybMean);

  const structure = shrink(source, 128);
  const { width, height } = structure;
  const values = luminance(structure);
  const edges = new Float32Array(values.length);
  const orientations = new Float64Array(2);
  let edgeCount = 0;
  let edgeSum = 0;
  for (let y = 1; y < height - 1; y += 1) {
    for (let x = 1; x < width - 1; x += 1) {
      const i = y * width + x;
      const gx = (values[i + 1] ?? 0) - (values[i - 1] ?? 0);
      const gy = (values[i + width] ?? 0) - (values[i - width] ?? 0);
      const magnitude = Math.hypot(gx, gy);
      edges[i] = magnitude;
      edgeSum += magnitude;
      // Doubled angles, so opposite directions add rather than cancel.
      orientations[0] = (orientations[0] ?? 0) + gx * gx - gy * gy;
      orientations[1] = (orientations[1] ?? 0) + 2 * gx * gy;
      if (magnitude > 0.16) edgeCount += 1;
    }
  }
  let energy = 0;
  for (let i = 0; i < values.length; i += 1) energy += (edges[i] ?? 0) ** 2;
  const orientation = {
    // The direction of the edges is square to their gradient.
    angle: 0.5 * Math.atan2(orientations[1] ?? 0, orientations[0] ?? 0) + Math.PI / 2,
    strength: energy > 1e-9 ? Math.min(1, Math.hypot(orientations[0] ?? 0, orientations[1] ?? 0) / energy) : 0,
  };

  let top = 0;
  let bottom = 0;
  let spread = 0;
  let mean = 0;
  for (const v of values) mean += v / values.length;
  for (let i = 0; i < values.length; i += 1) {
    const v = values[i] ?? 0;
    spread += (v - mean) ** 2 / values.length;
    if (i < values.length / 3) top += (3 * v) / values.length;
    if (i >= (2 * values.length) / 3) bottom += (3 * v) / values.length;
  }

  // The fingerprint: the same picture, the same seed, the same candidates.
  let hash = 2166136261;
  for (let i = 0; i < values.length; i += 7) {
    hash = Math.imul(hash ^ Math.round((values[i] ?? 0) * 255), 16777619);
  }

  return {
    palette,
    accents,
    lightness,
    colourfulness: Math.min(1, hasler / 110),
    mirror: mirrorScore(values, width, height),
    radial: radialScore(values, width, height),
    lattice: latticeScore(edges, width, height),
    orientation,
    density: edgeCount / Math.max(1, (width - 2) * (height - 2)),
    fineness: Math.min(1, (edgeSum / Math.max(1, values.length)) / (Math.sqrt(spread) + 0.02) / 0.6),
    horizon: Math.min(1, Math.abs(top - bottom) / (Math.sqrt(spread) * 2 + 0.02)),
    aspect: source.width / Math.max(1, source.height),
    seed: (hash >>> 0) % 100_000,
  };
}
