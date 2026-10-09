# Strobe status and gap audit

_Last audited: 2026-10-09 against the current integration commits and project docs._

This is an implementation handoff, not a claim that every platform path has been independently verified. “Implemented” means code and integration commits are present; hardware-dependent behavior remains gated on the evidence below.

## Current baseline

The repository has moved beyond the old “no window yet” state. The Windows desktop app wires together the control/output windows, scene loading, audio capture and synthetic fallback, QuickJS scene scripts, diagnostics/crash monitoring, and GPU-timed adaptive render scale. The benchmark can use a window context on systems without EGL.

Read [ARCHITECTURE.md](ARCHITECTURE.md), [DECISIONS.md](DECISIONS.md), [BENCHMARKS.md](BENCHMARKS.md), and [ROADMAP.md](ROADMAP.md) for deeper contracts.

## P0 — real Windows hardware verification

Linux/llvmpipe cannot prove WGL/ANGLE behavior, WASAPI lifecycle, Windows crash-monitor IPC, or integrated-GPU timings.

- [ ] Launch on the T480 with native GL; record adapter, driver, backend and startup log.
- [ ] Force ANGLE and verify the same scene corpus loads.
- [ ] Exercise audio absent, present, removed and reconnected; verify UI/log state transitions.
- [ ] Benchmark scenes at fixed resolution; record GPU frame time, render scale, backend and driver. Keep CPU time separate.
- [ ] Run the crash demo from an ordinary terminal, verify minidump creation, and retain the matching PDB. Test the packaged executable separately.
- [ ] Run at least 30 minutes with repeated scene switches; inspect crashes, log growth and resource stability.
- [ ] Save commands, environment, logs, screenshots and failed outcomes in [hardware/T480.md](hardware/T480.md).

## P0 — context-loss recovery

Diagnostics are not recovery. Implement a bounded state machine: detect make-current/swap/reset failures; stop issuing GL calls against a lost context; recreate native context once; try ANGLE after repeated native failures; rebuild resources and reload the active scene; emit one structured failure with backend/driver/retry context; prevent retry loops and report terminal failure clearly.

**Acceptance:** deterministic fault injection covers state transitions, retries are bounded, and at least one real Windows attempt is documented. Startup fallback alone does not satisfy this.

## P1 — performer-facing UI

The current egui UI is an engineering shell. The research conclusion is **Contextual Perform**: persistent preview/output status, transport/cues, performance macros and health; scene browser and inspector are contextual surfaces.

- Keep authoritative runtime state out of egui widget memory.
- Route actions through typed app commands; widgets should not own runtime side effects.
- Keep Preview separate from Load.
- Persist browser query, filters, sort, width and visibility as workflow state.
- Represent macro target states explicitly: resolved, unavailable, broken, disabled, learning and conflict.
- Keep expensive work out of paint; use caches/workers and messages.
- Treat 1280×720 as a first-class target. Test empty/loading/failure/missing-audio/script-fault/narrow-window states.
- Keep output visually independent from the control UI.

Read [UI_DESIGN.md](UI_DESIGN.md), [UI_IMPLEMENTATION_PLAN.md](UI_IMPLEMENTATION_PLAN.md), [UI_MOCKUP_WORKFLOW.md](UI_MOCKUP_WORKFLOW.md), and [the four-direction HTML mockup](mockups/strobe-perform-directions.html). Choose topology before broad redesign.

## P1 — SSF/script compatibility

- Script writes to `meta/` controls are currently dropped.
- `textureFinalPass` returns black; implement only with asynchronous/delayed readback such as a PBO ring, never synchronous per-frame GPU readback.
- Script-created uniforms appearing after initial preparation can trigger a logged recompile; preserve the no-compilation-during-normal-playback invariant.
- The benchmark does not execute scripts; keep this limitation explicit.
- Expand GLES translation tests for desktop-only SSF idioms.

Acceptance: fixtures per behavior, tests on both shader dialects where applicable, and explicit supported/degraded/unsupported compatibility claims.

## P1 — scene assets and Shadertoy

- Replace black `syn_Media` and placeholder `syn_MediaType` with a documented media path.
- Load cubemaps and report missing/invalid assets without crashing the show.
- Complete Shadertoy import including multipass dependencies, textures/channels, live audio, conversion diagnostics, attribution/license metadata, and userscript/drop-folder workflow.
- Preserve source/license metadata and allow inspection; do not infer commercial rights.

Acceptance: representative single/multipass imports work on native GL and ANGLE; missing assets fail visibly; attribution survives export.

## P2 — playlists, show control and release

- Complete playlist defaults → section → cue inheritance, scheduler/generator/resume, beat-locked timing, and precompile before cue activation.
- Wire presets, MIDI, OSC and hotkeys into the shared ControlBank. Do not duplicate toggle/bang/smoothing semantics.
- Run a tag-triggered release and inspect the zip, ANGLE DLL provenance, matching PDB archive, SBOM, checksums and attestations.
- Document clean-machine install/smoke test and symbolication steps. Portable-crate CI is not a working macOS app; a suitable GL context provider remains necessary.

## Regression traps

- `FrameInputs` and `strobe_scene::audio::SCALARS` are authoritative contracts; avoid parallel representations.
- Audio, scripts, playlist and input crates must not depend on `strobe-gfx`.
- Never put blocking device enumeration, shader compilation, heavy I/O or synchronous GPU readback in the frame/paint path.
- “Works under llvmpipe” does not prove Windows driver behavior.
- Keep benchmark frames and timing metadata together. llvmpipe CPU timings are not integrated-GPU bandwidth measurements.
- Crash symbols must match the exact executable build.

## Handoff sequence

1. Read [ROADMAP.md](ROADMAP.md) and [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).
2. Reproduce the baseline before editing.
3. Make one bounded change with a regression test.
4. Run `cargo xtask ci` and the relevant backend/hardware test.
5. Update roadmap and decision records in the same change.
6. Record evidence and uncertainty; do not turn “compiled” into “verified”.
