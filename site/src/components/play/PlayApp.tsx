"use client";

/*
 * The shader sandbox (D524): Strobe's own scene player running in the page,
 * an editor beside it, the scene's controls under it.
 *
 * - Edits recompile after a pause in typing (or at once with Ctrl+Enter). A
 *   failed compile keeps the last good picture and marks the error's line.
 * - script.js runs in a Web Worker (lib/play/script-worker.ts), one frame
 *   behind the picture; a frame that does not come back within a second
 *   stops the script, as the native host's deadline does.
 * - Audio is the synthetic 120 BPM track, the microphone, a file, or silence.
 * - Nothing is stored on a server: the scene autosaves in this browser,
 *   shares as a link with the files in the URL's fragment, and exports as a
 *   .synScene zip the app opens.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import CodeEditor, { type LineError } from "@/components/play/CodeEditor";
import ControlsPanel from "@/components/play/ControlsPanel";
import { type LiveAudio, openFile, openMicrophone } from "@/lib/play/audio";
import { type ControlJson, type Engine, controlDims, loadEngine } from "@/lib/play/engine";
import {
  BLANK,
  type Scene,
  decodeShare,
  encodeShare,
  fileOrder,
  parseScene,
  readZip,
  validFileName,
  writeZip,
} from "@/lib/play/files";
import type { ConsoleLine, ControlInfo, FromWorker, ToWorker } from "@/lib/play/script-protocol";
import { workerMain } from "@/lib/play/script-worker";

export type LibraryScene = { slug: string; title: string };

type AudioMode = "synthetic" | "mic" | "file" | "silent";

const STORAGE_KEY = "strobe.play.scene";
const RECOMPILE_MS = 450;
const SCRIPT_DEADLINE_MS = 1000;
const MAX_CONSOLE = 200;

/** `main.glsl:12:5: message` and `main.glsl:12: message`, as the player and the worker write them. */
function lineErrors(error: string): Map<string, LineError[]> {
  const out = new Map<string, LineError[]>();
  for (const line of error.split("\n")) {
    // The player may put `pass N: ` before the location.
    const match = /([\w.-]+\.(?:glsl|frag|js|json)):(\d+)(?::(\d+))?:\s*(.*)$/.exec(line.trim());
    if (!match) continue;
    const [, file = "", row = "0", column, message = ""] = match;
    const list = out.get(file) ?? [];
    list.push({ line: Number(row), column: column ? Number(column) : undefined, message });
    out.set(file, list);
  }
  return out;
}

/** The controls scene.json declares, read leniently for the script worker. */
function declaredControls(json: string | undefined): ControlInfo[] {
  try {
    const parsed = JSON.parse(json ?? "{}") as { CONTROLS?: unknown };
    if (!Array.isArray(parsed.CONTROLS)) return [];
    return parsed.CONTROLS.flatMap((raw: unknown) => {
      const control = raw as { NAME?: unknown; TYPE?: unknown; UI_GROUP?: unknown };
      if (typeof control.NAME !== "string") return [];
      return [
        {
          name: control.NAME,
          dims: controlDims(typeof control.TYPE === "string" ? control.TYPE : "slider"),
          group: typeof control.UI_GROUP === "string" ? control.UI_GROUP : undefined,
        },
      ];
    });
  } catch {
    return [];
  }
}

/**
 * A fresh script worker, started from the source of `workerMain` itself
 * rather than a bundled worker URL, which a Turbopack production build emits
 * as an unbundled .ts file.
 */
function startScriptWorker(): Worker {
  const source = `(${workerMain.toString()})();`;
  const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
  try {
    return new Worker(url);
  } finally {
    URL.revokeObjectURL(url);
  }
}

function readStored(): Scene | undefined {
  try {
    const text = window.localStorage.getItem(STORAGE_KEY);
    return text ? parseScene(JSON.parse(text)) : undefined;
  } catch {
    return undefined;
  }
}

function store(scene: Scene) {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(scene));
  } catch {
    // Storage full or blocked: the share link and the zip still work.
  }
}

const BUTTON =
  "inline-flex items-center gap-1.5 border border-hairline-strong px-2.5 py-1.5 font-mono text-[11px] uppercase tracking-[0.1em] text-bright transition-colors hover:border-accent hover:text-accent-text disabled:opacity-40";
const SELECT =
  "border border-hairline-strong bg-ground px-2 py-1.5 font-mono text-[11px] uppercase tracking-[0.08em] text-bright";

export default function PlayApp({ library }: { library: LibraryScene[] }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const engine = useRef<Engine | null>(null);
  const scalarNames = useRef<string[]>([]);
  const [renderer, setRenderer] = useState("");
  const [fatal, setFatal] = useState<string | null>(null);

  const [scene, setScene] = useState<Scene | null>(null);
  const [active, setActive] = useState("main.glsl");
  const [error, setError] = useState<string | null>(null);
  const [scriptError, setScriptError] = useState<string | null>(null);
  const [controls, setControls] = useState<ControlJson[]>([]);
  const [consoleLines, setConsoleLines] = useState<ConsoleLine[]>([]);
  const [playing, setPlaying] = useState(true);
  const [autoCompile, setAutoCompile] = useState(true);
  const [audioMode, setAudioMode] = useState<AudioMode>("synthetic");
  const [notice, setNotice] = useState<string | null>(null);
  const [fps, setFps] = useState(0);

  const live = useRef<LiveAudio | null>(null);
  const worker = useRef<Worker | null>(null);
  const awaiting = useRef<number | null>(null);
  const controlsDirty = useRef(false);
  const clock = useRef({ time: 0, frame: 0, last: 0 });
  const playingRef = useRef(playing);
  playingRef.current = playing;
  const mouse = useRef({ x: 0, y: 0, cx: 0, cy: 0, held: [0, 0, 0], inside: false });
  const sceneRef = useRef<Scene | null>(null);
  sceneRef.current = scene;

  const log = useCallback((lines: ConsoleLine[]) => {
    if (lines.length === 0) return;
    setConsoleLines((old) => [...old, ...lines].slice(-MAX_CONSOLE));
  }, []);

  /* ---- the engine ------------------------------------------------------ */

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const bindings = await loadEngine();
        if (cancelled || !canvas.current) return;
        const made = new bindings.Engine(canvas.current, 48_000);
        engine.current = made;
        scalarNames.current = JSON.parse(bindings.Engine.scalarNames()) as string[];
        setRenderer(made.renderer());
      } catch (caught) {
        setFatal(String(caught));
        return;
      }
      const fromHash = await decodeShare(window.location.hash).catch(() => undefined);
      const slug = new URLSearchParams(window.location.search).get("scene");
      let first = fromHash;
      if (!first && slug) first = await fetchScene(slug).catch(() => undefined);
      first ??= readStored() ?? BLANK;
      if (!cancelled) setScene(first);
    })();
    return () => {
      cancelled = true;
      engine.current?.free();
      engine.current = null;
    };
  }, []);

  /* ---- compiling -------------------------------------------------------- */

  const stopScript = useCallback(() => {
    worker.current?.terminate();
    worker.current = null;
    awaiting.current = null;
  }, []);

  const compile = useCallback(
    async (next: Scene) => {
      const made = engine.current;
      if (!made) return;
      stopScript();
      setScriptError(null);
      let extras: [string, number][] = [];
      let script: Worker | null = null;
      const source = next.files["script.js"];
      if (source !== undefined) {
        script = startScriptWorker();
        const loaded = await new Promise<Extract<FromWorker, { type: "loaded" }> | null>(
          (resolve) => {
            const timer = window.setTimeout(() => resolve(null), SCRIPT_DEADLINE_MS * 2);
            script?.addEventListener(
              "message",
              (event: MessageEvent<FromWorker>) => {
                window.clearTimeout(timer);
                resolve(event.data.type === "loaded" ? event.data : null);
              },
              { once: true },
            );
            const message: ToWorker = {
              type: "load",
              source,
              controls: declaredControls(next.files["scene.json"]),
              scalars: scalarNames.current,
            };
            script?.postMessage(message);
          },
        );
        if (!loaded) {
          script.terminate();
          script = null;
          setScriptError("script.js: setup() did not return; the script is stopped");
        } else {
          log(loaded.console);
          extras = loaded.uniforms;
          if (loaded.error) setScriptError(loaded.error);
        }
      }
      try {
        const json = made.load(next.title, next.files, JSON.stringify(extras));
        setControls(JSON.parse(json) as ControlJson[]);
        setError(null);
        if (script) {
          script.onmessage = (event: MessageEvent<FromWorker>) => {
            const data = event.data;
            if (data.type !== "frame" || !engine.current) return;
            awaiting.current = null;
            engine.current.setScriptUniforms(new Float32Array(data.uniforms));
            for (const write of data.writes) {
              if (write.op === "set") {
                const values = new Float32Array(write.values);
                if (write.normalized) engine.current.setControlNormalized(write.name, values);
                else engine.current.setControl(write.name, values);
              } else if (write.op === "default") {
                engine.current.defaultControl(write.name);
              }
            }
            // The panel catches up with script writes at most ten times a
            // second, from the frame loop: a script that writes every frame
            // would otherwise re-render the page every frame.
            if (data.writes.length > 0) controlsDirty.current = true;
            log(data.console);
            setScriptError(data.error ?? null);
          };
          worker.current = script;
        }
      } catch (caught) {
        script?.terminate();
        setError(String(caught));
      }
    },
    [log, stopScript],
  );

  // Compile a newly opened scene at once, an edit after a pause.
  const opened = useRef<Scene | null>(null);
  useEffect(() => {
    if (!scene) return;
    store(scene);
    const fresh = opened.current === null || opened.current.title !== scene.title;
    if (fresh || !autoCompile) {
      if (fresh) {
        opened.current = scene;
        void compile(scene);
      }
      return;
    }
    const timer = window.setTimeout(() => void compile(scene), RECOMPILE_MS);
    return () => window.clearTimeout(timer);
  }, [scene, autoCompile, compile]);

  /* ---- the frame loop --------------------------------------------------- */

  useEffect(() => {
    let raf = 0;
    let frames = 0;
    let since = performance.now();
    let panelAt = 0;
    const tick = (now: number) => {
      raf = requestAnimationFrame(tick);
      const made = engine.current;
      const element = canvas.current;
      if (!made || !element) return;
      const c = clock.current;
      const dt = c.last === 0 ? 1 / 60 : Math.min((now - c.last) / 1000, 0.1);
      c.last = now;
      if (!playingRef.current) return;
      c.time += dt;
      c.frame += 1;

      if (live.current) made.pushSamples(live.current.drain());
      const m = mouse.current;
      const sign = m.held.some((h) => h > 0) ? 1 : -1;
      const w = element.width;
      const h = element.height;
      const mouseValues = [
        m.x,
        m.y,
        m.cx * sign,
        m.cy * sign,
        ...m.held,
        m.x / w,
        m.y / h,
        (m.x - 0.5 * w) / h,
        (m.y - 0.5 * h) / h,
      ];
      made.frame(c.time, dt, c.frame, new Float32Array(mouseValues), Date.now() / 1000, w, h);

      const script = worker.current;
      if (script) {
        if (awaiting.current === null) {
          awaiting.current = now;
          const message: ToWorker = {
            type: "frame",
            dt,
            time: c.time,
            frame: c.frame,
            audio: Array.from(made.audioScalars()),
            controls: JSON.parse(made.controlOutputsJson()) as Record<string, number[]>,
            mouse: mouseValues,
          };
          script.postMessage(message);
        } else if (now - awaiting.current > SCRIPT_DEADLINE_MS) {
          script.terminate();
          worker.current = null;
          awaiting.current = null;
          setScriptError(
            `script.js: a frame ran past ${SCRIPT_DEADLINE_MS} ms; the script is stopped until the next compile`,
          );
        }
      }

      if (controlsDirty.current && now - panelAt >= 100) {
        controlsDirty.current = false;
        panelAt = now;
        setControls(JSON.parse(made.controlsJson()) as ControlJson[]);
      }

      frames += 1;
      if (now - since >= 1000) {
        setFps(Math.round((frames * 1000) / (now - since)));
        frames = 0;
        since = now;
      }
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  // The canvas's pixels follow its box, at up to twice the CSS size.
  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const observer = new ResizeObserver(() => {
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const width = Math.max(1, Math.round(element.clientWidth * ratio));
      const height = Math.max(1, Math.round(element.clientHeight * ratio));
      if (element.width === width && element.height === height) return;
      element.width = width;
      element.height = height;
      try {
        engine.current?.resize(width, height);
      } catch (caught) {
        setError(String(caught));
      }
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  /* ---- audio ------------------------------------------------------------ */

  const chooseAudio = useCallback(async (mode: AudioMode, file?: File) => {
    const made = engine.current;
    if (!made) return;
    live.current?.stop();
    live.current = null;
    try {
      if (mode === "mic" || mode === "file") {
        const opened = mode === "mic" ? await openMicrophone() : file ? await openFile(file) : null;
        if (!opened) return;
        live.current = opened;
        made.setSampleRate(opened.sampleRate);
        made.setAudioSource("live");
      } else {
        made.setAudioSource(mode);
      }
      setAudioMode(mode);
    } catch (caught) {
      made.setAudioSource("synthetic");
      setAudioMode("synthetic");
      setNotice(`Audio: ${String(caught)}`);
    }
  }, []);

  useEffect(() => () => live.current?.stop(), []);

  /* ---- scene actions ---------------------------------------------------- */

  async function fetchScene(slug: string): Promise<Scene> {
    const response = await fetch(`/generated/play/${encodeURIComponent(slug)}.json`);
    if (!response.ok) throw new Error(`no scene ${slug}`);
    const parsed = parseScene(await response.json());
    if (!parsed) throw new Error(`scene ${slug} has no main.glsl`);
    return parsed;
  }

  const open = useCallback((next: Scene) => {
    opened.current = null;
    clock.current.time = 0;
    clock.current.frame = 0;
    setConsoleLines([]);
    setActive("main.glsl");
    setScene(next);
    if (window.location.hash) {
      history.replaceState(null, "", window.location.pathname + window.location.search);
    }
  }, []);

  const edit = useCallback((file: string, text: string) => {
    setScene((old) => (old ? { ...old, files: { ...old.files, [file]: text } } : old));
  }, []);

  const setControl = useCallback((name: string, values: number[]) => {
    const made = engine.current;
    if (!made) return;
    made.setControl(name, new Float32Array(values));
    setControls(JSON.parse(made.controlsJson()) as ControlJson[]);
  }, []);

  async function share() {
    if (!scene) return;
    const fragment = await encodeShare(scene);
    const url = `${window.location.origin}${window.location.pathname}#${fragment}`;
    history.replaceState(null, "", `#${fragment}`);
    try {
      await navigator.clipboard.writeText(url);
      setNotice(`Link copied (${Math.round(url.length / 1024)} kB). The scene is in the link itself.`);
    } catch {
      setNotice("The link is in the address bar.");
    }
  }

  function exportZip() {
    if (!scene) return;
    const link = document.createElement("a");
    link.href = URL.createObjectURL(writeZip(scene));
    link.download = `${scene.title.replace(/[^\w.-]+/g, "-") || "untitled"}.synScene.zip`;
    link.click();
    window.setTimeout(() => URL.revokeObjectURL(link.href), 1000);
  }

  async function importZip(file: File) {
    try {
      open(await readZip(await file.arrayBuffer()));
    } catch (caught) {
      setNotice(`Import: ${String(caught)}`);
    }
  }

  function addFile() {
    if (!scene) return;
    const name = window.prompt("File name (script.js, or a .glsl for PASS_SOURCES)", "script.js");
    if (!name) return;
    if (!validFileName(name) || scene.files[name] !== undefined) {
      setNotice(`${name}: not a new .glsl, .json or .js name`);
      return;
    }
    const starter =
      name === "script.js"
        ? "function setup() {\n}\n\nfunction update(dt) {\n  // setUniform(\"pulse\", Math.sin(TIME));\n}\n"
        : "vec4 renderMain() {\n    return vec4(_uv, 0.5, 1.0);\n}\n";
    setScene({ ...scene, files: { ...scene.files, [name]: starter } });
    setActive(name);
  }

  function removeFile(name: string) {
    if (!scene || name === "main.glsl") return;
    if (!window.confirm(`Remove ${name}?`)) return;
    const files = { ...scene.files };
    delete files[name];
    setScene({ ...scene, files });
    setActive("main.glsl");
  }

  function rewind() {
    clock.current.time = 0;
    clock.current.frame = 0;
    try {
      engine.current?.restart();
    } catch (caught) {
      setError(String(caught));
    }
  }

  // Ctrl/Cmd+Enter compiles now.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key === "Enter" && sceneRef.current) {
        event.preventDefault();
        void compile(sceneRef.current);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [compile]);

  const errorsByFile = useMemo(() => lineErrors([error, scriptError].filter(Boolean).join("\n")), [
    error,
    scriptError,
  ]);
  const files = scene ? fileOrder(scene.files) : [];

  /* ---- pointer ---------------------------------------------------------- */

  function pointer(event: React.PointerEvent<HTMLCanvasElement>, kind: "move" | "down" | "up") {
    const element = event.currentTarget;
    const box = element.getBoundingClientRect();
    const x = ((event.clientX - box.left) / box.width) * element.width;
    const y = (1 - (event.clientY - box.top) / box.height) * element.height;
    const m = mouse.current;
    m.x = x;
    m.y = y;
    const slot = event.button === 2 ? 1 : event.button === 1 ? 2 : 0;
    if (kind === "down") {
      element.setPointerCapture(event.pointerId);
      m.held[slot] = 1;
      m.cx = x;
      m.cy = y;
    } else if (kind === "up") {
      m.held[slot] = 0;
    }
  }

  /* ---- layout ----------------------------------------------------------- */

  if (fatal) {
    return (
      <div className="mx-auto max-w-2xl px-6 py-24">
        <h1 className="font-display text-2xl text-text">The sandbox could not start</h1>
        <p className="mt-4 text-secondary">
          It needs WebGL2 and WebAssembly. The browser said:
        </p>
        <pre className="mt-3 whitespace-pre-wrap border border-hairline bg-panel p-4 font-mono text-sm text-bright">
          {fatal}
        </pre>
      </div>
    );
  }

  return (
    <div className="grid min-h-[calc(100dvh-4rem)] grid-cols-1 border-t border-hairline lg:h-[calc(100dvh-4rem)] lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)]">
      {/* The picture, its transport, its controls. */}
      <section className="flex min-h-0 flex-col border-hairline lg:border-r" aria-label="Preview">
        <div className="flex flex-wrap items-center gap-2 border-b border-hairline px-3 py-2">
          <label className="sr-only" htmlFor="play-scene">
            Open a scene
          </label>
          <select
            id="play-scene"
            className={SELECT}
            value=""
            onChange={(event) => {
              const slug = event.target.value;
              if (slug === "") return;
              if (slug === "-new") open({ ...BLANK, files: { ...BLANK.files } });
              else void fetchScene(slug).then(open, (e) => setNotice(String(e)));
            }}
          >
            <option value="">Open…</option>
            <option value="-new">New scene</option>
            <optgroup label="Library">
              {library.map((item) => (
                <option key={item.slug} value={item.slug}>
                  {item.title}
                </option>
              ))}
            </optgroup>
          </select>
          <button type="button" className={BUTTON} onClick={() => setPlaying((p) => !p)}>
            {playing ? "Pause" : "Play"}
          </button>
          <button type="button" className={BUTTON} onClick={rewind}>
            Rewind
          </button>
          <label className="sr-only" htmlFor="play-audio">
            Audio source
          </label>
          <select
            id="play-audio"
            className={SELECT}
            value={audioMode}
            onChange={(event) => {
              const mode = event.target.value as AudioMode;
              if (mode === "file") document.getElementById("play-audio-file")?.click();
              else void chooseAudio(mode);
            }}
          >
            <option value="synthetic">Audio: 120 BPM test</option>
            <option value="mic">Audio: microphone</option>
            <option value="file">Audio: a file…</option>
            <option value="silent">Audio: silent</option>
          </select>
          <input
            id="play-audio-file"
            type="file"
            accept="audio/*"
            className="hidden"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) void chooseAudio("file", file);
              event.target.value = "";
            }}
          />
          <button
            type="button"
            className={BUTTON}
            onClick={() => void canvas.current?.requestFullscreen()}
          >
            Fullscreen
          </button>
          <span className="ml-auto font-mono text-[11px] text-label" title={renderer}>
            {fps} fps
          </span>
        </div>
        <div className="relative aspect-video w-full bg-black lg:aspect-auto lg:min-h-0 lg:flex-3">
          <canvas
            ref={canvas}
            className="absolute inset-0 size-full touch-none"
            onPointerMove={(e) => pointer(e, "move")}
            onPointerDown={(e) => pointer(e, "down")}
            onPointerUp={(e) => pointer(e, "up")}
            onContextMenu={(e) => e.preventDefault()}
          />
          {!scene ? (
            <p className="absolute inset-0 grid place-items-center font-mono text-xs text-label">
              Loading the engine…
            </p>
          ) : null}
        </div>
        <div className="min-h-0 overflow-y-auto border-t border-hairline lg:flex-2">
          <h2 className="border-b border-hairline px-4 py-2 font-mono text-[11px] uppercase tracking-widest text-bright">
            Controls
          </h2>
          <ControlsPanel controls={controls} set={setControl} />
        </div>
      </section>

      {/* The files. */}
      <section className="flex min-h-[70vh] flex-col lg:min-h-0" aria-label="Editor">
        <div className="flex flex-wrap items-center gap-2 border-b border-hairline px-3 py-2">
          <label className="sr-only" htmlFor="play-title">
            Scene name
          </label>
          <input
            id="play-title"
            value={scene?.title ?? ""}
            onChange={(event) => {
              const title = event.target.value.slice(0, 80);
              opened.current = scene ? { ...scene, title } : null;
              setScene((old) => (old ? { ...old, title } : old));
            }}
            className="min-w-0 flex-1 border border-transparent bg-transparent px-1 py-1 font-display text-base text-text hover:border-hairline focus:border-accent focus:outline-none"
          />
          <button type="button" className={BUTTON} onClick={() => scene && void compile(scene)}>
            Compile
          </button>
          <label className="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.08em] text-label">
            <input
              type="checkbox"
              checked={autoCompile}
              onChange={(event) => setAutoCompile(event.target.checked)}
              className="accent-accent"
            />
            Auto
          </label>
          <button type="button" className={BUTTON} onClick={() => void share()}>
            Share
          </button>
          <button type="button" className={BUTTON} onClick={exportZip}>
            Export
          </button>
          <label className={`${BUTTON} cursor-pointer`}>
            Import
            <input
              type="file"
              accept=".zip"
              className="hidden"
              onChange={(event) => {
                const file = event.target.files?.[0];
                if (file) void importZip(file);
                event.target.value = "";
              }}
            />
          </label>
        </div>
        <div className="flex items-stretch overflow-x-auto border-b border-hairline" role="tablist">
          {files.map((name) => {
            const broken = (errorsByFile.get(name)?.length ?? 0) > 0;
            return (
              <div
                key={name}
                className={`flex items-center border-r border-hairline ${name === active ? "bg-panel" : ""}`}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={name === active}
                  onClick={() => setActive(name)}
                  className={`px-3 py-2 font-mono text-xs ${name === active ? "text-accent-text" : "text-secondary hover:text-text"}`}
                >
                  {name}
                  {broken ? <span className="ml-1.5 text-magenta">●</span> : null}
                </button>
                {name !== "main.glsl" ? (
                  <button
                    type="button"
                    aria-label={`Remove ${name}`}
                    onClick={() => removeFile(name)}
                    className="pr-2 text-xs text-mute hover:text-magenta"
                  >
                    ×
                  </button>
                ) : null}
              </div>
            );
          })}
          <button
            type="button"
            onClick={addFile}
            aria-label="Add a file"
            className="px-3 font-mono text-sm text-label hover:text-accent-text"
          >
            +
          </button>
        </div>
        <div className="min-h-0 flex-1">
          {scene ? (
            <CodeEditor
              file={active}
              text={scene.files[active] ?? ""}
              errors={errorsByFile.get(active) ?? []}
              onChange={edit}
            />
          ) : null}
        </div>
        <div
          className="max-h-48 min-h-16 overflow-y-auto border-t border-hairline bg-panel px-4 py-2 font-mono text-xs"
          aria-live="polite"
        >
          {notice ? (
            <p className="flex justify-between gap-3 text-accent-text">
              <span>{notice}</span>
              <button type="button" onClick={() => setNotice(null)} aria-label="Dismiss">
                ×
              </button>
            </p>
          ) : null}
          {error ? <pre className="whitespace-pre-wrap text-magenta">{error}</pre> : null}
          {scriptError ? (
            <pre className="whitespace-pre-wrap text-magenta">{scriptError}</pre>
          ) : null}
          {!error && !scriptError ? (
            <p className="text-label">
              Compiled · {renderer || "starting"} · Ctrl+Enter compiles now
            </p>
          ) : null}
          {consoleLines.map((line, index) => (
            <p
              // Lines are appended and never reordered.
               
              key={index}
              className={
                line.level === "error"
                  ? "text-magenta"
                  : line.level === "warn"
                    ? "text-[#f5c26b]"
                    : "text-secondary"
              }
            >
              {line.text}
            </p>
          ))}
        </div>
      </section>
    </div>
  );
}
