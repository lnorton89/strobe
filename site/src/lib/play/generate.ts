/*
 * Generate controls in the sandbox (D558): the page's half, without a canvas
 * or a React tree, so it can be tested alone. The engine does the measuring
 * (crates/strobe-autocontrol) a step at a time; this is what the page says
 * about it, how often it asks for a step, what a review sends back, and what
 * an undo restores.
 */

import { PRESET_DIR, type Scene } from "@/lib/play/files";

/** One proposed control, as `strobe_autocontrol::Kept` serialises it. */
export type KeptControl = {
  candidate: number;
  name: string;
  group: string;
  kind: string;
  default: number[];
  min: number[];
  max: number[];
  /** How much the picture moved when it was nudged, 0..1. */
  score: number | null;
  context: string;
};

export type GenerateProgress = {
  state: "running";
  phase: "preparing" | "scoring" | "checking";
  done: number;
  total: number;
};

export type GenerateProposal = {
  state: "done";
  scene: string;
  candidates: number;
  fidelity: number | null;
  kept: KeptControl[];
  suggestions: string[];
};

export type GenerateAnswer = GenerateProgress | GenerateProposal;

/** A control in the review, as the person has left it (D219, D246). */
export type ReviewRow = {
  control: KeptControl;
  keep: boolean;
  name: string;
  min: number;
  max: number;
};

/** What `strobe_autocontrol::Choice` reads. */
export type Choice = {
  candidate: number;
  name: string;
  context: string;
  min: number;
  max: number;
};

/** The line under the button while it runs. */
export function progressLabel(progress: GenerateProgress): string {
  switch (progress.phase) {
    case "preparing":
      return "Compiling the scene";
    case "scoring":
      return `Scoring ${Math.min(progress.done + 1, progress.total)} of ${progress.total}`;
    case "checking":
      return "Checking the result";
  }
}

/** How full the progress bar is, 0..1. */
export function progressFraction(progress: GenerateProgress): number {
  switch (progress.phase) {
    case "preparing":
      return 0;
    case "scoring":
      return progress.total > 0 ? progress.done / progress.total : 0;
    case "checking":
      return 1;
  }
}

/**
 * When the next step may run, for one that began at `started` and took `cost`
 * milliseconds: it waits as long as the last step took, so however slow the
 * machine, the preview and the page keep at least half the time. On a machine
 * where a step costs a few milliseconds this is simply the next animation
 * frame.
 */
export function nextStepAt(started: number, cost: number): number {
  return started + 2 * cost;
}

/** Whether two scenes run the same code: presets and the title do not count. */
export function sameSources(a: Scene, b: Scene): boolean {
  const code = (scene: Scene) =>
    Object.keys(scene.files)
      .filter((name) => !name.startsWith(PRESET_DIR))
      .sort();
  const names = code(a);
  const others = code(b);
  return (
    names.length === others.length &&
    names.every((name, index) => name === others[index] && a.files[name] === b.files[name])
  );
}

/** A proposal as the review starts it: everything kept, as the engine named it. */
export function reviewRows(proposal: GenerateProposal): ReviewRow[] {
  return proposal.kept.map((control) => ({
    control,
    keep: true,
    name: control.name,
    min: control.min[0] ?? 0,
    max: control.max[0] ?? 1,
  }));
}

/** Whether the row's control is a slider, which has a range to edit. */
export function isSlider(control: KeptControl): boolean {
  return control.default.length === 1 && control.min.length === 1;
}

/**
 * What the engine is asked to write: the kept rows, each slider's range
 * widened if need be to hold its default, which is what keeps the picture at
 * defaults the one that was measured (D219).
 */
export function choicesFrom(rows: ReviewRow[]): Choice[] {
  return rows
    .filter((row) => row.keep)
    .map((row) => {
      const value = row.control.default[0] ?? 0;
      return {
        candidate: row.control.candidate,
        name: row.name.trim(),
        context: row.control.context,
        min: isSlider(row.control) ? Math.min(row.min, value) : row.min,
        max: isSlider(row.control) ? Math.max(row.max, value) : row.max,
      };
    });
}

/** The order the app lists its groups in (strobe_autocontrol::Group). */
const GROUPS = ["Motion", "Shape", "Colour", "Quality"];

/** The rows by group, strongest first in each, with each row's index. */
export function groupRows(rows: ReviewRow[]): [string, { row: ReviewRow; index: number }[]][] {
  const groups = new Map<string, { row: ReviewRow; index: number }[]>();
  rows.forEach((row, index) => {
    const list = groups.get(row.control.group) ?? [];
    list.push({ row, index });
    groups.set(row.control.group, list);
  });
  const rank = (group: string) => {
    const at = GROUPS.indexOf(group);
    return at < 0 ? GROUPS.length : at;
  };
  return [...groups].sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b));
}

/** A score as the review prints it. */
export function effectLabel(score: number | null): string {
  return score === null ? "not measured" : `${(score * 100).toFixed(1)}%`;
}

/** The files `after` changes from `before`. */
export function changedFiles(before: Scene, after: Scene): string[] {
  return Object.keys({ ...before.files, ...after.files })
    .filter((name) => before.files[name] !== after.files[name])
    .sort();
}

/**
 * The scene with an accepted generation taken back: the files it changed go
 * back to what they were, and everything else stays, so a preset saved or a
 * file added since is kept. `lost` names the files edited since the accept,
 * whose edits the undo would discard: the page asks before losing them.
 */
export function undoGenerate(
  current: Scene,
  before: Scene,
  after: Scene,
): { scene: Scene; lost: string[] } {
  const files = { ...current.files };
  const lost: string[] = [];
  for (const name of changedFiles(before, after)) {
    if (current.files[name] !== after.files[name]) lost.push(name);
    const text = before.files[name];
    if (text === undefined) delete files[name];
    else files[name] = text;
  }
  return { scene: { ...current, files }, lost };
}

/** Whether a scene's code pulls in other files with #include. */
export function usesIncludes(scene: Scene): boolean {
  return Object.entries(scene.files).some(
    ([name, text]) => /\.(glsl|frag)$/i.test(name) && /^[ \t]*#[ \t]*include\b/m.test(text),
  );
}

/** What the strip says when nothing can be offered. */
export function nothingToOffer(proposal: GenerateProposal): string {
  return proposal.candidates === 0
    ? "No controls to offer: no number in this scene's code looks like one. The scene is unchanged."
    : `No controls to offer: ${proposal.candidates} number${proposal.candidates === 1 ? "" : "s"} measured, and none moved the picture enough. The scene is unchanged.`;
}

/** What the strip says when a run fails. */
export function failureMessage(error: unknown, scene: Scene): string {
  const text = String(error).replace(/^Error: /, "");
  const script =
    scene.files["script.js"] !== undefined
      ? " The script does not run while measuring, so a uniform only the running script can name is not declared."
      : "";
  return `No controls generated: ${text}${script}`;
}
