/*
 * Playing a generated scene off screen (D560): one hidden engine, one scene at
 * a time, so a gallery of nine costs one WebGL context rather than nine. Each
 * candidate plays a few seconds of the synthetic track at a fixed step, which
 * gives both the gates' frames and a flipbook for its tile.
 */

import type { Scene } from "@/lib/play/files";
import { type Engine, loadEngine } from "@/lib/play/engine";
import { type GateResult, gate } from "@/lib/play/image/gate";

const WIDTH = 256;
const HEIGHT = 144;
const SCREEN_WIDTH = 64;
const SCREEN_HEIGHT = 36;
const RATE = 30;
const SECONDS = 3;
const TILE_FRAMES = 12;
const NO_MOUSE = new Float32Array(10);

export type Played = {
  result: GateResult;
  /** JPEG data URLs for the tile. */
  flipbook: string[];
};

export class Previewer {
  private engine: Engine | null = null;
  private readonly canvas = document.createElement("canvas");
  private readonly shrink: CanvasRenderingContext2D;
  private readonly tile: CanvasRenderingContext2D;

  constructor() {
    this.canvas.width = WIDTH;
    this.canvas.height = HEIGHT;
    const small = document.createElement("canvas");
    small.width = SCREEN_WIDTH;
    small.height = SCREEN_HEIGHT;
    const shrink = small.getContext("2d", { willReadFrequently: true });
    const tile = document.createElement("canvas");
    tile.width = WIDTH;
    tile.height = HEIGHT;
    const tileContext = tile.getContext("2d");
    if (!shrink || !tileContext) throw new Error("this browser refused a 2D canvas");
    this.shrink = shrink;
    this.tile = tileContext;
  }

  private async ready(): Promise<Engine> {
    if (this.engine) return this.engine;
    const bindings = await loadEngine();
    this.engine = new bindings.Engine(this.canvas, 48_000);
    this.engine.setAudioSource("synthetic");
    return this.engine;
  }

  /** Compiles for real: the engine's error text, or undefined if it plays. */
  async compile(scene: Scene): Promise<string | undefined> {
    const engine = await this.ready();
    try {
      engine.load(scene.title, scene.files, {}, "[]");
      return undefined;
    } catch (caught) {
      return String(caught);
    }
  }

  /** Plays what `compile` last loaded. */
  async play(): Promise<Played> {
    const engine = await this.ready();
    try {
      engine.restart();
    } catch {
      // A scene with no buffers has nothing to clear.
    }
    const count = RATE * SECONDS;
    const frames: Uint8ClampedArray[] = [];
    const flipbook: string[] = [];
    const every = Math.max(1, Math.floor(count / TILE_FRAMES));
    const dt = 1 / RATE;
    for (let index = 0; index < count; index += 1) {
      const time = 2 + index * dt;
      engine.frame(time, dt, index + 1, NO_MOUSE, Date.now() / 1000, WIDTH, HEIGHT);
      this.shrink.drawImage(this.canvas, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);
      frames.push(this.shrink.getImageData(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT).data);
      if (index % every === 0) {
        this.tile.drawImage(this.canvas, 0, 0);
        flipbook.push(this.tile.canvas.toDataURL("image/jpeg", 0.72));
      }
      // Let the page breathe: the visible scene keeps its own frames.
      if (index % 15 === 14) await new Promise((resolve) => setTimeout(resolve, 0));
    }
    return {
      result: gate({ width: SCREEN_WIDTH, height: SCREEN_HEIGHT, rate: RATE, frames }),
      flipbook,
    };
  }

  /** A frame of the playing scene at the size a model reads, as base64 JPEG. */
  snapshot(): string {
    this.tile.drawImage(this.canvas, 0, 0);
    return this.tile.canvas.toDataURL("image/jpeg", 0.85).replace(/^data:image\/jpeg;base64,/, "");
  }

  /** Gives the WebGL context back. */
  close() {
    const context = this.canvas.getContext("webgl2");
    this.engine?.free();
    this.engine = null;
    context?.getExtension("WEBGL_lose_context")?.loseContext();
  }
}
