/*
 * Recognising a picture in the browser (D560): CLIP's vision half against the
 * text vectors in concepts.json. It is a neural net, not a language model: it
 * writes nothing and scores a picture against words chosen here, so it cannot
 * be talked into anything. The picture stays on the machine.
 *
 * The library and its wasm are this site's own files (scripts/copy-clip.mjs),
 * so a page that may hold a visitor's API key runs no script from another
 * site. The weights are about 89 MB once, from this site's R2 bucket, because
 * Hugging Face's CDN refuses a workers.dev Referer; then the browser's cache
 * has them. On a local server there is no bucket, so they come from Hugging Face.
 *
 * It got layouts right that the classical measures got wrong on the real
 * references (a wheel, a grid), but it names subjects badly, so it is never
 * asked to.
 */

import { type Layout, LAYOUTS, type Mood, MOODS, type Motif, MOTIFS, type Reading } from "@/lib/play/image/reading";

const BASE = "/generated/clip/";

type Tensor = { data: Float32Array };
type Library = {
  env: {
    allowLocalModels: boolean;
    remoteHost: string;
    remotePathTemplate: string;
    backends: { onnx: { wasm: { wasmPaths: string; numThreads: number } } };
  };
  AutoProcessor: { from_pretrained(model: string): Promise<(image: unknown) => Promise<unknown>> };
  CLIPVisionModelWithProjection: {
    from_pretrained(
      model: string,
      options: { dtype: string; progress_callback?: (event: Progress) => void },
    ): Promise<(inputs: unknown) => Promise<{ image_embeds: Tensor }>>;
  };
  RawImage: { fromBlob(blob: Blob): Promise<unknown> };
};

type Progress = { status?: string; progress?: number; file?: string };

/** About how much the first use downloads, to say so before it starts. */
export const CLIP_DOWNLOAD_MB = 89;

type Concepts = { model: string; groups: Record<string, Record<string, string>> };

function unpack(text: string): Float32Array {
  const bytes = Uint8Array.from(atob(text), (char) => char.charCodeAt(0));
  const out = new Float32Array(bytes.length);
  let size = 0;
  bytes.forEach((byte, index) => {
    const value = byte > 127 ? byte - 256 : byte;
    out[index] = value;
    size += value * value;
  });
  const scale = Math.sqrt(size) || 1;
  return out.map((value) => value / scale);
}

/** The word in `words` the vector `seen` is nearest, and by how much it won. */
export function bestWord(
  seen: Float32Array,
  words: Record<string, string>,
): { word: string; margin: number } {
  const scores = Object.entries(words)
    .map(([word, packed]) => {
      const vector = unpack(packed);
      let score = 0;
      vector.forEach((value, index) => {
        score += value * (seen[index] ?? 0);
      });
      return { word, score };
    })
    .sort((a, b) => b.score - a.score);
  return { word: scores[0]?.word ?? "", margin: (scores[0]?.score ?? 0) - (scores[1]?.score ?? 0) };
}

let vision: Promise<{ library: Library; model: string; run: (blob: Blob) => Promise<Float32Array> }> | undefined;

function load(onProgress: (fraction: number) => void) {
  vision ??= (async () => {
    const concepts = (await import("@/lib/play/image/concepts.json")).default as Concepts;
    const library = (await import(
      /* webpackIgnore: true */ /* turbopackIgnore: true */ `${BASE}transformers.min.js`
    )) as Library;
    library.env.allowLocalModels = false;
    if (window.location.hostname !== "localhost") {
      library.env.remoteHost = `${window.location.origin}/api/models/`;
      library.env.remotePathTemplate = "{model}/";
    }
    library.env.backends.onnx.wasm.wasmPaths = BASE;
    // Threads need cross-origin isolation, which this site does not ask for.
    library.env.backends.onnx.wasm.numThreads = 1;
    const processor = await library.AutoProcessor.from_pretrained(concepts.model);
    const model = await library.CLIPVisionModelWithProjection.from_pretrained(concepts.model, {
      dtype: "q8",
      progress_callback: (event) => {
        if (event.status === "progress" && event.file?.endsWith(".onnx") && event.progress !== undefined) {
          onProgress(event.progress / 100);
        }
      },
    });
    return {
      library,
      model: concepts.model,
      run: async (blob: Blob) => {
        const image = await library.RawImage.fromBlob(blob);
        const { image_embeds: embeds } = await model(await processor(image));
        let size = 0;
        for (const value of embeds.data) size += value * value;
        const scale = Math.sqrt(size) || 1;
        return embeds.data.map((value) => value / scale);
      },
    };
  })();
  vision.catch(() => {
    vision = undefined;
  });
  return vision;
}

/** Reads a picture. The first call downloads the model. */
export async function readWithClip(
  picture: Blob,
  onProgress: (fraction: number) => void,
): Promise<Partial<Omit<Reading, "by">>> {
  const concepts = (await import("@/lib/play/image/concepts.json")).default as Concepts;
  const { run } = await load(onProgress);
  const seen = await run(picture);
  const pick = (group: string) => bestWord(seen, concepts.groups[group] ?? {}).word;
  const layout = pick("layout") as Layout;
  const motif = pick("motif") as Motif;
  const mood = pick("mood") as Mood;
  return {
    ...(LAYOUTS.includes(layout) ? { layout } : {}),
    ...(MOTIFS.includes(motif) ? { motif } : {}),
    ...(MOODS.includes(mood) ? { mood } : {}),
  };
}
