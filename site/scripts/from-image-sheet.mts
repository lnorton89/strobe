/*
 * From image's check on real pictures (D560):
 *
 *   npx tsx scripts/from-image-sheet.mts <out> <picture>...
 *
 * writes each picture's candidates as scene folders under <out>, for
 * strobe-bench to compile on desktop GL and render as a contact sheet. ALL=1
 * writes every template instead, to judge the library. The pictures stay where
 * they are and nothing here is committed.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import sharp from "sharp";
import { analyse } from "../src/lib/play/image/analyse";
import { assignColours, candidates, emitScene } from "../src/lib/play/image/generate";
import { firstReading } from "../src/lib/play/image/reading";
import { TEMPLATES } from "../src/lib/play/image/templates";

const [out = "", ...pictures] = process.argv.slice(2);
for (const picture of pictures) {
  // CROP=0.1 takes a tenth off each edge, as a person confirming the crop would.
  const crop = Number(process.env.CROP ?? 0);
  const size = await sharp(picture).metadata();
  const left = Math.round((size.width ?? 1) * crop);
  const top = Math.round((size.height ?? 1) * crop);
  const { data, info } = await sharp(picture)
    .extract({
      left,
      top,
      width: (size.width ?? 1) - 2 * left,
      height: (size.height ?? 1) - 2 * top,
    })
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const measures = analyse({ width: info.width, height: info.height, data });
  const reading = firstReading(measures);
  const name = path.basename(picture).replace(/\W+/g, "-");
  console.log(
    name,
    reading.layout,
    reading.motif,
    reading.mood,
    `mirror ${measures.mirror.toFixed(2)} radial ${measures.radial.score.toFixed(2)} lattice ${measures.lattice.score.toFixed(2)} density ${measures.density.toFixed(3)}`,
  );
  const scenes = process.env.ALL
    ? TEMPLATES.map((template) => ({
        id: template.id,
        scene: emitScene(template, measures, reading, assignColours(measures), measures.seed),
      }))
    : candidates(measures, reading, 9);
  for (const candidate of scenes) {
    const dir = path.join(out, `${name}--${candidate.id}.synScene`);
    mkdirSync(dir, { recursive: true });
    for (const [file, text] of Object.entries(candidate.scene.files)) {
      writeFileSync(path.join(dir, file), text);
    }
  }
}
