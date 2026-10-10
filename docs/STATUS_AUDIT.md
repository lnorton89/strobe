# Strobe status and gap audit

_Last audited: 2026-10-09, after the context-recovery, playlist and
Shadertoy stage 1 merges. Started by the owner in
[lnorton89/strobe#5](https://github.com/lnorton89/strobe/pull/5)._

This is an implementation handoff, not a claim that every platform path
is verified. "Implemented" means the code is merged and tested in CI.
Hardware-dependent behaviour stays gated on the evidence listed here.
Ticked items cite where the evidence is. [ROADMAP.md](ROADMAP.md) carries
the priority order.

## Current baseline

The Windows desktop app wires together:

- the control and output windows on one GL context;
- scene loading;
- audio capture with a synthetic fallback;
- QuickJS scene scripts;
- diagnostics and crash monitoring;
- GPU-timed adaptive render scale;
- context-loss recovery with ANGLE fail-over.

The benchmark uses a window context where EGL is unavailable. Two crates
exist without app integration so far: the Shadertoy converter and the
playlist core.

For the deeper contracts, read [ARCHITECTURE.md](ARCHITECTURE.md),
[DECISIONS.md](DECISIONS.md) and [BENCHMARKS.md](BENCHMARKS.md).

## P0: real Windows hardware verification

Linux and llvmpipe cannot prove any of these:

- WGL or ANGLE behaviour;
- WASAPI lifecycle;
- Windows crash-monitor IPC;
- integrated-GPU timings.

- [x] Launch on the T480 with native GL, and record the adapter, driver,
  backend and startup log ([hardware/T480.md](hardware/T480.md)).
- [x] Force ANGLE and verify the same scene corpus loads (T480, all
  scenes).
- [ ] Exercise audio absent, present, removed and reconnected, and verify
  the UI and log state transitions. Live capture is verified so far;
  removal and reconnection are not.
- [x] Benchmark scenes at fixed resolution, recording GPU frame time,
  render scale, backend and driver, with CPU time kept separate
  ([BENCHMARKS.md](BENCHMARKS.md), 2026-10-09).
- [x] Run the crash demo from an ordinary terminal and verify a minidump
  is created (owner, 2026-10-09; bug 2 in T480.md). An agent inside the
  Claude desktop app cannot do this at the default location (MSIX
  virtualisation).
- [ ] Symbolicate that dump with the matching PDB (Q11). No debugger is
  installed on the T480 yet.
- [ ] Test the packaged release executable separately (Q12).
- [ ] Run for at least 30 minutes with repeated scene switches, and
  inspect for crashes, log growth and resource stability.
- [x] Keep commands, environment, logs, screenshots and failed outcomes in
  [hardware/T480.md](hardware/T480.md).

## P0: context-loss recovery

Diagnostics are not recovery. The bounded state machine is built (D66):

- it detects make-current, swap and reset failures;
- it stops GL calls against a lost context;
- it recreates the native context, and tries ANGLE after repeated native
  failures;
- it rebuilds both windows and reloads the active scene;
- it emits a structured `CONTEXT` record with the backend, error and
  reset status;
- its retries are bounded, and terminal failure is clear.

- [x] Deterministic fault injection (`STROBE_FAULT`) covers the state
  transitions, and the retries are bounded (22 policy tests, plus Xvfb
  runs).
- [x] At least one real Windows attempt documented. On the T480 (Task 8),
  an injected loss recovered on native in 181 ms and failed over to ANGLE
  in 171 ms, and the give-up path ended cleanly
  ([hardware/T480.md](hardware/T480.md), bug 1). No real driver reset has
  been observed yet: bug 1 no longer reproduces.

## P1: performer-facing UI

The current egui UI is an engineering shell. The research conclusion is
**Contextual Perform**: a persistent preview and output status, transport
and cues, performance macros and health. The scene browser and inspector
are contextual surfaces.

- Keep authoritative runtime state out of egui widget memory.
- Route actions through typed app commands; widgets should not own
  runtime side effects.
- Keep Preview separate from Load.
- Persist the browser's query, filters, sort, width and visibility as
  workflow state.
- Represent each macro target's state explicitly: resolved, unavailable,
  broken, disabled, learning and conflict.
- Keep expensive work out of paint; use caches, workers and messages.
- Treat 1280x720 as a first-class target. Test these states:
  - empty and loading;
  - failure, missing audio and script fault;
  - a narrow window.
- Keep the output visually independent from the control UI. The output
  window starts hidden; the operator shows it.

Read [UI_DESIGN.md](UI_DESIGN.md),
[UI_IMPLEMENTATION_PLAN.md](UI_IMPLEMENTATION_PLAN.md),
[UI_MOCKUP_WORKFLOW.md](UI_MOCKUP_WORKFLOW.md) and
[the four-direction mockup](mockups/strobe-perform-directions.html). Choose
the topology before any broad redesign.

## P1: SSF and script compatibility

- [ ] Script writes to `meta/` controls are dropped.
- [ ] `textureFinalPass` returns black. Implement it only with an
  asynchronous, delayed readback such as a PBO ring, never a synchronous
  per-frame GPU readback (Q14).
- [ ] A script-created uniform that appears after initial preparation can
  trigger a logged recompile. Keep the invariant: no compilation during
  normal playback.
- [ ] The benchmark does not execute scripts. Keep this limitation
  explicit.
- [ ] Expand the GLES translation tests for desktop-only SSF idioms (Q8).

*Acceptance:* a fixture per behaviour, tests on both shader dialects where
they apply, and explicit supported, degraded or unsupported claims.

## P1: scene assets, Shadertoy and auto-generated controls

- [ ] Replace the black `syn_Media` and placeholder `syn_MediaType` with a
  documented media path.
- [ ] Load cubemaps, and report missing or invalid assets without
  crashing the show.
- [x] Shadertoy conversion (stage 1, D90-D95). It covers:
  - multipass dependencies in Shadertoy order, and buffer feedback;
  - texture and buffer channels;
  - a live audio texture, `iDate` and the "Strobe" control group;
  - token-level dialect rewriting;
  - attribution and licence metadata in `SOURCE`.
  
  Nine original fixtures render on both dialects.
- [ ] Shadertoy acquisition and workflow (stage 2): the REST client, the
  userscript with its localhost listener, the drop folder, the asset
  downloader, the app's Import UI, conversion diagnostics in the UI, and
  the Needs-work bin.
- [x] Auto-generated controls engine, stage 1 (D123-D126;
  [ROADMAP.md](ROADMAP.md), "Auto-generated controls"). Evidence:
  `crates/strobe-autocontrol/tests/engine.rs` on headless llvmpipe, both
  GL 3.3 core and GLES 3.0. Every library scene with `CONTROLS` stripped
  (11 of 11) and 8 of 14 Shadertoy fixtures get controls; the other 6 are
  structural and documented in `NO_CONTROLS`. Each kept control changes the
  image (scored, then re-checked on the finished scene), the rewrite
  reproduces the original at its defaults (fidelity 0 in all 25), a second
  run is identical. **Not verified:** a real GPU or the T480 (llvmpipe only),
  and the Import/Inspector review panel does not exist, so nothing in the app
  calls the engine yet.
- Preserve source and licence metadata, and make it inspectable. Never
  infer commercial rights.

*Acceptance:* representative single-pass and multipass imports work on
native GL and on ANGLE, missing assets fail visibly, and attribution
survives export.

## P2: playlists, show control and release

- [x] Playlist inheritance (defaults, then section, then cue), the
  generator, the timeline, the playback clock with beat-locked ends, and
  resume. This is the pure core (D73-D77), with 4-hour soaks.
- [ ] Playlist app integration: precompile before cue activation,
  warm-up, transitions, the UI, file watching and resume saves.
- [ ] Wire presets, MIDI, OSC and hotkeys into the shared `ControlBank`.
  Don't duplicate its toggle, bang or smoothing semantics.
- [ ] Run a tag-triggered release, then inspect:
  - the zip and the ANGLE DLL provenance;
  - the matching PDB archive;
  - the SBOM, checksums and attestations.
- [ ] Document a clean-machine install, a smoke test, and the
  symbolication steps. Portable-crate CI is not a working macOS app; that
  still needs a suitable GL context provider.

## Regression traps

- `FrameInputs` and `strobe_scene::audio::SCALARS` are authoritative
  contracts; avoid parallel representations.
- Audio, scripts, playlists and input crates must not depend on
  `strobe-gfx`.
- Never put any of these in the frame or paint path: blocking device
  enumeration, shader compilation, heavy I/O, or synchronous GPU readback.
- "Works under llvmpipe" does not prove Windows driver behaviour.
- Keep benchmark frames and timing metadata together. llvmpipe CPU
  timings are not integrated-GPU bandwidth measurements.
- Crash symbols must match the exact executable build.
- On the T480, an agent's crash drill must set `STROBE_DIAG_DIR` inside the
  repo, because the Claude desktop app's MSIX package context breaks the
  AF_UNIX socket at the default location.

## Handoff sequence

1. Read [ROADMAP.md](ROADMAP.md) and
   [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).
2. Reproduce the baseline before editing.
3. Make one bounded change, with a regression test.
4. Run `cargo xtask ci` and the relevant backend or hardware test.
5. Update the roadmap and the decision records in the same change.
6. Record evidence and uncertainty. Do not turn "compiled" into
   "verified".
