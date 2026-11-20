/*
 * A shared scene's card (D565): the picture a `/s/<id>` link shows when it
 * is pasted into a chat or a post.
 *
 * Only a browser can draw a scene, so the picture comes from the one that
 * shares it: Share sends a 1200x630 JPEG of the canvas beside the scene.
 * The Worker checks it is a JPEG of a sane size and writes it once, as
 * `cards/<id>.jpg`. It is not part of the scene, so it does not change the
 * scene's name; the first card a scene gets is the one it keeps. A link with
 * no card (shared before this, or posted without one) shows the `/play`
 * page's own card, as it always did.
 *
 * `cardPage` rewrites the `/play` page's social tags for one scene. It is a
 * pure function over the page's text, so it is tested without Cloudflare
 * (tests/unit/share-card.test.ts).
 */

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
 * The card an upload carries, as bytes; undefined when it carries none or
 * what it carries is not a JPEG of an acceptable size. A bad card is dropped
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
  return jpeg ? bytes : undefined;
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
