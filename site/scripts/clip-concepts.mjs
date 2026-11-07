/*
 * Writes src/lib/play/image/concepts.json (D560): CLIP's text vectors for the
 * words From image can use, worked out once here so the browser only needs the
 * vision half of the model.
 *
 *   node scripts/clip-concepts.mjs               regenerate the file
 *   node scripts/clip-concepts.mjs <picture>...  also print what it reads in pictures
 *
 * Needs the network once, to fetch the model. Run it when the words change and
 * commit the result; neither a build nor a page load does this.
 */

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import {
  AutoProcessor,
  AutoTokenizer,
  CLIPTextModelWithProjection,
  CLIPVisionModelWithProjection,
  RawImage,
} from "@huggingface/transformers";

const MODEL = "Xenova/clip-vit-base-patch32";
const OUT = path.join(process.cwd(), "src", "lib", "play", "image", "concepts.json");

/** Each word's sentences; a word's vector is their mean. */
const WORDS = {
  layout: {
    radial: ["a circular mandala", "a wheel with symbols arranged around its centre", "a radial design with rings"],
    axial: ["a totem pole", "a tall symmetrical figure", "a mirrored vertical stack of shapes"],
    grid: ["a grid of symbols", "rows and columns of repeated marks", "a wall of glyphs in rows"],
    path: ["a long winding snake", "one sinuous ribbon across the picture", "a single curving dragon"],
    layers: ["a landscape with a horizon", "mountains under the sky", "a seascape with layers of waves"],
    field: ["an abstract texture", "an all-over pattern with no subject", "swirling paint with no one shape"],
  },
  motif: {
    water: ["water and waves", "the ocean", "a river or a flood"],
    fire: ["flames and fire", "burning embers", "lava"],
    sky: ["clouds in the sky", "a sunset", "the sun and the moon over land"],
    cosmic: ["outer space with stars", "a galaxy", "a nebula"],
    organic: ["plants and leaves", "an animal or creature", "flowers and living forms"],
    city: ["a city skyline", "buildings and architecture", "a temple or gate"],
    glyphs: ["calligraphy and symbols", "ancient writing", "occult sigils and runes"],
    geometric: ["geometric shapes", "a hard-edged graphic design", "tiles and polygons"],
  },
  mood: {
    calm: ["a calm and peaceful picture", "a quiet, still scene"],
    driving: ["a bold and energetic picture", "a strong rhythmic design"],
    wild: ["a chaotic and intense picture", "an explosive, frantic scene"],
  },
};

function normalise(values) {
  const size = Math.hypot(...values) || 1;
  return values.map((value) => value / size);
}

/** A unit vector as base64 of signed bytes: small, and exact enough to rank with. */
function pack(vector) {
  return Buffer.from(Int8Array.from(vector.map((value) => Math.round(value * 127))).buffer).toString("base64");
}

function unpack(text) {
  const bytes = Buffer.from(text, "base64");
  return normalise(Array.from(new Int8Array(bytes.buffer, bytes.byteOffset, bytes.length)));
}

const tokenizer = await AutoTokenizer.from_pretrained(MODEL);
const textModel = await CLIPTextModelWithProjection.from_pretrained(MODEL, { dtype: "q8" });

const groups = {};
for (const [group, words] of Object.entries(WORDS)) {
  groups[group] = {};
  for (const [word, sentences] of Object.entries(words)) {
    const inputs = tokenizer(sentences, { padding: true, truncation: true });
    const { text_embeds: embeds } = await textModel(inputs);
    const dims = embeds.dims[1];
    const mean = new Array(dims).fill(0);
    for (let i = 0; i < sentences.length; i += 1) {
      const one = normalise(Array.from(embeds.data.slice(i * dims, (i + 1) * dims)));
      one.forEach((value, at) => {
        mean[at] += value / sentences.length;
      });
    }
    groups[group][word] = pack(normalise(mean));
  }
}
writeFileSync(OUT, `${JSON.stringify({ model: MODEL, groups }, null, 0)}\n`);
console.log(`wrote ${OUT}`);

const pictures = process.argv.slice(2);
if (pictures.length > 0) {
  const processor = await AutoProcessor.from_pretrained(MODEL);
  const vision = await CLIPVisionModelWithProjection.from_pretrained(MODEL, { dtype: "q8" });
  for (const picture of pictures) {
    const image = await RawImage.fromBlob(new Blob([readFileSync(picture)]));
    const started = Date.now();
    const { image_embeds: embeds } = await vision(await processor(image));
    const seen = normalise(Array.from(embeds.data));
    const line = Object.entries(groups).map(([group, words]) => {
      const scores = Object.entries(words)
        .map(([word, packed]) => [word, unpack(packed).reduce((sum, value, at) => sum + value * seen[at], 0)])
        .sort((a, b) => b[1] - a[1]);
      return `${group}: ${scores.slice(0, 2).map(([word, score]) => `${word} ${score.toFixed(3)}`).join(", ")}`;
    });
    console.log(path.basename(picture), `${Date.now() - started} ms`, line.join(" | "));
  }
}
