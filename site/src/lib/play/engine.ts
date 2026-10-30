/*
 * Loading the /play page's engine: crates/strobe-web, built by
 * scripts/build-engine.mjs into public/generated/engine/ (D524).
 *
 * The module is fetched at run time rather than bundled: it is a build
 * product of the Rust workspace, not a source file of the site.
 */

/** The wasm-bindgen class, as crates/strobe-web/src/web.rs declares it. */
export interface Engine {
  renderer(): string;
  load(title: string, files: Record<string, string>, extras: string): string;
  controlsJson(): string;
  controlOutputsJson(): string;
  extrasJson(): string;
  setControl(name: string, values: Float32Array): boolean;
  setControlNormalized(name: string, values: Float32Array): boolean;
  defaultControl(name: string): boolean;
  setScriptUniforms(flat: Float32Array): void;
  setAudioSource(name: "synthetic" | "live" | "silent"): boolean;
  pushSamples(samples: Float32Array): void;
  setSampleRate(rate: number): void;
  audioScalars(): Float32Array;
  resize(width: number, height: number): void;
  restart(): void;
  frame(
    time: number,
    dt: number,
    frame: number,
    mouse: Float32Array,
    unixSeconds: number,
    width: number,
    height: number,
  ): void;
  free(): void;
}

type EngineModule = {
  default: (options?: { module_or_path?: string }) => Promise<unknown>;
  Engine: (new (canvas: HTMLCanvasElement, sampleRate: number) => Engine) & {
    scalarNames(): string;
  };
};

const BASE = "/generated/engine/";

let loading: Promise<EngineModule> | undefined;

/** The engine module, initialised once per page. */
export function loadEngine(): Promise<EngineModule> {
  loading ??= (async () => {
    const bindings = (await import(
      /* webpackIgnore: true */ /* turbopackIgnore: true */ `${BASE}strobe_web.js`
    )) as EngineModule;
    await bindings.default({ module_or_path: `${BASE}strobe_web_bg.wasm` });
    return bindings;
  })();
  return loading;
}

/** A control as the engine describes it: its scene.json form plus VALUE. */
export type ControlJson = {
  NAME: string;
  TYPE: string;
  MIN?: number | number[];
  MAX?: number | number[];
  DEFAULT?: number | number[];
  VALUE?: number[];
  LABELS?: string[];
  VALUES?: number[];
  UI_GROUP?: string;
  DESCRIPTION?: string;
};

/** Components a control's TYPE carries. */
export function controlDims(type: string): number {
  const base = type.split(/[\s_-]/)[0]?.toLowerCase() ?? "";
  if (base === "xy") return 2;
  if (base === "color" || base === "colour") return 3;
  return 1;
}
