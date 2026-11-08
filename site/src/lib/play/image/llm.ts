/*
 * A visitor's own LLM (D560). Requests go straight from their browser to the
 * provider they chose, with their own key: never through this site's Worker,
 * which never sees a key or (on this path) a picture.
 *
 * Sign-in is offered where a provider has a real way for another site to get a
 * key: OpenRouter's OAuth with PKCE. OpenAI, Anthropic and Google have an API
 * key for this and no sign-in a third-party site may use with a consumer
 * subscription, so they take a key. A key lives in this page's memory, or in
 * this tab or browser if the visitor asks.
 *
 * All four answer a browser's CORS preflight (checked 2026-11-06). Anthropic
 * also wants the header saying the caller knows the key is in a browser.
 */

export const PROVIDERS = [
  {
    id: "openrouter",
    label: "OpenRouter",
    keysUrl: "https://openrouter.ai/settings/keys",
    signIn: true,
    note: "Sign in or use a key. One account for most models, some of them free.",
  },
  {
    id: "anthropic",
    label: "Anthropic (Claude)",
    keysUrl: "https://console.anthropic.com/settings/keys",
    signIn: false,
    note: "An API key from the Anthropic Console. A Claude subscription does not include one.",
  },
  {
    id: "openai",
    label: "OpenAI (GPT)",
    keysUrl: "https://platform.openai.com/api-keys",
    signIn: false,
    note: "An API key from the OpenAI platform. A ChatGPT subscription does not include one.",
  },
  {
    id: "google",
    label: "Google (Gemini)",
    keysUrl: "https://aistudio.google.com/apikey",
    signIn: false,
    note: "An API key from Google AI Studio, which has a free tier.",
  },
] as const;

export type ProviderId = (typeof PROVIDERS)[number]["id"];

export type Part = { text: string } | { jpegBase64: string };
export type Message = { role: "user" | "assistant"; parts: Part[] };

export type Completion = {
  provider: ProviderId;
  key: string;
  model: string;
  system: string;
  messages: Message[];
  maxTokens: number;
  signal?: AbortSignal;
};

type Fetch = typeof fetch;

async function check(response: Response, provider: ProviderId): Promise<unknown> {
  const text = await response.text();
  let body: unknown;
  try {
    body = JSON.parse(text) as unknown;
  } catch {
    body = undefined;
  }
  if (!response.ok) {
    const detail = body as { error?: { message?: string } | string; message?: string } | undefined;
    const message =
      (typeof detail?.error === "string" ? detail.error : detail?.error?.message) ??
      detail?.message ??
      text.slice(0, 200);
    if (response.status === 401 || response.status === 403) {
      throw new Error(`${provider} refused the key (${response.status}): ${message}`);
    }
    throw new Error(`${provider} answered ${response.status}: ${message}`);
  }
  return body;
}

function headersFor(provider: ProviderId, key: string): Record<string, string> {
  const json = { "Content-Type": "application/json" };
  if (provider === "anthropic") {
    return {
      ...json,
      "x-api-key": key,
      "anthropic-version": "2023-06-01",
      "anthropic-dangerous-direct-browser-access": "true",
    };
  }
  if (provider === "google") return { ...json, "x-goog-api-key": key };
  return { ...json, Authorization: `Bearer ${key}` };
}

/** Sends one conversation and returns the text of the reply. */
export async function complete(request: Completion, fetcher: Fetch = fetch): Promise<string> {
  const { provider, key, model, system, messages, maxTokens, signal } = request;
  if (provider === "anthropic") {
    const body = await check(
      await fetcher("https://api.anthropic.com/v1/messages", {
        method: "POST",
        headers: headersFor(provider, key),
        signal,
        body: JSON.stringify({
          model,
          max_tokens: maxTokens,
          system,
          messages: messages.map((message) => ({
            role: message.role,
            content: message.parts.map((part) =>
              "text" in part
                ? { type: "text", text: part.text }
                : {
                    type: "image",
                    source: { type: "base64", media_type: "image/jpeg", data: part.jpegBase64 },
                  },
            ),
          })),
        }),
      }),
      provider,
    );
    const reply = body as { content?: { type: string; text?: string }[] };
    return (reply.content ?? []).map((block) => block.text ?? "").join("");
  }
  if (provider === "google") {
    const body = await check(
      await fetcher(
        `https://generativelanguage.googleapis.com/v1beta/models/${encodeURIComponent(model)}:generateContent`,
        {
          method: "POST",
          headers: headersFor(provider, key),
          signal,
          body: JSON.stringify({
            systemInstruction: { parts: [{ text: system }] },
            generationConfig: { maxOutputTokens: maxTokens },
            contents: messages.map((message) => ({
              role: message.role === "assistant" ? "model" : "user",
              parts: message.parts.map((part) =>
                "text" in part
                  ? { text: part.text }
                  : { inlineData: { mimeType: "image/jpeg", data: part.jpegBase64 } },
              ),
            })),
          }),
        },
      ),
      provider,
    );
    const reply = body as { candidates?: { content?: { parts?: { text?: string }[] } }[] };
    return (reply.candidates?.[0]?.content?.parts ?? []).map((part) => part.text ?? "").join("");
  }
  const url =
    provider === "openai"
      ? "https://api.openai.com/v1/chat/completions"
      : "https://openrouter.ai/api/v1/chat/completions";
  const body = await check(
    await fetcher(url, {
      method: "POST",
      headers: {
        ...headersFor(provider, key),
        ...(provider === "openrouter" ? { "X-Title": "Strobe" } : {}),
      },
      signal,
      body: JSON.stringify({
        model,
        // OpenAI's newer models take only this name; OpenRouter accepts the older one.
        ...(provider === "openai" ? { max_completion_tokens: maxTokens } : { max_tokens: maxTokens }),
        messages: [
          { role: "system", content: system },
          ...messages.map((message) => ({
            role: message.role,
            content: message.parts.map((part) =>
              "text" in part
                ? { type: "text", text: part.text }
                : { type: "image_url", image_url: { url: `data:image/jpeg;base64,${part.jpegBase64}` } },
            ),
          })),
        ],
      }),
    }),
    provider,
  );
  const reply = body as { choices?: { message?: { content?: string } }[] };
  return reply.choices?.[0]?.message?.content ?? "";
}

/**
 * The models this key can use that take pictures, best guess first, read from
 * the provider rather than written here, so a new model needs no new release.
 */
export async function listModels(
  provider: ProviderId,
  key: string,
  fetcher: Fetch = fetch,
): Promise<string[]> {
  const get = async (url: string) =>
    check(await fetcher(url, { headers: headersFor(provider, key) }), provider);
  if (provider === "anthropic") {
    const body = (await get("https://api.anthropic.com/v1/models?limit=100")) as {
      data?: { id: string }[];
    };
    // Newest first, as the API lists them; the middle of the family leads.
    const ids = (body.data ?? []).map((model) => model.id);
    return [...ids.filter((id) => id.includes("sonnet")), ...ids.filter((id) => !id.includes("sonnet"))];
  }
  if (provider === "google") {
    const body = (await get("https://generativelanguage.googleapis.com/v1beta/models?pageSize=200")) as {
      models?: { name: string; supportedGenerationMethods?: string[] }[];
    };
    const ids = (body.models ?? [])
      .filter((model) => model.supportedGenerationMethods?.includes("generateContent"))
      .map((model) => model.name.replace(/^models\//, ""))
      .filter((id) => id.startsWith("gemini") && !/image|tts|audio|embedding|live/.test(id));
    return ids.sort((a, b) => Number(/flash/.test(b)) - Number(/flash/.test(a)) || b.localeCompare(a));
  }
  if (provider === "openai") {
    const body = (await get("https://api.openai.com/v1/models")) as { data?: { id: string }[] };
    const ids = (body.data ?? [])
      .map((model) => model.id)
      .filter((id) => /^(gpt-|o\d)/.test(id))
      .filter((id) => !/audio|realtime|transcribe|tts|image|search|instruct|codex|embedding/.test(id));
    return ids.sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
  }
  const body = (await get("https://openrouter.ai/api/v1/models")) as {
    data?: { id: string; architecture?: { input_modalities?: string[] } }[];
  };
  const ids = (body.data ?? [])
    .filter((model) => model.architecture?.input_modalities?.includes("image"))
    .map((model) => model.id);
  return ids.sort((a, b) => a.localeCompare(b));
}

/* ---- OpenRouter sign-in ------------------------------------------------- */

const VERIFIER_KEY = "strobe.play.openrouter-verifier";
export const KEY_SESSION = "strobe.play.llm-session";
export const KEY_REMEMBERED = "strobe.play.llm-remembered";

function base64Url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/** PKCE's S256 challenge for a verifier. */
export async function challengeFor(verifier: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
  return base64Url(new Uint8Array(digest));
}

/** Leaves for OpenRouter's sign-in page; it comes back to /play with ?code=. */
export async function beginOpenRouterSignIn(): Promise<void> {
  const verifier = base64Url(crypto.getRandomValues(new Uint8Array(32)));
  sessionStorage.setItem(VERIFIER_KEY, verifier);
  const challenge = await challengeFor(verifier);
  const back = `${window.location.origin}/play/`;
  window.location.assign(
    `https://openrouter.ai/auth?callback_url=${encodeURIComponent(back)}&code_challenge=${challenge}&code_challenge_method=S256`,
  );
}

/**
 * On return from the sign-in: the key, or undefined if this address is not one.
 * Only a code this tab asked for counts, so a link with ?code= in it does nothing.
 */
export async function finishOpenRouterSignIn(fetcher: Fetch = fetch): Promise<string | undefined> {
  const code = new URLSearchParams(window.location.search).get("code");
  const verifier = sessionStorage.getItem(VERIFIER_KEY);
  if (!code || !verifier) return undefined;
  sessionStorage.removeItem(VERIFIER_KEY);
  history.replaceState(null, "", window.location.pathname);
  const body = (await check(
    await fetcher("https://openrouter.ai/api/v1/auth/keys", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code, code_verifier: verifier, code_challenge_method: "S256" }),
    }),
    "openrouter",
  )) as { key?: string };
  if (!body.key) throw new Error("OpenRouter's sign-in returned no key");
  return body.key;
}

/* ---- keeping a key ------------------------------------------------------ */

export type SavedKey = { provider: ProviderId; key: string; model?: string };

function parseSaved(text: string | null): SavedKey | undefined {
  if (!text) return undefined;
  try {
    const parsed = JSON.parse(text) as SavedKey;
    return PROVIDERS.some((provider) => provider.id === parsed.provider) && typeof parsed.key === "string"
      ? parsed
      : undefined;
  } catch {
    return undefined;
  }
}

export function readSavedKey(): SavedKey | undefined {
  try {
    return parseSaved(sessionStorage.getItem(KEY_SESSION)) ?? parseSaved(localStorage.getItem(KEY_REMEMBERED));
  } catch {
    return undefined;
  }
}

/** Keeps a key in this tab, and in this browser only if the visitor asked. */
export function saveKey(saved: SavedKey, remember: boolean) {
  try {
    sessionStorage.setItem(KEY_SESSION, JSON.stringify(saved));
    if (remember) localStorage.setItem(KEY_REMEMBERED, JSON.stringify(saved));
    else localStorage.removeItem(KEY_REMEMBERED);
  } catch {
    // Storage blocked: the key lives in this page until it closes.
  }
}

export function forgetKey() {
  try {
    sessionStorage.removeItem(KEY_SESSION);
    localStorage.removeItem(KEY_REMEMBERED);
  } catch {
    // Nothing was stored.
  }
}
