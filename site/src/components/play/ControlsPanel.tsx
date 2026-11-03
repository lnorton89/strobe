"use client";

/*
 * The scene's controls, drawn from the engine's description of them: the
 * scene.json types the app's own panel draws, grouped by UI_GROUP. A change
 * goes straight to the engine, which eases it by the control's PARAMS as the
 * app does.
 */

import { useRef, type PointerEvent } from "react";
import { type ControlJson, controlDims } from "@/lib/play/engine";

export type SetControl = (name: string, values: number[]) => void;

const list = (value: number | number[] | undefined, dims: number, fallback: number) =>
  Array.from({ length: dims }, (_, i) =>
    Array.isArray(value) ? (value[i] ?? fallback) : (value ?? fallback),
  );

const LABEL = "font-mono text-[11px] uppercase tracking-[0.08em] text-label";

function toHex(rgb: number[]): string {
  return `#${rgb
    .slice(0, 3)
    .map((v) =>
      Math.round(Math.min(1, Math.max(0, v)) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function fromHex(hex: string): number[] {
  return [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255);
}

function format(value: number): string {
  return Math.abs(value) >= 100 ? value.toFixed(0) : value.toFixed(2);
}

function XyPad({
  min,
  max,
  value,
  onChange,
  label,
}: {
  min: number[];
  max: number[];
  value: number[];
  onChange: (values: number[]) => void;
  label: string;
}) {
  const pad = useRef<HTMLDivElement>(null);
  const span = (i: number) => (max[i] ?? 1) - (min[i] ?? 0) || 1;
  const t = (i: number) => ((value[i] ?? 0) - (min[i] ?? 0)) / span(i);
  const move = (event: PointerEvent<HTMLDivElement>) => {
    if (event.buttons === 0 || !pad.current) return;
    const box = pad.current.getBoundingClientRect();
    const u = Math.min(1, Math.max(0, (event.clientX - box.left) / box.width));
    const v = Math.min(1, Math.max(0, 1 - (event.clientY - box.top) / box.height));
    onChange([(min[0] ?? 0) + u * span(0), (min[1] ?? 0) + v * span(1)]);
  };
  return (
    <div
      ref={pad}
      role="slider"
      aria-label={label}
      aria-valuenow={value[0] ?? 0}
      aria-valuemin={min[0] ?? 0}
      aria-valuemax={max[0] ?? 1}
      aria-valuetext={`${format(value[0] ?? 0)}, ${format(value[1] ?? 0)}`}
      tabIndex={0}
      onKeyDown={(event) => {
        // Arrow keys move the point a twentieth of the pad at a time.
        const step: Record<string, [number, number]> = {
          ArrowLeft: [-1, 0],
          ArrowRight: [1, 0],
          ArrowUp: [0, 1],
          ArrowDown: [0, -1],
        };
        const [dx, dy] = step[event.key] ?? [0, 0];
        if (dx === 0 && dy === 0) return;
        event.preventDefault();
        const clamp = (i: number, t: number) =>
          (min[i] ?? 0) + Math.min(1, Math.max(0, t)) * span(i);
        onChange([clamp(0, t(0) + dx / 20), clamp(1, t(1) + dy / 20)]);
      }}
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId);
        move(event);
      }}
      onPointerMove={move}
      className="relative aspect-2/1 w-full cursor-crosshair touch-none border border-hairline bg-ground"
    >
      <span
        className="pointer-events-none absolute size-2.5 -translate-x-1/2 translate-y-1/2 rounded-full bg-accent"
        style={{ left: `${t(0) * 100}%`, bottom: `${t(1) * 100}%` }}
      />
    </div>
  );
}

function Control({ control, set }: { control: ControlJson; set: SetControl }) {
  const type = control.TYPE.split(" ")[0] ?? "slider";
  const dims = controlDims(control.TYPE);
  const value = control.VALUE ?? list(control.DEFAULT, dims, 0);
  const min = list(control.MIN, dims, 0);
  const max = list(control.MAX, dims, 1);
  const name = control.NAME;
  const id = `control-${name}`;
  const title = control.DESCRIPTION;

  switch (type) {
    case "toggle": {
      const on = (value[0] ?? 0) >= ((min[0] ?? 0) + (max[0] ?? 1)) / 2;
      return (
        <button
          type="button"
          role="switch"
          aria-checked={on}
          title={title}
          onClick={() => set(name, [on ? (min[0] ?? 0) : (max[0] ?? 1)])}
          className="flex w-full items-center justify-between gap-3 py-1 text-left"
        >
          <span className={LABEL}>{name}</span>
          <span
            className={`h-4 w-8 border transition-colors ${on ? "border-accent bg-accent" : "border-hairline-strong"}`}
          >
            <span
              className={`block size-3 translate-y-px bg-text transition-transform ${on ? "translate-x-[17px] bg-on-accent" : "translate-x-px"}`}
            />
          </span>
        </button>
      );
    }
    case "bang":
      return (
        <button
          type="button"
          title={title}
          onClick={() => set(name, [1])}
          className="w-full border border-hairline-strong px-3 py-1.5 font-mono text-[11px] uppercase tracking-widest text-bright hover:border-accent hover:text-accent-text active:bg-accent active:text-on-accent"
        >
          {name}
        </button>
      );
    case "color":
    case "colour":
      return (
        <label htmlFor={id} className="flex items-center justify-between gap-3 py-1" title={title}>
          <span className={LABEL}>{name}</span>
          <input
            id={id}
            type="color"
            value={toHex(value)}
            onChange={(event) => set(name, fromHex(event.target.value))}
            className="h-6 w-12 cursor-pointer border border-hairline-strong bg-transparent"
          />
        </label>
      );
    case "dropdown":
      return (
        <label htmlFor={id} className="flex items-center justify-between gap-3 py-1" title={title}>
          <span className={LABEL}>{name}</span>
          <select
            id={id}
            value={Math.round(value[0] ?? 0)}
            onChange={(event) => set(name, [Number(event.target.value)])}
            className="max-w-[60%] border border-hairline-strong bg-ground px-2 py-1 text-sm text-text"
          >
            {(control.LABELS ?? []).map((label, index) => (
              <option key={label + String(index)} value={index}>
                {label}
              </option>
            ))}
          </select>
        </label>
      );
    case "xy":
      return (
        <div className="flex flex-col gap-1.5 py-1" title={title}>
          <span className={LABEL}>
            {name}{" "}
            <span className="text-secondary">
              {format(value[0] ?? 0)}, {format(value[1] ?? 0)}
            </span>
          </span>
          <XyPad min={min} max={max} value={value} label={name} onChange={(v) => set(name, v)} />
        </div>
      );
    default:
      return (
        <label htmlFor={id} className="flex flex-col gap-1 py-1" title={title}>
          <span className="flex justify-between">
            <span className={LABEL}>{name}</span>
            <span className="font-mono text-[11px] text-secondary">
              {format(value[0] ?? 0)}
            </span>
          </span>
          <input
            id={id}
            type="range"
            min={min[0]}
            max={max[0]}
            step={((max[0] ?? 1) - (min[0] ?? 0)) / 1000 || 0.001}
            value={value[0] ?? 0}
            onChange={(event) => set(name, [Number(event.target.value)])}
            className="w-full accent-accent"
          />
        </label>
      );
  }
}

export default function ControlsPanel({
  controls,
  set,
}: {
  controls: ControlJson[];
  set: SetControl;
}) {
  if (controls.length === 0) {
    return (
      <p className="px-4 py-3 text-sm text-secondary">
        No controls. Add a CONTROLS list to scene.json and each one becomes a uniform here.
      </p>
    );
  }
  const groups = new Map<string, ControlJson[]>();
  for (const control of controls) {
    const group = control.UI_GROUP ?? "";
    groups.set(group, [...(groups.get(group) ?? []), control]);
  }
  return (
    <div className="flex flex-col">
      {[...groups].map(([group, members]) => (
        <section key={group || "-"} className="border-b border-hairline px-4 py-2">
          {group ? <h3 className={`${LABEL} mb-1 text-bright`}>{group}</h3> : null}
          {members.map((control) => (
            <Control key={control.NAME} control={control} set={set} />
          ))}
        </section>
      ))}
    </div>
  );
}
