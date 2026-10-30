/*
 * The script host for the /play page: a shim of Strobe's script API in a
 * Web Worker (D524).
 *
 * The worker runs nothing but the scene's own script, so a script cannot
 * touch the page, and a script that never returns is stopped by the page
 * terminating the worker — the browser's stand-in for the native host's
 * per-frame deadline.
 *
 * The API follows crates/strobe-script: TIME, FRAMECOUNT, every syn_* scalar
 * and every control as globals (a vector as one object whose fields are
 * overwritten each frame), _mouse/_click/_muv/_muvc, setUniform, setControl,
 * setControlNormalized, defaultControl, randomizeControl, defaultGroup,
 * randomizeGroup, the five event registrations, the deprecated `uniforms`
 * object, console/print and printGlobalKeys. textureFinalPass answers black:
 * the browser engine does not read the picture back yet (backlog W-01).
 */

import type {
  ConsoleLine,
  ControlInfo,
  ControlWrite,
  FromWorker,
  ToWorker,
} from "./script-protocol";

type Vector = Record<string, number>;
type Callback = string | ((...args: unknown[]) => unknown);
type Handler = {
  kind: "change" | "offToOn" | "onToOff" | "whileOn" | "whileOff";
  target: string;
  callback: Callback;
  previous?: number;
};

/**
 * The whole worker. It runs from a Blob of its own source
 * (`(${workerMain})()`), so it must not reach anything outside this
 * function: only types (erased) live at module level. Starting it this way
 * needs no bundler support for workers, which Turbopack builds lack.
 */
export function workerMain(): void {
  const scope = self as unknown as Record<string, unknown> & {
    postMessage(message: FromWorker): void;
  };

  const KEYS: [string[], string[]] = [
    ["x", "y", "z", "w"],
    ["r", "g", "b", "a"],
  ];

  let controls: ControlInfo[] = [];
  let scalars: string[] = [];
  let consoleLines: ConsoleLine[] = [];
  let writes: ControlWrite[] = [];
  const handlers: Handler[] = [];
  /** Uniform values staged this frame, by name. */
  let staged = new Map<string, number[]>();
  /** The uniforms the scene was built with, in order; sealed after load. */
  let declared: [string, number][] = [];
  let sealed = false;
  let faults = 0;
  const warned = new Set<string>();

  function say(level: ConsoleLine["level"], args: unknown[]) {
    const text = args
      .map((arg) => {
        if (typeof arg === "string") return arg;
        try {
          return JSON.stringify(arg);
        } catch {
          return String(arg);
        }
      })
      .join(" ");
    if (consoleLines.length < 200) consoleLines.push({ level, text });
  }

  /** An error once per distinct message, as the native host logs it. */
  function once(text: string) {
    if (warned.has(text)) return;
    warned.add(text);
    say("error", [text]);
  }

  function finite(value: unknown): number {
    const n = typeof value === "boolean" ? Number(value) : Number(value);
    return Number.isFinite(n) && Math.abs(n) <= 3.4028234663852886e38 ? n : 0;
  }

  /** Loose numbers, one array, or one {x,y,z,w} / {r,g,b,a} object. */
  function components(args: unknown[], max: number, truncate: boolean): number[] {
    let list: unknown[] = args;
    const [first] = args;
    if (args.length === 1 && first !== null && typeof first === "object") {
      if (Array.isArray(first) || ArrayBuffer.isView(first)) {
        list = Array.from(first as ArrayLike<unknown>);
      } else {
        const object = first as Vector;
        const keys = KEYS.find((names) => names[0] in object) ?? KEYS[0];
        list = keys.filter((key) => key in object).map((key) => object[key]);
      }
    }
    if (list.length === 0) throw new TypeError("expected at least one number");
    if (list.length > max && !truncate) throw new TypeError(`expected at most ${max} numbers`);
    return list.slice(0, max).map(finite);
  }

  const RESERVED = new Set(["TIME", "FRAMECOUNT", "RENDERSIZE", "PASSINDEX"]);

  function setUniform(name: unknown, ...values: unknown[]) {
    if (typeof name !== "string" || name.length === 0) {
      throw new TypeError("setUniform: the name must be a string");
    }
    if (RESERVED.has(name) || scalars.includes(name) || controls.some((c) => c.name === name)) {
      throw new TypeError(`setUniform: ${name} is a built-in uniform or a control`);
    }
    const numbers = components(values, 4, false);
    if (sealed && !declared.some(([known]) => known === name)) {
      once(`setUniform("${name}"): uniforms must be created in setup() or the first update()`);
      return;
    }
    staged.set(name, numbers);
  }

  function findControl(fn: string, name: unknown): ControlInfo | undefined {
    if (typeof name !== "string") throw new TypeError(`${fn}: the name must be a string`);
    const control = controls.find((c) => c.name === name);
    if (!control) once(`${fn}: no control named "${name}"`);
    return control;
  }

  function setter(fn: string, normalized: boolean) {
    return (name: unknown, ...values: unknown[]) => {
      const control = findControl(fn, name);
      if (!control) return;
      // An options object ({ dimension, bank }) as the last argument is read
      // by the native host; the sandbox has no meta bank and sets all
      // components unless one dimension is named.
      const last = values.at(-1);
      let dimension: number | undefined;
      if (values.length > 1 && last !== null && typeof last === "object" && !Array.isArray(last)) {
        const options = last as { dimension?: unknown };
        if (options.dimension !== undefined) dimension = Math.floor(finite(options.dimension));
        values = values.slice(0, -1);
      }
      let numbers = components(values, control.dims, true);
      if (dimension !== undefined) {
        const current = currentControl(control);
        numbers = current.map((value, i) => (i === dimension ? (numbers[0] ?? value) : value));
      }
      writes.push({ op: "set", name: control.name, values: numbers, normalized });
    };
  }

  function operator(fn: string, op: "default" | "randomize") {
    return (name: unknown) => {
      const control = findControl(fn, name);
      if (control) writes.push({ op, name: control.name });
    };
  }

  function groupOperator(fn: string, op: "default" | "randomize") {
    return (group: unknown) => {
      if (typeof group !== "string") throw new TypeError(`${fn}: the group must be a string`);
      const members = controls.filter((c) => c.group === group);
      if (members.length === 0) once(`${fn}: no group named "${group}"`);
      for (const control of members) writes.push({ op, name: control.name });
    };
  }

  function register(kind: Handler["kind"]) {
    return (target: unknown, callback: unknown) => {
      if (typeof target !== "string") throw new TypeError("the input's name must be a string");
      if (typeof callback !== "string" && typeof callback !== "function") {
        throw new TypeError("the callback must be a function or its name");
      }
      handlers.push({ kind, target, callback: callback as Callback });
    };
  }

  /** A global's numeric value for an event: a vector's first component. */
  function inputValue(name: string): number | undefined {
    const value = scope[name];
    if (typeof value === "number") return value;
    if (value !== null && typeof value === "object") {
      const vector = value as Vector;
      return vector.x ?? vector.r;
    }
    return undefined;
  }

  function fires(kind: Handler["kind"], previous: number, current: number): boolean {
    const on = (v: number) => v >= 0.5;
    switch (kind) {
      case "change":
        return previous !== current && !(Number.isNaN(previous) && Number.isNaN(current));
      case "offToOn":
        return !on(previous) && on(current);
      case "onToOff":
        return on(previous) && !on(current);
      case "whileOn":
        return on(current);
      case "whileOff":
        return !on(current);
    }
  }

  function call(callback: Callback) {
    if (typeof callback === "function") {
      callback();
      return;
    }
    const fn = scope[callback];
    if (typeof fn === "function") (fn as () => void)();
    else once(`no function named "${callback}" to call`);
  }

  function dispatch() {
    for (const handler of handlers) {
      const current = inputValue(handler.target);
      if (current === undefined) {
        once(`nothing named "${handler.target}" to watch`);
        continue;
      }
      // A first frame has no previous value and takes the current one, so a
      // change or an edge does not fire just because the scene started.
      const previous = handler.previous ?? current;
      handler.previous = current;
      if (fires(handler.kind, previous, current)) call(handler.callback);
    }
  }

  /** The current output of a control, from its global. */
  function currentControl(control: ControlInfo): number[] {
    const value = scope[control.name];
    if (typeof value === "number") return [value];
    const vector = (value ?? {}) as Vector;
    const keys = control.dims === 3 ? ["r", "g", "b"] : KEYS[0];
    return keys.slice(0, control.dims).map((key) => vector[key] ?? 0);
  }

  /** A vector global, created once and overwritten in place each frame. */
  function vector(name: string, keys: string[]): Vector {
    let object = scope[name] as Vector | undefined;
    if (object === undefined || typeof object !== "object") {
      object = Object.fromEntries(keys.map((key) => [key, 0]));
      scope[name] = object;
    }
    return object;
  }

  function writeVector(name: string, values: number[], keys: string[], aliases?: string[]) {
    const object = vector(name, [...keys, ...(aliases ?? [])]);
    values.forEach((value, i) => {
      const key = keys[i];
      if (key !== undefined) object[key] = value;
      const alias = aliases?.[i];
      if (alias !== undefined) object[alias] = value;
    });
  }

  function writeGlobals(message: Extract<ToWorker, { type: "frame" }>) {
    scope.TIME = message.time;
    scope.FRAMECOUNT = message.frame;
    scalars.forEach((name, i) => {
      scope[name] = message.audio[i] ?? 0;
    });
    for (const control of controls) {
      const values = message.controls[control.name] ?? [];
      if (control.dims === 1) scope[control.name] = values[0] ?? 0;
      else if (control.dims === 3) writeVector(control.name, values, ["r", "g", "b"], ["x", "y", "z"]);
      else writeVector(control.name, values, KEYS[0].slice(0, control.dims));
    }
    const m = message.mouse;
    writeVector("_mouse", m.slice(0, 4), KEYS[0]);
    writeVector("_click", m.slice(4, 7), ["x", "y", "z"]);
    writeVector("_muv", m.slice(7, 9), ["x", "y"]);
    writeVector("_muvc", m.slice(9, 11), ["x", "y"]);
  }

  /** The deprecated `uniforms` object: every property is a setUniform. */
  function harvestUniformsObject() {
    const object = scope.uniforms;
    if (object === null || typeof object !== "object") return;
    for (const [name, value] of Object.entries(object as Record<string, unknown>)) {
      setUniform(name, value);
    }
  }

  function install() {
    Object.assign(scope, {
      setUniform,
      setControl: setter("setControl", false),
      setControlNormalized: setter("setControlNormalized", true),
      defaultControl: operator("defaultControl", "default"),
      randomizeControl: operator("randomizeControl", "randomize"),
      defaultGroup: groupOperator("defaultGroup", "default"),
      randomizeGroup: groupOperator("randomizeGroup", "randomize"),
      onChange: register("change"),
      onOffToOn: register("offToOn"),
      onOnToOff: register("onToOff"),
      whileOn: register("whileOn"),
      whileOff: register("whileOff"),
      textureFinalPass: () => [0, 0, 0],
      print: (...args: unknown[]) => say("log", args),
      printGlobalKeys: () =>
        say("log", [
          Object.keys(scope)
            .filter((key) => !key.startsWith("on") || key.length < 3)
            .sort()
            .join(" "),
        ]),
    });
    scope.console = {
      log: (...args: unknown[]) => say("log", args),
      info: (...args: unknown[]) => say("log", args),
      debug: (...args: unknown[]) => say("log", args),
      warn: (...args: unknown[]) => say("warn", args),
      error: (...args: unknown[]) => say("error", args),
    };
  }

  function describe(error: unknown): string {
    if (error instanceof Error) {
      const line = /<anonymous>:(\d+)/.exec(error.stack ?? "")?.[1];
      return line ? `script.js:${line}: ${error.message}` : `script.js: ${error.message}`;
    }
    return `script.js: ${String(error)}`;
  }

  function flatten(): number[] {
    const out: number[] = [];
    for (const [name] of declared) {
      const values = staged.get(name) ?? lastValues.get(name) ?? [];
      lastValues.set(name, values);
      out.push(values[0] ?? 0, values[1] ?? 0, values[2] ?? 0, values[3] ?? 0);
    }
    return out;
  }
  /** The last committed value of each uniform, kept through a faulted frame. */
  const lastValues = new Map<string, number[]>();

  function runFrame(message: Extract<ToWorker, { type: "frame" }>): string | undefined {
    writes = [];
    staged = new Map();
    writeGlobals(message);
    try {
      dispatch();
      const update = scope.update;
      if (typeof update === "function") (update as (dt: number) => void)(message.dt);
      harvestUniformsObject();
      faults = 0;
      return undefined;
    } catch (error) {
      faults += 1;
      staged = new Map();
      writes = [];
      return describe(error);
    }
  }

  scope.onmessage = (event: MessageEvent<ToWorker>) => {
    const message = event.data;
    consoleLines = [];
    if (message.type === "load") {
      controls = message.controls;
      scalars = message.scalars;
      install();
      let error: string | undefined;
      try {
        // Indirect eval: the script's top-level declarations become globals,
        // as they are in the native host.
        (0, eval)(message.source);
        const setup = scope.setup;
        if (typeof setup === "function") (setup as () => void)();
      } catch (caught) {
        error = describe(caught);
      }
      if (error === undefined) {
        error = runFrame({
          type: "frame",
          dt: 0,
          time: 0,
          frame: 0,
          audio: [],
          controls: {},
          mouse: [],
        });
      }
      declared = [...staged].map(([name, values]) => [name, values.length]);
      for (const [name, values] of staged) lastValues.set(name, values);
      sealed = true;
      // A fault in the load's own update is reported with "loaded"; the
      // three-in-a-row count starts with the first real frame.
      faults = 0;
      scope.postMessage({ type: "loaded", uniforms: declared, console: consoleLines, error });
      return;
    }
    // Three faulted frames in a row disable the script until it is reloaded.
    if (faults >= 3) {
      scope.postMessage({ type: "frame", uniforms: flatten(), writes: [], console: [] });
      return;
    }
    const error = runFrame(message);
    if (faults >= 3) say("error", ["the script failed three frames in a row and is disabled"]);
    scope.postMessage({
      type: "frame",
      uniforms: flatten(),
      writes,
      console: consoleLines,
      error,
    });
  };
}
