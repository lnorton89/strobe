/*
 * The free reading of a picture for /play's From image (D560): a vision model
 * on Workers AI, inside the account's free daily allowance, for visitors who
 * want recognition with neither a download nor a key of their own. It answers
 * only with our own words (`cleanReading`), it writes no code, and the picture
 * is neither stored nor logged here. The panel says it sends the picture
 * before the visitor presses the button.
 *
 * The allowance is shared by everyone, so it is rate limited per visitor, and
 * when it runs out the answer says so and the tool still works without it.
 */

import { READING_PROMPT, cleanReading, firstJsonObject } from "../src/lib/play/image/reading";

export type Ai = {
  run(model: string, inputs: Record<string, unknown>): Promise<unknown>;
};

/** The picture's base64 at most: a 512-pixel JPEG is far smaller. */
export const MAX_READ_BYTES = 256 * 1024;

export const READ_MODEL = "@cf/meta/llama-3.2-11b-vision-instruct";

type Env = {
  AI?: Ai;
  READ_LIMIT?: { limit(options: { key: string }): Promise<{ success: boolean }> };
};

function answer(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", "Cache-Control": "no-store" },
  });
}

/** What a person is told about a failed model call. */
export function explainFailure(message: string): { error: string; status: number } {
  if (/agree/i.test(message)) {
    return { error: "the free service has not been switched on yet; the other options still work", status: 503 };
  }
  if (/4006|neuron|allocation|quota|limit/i.test(message)) {
    return { error: "today's free allowance is used up; it resets at midnight UTC, and the other options still work", status: 429 };
  }
  return { error: "the free service could not read the picture; the other options still work", status: 502 };
}

export async function readImage(request: Request, env: Env): Promise<Response> {
  if (!env.AI) return answer({ error: "the free service is not available here" }, 503);
  const visitor = request.headers.get("CF-Connecting-IP") ?? "unknown";
  if (env.READ_LIMIT && !(await env.READ_LIMIT.limit({ key: visitor })).success) {
    return answer({ error: "too many readings; wait a minute" }, 429);
  }
  if (Number(request.headers.get("Content-Length") ?? "0") > MAX_READ_BYTES) {
    return answer({ error: "the picture is too large" }, 413);
  }
  const text = await request.text();
  if (text.length > MAX_READ_BYTES) return answer({ error: "the picture is too large" }, 413);
  let jpegBase64: unknown;
  try {
    jpegBase64 = (JSON.parse(text) as { jpegBase64?: unknown }).jpegBase64;
  } catch {
    return answer({ error: "not JSON" }, 400);
  }
  if (typeof jpegBase64 !== "string" || !/^[A-Za-z0-9+/]+=*$/.test(jpegBase64)) {
    return answer({ error: "no picture" }, 400);
  }
  try {
    const reply = (await env.AI.run(READ_MODEL, {
      messages: [
        {
          role: "user",
          content: [
            { type: "text", text: READING_PROMPT },
            { type: "image_url", image_url: { url: `data:image/jpeg;base64,${jpegBase64}` } },
          ],
        },
      ],
      max_tokens: 120,
      temperature: 0.2,
    })) as { response?: unknown };
    const words = typeof reply.response === "string" ? reply.response : JSON.stringify(reply.response);
    return answer({ reading: cleanReading(firstJsonObject(words)) });
  } catch (caught) {
    const { error, status } = explainFailure(String(caught));
    return answer({ error }, status);
  }
}
