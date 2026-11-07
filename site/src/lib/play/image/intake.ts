/*
 * Getting a picture into the tool (D560): decode it in the browser, straighten
 * the part a person marked, and turn results into pixels and JPEGs. Decoding by
 * the browser means no image decoders ship with the site, and a canvas drawn
 * anew carries no EXIF, so a photo's location and camera never reach a model.
 */

import type { Pixels } from "@/lib/play/image/analyse";

/** Corners of the part that matters, clockwise from top left, each 0 to 1. */
export type Corners = [[number, number], [number, number], [number, number], [number, number]];

export const WHOLE_PICTURE: Corners = [
  [0, 0],
  [1, 0],
  [1, 1],
  [0, 1],
];

export type Loaded = { bitmap: ImageBitmap; width: number; height: number };

const WORK_SIZE = 1024;

/** Decodes a file, or throws why it cannot. */
export async function loadPicture(file: Blob): Promise<Loaded> {
  const bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
  return { bitmap, width: bitmap.width, height: bitmap.height };
}

/** Solves for the projective map from the unit square to `corners`. */
export function squareToQuad(corners: Corners): number[] {
  const [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = corners;
  const dx1 = x1 - x2;
  const dx2 = x3 - x2;
  const dy1 = y1 - y2;
  const dy2 = y3 - y2;
  const sx = x0 - x1 + x2 - x3;
  const sy = y0 - y1 + y2 - y3;
  const under = dx1 * dy2 - dx2 * dy1 || 1e-9;
  const g = (sx * dy2 - dx2 * sy) / under;
  const h = (dx1 * sy - sx * dy1) / under;
  return [x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h];
}

/** Where (u, v) in the straightened picture is in the source. */
export function mapPoint(m: readonly number[], u: number, v: number): [number, number] {
  const [a = 1, b = 0, c = 0, d = 0, e = 1, f = 0, g = 0, h = 0] = m;
  const w = g * u + h * v + 1 || 1e-9;
  return [(a * u + b * v + c) / w, (d * u + e * v + f) / w];
}

function drawable(width: number, height: number): CanvasRenderingContext2D {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("this browser refused a 2D canvas");
  return context;
}

/** The marked part as a straight picture, no longer than the work size. */
export function straighten(loaded: Loaded, corners: Corners): CanvasRenderingContext2D {
  const scale = Math.min(1, WORK_SIZE / Math.max(loaded.width, loaded.height));
  const source = drawable(Math.round(loaded.width * scale), Math.round(loaded.height * scale));
  source.drawImage(loaded.bitmap, 0, 0, source.canvas.width, source.canvas.height);
  const whole = corners.every(([x, y], index) => {
    const [wx, wy] = WHOLE_PICTURE[index] ?? [0, 0];
    return Math.abs(x - wx) < 1e-3 && Math.abs(y - wy) < 1e-3;
  });
  if (whole) return source;

  const span = (a: [number, number], b: [number, number]) =>
    Math.hypot((a[0] - b[0]) * source.canvas.width, (a[1] - b[1]) * source.canvas.height);
  const width = Math.max(32, Math.round(Math.max(span(corners[0], corners[1]), span(corners[3], corners[2]))));
  const height = Math.max(32, Math.round(Math.max(span(corners[0], corners[3]), span(corners[1], corners[2]))));
  const from = source.getImageData(0, 0, source.canvas.width, source.canvas.height);
  const to = drawable(width, height);
  const out = to.createImageData(width, height);
  const m = squareToQuad(corners);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const [sx, sy] = mapPoint(m, (x + 0.5) / width, (y + 0.5) / height);
      const px = Math.min(from.width - 1, Math.max(0, Math.round(sx * from.width)));
      const py = Math.min(from.height - 1, Math.max(0, Math.round(sy * from.height)));
      const at = (py * from.width + px) * 4;
      const write = (y * width + x) * 4;
      out.data[write] = from.data[at] ?? 0;
      out.data[write + 1] = from.data[at + 1] ?? 0;
      out.data[write + 2] = from.data[at + 2] ?? 0;
      out.data[write + 3] = 255;
    }
  }
  to.putImageData(out, 0, 0);
  return to;
}

export function pixelsOf(context: CanvasRenderingContext2D): Pixels {
  const { width, height } = context.canvas;
  const { data } = context.getImageData(0, 0, width, height);
  return { width, height, data };
}

/** A JPEG no longer than `longest`, as base64 with no prefix. */
export function jpegOf(context: CanvasRenderingContext2D, longest: number, quality = 0.85): string {
  const { width, height } = context.canvas;
  const scale = Math.min(1, longest / Math.max(width, height));
  const small = drawable(Math.round(width * scale), Math.round(height * scale));
  small.drawImage(context.canvas, 0, 0, small.canvas.width, small.canvas.height);
  return small.canvas.toDataURL("image/jpeg", quality).replace(/^data:image\/jpeg;base64,/, "");
}

export function blobOf(context: CanvasRenderingContext2D): Promise<Blob> {
  return new Promise((resolve, reject) => {
    context.canvas.toBlob(
      (blob) => (blob ? resolve(blob) : reject(new Error("could not encode the picture"))),
      "image/jpeg",
      0.9,
    );
  });
}
