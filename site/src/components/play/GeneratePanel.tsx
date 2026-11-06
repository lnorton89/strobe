"use client";

/*
 * Generate controls in the sandbox (D558): the strip above the controls that
 * shows a run's progress, its proposal for review, and the way back. It holds
 * no engine and no scene; PlayApp owns both and passes in what happened.
 *
 * The review is the app's (D202, D219, D246): each control kept or dropped,
 * renamed, a slider re-ranged, how much it moved the picture, and the audio
 * pairings the engine suggests. Nothing reaches the scene until Add.
 */

import { useState } from "react";
import {
  type GenerateProgress,
  type GenerateProposal,
  type ReviewRow,
  effectLabel,
  groupRows,
  isSlider,
  progressFraction,
  progressLabel,
} from "@/lib/play/generate";
import type { Scene } from "@/lib/play/files";

export type GenerateState =
  | { state: "running"; progress: GenerateProgress }
  | {
      state: "review";
      proposal: GenerateProposal;
      rows: ReviewRow[];
      /** The scene as it was measured. */
      snapshot: Scene;
      seconds: number;
      problem?: string;
    }
  | { state: "message"; tone: "info" | "error"; text: string }
  | { state: "accepted"; count: number; files: string[]; before: Scene; after: Scene };

const BUTTON =
  "inline-flex items-center gap-1.5 border border-hairline-strong px-2.5 py-1.5 font-mono text-[11px] uppercase tracking-[0.1em] text-bright transition-colors hover:border-accent hover:text-accent-text disabled:opacity-40";
const PRIMARY =
  "inline-flex items-center gap-1.5 border border-accent bg-accent px-2.5 py-1.5 font-mono text-[11px] uppercase tracking-[0.1em] text-on-accent transition-opacity hover:opacity-90 disabled:opacity-40";
const LABEL = "font-mono text-[11px] uppercase tracking-[0.08em] text-label";
const FIELD =
  "border border-hairline-strong bg-ground px-1.5 py-1 font-mono text-xs text-text focus:border-accent focus:outline-none";

function toCss(rgb: number[]): string {
  const [r = 0, g = 0, b = 0] = rgb.map((v) => Math.round(Math.min(1, Math.max(0, v)) * 255));
  return `rgb(${r} ${g} ${b})`;
}

/**
 * One end of a slider's range. It holds what is typed as text: a half-typed
 * number ("-", ".") reads as empty, and a field bound to the number would
 * overwrite it with 0, so "-1" could not be typed. The row takes each value
 * that parses, and leaving the field shows the value the row has.
 */
function RangeField({
  id,
  label,
  value,
  disabled,
  change,
}: {
  id: string;
  label: string;
  value: number;
  disabled: boolean;
  change: (next: number) => void;
}) {
  const [text, setText] = useState(String(value));
  return (
    <>
      <label className="sr-only" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        type="number"
        step="any"
        value={text}
        disabled={disabled}
        onChange={(event) => {
          const typed = event.target.value;
          setText(typed);
          if (typed !== "" && Number.isFinite(Number(typed))) change(Number(typed));
        }}
        onBlur={() => setText(String(value))}
        className={`${FIELD} w-20 disabled:opacity-50`}
      />
    </>
  );
}

function Row({
  row,
  strongest,
  change,
}: {
  row: ReviewRow;
  strongest: number;
  change: (next: ReviewRow) => void;
}) {
  const { control } = row;
  const slider = isSlider(control);
  const id = `generate-${control.candidate}`;
  const fraction = Math.min(1, (control.score ?? 0) / strongest);
  return (
    <li className="border-hairline flex flex-col gap-1.5 border-t py-2" title={control.context}>
      <div className="flex flex-wrap items-center gap-2">
        <input
          id={`${id}-keep`}
          type="checkbox"
          checked={row.keep}
          aria-label={`Keep ${control.name}`}
          onChange={(event) => change({ ...row, keep: event.target.checked })}
          className="accent-accent"
        />
        <label className="sr-only" htmlFor={`${id}-name`}>
          Name of {control.name}
        </label>
        <input
          id={`${id}-name`}
          value={row.name}
          disabled={!row.keep}
          spellCheck={false}
          onChange={(event) => change({ ...row, name: event.target.value })}
          className={`${FIELD} w-40 max-w-full disabled:opacity-50`}
        />
        <span
          className="flex items-center gap-1.5"
          title="How much the picture moved when this number was nudged"
        >
          <span className="bg-ground border-hairline block h-1.5 w-14 border" aria-hidden="true">
            <span className="bg-accent block h-full" style={{ width: `${fraction * 100}%` }} />
          </span>
          <span className="text-secondary font-mono text-[11px]">
            {effectLabel(control.score)}
          </span>
        </span>
        {slider ? (
          <span className="flex flex-wrap items-center gap-1.5 font-mono text-[11px] text-secondary">
            <RangeField
              id={`${id}-min`}
              label={`Lowest value of ${control.name}`}
              value={row.min}
              disabled={!row.keep}
              change={(min) => change({ ...row, min })}
            />
            <span>to</span>
            <RangeField
              id={`${id}-max`}
              label={`Highest value of ${control.name}`}
              value={row.max}
              disabled={!row.keep}
              change={(max) => change({ ...row, max })}
            />
            <span>default {control.default[0]}</span>
          </span>
        ) : (
          <span
            className="border-hairline-strong inline-block size-4 border"
            style={{ background: toCss(control.default) }}
            title={`Default colour: ${control.default.join(", ")}`}
            aria-hidden="true"
          />
        )}
      </div>
      <code className="text-mute truncate font-mono text-[11px]">{control.context}</code>
    </li>
  );
}

export default function GeneratePanel({
  state,
  stale,
  notes,
  cancel,
  dismiss,
  changeRow,
  accept,
  undo,
}: {
  state: GenerateState;
  /** The scene changed after it was measured. */
  stale: boolean;
  /** What this scene's review should say about it. */
  notes: string[];
  cancel: () => void;
  dismiss: () => void;
  changeRow: (index: number, next: ReviewRow) => void;
  accept: () => void;
  undo: () => void;
}) {
  switch (state.state) {
    case "running": {
      const fraction = progressFraction(state.progress);
      return (
        <div
          className="border-hairline bg-panel-raised flex flex-wrap items-center gap-3 border-b px-4 py-2"
          role="status"
          aria-live="polite"
          data-testid="generate-progress"
        >
          <span className="font-mono text-xs text-accent-text">{progressLabel(state.progress)}</span>
          <span className="bg-ground border-hairline block h-1.5 min-w-20 flex-1 border" aria-hidden="true">
            <span className="bg-accent block h-full" style={{ width: `${fraction * 100}%` }} />
          </span>
          <span className="text-secondary text-xs">The scene keeps playing.</span>
          <button type="button" className={BUTTON} onClick={cancel}>
            Cancel
          </button>
        </div>
      );
    }
    case "message":
      return (
        <p
          className={`border-hairline flex items-start justify-between gap-3 border-b px-4 py-2 text-sm whitespace-pre-wrap ${state.tone === "error" ? "text-magenta" : "text-secondary"}`}
          role="status"
          data-testid="generate-message"
        >
          <span className="min-w-0 wrap-anywhere">{state.text}</span>
          <button type="button" className={BUTTON} onClick={dismiss}>
            Dismiss
          </button>
        </p>
      );
    case "accepted":
      return (
        <div
          className="border-hairline bg-panel-raised flex flex-wrap items-center justify-between gap-3 border-b px-4 py-2"
          role="status"
          data-testid="generate-accepted"
        >
          <span className="text-secondary text-sm">
            Added {state.count} control{state.count === 1 ? "" : "s"}. Changed:{" "}
            {state.files.join(", ")}.
          </span>
          <span className="flex gap-2">
            <button type="button" className={BUTTON} onClick={undo}>
              Undo
            </button>
            <button type="button" className={BUTTON} onClick={dismiss}>
              Dismiss
            </button>
          </span>
        </div>
      );
    case "review": {
      const { proposal, rows } = state;
      const keeping = rows.filter((row) => row.keep).length;
      const strongest = Math.max(Number.EPSILON, ...rows.map((row) => row.control.score ?? 0));
      const faithful =
        proposal.fidelity === null ? "not measured" : `${(proposal.fidelity * 100).toFixed(1)}% off`;
      return (
        <section
          className="border-accent bg-panel-raised flex flex-col gap-2 border-b px-4 py-3"
          aria-label="Proposed controls"
          data-testid="generate-review"
        >
          {/* First, not last: a long list runs past the bottom of this pane. */}
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h3 className="text-text text-sm font-semibold">
              {keeping} of {rows.length} proposed control{rows.length === 1 ? "" : "s"} kept
            </h3>
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                className={PRIMARY}
                disabled={keeping === 0 || stale}
                onClick={accept}
              >
                Add {keeping} control{keeping === 1 ? "" : "s"}
              </button>
              <button type="button" className={BUTTON} onClick={dismiss}>
                Discard
              </button>
            </div>
          </div>
          {stale ? (
            <p className="text-magenta text-xs">
              The scene has changed since it was measured. Generate again to add controls to it.
            </p>
          ) : null}
          {state.problem ? (
            <p className="text-magenta text-xs wrap-anywhere">{state.problem}</p>
          ) : null}
          <p className="text-secondary text-xs">
            Measured in {state.seconds.toFixed(1)} s. At their defaults the picture is the
            original&apos;s ({faithful}). Nothing changes until you add them.
          </p>
          {notes.map((note) => (
            <p key={note} className="text-secondary text-xs">
              {note}
            </p>
          ))}
          {groupRows(rows).map(([group, members]) => (
            <div key={group}>
              <h4 className={`${LABEL} text-bright`}>{group}</h4>
              <ul>
                {members.map(({ row, index }) => (
                  <Row
                    key={row.control.candidate}
                    row={row}
                    strongest={strongest}
                    change={(next) => changeRow(index, next)}
                  />
                ))}
              </ul>
            </div>
          ))}
          {proposal.suggestions.map((line) => (
            <p key={line} className="text-mute text-xs">
              Audio idea: {line}
            </p>
          ))}
        </section>
      );
    }
  }
}
