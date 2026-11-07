/*
 * What a generated scene must pass before it is offered (D560), measured on
 * its own rendered frames: not black, not blown out, not a flat fill, not
 * frozen, and no flashing. No beauty score: the published aesthetic measures
 * do not hold up, so a person chooses among what survives.
 *
 * The flash rule follows WCAG 2.3.1 and ITU-R BT.1702: a flash is a pair of
 * opposing changes in relative luminance of at least 0.10 with the darker
 * state under 0.80, and no more than three a second over a quarter of the
 * screen. This is a screen, not a certificate, and the page says so.
 */

/** RGBA frames of one size, evenly spaced in time. */
export type FrameSet = {
  width: number;
  height: number;
  /** Frames a second they were taken at. */
  rate: number;
  frames: (Uint8ClampedArray | Uint8Array)[];
};

export type GateResult = {
  passed: boolean;
  /** What a person is told about a rejection. */
  problems: string[];
  /** The most flashes a second any quarter of the picture had. */
  flashesPerSecond: number;
  mean: number;
};

function linear(byte: number): number {
  const c = byte / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/** Relative luminance per cell of a coarse grid, one array per frame. */
function cellLuminance(set: FrameSet, columns: number, rows: number): Float32Array[] {
  return set.frames.map((data) => {
    const sums = new Float32Array(columns * rows);
    const counts = new Uint32Array(columns * rows);
    for (let y = 0; y < set.height; y += 1) {
      const row = Math.min(rows - 1, Math.floor((y * rows) / set.height));
      for (let x = 0; x < set.width; x += 1) {
        const column = Math.min(columns - 1, Math.floor((x * columns) / set.width));
        const i = (y * set.width + x) * 4;
        const cell = row * columns + column;
        sums[cell] =
          (sums[cell] ?? 0) +
          0.2126 * linear(data[i] ?? 0) +
          0.7152 * linear(data[i + 1] ?? 0) +
          0.0722 * linear(data[i + 2] ?? 0);
        counts[cell] = (counts[cell] ?? 0) + 1;
      }
    }
    return sums.map((value, index) => value / Math.max(1, counts[index] ?? 1));
  });
}

/** Flashes in one cell's luminance over time: pairs of opposing large changes. */
export function countFlashes(values: readonly number[]): number {
  let transitions = 0;
  let direction = 0;
  let turn = values[0] ?? 0;
  const large = (a: number, b: number) => Math.abs(a - b) >= 0.1 && Math.min(a, b) < 0.8;
  for (const value of values) {
    if (direction === 0) {
      if (large(value, turn)) {
        direction = Math.sign(value - turn);
        transitions += 1;
        turn = value;
      }
    } else if ((value - turn) * direction > 0) {
      // Still going the same way: the turning point moves with it.
      turn = value;
    } else if (large(value, turn)) {
      direction = -direction;
      transitions += 1;
      turn = value;
    }
  }
  return Math.floor(transitions / 2);
}

export function gate(set: FrameSet): GateResult {
  const COLUMNS = 8;
  const ROWS = 6;
  const cells = cellLuminance(set, COLUMNS, ROWS);
  const problems: string[] = [];
  const seconds = Math.max(1 / set.rate, (set.frames.length - 1) / set.rate);

  // Per cell, the flashes it has; the 75th percentile is what a quarter of the
  // screen reaches or passes.
  const perCell: number[] = [];
  for (let cell = 0; cell < COLUMNS * ROWS; cell += 1) {
    perCell.push(countFlashes(cells.map((frame) => frame[cell] ?? 0)) / seconds);
  }
  perCell.sort((a, b) => a - b);
  const flashesPerSecond = perCell[Math.floor(perCell.length * 0.75)] ?? 0;
  if (flashesPerSecond > 3) {
    problems.push(`flashes ${flashesPerSecond.toFixed(1)} times a second over a quarter of the picture`);
  }

  let total = 0;
  let squared = 0;
  let blown = 0;
  let count = 0;
  for (const data of set.frames) {
    for (let i = 0; i < data.length; i += 4) {
      const v =
        (0.2126 * (data[i] ?? 0) + 0.7152 * (data[i + 1] ?? 0) + 0.0722 * (data[i + 2] ?? 0)) / 255;
      total += v;
      squared += v * v;
      if (v > 0.97) blown += 1;
      count += 1;
    }
  }
  const mean = total / Math.max(1, count);
  const spread = Math.sqrt(Math.max(0, squared / Math.max(1, count) - mean * mean));
  if (mean < 0.012) problems.push("almost black");
  if (spread < 0.012 && mean >= 0.012) problems.push("one flat colour");
  if (blown / Math.max(1, count) > 0.5) problems.push("blown out");

  // The most any frame differs from the first, since a loop can end where it began.
  let change = 0;
  const first = set.frames[0];
  if (first) {
    for (const data of set.frames) {
      let sum = 0;
      for (let i = 0; i < first.length; i += 4) sum += Math.abs((first[i + 1] ?? 0) - (data[i + 1] ?? 0)) / 255;
      change = Math.max(change, sum / (first.length / 4));
    }
    if (set.frames.length > 1 && change < 0.0008) problems.push("does not move");
  }

  return { passed: problems.length === 0, problems, flashesPerSecond, mean };
}
