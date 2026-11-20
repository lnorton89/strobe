/*
 * The picture a shared scene's link shows (D565): the sandbox's canvas as it
 * is when Share is pressed, cropped to a social card's shape.
 *
 * The engine keeps its drawing buffer (`preserveDrawingBuffer`), so the last
 * frame can be copied at any moment. Everything here may fail (no 2D
 * context, a tainted or empty canvas), and then there is simply no card: the
 * link falls back to the /play page's own.
 */

/** A social card's size, the one the site's other cards use. */
export const CARD_WIDTH = 1200;
export const CARD_HEIGHT = 630;
/** Must not exceed the Worker's `CARD_BYTES` (worker/share-card.ts). */
export const CARD_MAX_BYTES = 160 * 1024;

/** JPEG qualities to try, best first, until the card fits. */
const QUALITIES = [0.84, 0.72, 0.6, 0.45, 0.3];

/** Bytes a base64 string decodes to. */
function decodedBytes(base64: string): number {
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  return (base64.length / 4) * 3 - padding;
}

/**
 * The source rectangle of a `width` by `height` picture that fills a card:
 * the largest centred part with the card's shape.
 */
export function cardCrop(width: number, height: number): { x: number; y: number; w: number; h: number } {
  const shape = CARD_WIDTH / CARD_HEIGHT;
  if (width / height > shape) {
    const w = height * shape;
    return { x: (width - w) / 2, y: 0, w, h: height };
  }
  const h = width / shape;
  return { x: 0, y: (height - h) / 2, w: width, h };
}

/** `source`'s current frame as a card: base64 JPEG, or undefined if it cannot be made. */
export function cardFromCanvas(source: HTMLCanvasElement | null): string | undefined {
  if (!source || source.width < 16 || source.height < 16) return undefined;
  try {
    const card = document.createElement("canvas");
    card.width = CARD_WIDTH;
    card.height = CARD_HEIGHT;
    const context = card.getContext("2d");
    if (!context) return undefined;
    const { x, y, w, h } = cardCrop(source.width, source.height);
    context.imageSmoothingQuality = "high";
    context.drawImage(source, x, y, w, h, 0, 0, CARD_WIDTH, CARD_HEIGHT);
    for (const quality of QUALITIES) {
      const data = card.toDataURL("image/jpeg", quality);
      const prefix = "data:image/jpeg;base64,";
      if (!data.startsWith(prefix)) return undefined;
      const base64 = data.slice(prefix.length);
      if (decodedBytes(base64) <= CARD_MAX_BYTES) return base64;
    }
  } catch {
    // No card; the link still works.
  }
  return undefined;
}
