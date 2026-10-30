/*
 * A sandbox scene as files, and the three ways it leaves the page with no
 * server behind it (D524): a share link with the files compressed into the
 * URL's fragment, a .synScene zip, and the browser's own storage.
 *
 * The fragment never reaches a server, so a link carries a scene without
 * the site storing or seeing anything. Compression and decompression are the
 * browser's own streams (deflate-raw), as is reading a zip someone else made.
 */

export type SceneFiles = Record<string, string>;

export type Scene = { title: string; files: SceneFiles };

/** The files a new scene starts with. */
export const BLANK: Scene = {
  title: "untitled",
  files: {
    "main.glsl": `// A Strobe scene: renderMain() returns this pixel's colour.
// _uv is 0..1 across the picture, TIME is seconds, syn_BassLevel is the bass.

vec4 renderMain() {
    vec2 p = _uvc;
    float r = length(p);
    float ring = 0.5 + 0.5 * sin(18.0 * r - TIME * 2.0 - syn_BassLevel * 4.0);
    vec3 colour = mix(tint, vec3(1.0), ring * glow);
    return vec4(colour * smoothstep(1.2, 0.2, r), 1.0);
}
`,
    "scene.json": `{
  "TITLE": "Untitled",
  "CONTROLS": [
    { "NAME": "tint", "TYPE": "color", "DEFAULT": [0.0, 0.9, 1.0] },
    { "NAME": "glow", "TYPE": "slider", "MIN": 0, "MAX": 1, "DEFAULT": 0.4 }
  ]
}
`,
  },
};

/** The order files are shown in: the shader, its JSON, the script, the rest. */
export function fileOrder(files: SceneFiles): string[] {
  const rank = (name: string) =>
    name === "main.glsl" ? 0 : name === "scene.json" ? 1 : name === "script.js" ? 2 : 3;
  return Object.keys(files).sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
}

/** A file name a scene may hold: a plain name ending in a text type. */
export function validFileName(name: string): boolean {
  return /^[\w.-]+\.(glsl|frag|json|js)$/i.test(name) && !name.startsWith(".");
}

async function pump(bytes: Uint8Array, stream: CompressionStream | DecompressionStream) {
  const output = new Blob([bytes as BlobPart]).stream().pipeThrough(stream);
  return new Uint8Array(await new Response(output).arrayBuffer());
}

const deflate = (bytes: Uint8Array) => pump(bytes, new CompressionStream("deflate-raw"));
const inflate = (bytes: Uint8Array) => pump(bytes, new DecompressionStream("deflate-raw"));

function toBase64Url(bytes: Uint8Array): string {
  let text = "";
  for (const byte of bytes) text += String.fromCharCode(byte);
  return btoa(text).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

function fromBase64Url(text: string): Uint8Array {
  const binary = atob(text.replaceAll("-", "+").replaceAll("_", "/"));
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

const FRAGMENT_KEY = "s=";

/** The fragment (without '#') that carries `scene`. */
export async function encodeShare(scene: Scene): Promise<string> {
  const json = new TextEncoder().encode(JSON.stringify(scene));
  return FRAGMENT_KEY + toBase64Url(await deflate(json));
}

/** The scene a fragment carries, or undefined if it carries none. */
export async function decodeShare(fragment: string): Promise<Scene | undefined> {
  const text = fragment.replace(/^#/, "");
  if (!text.startsWith(FRAGMENT_KEY)) return undefined;
  const json = new TextDecoder().decode(await inflate(fromBase64Url(text.slice(FRAGMENT_KEY.length))));
  return parseScene(JSON.parse(json));
}

/*
 * Short links (D527): the site's Worker stores a shared scene as one file in
 * R2 named by its content's hash and answers with `/s/<id>`. Where there is
 * no Worker (`next dev`, a plain static host) the upload fails and Share
 * falls back to the fragment link above, which needs nothing.
 */

/** The scene id a short link's path carries (`/s/<id>`), if it is one. */
export function sharedIdFromPath(pathname: string): string | undefined {
  return /^\/s\/([2-9a-km-np-z]{10,16})\/?$/.exec(pathname)?.[1];
}

/** Uploads `scene` and returns its short link. Throws when the site cannot store it. */
export async function uploadShare(scene: Scene): Promise<string> {
  const response = await fetch("/api/share", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(scene),
  });
  const body = (await response.json().catch(() => ({}))) as { url?: unknown; error?: unknown };
  if (!response.ok || typeof body.url !== "string") {
    throw new Error(typeof body.error === "string" ? body.error : `the site answered ${response.status}`);
  }
  return body.url;
}

/** The scene a short link names. */
export async function fetchShared(id: string): Promise<Scene> {
  const response = await fetch(`/api/scene/${encodeURIComponent(id)}`);
  if (!response.ok) throw new Error(response.status === 404 ? "no such shared scene" : `the site answered ${response.status}`);
  const scene = parseScene(await response.json());
  if (!scene) throw new Error("the shared scene is damaged");
  return scene;
}

/** A scene from untrusted JSON: only string files with valid names survive. */
export function parseScene(value: unknown): Scene | undefined {
  if (value === null || typeof value !== "object") return undefined;
  const { title, files } = value as { title?: unknown; files?: unknown };
  if (files === null || typeof files !== "object") return undefined;
  const clean: SceneFiles = {};
  for (const [name, text] of Object.entries(files as Record<string, unknown>)) {
    if (validFileName(name) && typeof text === "string") clean[name] = text;
  }
  if (!clean["main.glsl"]) return undefined;
  return { title: typeof title === "string" && title ? title.slice(0, 80) : "untitled", files: clean };
}

/* ---- zip ---------------------------------------------------------------- */

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = (CRC_TABLE[(crc ^ byte) & 0xff] ?? 0) ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

/**
 * A .synScene zip: one folder named for the scene holding its files, stored
 * uncompressed (text this small gains little, and stored entries need no
 * compressor to write).
 */
export function writeZip(scene: Scene): Blob {
  const encoder = new TextEncoder();
  const folder = `${scene.title.replace(/[^\w.-]+/g, "-") || "untitled"}.synScene/`;
  const parts: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  for (const name of fileOrder(scene.files)) {
    const path = encoder.encode(folder + name);
    const data = encoder.encode(scene.files[name] ?? "");
    const crc = crc32(data);
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(4, 20, true);
    local.setUint16(6, 0x0800, true); // UTF-8 names
    local.setUint32(14, crc, true);
    local.setUint32(18, data.length, true);
    local.setUint32(22, data.length, true);
    local.setUint16(26, path.length, true);
    const entry = new DataView(new ArrayBuffer(46));
    entry.setUint32(0, 0x02014b50, true);
    entry.setUint16(4, 20, true);
    entry.setUint16(6, 20, true);
    entry.setUint16(8, 0x0800, true);
    entry.setUint32(16, crc, true);
    entry.setUint32(20, data.length, true);
    entry.setUint32(24, data.length, true);
    entry.setUint16(28, path.length, true);
    entry.setUint32(42, offset, true);
    parts.push(new Uint8Array(local.buffer), path, data);
    central.push(new Uint8Array(entry.buffer), path);
    offset += 30 + path.length + data.length;
  }
  const size = central.reduce((sum, part) => sum + part.length, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, central.length / 2, true);
  end.setUint16(10, central.length / 2, true);
  end.setUint32(12, size, true);
  end.setUint32(16, offset, true);
  return new Blob([...parts, ...central, new Uint8Array(end.buffer)] as BlobPart[], {
    type: "application/zip",
  });
}

/**
 * The scene in a zip: the folder holding main.glsl (at any depth, so a zip
 * of a folder of scenes opens its first scene), stored or deflated entries.
 */
export async function readZip(buffer: ArrayBuffer): Promise<Scene> {
  const view = new DataView(buffer);
  let end = -1;
  for (let i = buffer.byteLength - 22; i >= Math.max(0, buffer.byteLength - 65_557); i -= 1) {
    if (view.getUint32(i, true) === 0x06054b50) {
      end = i;
      break;
    }
  }
  if (end < 0) throw new Error("not a zip file");
  const count = view.getUint16(end + 10, true);
  let at = view.getUint32(end + 16, true);
  const decoder = new TextDecoder();
  const entries = new Map<string, Uint8Array>();
  for (let i = 0; i < count; i += 1) {
    if (view.getUint32(at, true) !== 0x02014b50) throw new Error("a damaged zip directory");
    const method = view.getUint16(at + 10, true);
    const compressed = view.getUint32(at + 20, true);
    const nameLength = view.getUint16(at + 28, true);
    const extra = view.getUint16(at + 30, true);
    const comment = view.getUint16(at + 32, true);
    const local = view.getUint32(at + 42, true);
    const name = decoder.decode(new Uint8Array(buffer, at + 46, nameLength));
    at += 46 + nameLength + extra + comment;
    if (name.endsWith("/") || compressed > 2_000_000) continue;
    const start =
      local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true);
    const raw = new Uint8Array(buffer, start, compressed);
    if (method === 0) entries.set(name, raw);
    else if (method === 8) entries.set(name, await inflate(raw));
  }
  const main = [...entries.keys()]
    .filter((name) => name.split("/").pop() === "main.glsl")
    .sort((a, b) => a.length - b.length)[0];
  if (main === undefined) throw new Error("the zip has no main.glsl");
  const folder = main.slice(0, -"main.glsl".length);
  const files: SceneFiles = {};
  for (const [name, bytes] of entries) {
    const rest = name.slice(folder.length);
    if (name.startsWith(folder) && validFileName(rest)) files[rest] = decoder.decode(bytes);
  }
  const title =
    folder
      .replace(/\/$/, "")
      .split("/")
      .pop()
      ?.replace(/\.synScene$/i, "") || "imported";
  return { title, files };
}
