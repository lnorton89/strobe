"use client";

/*
 * From image (D560): a picture in, a gallery of playing scenes out, and the
 * visitor picks. Everything runs in this page unless they choose otherwise, and
 * each choice that sends the picture anywhere says where before it does.
 *
 * Recognising is optional and layered: the measures' first guess is instant,
 * CLIP runs in the browser after one download, the free service uses this
 * site's Cloudflare allowance, and a visitor's own LLM can read the picture or
 * write a whole scene. The visitor's click on a layout or motif beats them all.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Scene } from "@/lib/play/files";
import { type Measures, analyse } from "@/lib/play/image/analyse";
import { CLIP_DOWNLOAD_MB, readWithClip } from "@/lib/play/image/clip";
import { type Candidate, candidates } from "@/lib/play/image/generate";
import {
  type Corners,
  type Loaded,
  WHOLE_PICTURE,
  blobOf,
  jpegOf,
  loadPicture,
  pixelsOf,
  straighten,
} from "@/lib/play/image/intake";
import {
  type ProviderId,
  PROVIDERS,
  beginOpenRouterSignIn,
  complete,
  finishOpenRouterSignIn,
  forgetKey,
  listModels,
  readSavedKey,
  saveKey,
} from "@/lib/play/image/llm";
import { type Played, Previewer } from "@/lib/play/image/preview";
import {
  LAYOUTS,
  LAYOUT_LABELS,
  MOODS,
  MOTIFS,
  READING_PROMPT,
  type Reading,
  cleanReading,
  firstJsonObject,
  firstReading,
} from "@/lib/play/image/reading";
import { writeScene } from "@/lib/play/image/writer";

const BUTTON =
  "inline-flex items-center gap-1.5 border border-hairline-strong px-2.5 py-1.5 font-mono text-[11px] uppercase tracking-[0.1em] text-bright transition-colors hover:border-accent hover:text-accent-text disabled:opacity-40";
const PRIMARY =
  "inline-flex items-center gap-1.5 border border-accent bg-accent px-3 py-2 font-mono text-[11px] uppercase tracking-[0.1em] text-on-accent transition-opacity hover:opacity-90 disabled:opacity-40";
const LABEL = "font-mono text-[11px] uppercase tracking-[0.08em] text-label";
const FIELD =
  "border border-hairline-strong bg-ground px-2 py-1.5 font-mono text-xs text-text focus:border-accent focus:outline-none";
const MAX_FILE_BYTES = 40 * 1024 * 1024;

type Tile = Candidate & { played: Played };

function Chips<T extends string>({
  label,
  options,
  value,
  labels,
  onChange,
}: {
  label: string;
  options: readonly T[];
  value: T;
  labels?: Record<T, string>;
  onChange: (next: T) => void;
}) {
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className={LABEL}>{label}</legend>
      <div className="mt-1.5 flex flex-wrap gap-1">
        {options.map((option) => (
          <button
            key={option}
            type="button"
            aria-pressed={option === value}
            onClick={() => onChange(option)}
            className={`border px-2 py-1 font-mono text-[11px] ${
              option === value
                ? "border-accent bg-panel-raised text-accent-text"
                : "border-hairline-strong text-secondary hover:text-text"
            }`}
          >
            {labels?.[option] ?? option}
          </button>
        ))}
      </div>
    </fieldset>
  );
}

/** The picture with four corners to drag or nudge with the arrow keys. */
function CornerPicker({
  url,
  corners,
  onChange,
}: {
  url: string;
  corners: Corners;
  onChange: (next: Corners) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const NAMES = ["top left", "top right", "bottom right", "bottom left"];
  const move = (index: number, x: number, y: number) => {
    const next = corners.map((corner) => [...corner]) as Corners;
    next[index] = [Math.min(1, Math.max(0, x)), Math.min(1, Math.max(0, y))];
    onChange(next);
  };
  return (
    <div ref={box} className="relative w-full touch-none select-none">
      {/* eslint-disable-next-line @next/next/no-img-element -- an object URL of the visitor's own file */}
      <img src={url} alt="The picture you chose" className="block max-h-[52vh] w-full object-contain" />
      <svg className="pointer-events-none absolute inset-0 size-full" viewBox="0 0 1 1" preserveAspectRatio="none" aria-hidden>
        <polygon
          points={corners.map(([x, y]) => `${x},${y}`).join(" ")}
          fill="none"
          stroke="var(--color-accent, currentColor)"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      {corners.map(([x, y], index) => (
        <button
          key={NAMES[index]}
          type="button"
          aria-label={`Move the ${NAMES[index]} corner of the part to use; arrow keys nudge it`}
          className="absolute size-4 -translate-1/2 cursor-grab rounded-full border-2 border-accent bg-ground focus:outline-2 focus:outline-accent"
          style={{ left: `${x * 100}%`, top: `${y * 100}%` }}
          onPointerDown={(event) => event.currentTarget.setPointerCapture(event.pointerId)}
          onPointerMove={(event) => {
            if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
            const rect = box.current?.getBoundingClientRect();
            if (rect) move(index, (event.clientX - rect.left) / rect.width, (event.clientY - rect.top) / rect.height);
          }}
          onKeyDown={(event) => {
            const step = event.shiftKey ? 0.05 : 0.01;
            const delta: Record<string, [number, number]> = {
              ArrowLeft: [-step, 0],
              ArrowRight: [step, 0],
              ArrowUp: [0, -step],
              ArrowDown: [0, step],
            };
            const by = delta[event.key];
            if (by) {
              event.preventDefault();
              move(index, x + by[0], y + by[1]);
            }
          }}
        />
      ))}
    </div>
  );
}

function FlipTile({
  tile,
  frame,
  onPick,
}: {
  tile: Tile;
  frame: number;
  onPick: () => void;
}) {
  const book = tile.played.flipbook;
  return (
    <button
      type="button"
      onClick={onPick}
      className="group flex flex-col border border-hairline text-left transition-colors hover:border-accent focus:border-accent"
    >
      {/* eslint-disable-next-line @next/next/no-img-element -- frames just rendered in this page */}
      <img
        src={book[frame % Math.max(1, book.length)]}
        alt={`${tile.title}, playing`}
        className="aspect-video w-full bg-black object-cover"
      />
      <span className="flex items-baseline justify-between gap-2 px-2 py-1.5">
        <span className="font-display text-sm text-text group-hover:text-accent-text">{tile.title}</span>
        <span className="truncate font-mono text-[10px] text-label">{tile.note}</span>
      </span>
    </button>
  );
}

export default function FromImagePanel({
  open,
  onClose,
  onScene,
  onSignedIn,
}: {
  open: boolean;
  onClose: () => void;
  onScene: (scene: Scene) => void;
  /** The sign-in brought the visitor back: show this panel. */
  onSignedIn: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const previewer = useRef<Previewer | null>(null);
  const run = useRef(0);

  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [url, setUrl] = useState("");
  const [corners, setCorners] = useState<Corners>(WHOLE_PICTURE);
  const [reading, setReading] = useState<Reading | null>(null);
  const [measures, setMeasures] = useState<Measures | null>(null);
  const [status, setStatus] = useState("");
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const [tiles, setTiles] = useState<Tile[]>([]);
  const [rejected, setRejected] = useState<string[]>([]);
  const [frame, setFrame] = useState(0);
  const [variation, setVariation] = useState(0);

  const [provider, setProvider] = useState<ProviderId>(() => readSavedKey()?.provider ?? "openrouter");
  const [apiKey, setApiKey] = useState(() => readSavedKey()?.key ?? "");
  const [remember, setRemember] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState(() => readSavedKey()?.model ?? "");
  const [stoppable, setStoppable] = useState(false);
  const [critiques, setCritiques] = useState(1);
  const abort = useRef<AbortController | null>(null);

  const providerInfo = PROVIDERS.find((item) => item.id === provider) ?? PROVIDERS[0];

  /* ---- the dialog ------------------------------------------------------- */

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) element.showModal();
    if (!open && element.open) element.close();
  }, [open]);

  // Back from OpenRouter's sign-in.
  useEffect(() => {
    finishOpenRouterSignIn()
      .then((key) => {
        if (!key) return;
        setProvider("openrouter");
        setApiKey(key);
        saveKey({ provider: "openrouter", key }, false);
        setStatus("Signed in to OpenRouter. Choose your picture.");
        onSignedIn();
      })
      .catch((caught: unknown) => {
        setProblem(`OpenRouter's sign-in failed: ${String(caught).replace(/^Error: /, "")}`);
        onSignedIn();
      });
  }, [onSignedIn]);

  useEffect(
    () => () => {
      previewer.current?.close();
      previewer.current = null;
      abort.current?.abort();
    },
    [],
  );

  useEffect(() => {
    if (tiles.length === 0 || !open) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const timer = window.setInterval(() => setFrame((old) => old + 1), 160);
    return () => window.clearInterval(timer);
  }, [tiles.length, open]);

  /* ---- the picture ------------------------------------------------------ */

  const remeasure = useCallback(
    (next: Loaded, marked: Corners) => {
      const context = straighten(next, marked);
      const made = analyse(pixelsOf(context));
      setMeasures(made);
      return { context, made };
    },
    [],
  );

  async function choose(file: File | undefined) {
    if (!file) return;
    setProblem("");
    setTiles([]);
    setRejected([]);
    if (file.size > MAX_FILE_BYTES) {
      setProblem(`That file is over ${MAX_FILE_BYTES / 1024 / 1024} MB.`);
      return;
    }
    try {
      const next = await loadPicture(file);
      setUrl((old) => {
        if (old) URL.revokeObjectURL(old);
        return URL.createObjectURL(file);
      });
      setLoaded(next);
      setCorners(WHOLE_PICTURE);
      setVariation(0);
      const { made } = remeasure(next, WHOLE_PICTURE);
      setReading(firstReading(made));
      setStatus("Measured here, in your browser. Mark the part that matters if it is a photo of something.");
    } catch (caught) {
      setProblem(`This browser could not open that picture: ${String(caught)}`);
    }
  }

  const changeCorners = useCallback((next: Corners) => setCorners(next), []);

  // Colours follow the corners, a moment after they stop moving.
  useEffect(() => {
    if (!loaded) return;
    const timer = window.setTimeout(() => {
      try {
        remeasure(loaded, corners);
      } catch (caught) {
        setProblem(String(caught));
      }
    }, 200);
    return () => window.clearTimeout(timer);
  }, [corners, loaded, remeasure]);

  const swatches = useMemo(
    () => (measures ? [...measures.palette, ...measures.accents] : []),
    [measures],
  );

  /* ---- making ----------------------------------------------------------- */

  function getPreviewer(): Previewer {
    previewer.current ??= new Previewer();
    return previewer.current;
  }

  async function make(more = variation) {
    if (!loaded || !reading) return;
    const token = (run.current += 1);
    setBusy(true);
    setProblem("");
    setTiles([]);
    setRejected([]);
    try {
      const { made } = remeasure(loaded, corners);
      const shifted = { ...made, seed: (made.seed + more * 1733) % 100_000 };
      const list = candidates(shifted, reading, 9);
      const player = getPreviewer();
      const failed: string[] = [];
      let index = 0;
      for (const candidate of list) {
        index += 1;
        setStatus(`Compiling and playing ${candidate.title}: ${index} of ${list.length}…`);
        const error = await player.compile(candidate.scene);
        if (token !== run.current) return;
        if (error !== undefined) {
          failed.push(`${candidate.title}: did not compile`);
          console.warn(`from image: ${candidate.id}: ${error}`);
          continue;
        }
        const played = await player.play();
        if (token !== run.current) return;
        if (!played.result.passed) {
          failed.push(`${candidate.title}: ${played.result.problems.join(", ")}`);
          continue;
        }
        setTiles((old) => [...old, { ...candidate, played }]);
      }
      setRejected(failed);
      setStatus("Pick one to open it. Every scene is screened for flashing, which is a check and not a certificate.");
    } catch (caught) {
      setProblem(String(caught));
    } finally {
      if (token === run.current) setBusy(false);
    }
  }

  function pick(scene: Scene) {
    onScene(scene);
    onClose();
  }

  /* ---- recognising ------------------------------------------------------ */

  function applyReading(found: Partial<Omit<Reading, "by">>, by: string) {
    setReading((old) => (old ? { ...old, ...found, by } : old));
  }

  async function recogniseInBrowser() {
    if (!loaded) return;
    setBusy(true);
    setProblem("");
    setStatus("Starting the model in your browser…");
    try {
      const blob = await blobOf(straighten(loaded, corners));
      const found = await readWithClip(blob, (fraction) => {
        setStatus(`Downloading the model, once: ${Math.round(fraction * 100)}%`);
      });
      applyReading(found, "CLIP, in your browser");
      setStatus("Read in your browser. Change anything that looks wrong.");
    } catch (caught) {
      setProblem(`The in-browser model did not run: ${String(caught)}`);
    } finally {
      setBusy(false);
    }
  }

  async function recogniseWithService() {
    if (!loaded) return;
    setBusy(true);
    setProblem("");
    setStatus("Asking the free service…");
    try {
      const jpegBase64 = jpegOf(straighten(loaded, corners), 512, 0.8);
      const response = await fetch("/api/read-image", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ jpegBase64 }),
      });
      const body = (await response.json()) as { reading?: unknown; error?: string };
      if (!response.ok) throw new Error(body.error ?? `answered ${response.status}`);
      applyReading(cleanReading(body.reading), "the free service");
      setStatus("Read by the free service. Nothing was kept. Change anything that looks wrong.");
    } catch (caught) {
      setProblem(`The free service: ${String(caught).replace(/^Error: /, "")}`);
    } finally {
      setBusy(false);
    }
  }

  /* ---- a visitor's own LLM ---------------------------------------------- */

  function keep(next: { key?: string; model?: string; remember?: boolean; provider?: ProviderId }) {
    saveKey(
      {
        provider: next.provider ?? provider,
        key: next.key ?? apiKey,
        model: next.model ?? model,
      },
      next.remember ?? remember,
    );
  }

  async function fetchModels() {
    setBusy(true);
    setProblem("");
    setStatus(`Asking ${providerInfo.label} for its models…`);
    try {
      const found = await listModels(provider, apiKey.trim());
      setModels(found);
      const chosen = found.includes(model) ? model : (found[0] ?? "");
      setModel(chosen);
      keep({ key: apiKey.trim(), model: chosen });
      setStatus(found.length > 0 ? `${found.length} models take pictures.` : "This key lists no model that takes pictures.");
    } catch (caught) {
      setProblem(String(caught).replace(/^Error: /, ""));
    } finally {
      setBusy(false);
    }
  }

  async function recogniseWithLlm() {
    if (!loaded) return;
    abort.current = new AbortController();
    setStoppable(true);
    setBusy(true);
    setProblem("");
    setStatus(`Asking ${model}…`);
    try {
      const reply = await complete({
        provider,
        key: apiKey.trim(),
        model,
        system: "You read pictures and answer in JSON.",
        messages: [
          {
            role: "user",
            parts: [{ jpegBase64: jpegOf(straighten(loaded, corners), 768) }, { text: READING_PROMPT }],
          },
        ],
        maxTokens: 400,
        signal: abort.current.signal,
      });
      applyReading(cleanReading(firstJsonObject(reply)), model);
      setStatus("Read by your model. Change anything that looks wrong.");
    } catch (caught) {
      setProblem(String(caught).replace(/^Error: /, ""));
    } finally {
      setBusy(false);
      setStoppable(false);
    }
  }

  async function writeWithLlm() {
    if (!loaded || !reading || !measures) return;
    abort.current = new AbortController();
    setStoppable(true);
    setBusy(true);
    setProblem("");
    try {
      const player = getPreviewer();
      const written = await writeScene({
        provider,
        key: apiKey.trim(),
        model,
        picture: jpegOf(straighten(loaded, corners), 1024),
        measures,
        reading,
        critiques,
        signal: abort.current.signal,
        onProgress: setStatus,
        tools: {
          compile: (scene) => player.compile(scene),
          render: async () => {
            const played = await player.play();
            return { result: played.result, jpegBase64: player.snapshot() };
          },
        },
      });
      setStatus(written.warnings.join(" ") || "Written and playing.");
      pick(written.scene);
    } catch (caught) {
      if (abort.current?.signal.aborted) setStatus("Stopped. Nothing changed.");
      else setProblem(String(caught).replace(/^Error: /, ""));
    } finally {
      setBusy(false);
      setStoppable(false);
    }
  }

  /* ---- the page --------------------------------------------------------- */

  const haveKey = apiKey.trim().length > 0;
  const haveModel = haveKey && model.length > 0;

  return (
    <dialog
      ref={dialog}
      aria-label="Make a scene from a picture"
      onClose={onClose}
      className="m-auto max-h-[94dvh] w-[min(72rem,96vw)] border border-hairline-strong bg-ground p-0 text-text backdrop:bg-black/70"
    >
      <div className="flex items-center justify-between border-b border-hairline px-4 py-3">
        <h2 className="font-display text-lg text-text">From image</h2>
        <button type="button" aria-label="Close" onClick={onClose} className="px-2 text-lg text-label hover:text-text">
          ×
        </button>
      </div>

      <div className="grid gap-4 p-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
        <div className="flex min-w-0 flex-col gap-3">
          <label
            className={`${BUTTON} cursor-pointer self-start`}
            onDragOver={(event) => event.preventDefault()}
            onDrop={(event) => {
              event.preventDefault();
              void choose(event.dataTransfer.files[0]);
            }}
          >
            {loaded ? "Choose another picture" : "Choose or drop a picture"}
            <input
              data-testid="from-image-file"
              type="file"
              accept="image/*"
              className="hidden"
              onChange={(event) => {
                void choose(event.target.files?.[0]);
                event.target.value = "";
              }}
            />
          </label>
          {url ? (
            <>
              <CornerPicker url={url} corners={corners} onChange={changeCorners} />
              <p className="text-xs text-secondary">
                Drag the corners onto the part that matters. A straightened painting gives far better
                colours than a photo of a room.
              </p>
            </>
          ) : (
            <p className="text-sm text-secondary">
              It makes a new scene inspired by a picture: its colours and its feel, drawn by the
              scene&apos;s own rules, not a copy. The picture stays in your browser unless you choose an
              option below that says otherwise.
            </p>
          )}
          {swatches.length > 0 ? (
            <div className="flex flex-wrap gap-1" aria-label="Colours measured from the picture">
              {swatches.map((swatch) => (
                <span
                  key={swatch.rgb.join()}
                  title={`${Math.round(swatch.weight * 100)}%`}
                  className="size-5 border border-hairline"
                  style={{
                    backgroundColor: `rgb(${swatch.rgb.map((value) => Math.round(value * 255)).join(" ")})`,
                  }}
                />
              ))}
            </div>
          ) : null}
        </div>

        <div className="flex min-w-0 flex-col gap-4">
          {reading ? (
            <>
              <Chips
                label={`Layout (${reading.by})`}
                options={LAYOUTS}
                labels={LAYOUT_LABELS}
                value={reading.layout}
                onChange={(layout) => setReading({ ...reading, layout, by: "you" })}
              />
              <Chips
                label="Motif"
                options={MOTIFS}
                value={reading.motif}
                onChange={(motif) => setReading({ ...reading, motif, by: "you" })}
              />
              <Chips
                label="Mood"
                options={MOODS}
                value={reading.mood}
                onChange={(mood) => setReading({ ...reading, mood, by: "you" })}
              />
              <div className="flex flex-wrap items-center gap-2">
                <span className={LABEL}>Read it for me</span>
                <button
                  type="button"
                  className={BUTTON}
                  disabled={busy}
                  title={`Runs in your browser; the picture stays here. Downloads about ${CLIP_DOWNLOAD_MB} MB the first time.`}
                  onClick={() => void recogniseInBrowser()}
                >
                  In this browser ({CLIP_DOWNLOAD_MB} MB once)
                </button>
                <button
                  type="button"
                  className={BUTTON}
                  disabled={busy}
                  title="Sends a small copy of the picture to this site's server and Cloudflare's AI service, which reads it and keeps nothing. Limited each day."
                  onClick={() => void recogniseWithService()}
                >
                  Free service (sends the picture)
                </button>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <button type="button" className={PRIMARY} disabled={busy} onClick={() => void make()}>
                  {tiles.length > 0 ? "Make them again" : "Make scenes"}
                </button>
                {tiles.length > 0 ? (
                  <button
                    type="button"
                    className={BUTTON}
                    disabled={busy}
                    onClick={() => {
                      const next = variation + 1;
                      setVariation(next);
                      void make(next);
                    }}
                  >
                    Different ones
                  </button>
                ) : null}
              </div>
            </>
          ) : null}

          <details className="border border-hairline">
            <summary className={`${LABEL} cursor-pointer px-3 py-2`}>Use your own AI model</summary>
            <div className="flex flex-col gap-3 border-t border-hairline p-3">
              <p className="text-xs text-secondary">
                A model can read the picture properly and write a one-off scene, which the built-in
                templates cannot. The picture goes from your browser straight to the provider you
                choose, and your key never reaches this site&apos;s server. Using it may cost you money
                on that account.
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <label className={LABEL} htmlFor="from-image-provider">
                  Provider
                </label>
                <select
                  id="from-image-provider"
                  className={FIELD}
                  value={provider}
                  onChange={(event) => {
                    setProvider(event.target.value as ProviderId);
                    setModels([]);
                    setModel("");
                    setApiKey("");
                  }}
                >
                  {PROVIDERS.map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.label}
                    </option>
                  ))}
                </select>
                {providerInfo.signIn ? (
                  <button type="button" className={BUTTON} disabled={busy} onClick={() => void beginOpenRouterSignIn()}>
                    Sign in with OpenRouter
                  </button>
                ) : null}
              </div>
              <p className="text-xs text-secondary">
                {providerInfo.note}{" "}
                <a className="underline hover:text-accent-text" href={providerInfo.keysUrl} target="_blank" rel="noreferrer">
                  Get a key
                </a>
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <label className={LABEL} htmlFor="from-image-key">
                  API key
                </label>
                <input
                  id="from-image-key"
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  value={apiKey}
                  onChange={(event) => setApiKey(event.target.value)}
                  className={`${FIELD} min-w-0 flex-1`}
                />
                <button type="button" className={BUTTON} disabled={busy || !haveKey} onClick={() => void fetchModels()}>
                  Load models
                </button>
                <button
                  type="button"
                  className={BUTTON}
                  disabled={!haveKey}
                  onClick={() => {
                    forgetKey();
                    setApiKey("");
                    setModels([]);
                    setModel("");
                    setRemember(false);
                  }}
                >
                  Forget
                </button>
              </div>
              <label className={`${LABEL} flex items-center gap-1.5`}>
                <input
                  type="checkbox"
                  checked={remember}
                  className="accent-accent"
                  onChange={(event) => {
                    setRemember(event.target.checked);
                    if (haveKey) keep({ remember: event.target.checked });
                  }}
                />
                Remember the key in this browser (otherwise this tab only)
              </label>
              {models.length > 0 || model ? (
                <div className="flex flex-wrap items-center gap-2">
                  <label className={LABEL} htmlFor="from-image-model">
                    Model
                  </label>
                  <select
                    id="from-image-model"
                    className={`${FIELD} min-w-0 flex-1`}
                    value={model}
                    onChange={(event) => {
                      setModel(event.target.value);
                      keep({ model: event.target.value });
                    }}
                  >
                    {(models.length > 0 ? models : [model]).map((item) => (
                      <option key={item} value={item}>
                        {item}
                      </option>
                    ))}
                  </select>
                  <label className={LABEL} htmlFor="from-image-rounds">
                    Critique rounds
                  </label>
                  <select
                    id="from-image-rounds"
                    className={FIELD}
                    value={critiques}
                    onChange={(event) => setCritiques(Number(event.target.value))}
                  >
                    {[0, 1, 2, 3].map((count) => (
                      <option key={count} value={count}>
                        {count}
                      </option>
                    ))}
                  </select>
                </div>
              ) : null}
              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  className={BUTTON}
                  disabled={busy || !haveModel || !loaded}
                  onClick={() => void recogniseWithLlm()}
                >
                  Read the picture
                </button>
                <button
                  type="button"
                  className={PRIMARY}
                  disabled={busy || !haveModel || !loaded}
                  onClick={() => void writeWithLlm()}
                >
                  Write a scene
                </button>
                {stoppable ? (
                  <button type="button" className={BUTTON} onClick={() => abort.current?.abort()}>
                    Stop
                  </button>
                ) : null}
              </div>
            </div>
          </details>

          <div aria-live="polite" className="min-h-5 text-xs">
            {status ? <p className="text-secondary">{status}</p> : null}
            {problem ? <p className="whitespace-pre-wrap text-magenta">{problem}</p> : null}
          </div>
        </div>
      </div>

      {tiles.length > 0 ? (
        <section aria-label="Candidates" className="border-t border-hairline p-4">
          <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
            {tiles.map((tile) => (
              <FlipTile key={tile.id} tile={tile} frame={frame} onPick={() => pick(tile.scene)} />
            ))}
          </div>
          {rejected.length > 0 ? (
            <p className="mt-3 text-xs text-label">Left out: {rejected.join("; ")}.</p>
          ) : null}
        </section>
      ) : null}
    </dialog>
  );
}
