import { describe, expect, it } from "vitest";
import {
  CARD_BASE64_CHARS,
  CARD_BYTES,
  CARD_HEIGHT as WORKER_HEIGHT,
  CARD_WIDTH as WORKER_WIDTH,
  type CardBucket,
  cardPage,
  hasCard,
  jpegSize,
  loadCard,
  readCard,
  storeCard,
} from "../../worker/share-card";
import { CARD_HEIGHT, CARD_MAX_BYTES, CARD_WIDTH, cardCrop } from "../../src/lib/play/card";

/*
 * A shared scene's card (D565): what the Worker accepts as one, that a scene
 * keeps its first, and how the /play page's social tags are rewritten. The
 * bucket is an in-memory fake with R2's head/get/put shape.
 */

function fakeBucket() {
  const objects = new Map<string, Uint8Array>();
  const bucket: CardBucket = {
    async head(key) {
      return objects.has(key) ? {} : null;
    },
    async get(key) {
      const bytes = objects.get(key);
      return bytes ? { arrayBuffer: async () => bytes.slice().buffer } : null;
    },
    async put(key, value) {
      objects.set(key, value);
    },
  };
  return { bucket, objects };
}

function base64(bytes: number[]): string {
  // One character at a time: a card-sized array is too long to spread.
  let text = "";
  for (const byte of bytes) text += String.fromCharCode(byte);
  return btoa(text);
}

/** A frame header (`marker` C0 is baseline, C2 progressive) for a `width` by `height` picture. */
function frameHeader(width: number, height: number, marker = 0xc0): number[] {
  return [0xff, marker, 0x00, 0x11, 8, height >> 8, height & 0xff, width >> 8, width & 0xff, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
}

/** A short APP0 segment, so the size is found by walking past one. */
const APP0 = [0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];

/**
 * The smallest thing the Worker reads as a card: SOI, an APP0 segment, a
 * frame header of `width` by `height`, filler and EOI, `length` bytes in all.
 */
function jpeg(length = 64, fill = 0, width = CARD_WIDTH, height = CARD_HEIGHT): number[] {
  const head = [0xff, 0xd8, ...APP0, ...frameHeader(width, height)];
  return [...head, ...Array.from({ length: length - head.length - 2 }, () => fill), 0xff, 0xd9];
}

// The tags as the site's build writes them for /play.
const PAGE = [
  "<title>Play · Strobe</title>",
  '<meta property="og:title" content="Write a Strobe scene in your browser."/>',
  '<meta property="og:url" content="https://strobe.example/play/"/>',
  '<meta property="og:image" content="https://strobe.example/play/og-image.jpg?v=ee2fe20c"/>',
  '<meta property="og:image:width" content="1200"/>',
  '<meta property="og:image:alt" content="Strobe&#x27;s browser sandbox."/>',
  '<meta name="twitter:card" content="summary_large_image"/>',
  '<meta name="twitter:title" content="Write a Strobe scene in your browser."/>',
  '<meta name="twitter:image" content="https://strobe.example/play/og-image.jpg?v=ee2fe20c"/>',
].join("");

describe("a shared scene's card", () => {
  it("reads a JPEG and drops anything else", () => {
    expect(readCard(base64(jpeg()))).toEqual(Uint8Array.from(jpeg()));
    expect(readCard(undefined)).toBeUndefined();
    expect(readCard("")).toBeUndefined();
    expect(readCard(42)).toBeUndefined();
    expect(readCard("not base64!")).toBeUndefined();
    // A PNG's first bytes, and a JPEG cut short.
    expect(readCard(base64([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))).toBeUndefined();
    expect(readCard(base64(jpeg().slice(0, 10)))).toBeUndefined();
  });

  it("reads the size from the frame header and drops any but the card's", () => {
    for (const [width, height] of [
      [1, 1],
      [CARD_HEIGHT, CARD_WIDTH],
      [CARD_WIDTH * 2, CARD_HEIGHT * 2],
      [CARD_WIDTH, CARD_HEIGHT + 1],
    ]) {
      expect(readCard(base64(jpeg(64, 0, width, height))), `${width}x${height}`).toBeUndefined();
    }
    const end = [0xff, 0xd9];
    // Fill bytes before a marker, a DHT segment (C4 is not a frame header)
    // and a progressive frame header are all read.
    const dht = [0xff, 0xc4, 0x00, 0x04, 0x00, 0x00];
    const fancy = [0xff, 0xd8, 0xff, 0xff, ...APP0.slice(1), ...dht, ...frameHeader(CARD_WIDTH, CARD_HEIGHT, 0xc2), ...end];
    expect(jpegSize(Uint8Array.from(fancy))).toEqual({ width: CARD_WIDTH, height: CARD_HEIGHT });
    expect(readCard(base64(fancy))).toEqual(Uint8Array.from(fancy));
    // A scan before any frame header, a segment that runs past the data, and
    // a frame header too short to hold a size have none.
    const scan = [0xff, 0xda, 0x00, 0x04, 0x00, 0x00];
    expect(jpegSize(Uint8Array.from([0xff, 0xd8, ...scan, ...frameHeader(CARD_WIDTH, CARD_HEIGHT), ...end]))).toBeUndefined();
    expect(jpegSize(Uint8Array.from([0xff, 0xd8, 0xff, 0xe0, 0x40, 0x00, ...end]))).toBeUndefined();
    expect(jpegSize(Uint8Array.from([0xff, 0xd8, 0xff, 0xc0, 0x00, 0x05, 8, 0x02, 0x76, ...end]))).toBeUndefined();
    expect(jpegSize(Uint8Array.from([0xff, 0xd8, ...end]))).toBeUndefined();
  });

  it("holds the page and the Worker to one size limit", () => {
    expect(CARD_MAX_BYTES).toBeLessThanOrEqual(CARD_BYTES);
    expect([WORKER_WIDTH, WORKER_HEIGHT]).toEqual([CARD_WIDTH, CARD_HEIGHT]);
    expect(readCard(base64(jpeg(CARD_BYTES)))?.length).toBe(CARD_BYTES);
    expect(base64(jpeg(CARD_BYTES)).length).toBeLessThanOrEqual(CARD_BASE64_CHARS);
    expect(readCard(base64(jpeg(CARD_BYTES + 3)))).toBeUndefined();
  });

  it("keeps the first card a scene gets", async () => {
    const { bucket, objects } = fakeBucket();
    expect(await hasCard(bucket, "abcdefghjk")).toBe(false);
    expect(await loadCard(bucket, "abcdefghjk")).toBeUndefined();
    expect(await storeCard(bucket, "abcdefghjk", Uint8Array.from(jpeg(64, 1)))).toBe(true);
    expect(await storeCard(bucket, "abcdefghjk", Uint8Array.from(jpeg(64, 2)))).toBe(false);
    expect([...objects.keys()]).toEqual(["cards/abcdefghjk.jpg"]);
    const kept = await loadCard(bucket, "abcdefghjk");
    expect(kept && new Uint8Array(kept)).toEqual(Uint8Array.from(jpeg(64, 1)));
  });

  it("names the scene in the page's social tags", () => {
    const page = cardPage(PAGE, {
      title: "Loaded Knife",
      url: "https://strobe.example/s/abcdefghjk",
      image: "https://strobe.example/api/card/abcdefghjk.jpg",
    });
    expect(page).toContain('<meta property="og:title" content="Loaded Knife · a Strobe scene"/>');
    expect(page).toContain('<meta name="twitter:title" content="Loaded Knife · a Strobe scene"/>');
    expect(page).toContain('<meta property="og:url" content="https://strobe.example/s/abcdefghjk"/>');
    expect(page).toContain('<meta property="og:image" content="https://strobe.example/api/card/abcdefghjk.jpg"/>');
    expect(page).toContain('<meta name="twitter:image" content="https://strobe.example/api/card/abcdefghjk.jpg"/>');
    expect(page).toContain(`content="A frame of Loaded Knife, a scene shared from Strobe's sandbox."`);
    // What it does not own stays as it was.
    expect(page).toContain("<title>Play · Strobe</title>");
    expect(page).toContain('<meta property="og:image:width" content="1200"/>');
    expect(page).toContain('<meta name="twitter:card" content="summary_large_image"/>');
  });

  it("keeps the page's own picture for a scene without a card", () => {
    const page = cardPage(PAGE, { title: "rings", url: "https://strobe.example/s/abcdefghjk" });
    expect(page).toContain('<meta property="og:title" content="rings · a Strobe scene"/>');
    expect(page).toContain('<meta property="og:image" content="https://strobe.example/play/og-image.jpg?v=ee2fe20c"/>');
    expect(page).toContain("Strobe&#x27;s browser sandbox.");
  });

  it("cannot be used to write markup into the page", () => {
    const page = cardPage(PAGE, { title: '"><script>alert(1)</script>', url: "https://strobe.example/s/abcdefghjk" });
    expect(page).not.toContain("<script>");
    expect(page).toContain("&quot;&gt;&lt;script&gt;");
  });

  it("leaves a page without the tags alone", () => {
    expect(cardPage("<p>plain</p>", { title: "x", url: "y", image: "z" })).toBe("<p>plain</p>");
  });

  it("crops any canvas to the card's shape, centred", () => {
    const shape = CARD_WIDTH / CARD_HEIGHT;
    const wide = cardCrop(2560, 720);
    expect(wide.h).toBe(720);
    expect(wide.w / wide.h).toBeCloseTo(shape, 6);
    expect(wide.x).toBeCloseTo((2560 - wide.w) / 2, 6);
    const tall = cardCrop(800, 1200);
    expect(tall.w).toBe(800);
    expect(tall.w / tall.h).toBeCloseTo(shape, 6);
    expect(tall.y).toBeCloseTo((1200 - tall.h) / 2, 6);
  });
});
