# Roadmap and status

**Update this file whenever you finish something.** It is the first file the next agent should read. Design detail lives in [PLAN.md](PLAN.md), decisions in [DECISIONS.md](DECISIONS.md), and the current gap assessment in [STATUS_AUDIT_2026-10-09.md](STATUS_AUDIT_2026-10-09.md).

_Last updated: 2026-10-09._

## Now

**Owner goal:** maintain a working, testable, benchmarkable VJ engine for older hardware. Prefer measured improvements and explicit acceptance tests over speculative tuning.

### Implemented and integrated

- Workspace conventions, pinned toolchain/actions, CI, repository policy checks, and release plumbing.
- `strobe-scene`: SSF parsing/validation, controls runtime, and shader assembly; `strobe-ssf-lib`: SSF GLSL helpers.
- `strobe-gfx`: native GL/ANGLE context providers, headless EGL, multipass rendering, ping-pong targets, implicit final pass, image textures, GL diagnostics, and GPU timer-query wrappers.
- `strobe-bench`: benchmark rendering, timings, contact sheets, and headless/window/auto context and backend selection.
- Eleven original scenes, including `11-script-orbits`; see [BENCHMARKS.md](BENCHMARKS.md) and [hardware/T480.md](hardware/T480.md).
- `strobe-app`: Windows desktop control shell, separate fullscreen output, scene loading/controls, preview, audio status, diagnostics console, and native GL with ANGLE fallback.
- `strobe-audio`: capture, analysis, resilient retry, synthetic fallback, and audio textures; device opening/enumeration run off the render thread.
- `strobe-script`: persistent QuickJS scene scripting, events/update/control integration, execution budget, fault display, and script-created uniform preparation.
- `strobe-diag`: session logging, structured failures, panic reporting, native crash monitoring/minidumps, and GL debug output.
- GPU timer queries drive adaptive render scale; see decisions D64-D65.
- Research/UI handoff: [AGENT_RESEARCH.md](AGENT_RESEARCH.md), [UI_DESIGN.md](UI_DESIGN.md), [UI_IMPLEMENTATION_PLAN.md](UI_IMPLEMENTATION_PLAN.md), and [mockups/strobe-perform-directions.html](mockups/strobe-perform-directions.html).

### In flight

- **T480 hardware verification:** native GL and ANGLE smoke tests, per-scene frame timing, audio-device lifecycle, and crash-capture drill. Record evidence in [hardware/T480.md](hardware/T480.md).
- **Context-loss recovery:** detect context/surface loss, recover or recreate the context, try ANGLE after repeated native failures, reload the active scene, and report structured failure. Startup fallback is not equivalent to recovery.
- **Perform-first UI:** the current app is a functional engineering shell, not the final performer-facing UI. The leading direction is contextual Perform: persistent preview/output health, transport/cues and macros, with browser and inspector surfaces becoming contextual.

## Next, in priority order

1. Verify native GL, ANGLE, audio lifecycle, crash capture and GPU timing on real Windows hardware. Preserve logs, environment, screenshots and failed outcomes.
2. Implement bounded context-loss recovery with fault-injection tests and explicit terminal failure behavior.
3. Choose the Perform topology from the HTML mockups, then test at 1280×720 and 1440×900. Separate UI state from runtime state; route actions through typed commands; keep expensive work out of egui paint.
4. Close SSF/script gaps: `meta/` control writes and `textureFinalPass` via asynchronous/delayed readback, never a synchronous per-frame GPU readback.
5. Complete Shadertoy import: multipass dependencies, assets, live audio mapping, conversion errors, source/license preservation, and userscript/drop-folder workflow.
6. Complete playlist inheritance, scheduler/generator/resume behavior and precompile before cue activation.
7. Wire presets, MIDI, OSC and hotkeys to the shared ControlBank.
8. Implement media/cubemap loading and transitions before NDI or other output transports.
9. Run a real tag-triggered release and inspect the archive, matching PDBs, SBOM, checksums and attestations.

## Phases

| Phase | Scope | State |
|---|---|---|
| 0 | Workspace, conventions, CI, GL contexts | Implemented; validate packaged Windows behavior on hardware. |
| 1 | Renderer, SSF, controls, output window | Functional desktop shell exists; Perform-first UI remains. Images work; media/cubemaps are placeholders. |
| 2 | JavaScript API | QuickJS integrated; `meta/` writes and full `textureFinalPass` remain open. |
| 3 | Audio | Capture, analysis, fallback, settings and app textures integrated; bench still uses synthetic stand-in values. |
| 4 | Shadertoy import | Not complete end to end. |
| 5 | Playlists | Model/planning exists; app workflow and playback integration remain. |
| 6 | Presets, MIDI, OSC, hotkeys | Not complete. |
| 7 | NDI, media, transitions | Not complete; media/cubemap placeholders remain. |

## Known gaps

- The current app is not the finished performer-facing UI.
- Context-loss recovery is not complete.
- Bench `syn_Spectrum` and `syn_LevelTrail` are synthetic placeholders; the app uploads real or synthetic audio textures.
- `syn_Media` is black and `syn_MediaType` is 0; cubemaps are reserved but not loaded.
- The benchmark does not execute scene scripts. The scripted scene intentionally uses controls so it remains benchmark-renderable.
- `textureFinalPass` returns black in scripts; `meta/` control writes are dropped.
- GLES translation does not cover every desktop-only SSF idiom (Q8 in [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md)); keep the corpus ES-clean and add fixtures as compatibility work lands.
- Adaptive-resolution thresholds need real-hardware validation beyond llvmpipe.
- The tag-triggered release workflow has not yet been proven end to end.
