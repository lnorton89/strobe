/*
 * The messages between the /play page and its script worker.
 *
 * A scene's script.js runs in a Web Worker rather than in Strobe's QuickJS
 * host, which is C and does not build for the browser's wasm target (D524).
 * The worker mirrors the host's frame order: write globals, dispatch events,
 * update(dt), harvest. The page applies what comes back — uniform values and
 * control writes — to the engine one frame later, which a worker's
 * asynchronous messages cannot avoid and a scene cannot see.
 */

export type ControlInfo = {
  name: string;
  /** Components: 1 for a slider, 2 for an xy pad, 3 for a colour. */
  dims: number;
  group?: string;
};

export type ConsoleLine = { level: "log" | "warn" | "error"; text: string };

export type ControlWrite =
  | { op: "set"; name: string; values: number[]; normalized: boolean }
  | { op: "default" | "randomize"; name: string };

export type ToWorker =
  | {
      type: "load";
      source: string;
      controls: ControlInfo[];
      scalars: string[];
    }
  | {
      type: "frame";
      dt: number;
      time: number;
      frame: number;
      audio: number[];
      controls: Record<string, number[]>;
      /** _mouse x, y, click x, click y; _click l, r, m; _muv; _muvc. */
      mouse: number[];
    };

export type FromWorker =
  | {
      type: "loaded";
      /** The uniforms setup() and the first update() created: [name, dims]. */
      uniforms: [string, number][];
      console: ConsoleLine[];
      error?: string;
    }
  | {
      type: "frame";
      /** Four floats per uniform, in the order "loaded" listed them. */
      uniforms: number[];
      writes: ControlWrite[];
      console: ConsoleLine[];
      error?: string;
    };
