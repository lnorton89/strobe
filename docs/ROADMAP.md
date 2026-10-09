# Roadmap and status

**Update this file whenever you finish something.** It is the first file the next agent should read. Keep Now, Next, and the phase table current. Design detail lives in [PLAN.md](PLAN.md), decisions in [DECISIONS.md](DECISIONS.md), and the current gap assessment in [STATUS_AUDIT_2026-10-09.md](STATUS_AUDIT_2026-10-09.md).

_Last updated: 2026-10-09._

## Now

**Owner goal:** keep a working, testable, benchmarkable VJ engine moving toward reliable real-world use on older hardware. The ten-scene library is both the visual baseline and performance workload. Prefer measured improvements and explicit acceptance tests over speculative tuning.

### Implemented and integrated

- Workspace conventions and CI, including formatting, clippy, tests, pinned toolchain/actions, and repository policy checks.
- `strobe-scene`: SSF parsing/validation, controls runtime, shader assembly, and the authoritative scene model.
- `strobe-ssf-lib`: SSF GLSL helpers, compiled against desktop GL 3.3 and GLES 3.0.
- `strobe-gfx`: GL provider abstraction, native GL/ANGLE window contexts, headless EGL, multipass rendering, ping-pong targets, implicit final pass, image textures, diagnostics, and GPU timer-query wrappers.
- `strobe-bench`: scene rendering, timings, contact sheets, headless/window/auto context selection, and native/ANGLE/auto backend selection. CI retains rendered frames as artifacts.
- Eleven original scenes, including `11-script-orbits`; baseline and real-hardware findings live in [BENCHMARKS.md](BENCHMARKS.md) and [hardware/T480.md](hardware/T480.md).
- `strobe-app`: desktop control window and separate fullscreen output, scene selection/controls, output preview, audio status/device selection, diagnostics console, and native GL with ANGLE fallback.
- `strobe-audio`: capture, analysis, resilient device retry, synthetic fallback, and audio textures. Capture and device enumeration run off the render thread.
- `strobe-script`: persistent QuickJS scene scripting, event/update/control integration, bounded execution, script logging/fault display, and script-created uniforms prepared before initial shader compilation where possible.
- `strobe-diag`: session logging, structured failures, panic reporting, native crash monitor/minidumps, and GL debug output. The crash-capture path has been exercised on Linux and the Windows monitor/socket behavior is being verified on hardware.
- Adaptive render scale is driven by non-blocking GPU timer queries. The ceiling is user-controlled; adaptation pauses after scene loads. See decisions D64-D65.
- Release/repository plumbing: `cargo xtask ci|changelog|bench`, generated changelog, Windows release workflow, separate matching PDB archive, SBOM/checksums/provenance, issue forms, Dependabot, CODEOWNERS, and SECURITY.md. The first tag-triggered release still needs an end-to-end release run.
- Agent research and UI handoff: [AGENT_RESEARCH.md](AGENT_RESEARCH.md), [UI_DESIGN.md](UI_DESIGN.md), [UI_IMPLEMENTATION_PLAN.md](UI_IMPLEMENTATION_PLAN.md), and [mockups/strobe-perform-directions.html](mockups/strobe-perform-directions.html).

### In flight

- **T480 real-hardware verification:** native GL and ANGLE smoke tests, per-scene frame-time measurements, crash-capture drill, and context-loss reproduction. Record reproducible steps, driver/backend, expected/actual behavior, and logs in [hardware/T480.md](hardware/T480.md).
- **Context-loss recovery:** detect failed make-current/swap or graphics reset, recover or recreate the context, try ANGLE after repeated native failures, reload the active scene, and report a structured failure. Do not claim recovery is implemented until a fault-injection or reproducible hardware test passes.
- **Perform-first UI:** the current app is a functional engineering shell, not the final performance-oriented product UI. The intended topology is contextual: persistent preview/transport/cues/macros/health, with browser and inspector becoming contextual. Use the HTML directions to decide the interaction model before committing to a large UI rewrite.

## Next, in priority order

1. **Close the reliability loop on Windows hardware.** Finish the T480 matrix: startup, native GL, forced ANGLE, audio device absent/present/lost/reconnected, scene changes, adaptive scaling, crash capture, and long-running stability. Attach evidence rather than just marking tasks done.
2. **Implement GL context-loss recovery.** Keep output failure visible and actionable; avoid silent black output or repeated failure loops. Add a deterministic test seam where platform APIs permit it.
3. **Make the UI decision explicit, then build the Perform surface.** Test at 1280×720 and 1440×900, with scene browser open/closed, narrow window, missing audio, broken scene, and script fault. Separate UI presentation state from runtime state; send actions through a command model. Keep expensive work out of egui paint.
4. **Finish scene compatibility gaps.** Prioritize script `meta/` control writes and `textureFinalPass` via a non-blocking/asynchronous readback design; do not add a synchronous per-frame GPU readback. Add conformance fixtures for each supported Synesthesia behavior.
5. **Shadertoy import end to end.** Conversion, buffer dependencies, texture acquisition, live audio mapping, attribution/license preservation, failure reporting, and userscript/drop-folder workflow. Treat source licenses as data that must survive import/export.
6. **Playlists and precompilation.** Complete inheritance/scheduler/generator/resume semantics, precompile before cue changes, and test beat-locked and wall-clock behavior. Keep shader compilation out of active playback.
7. **Input and show control.** Presets, MIDI, OSC, hotkeys, and a documented mapping model targeting the shared ControlBank rather than separate control semantics.
8. **Media and transitions.** Replace placeholder `syn_Media` and cubemap behavior with real loading; implement and test transitions before NDI or other output transports.
9. **Release validation.** Run a real tag-triggered release, inspect the installed archive and symbols archive, verify diagnosis/crash artifacts, and document rollback/repro steps.

## Phases

| Phase | Scope | State |
|---|---|---|
| 0 | Workspace, conventions, CI, GL contexts | Implemented; Linux CI and native/ANGLE paths exist. Validate packaged Windows behavior on hardware. |
| 1 | Renderer, SSF scene loading, multipass, controls, output window | Functional desktop shell exists. Perform-first UI is still a design/implementation gap. Images work; cubemaps and media are placeholders. |
| 2 | JavaScript API | QuickJS host is integrated: events, update, controls, setUniform, console/fault state. Meta control writes and full textureFinalPass remain open. |
| 3 | Audio analysis | Capture, analysis, retry/fallback, app settings, and textures integrated. Bench still uses synthetic values and does not exercise the app capture path. |
| 4 | Shadertoy import | Not implemented end to end. |
| 5 | Playlists | Model/planning exists; full app workflow and playback integration remain. |
| 6 | Presets, MIDI, OSC, hotkeys | Not complete. |
| 7 | NDI, media input, song-change transitions | Not complete; media/cubemap placeholders and transition behavior remain. |

## Known gaps and constraints

- The app is not yet a finished performer-facing UI. The current scene browser/control shell is functional but the Perform-first information architecture has not been implemented.
- Context-loss recovery is not complete. A failed driver call can still require restart/reload rather than a managed recovery path.
- In the bench, `syn_Spectrum` and `syn_LevelTrail` remain synthetic placeholder textures; the app uploads real or synthetic audio textures.
- `syn_Media` is a black 1×1 texture, and `syn_MediaType` is 0.
- Cubemap samplers reserve units but cubemap assets are not loaded.
- The bench does not run scene scripts. `11-script-orbits` intentionally uses controls so it remains renderable in the bench; script-created uniforms are tested in app-side tests.
- `textureFinalPass` currently returns black in scripts. The desired implementation needs delayed/asynchronous GPU readback to avoid stalling every frame.
- Script `meta/` control writes are dropped.
- GLES translation does not cover every desktop-only SSF idiom (Q8 in [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md)); keep the scene corpus ES-clean and add regression fixtures when compatibility work is done.
- Adaptive-resolution thresholds and scaling behavior need real-hardware validation beyond llvmpipe. The T480 findings should drive any threshold changes.
- The tag-triggered release workflow has not yet been proven end to end.

