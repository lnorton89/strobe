/*
 * A shared scene's card (D565): the picture a `/s/<id>` link shows when it
 * is pasted into a chat or a post.
 *
 * Only a browser can draw a scene, so the picture comes from the one that
 * shares it: Share sends a 1200x630 JPEG of the canvas beside the scene.
 * The Worker checks it is a 1200x630 JPEG within the byte limit (its frame
 * header, not just its first and last bytes) and writes it once, as
 * `cards/<id>.jpg`. It is not part of the scene, so it does not change the
 * scene's name; the first card a scene gets is the one it keeps. A link with
 * no card (shared before this, or posted without one) shows the `/play`
 * page's own card, as it always did.
 *
 * `cardPage` rewrites the `/play` page's social tags for one scene. It is a
 * pure function over the page's text, so it is tested without Cloudflare
 * (tests/unit/share-card.test.ts).
 */

/** A card's size in pixels. Must equal `card.ts`'s, which a test checks. */
export const CARD_WIDTH = 1200;
export const CARD_HEIGHT = 630;
/** Largest card accepted, in bytes: a busy 1200x630 JPEG fits at a middling quality. */
export const CARD_BYTES = 160 * 1024;
/** The same limit as the base64 text the upload carries. */
export const CARD_BASE64_CHARS = Math.ceil(CARD_BYTES / 3) * 4;

/** The minimal R2 surface the cards use, so tests can pass a fake. */
export interface CardBucket {
  head(key: string): Promise<unknown | null>;
  get(key: string): Promise<{ arrayBuffer(): Promise<ArrayBuffer> } | null>;
  put(
    key: string,
    value: Uint8Array,
    options: { httpMetadata: { contentType: string } },
  ): Promise<unknown>;
}

const key = (id: string) => `cards/${id}.jpg`;

/**
 * A JPEG's size from its first frame header (SOF), or undefined when the
 * segments before it are malformed or a scan comes first. Only the markers
 * are walked; the picture itself is never decoded.
 */
export function jpegSize(bytes: Uint8Array): { width: number; height: number } | undefined {
  // C0-CF are the frame headers, except DHT (C4), JPG (C8) and DAC (CC).
  const isFrameHeader = (m: number) => m >= 0xc0 && m <= 0xcf && m !== 0xc4 && m !== 0xc8 && m !== 0xcc;
  let at = 2; // after SOI, FF D8
  while (at + 1 < bytes.length) {
    if (bytes[at] !== 0xff) return undefined;
    const marker = bytes[at + 1];
    if (marker === 0xff) {
      at += 1; // fill byte before a marker
      continue;
    }
    // Markers with no length: TEM and RST0-7.
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) {
      at += 2;
      continue;
    }
    // A scan or the end before any frame header: no size to read.
    if (marker === 0xda || marker === 0xd9) return undefined;
    if (at + 3 >= bytes.length) return undefined;
    const length = (bytes[at + 2] << 8) | bytes[at + 3];
    if (length < 2 || at + 2 + length > bytes.length) return undefined;
    if (isFrameHeader(marker)) {
      // Length, precision, height, width: the header needs eight bytes.
      if (length < 8) return undefined;
      const height = (bytes[at + 5] << 8) | bytes[at + 6];
      const width = (bytes[at + 7] << 8) | bytes[at + 8];
      return { width, height };
    }
    at += 2 + length;
  }
  return undefined;
}

/**
 * The card an upload carries, as bytes; undefined when it carries none or
 * what it carries is not a CARD_WIDTH x CARD_HEIGHT JPEG within CARD_BYTES.
 * A bad card is dropped
 * rather than refused: the scene is what was shared, and it still gets its
 * link.
 */
export function readCard(value: unknown): Uint8Array | undefined {
  if (typeof value !== "string" || value.length === 0 || value.length > CARD_BASE64_CHARS) {
    return undefined;
  }
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(value)) return undefined;
  let binary: string;
  try {
    binary = atob(value);
  } catch {
    return undefined;
  }
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
  // A JPEG opens FF D8 FF and closes FF D9.
  const jpeg =
    bytes.length > 4 &&
    bytes[0] === 0xff &&
    bytes[1] === 0xd8 &&
    bytes[2] === 0xff &&
    bytes[bytes.length - 2] === 0xff &&
    bytes[bytes.length - 1] === 0xd9;
  if (!jpeg) return undefined;
  const size = jpegSize(bytes);
  return size?.width === CARD_WIDTH && size.height === CARD_HEIGHT ? bytes : undefined;
}

/** Whether the scene `id` has a card. */
export async function hasCard(bucket: CardBucket, id: string): Promise<boolean> {
  return (await bucket.head(key(id))) !== null;
}

/** Keeps `card` for the scene `id` unless it already has one. True when it was written. */
export async function storeCard(bucket: CardBucket, id: string, card: Uint8Array): Promise<boolean> {
  if (await hasCard(bucket, id)) return false;
  await bucket.put(key(id), card, { httpMetadata: { contentType: "image/jpeg" } });
  return true;
}

/** The card's bytes, or undefined for a scene without one. */
export async function loadCard(bucket: CardBucket, id: string): Promise<ArrayBuffer | undefined> {
  const object = await bucket.get(key(id));
  return object ? object.arrayBuffer() : undefined;
}

function attribute(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

/** Sets the `content` of every `<meta property|name="key" content="...">` in `html`. */
function setMeta(html: string, name: string, content: string): string {
  const tag = new RegExp(`(<meta\\s+(?:property|name)="${name}"\\s+content=")[^"]*(")`, "g");
  return html.replace(tag, (_, open: string, close: string) => open + attribute(content) + close);
}

/**
 * The `/play` page as the page of one shared scene: its title and address in
 * the social tags, and its card as the picture when it has one (`image`, an
 * absolute URL). Tags the page does not have are left alone, so a changed
 * page degrades to the generic card and never breaks.
 */
export function cardPage(
  html: string,
  scene: { title: string; url: string; image?: string },
): string {
  const title = `${scene.title} · a Strobe scene`;
  let page = html;
  for (const name of ["og:title", "twitter:title"]) page = setMeta(page, name, title);
  page = setMeta(page, "og:url", scene.url);
  if (scene.image !== undefined) {
    for (const name of ["og:image", "twitter:image"]) page = setMeta(page, name, scene.image);
    const alt = `A frame of ${scene.title}, a scene shared from Strobe's sandbox.`;
    for (const name of ["og:image:alt", "twitter:image:alt"]) page = setMeta(page, name, alt);
  }
  return page;
}
