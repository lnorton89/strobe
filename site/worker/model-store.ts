/*
 * The in-browser CLIP's files for /play's From image (D560), from this site's
 * own R2 bucket. Hugging Face's CDN refuses requests whose Referer is a
 * workers.dev address, and R2 has free egress, so the weights are served from
 * here. Only the named files can be read: this is not a window on the bucket.
 *
 * scripts/upload-clip-model.mjs puts them there. Their names never change, so a
 * browser may keep them for good; a new model means a new folder.
 */

export const MODEL_FILES: Record<string, string> = {
  "Xenova/clip-vit-base-patch32/config.json": "application/json",
  "Xenova/clip-vit-base-patch32/preprocessor_config.json": "application/json",
  "Xenova/clip-vit-base-patch32/onnx/vision_model_quantized.onnx": "application/octet-stream",
};

export type ModelBucket = {
  get(key: string): Promise<{ body: ReadableStream; size: number; httpEtag: string } | null>;
  head(key: string): Promise<{ size: number; httpEtag: string } | null>;
};

export async function serveModelFile(
  name: string,
  method: string,
  bucket: ModelBucket | undefined,
): Promise<Response> {
  const type = MODEL_FILES[name];
  if (!type) return new Response("no such file", { status: 404 });
  if (method !== "GET" && method !== "HEAD") {
    return new Response("GET a file", { status: 405, headers: { Allow: "GET, HEAD" } });
  }
  if (!bucket) return new Response("the model is not available here", { status: 503 });
  const head = method === "HEAD";
  const object = head ? await bucket.head(name) : await bucket.get(name);
  if (!object) return new Response("the model has not been uploaded", { status: 404 });
  const headers = {
    "Content-Type": type,
    "Content-Length": String(object.size),
    ETag: object.httpEtag,
    "Cache-Control": "public, max-age=31536000, immutable",
  };
  const body = !head && "body" in object ? object.body : null;
  return new Response(body as ReadableStream | null, { headers });
}
