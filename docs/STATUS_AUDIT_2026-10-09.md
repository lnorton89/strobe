# Strobe status and gap audit

_Last audited: 2026-10-09 against the current integration commits and project docs._

This is a handoff document, not a claim that every item has been independently exercised on every platform. “Implemented” means the code and integration commits are present; hardware-dependent behavior remains gated on the evidence listed below.

## Current baseline

The repository has moved beyond the old “no window yet” state. The Windows desktop app now wires together the control/output windows, scene loading, audio capture and synthetic fallback, QuickJS scene scripts, diagnostics/crash monitoring, and GPU-timed adaptive render scale. The benchmark can use a window context on systems without EGL. The 10-scene performance corpus has an additional scripted scene.

The implementation baseline and invariants are documented in [ARCHITECTURE.md](ARCHITECTURE.md), [DECISIONS.md](DECISIONS.md), [BENCHMARKS.md](BENCHMARKS.md), and [ROADMAP.md](ROADMAP.md). Treat those as the source of truth for detailed contracts.

## Prioritized gaps

### P0 — prove the current app on real Windows hardware

The most important gap is evidence, not another feature. Linux/llvmpipe proves shader compilation and deterministic rendering, but it cannot establish WGL/ANGLE behavior, WASAPI device lifecycle, Windows crash-monitor IPC, or GPU timing on an older integrated GPU.

Acceptance checklist:

- [ ] Launch on the T480 with native GL and record adapter, driver, backend, and startup log.
- [ ] Launch with ANGLE forced and verify the same scene corpus loads.
- [ ] Exercise audio absent, present, device removed, and device reconnected. Confirm fallback and recovery state transitions in both UI and logs.
- [ ] Run all benchmark scenes at a fixed resolution and record median/p95 GPU frame time, chosen render scale, backend, and driver. Keep CPU wall time separate from GPU time.
- [ ] Run the crash demo from an ordinary terminal, verify a minidump is produced, and retain the matching symbols/PDB. Test the packaged executable separately from a development build.
- [ ] Run for at least 30 minutes with repeated scene switches; inspect process exit, log growth, and GPU-resource stability.
- [ ] Save commands, environment, logs, screenshots, and outcomes in [hardware/T480.md](hardware/T480.md). A failed check is a result, not a reason to omit the row.

### P0 — context-loss recovery

Current diagnostics can explain many failures, but a structured failure report is not the same as recovery. Implement and test a bounded state machine:

1. Detect context/surface loss at make-current, swap, and supported graphics-reset checks.
2. Stop issuing GL work against the invalid context.
3. Recreate the native context once; if native repeatedly fails, try ANGLE where available.
4. Rebuild GL resources from CPU-side scene/source data and reload the active scene.
5. Keep the output state explicit during recovery and emit one structured failure with backend, driver, and recovery attempt.
6. Prevent rapid retry loops; preserve the session log and return a clear terminal failure if recovery fails.

Acceptance: deterministic fault injection covers the state transitions, retries are bounded, and at least one real Windows recovery attempt is documented. Never claim this is complete solely because startup fallback works.

### P1 — performer-facing UI

The current egui UI is an engineering shell, not the finished product surface. The research conclusion is a **contextual Perform** layout: persistent preview/output status, transport/cues, performance macros and health; scene browser and inspector are contextual surfaces that can recede when performing.

Implementation constraints:

- Keep authoritative runtime state out of egui widget memory.
- Route user intent through typed app commands; widgets should not own runtime side effects.
- Keep preview distinct from Load. Preview must not mutate the active performance.
- Persist browser query, filters, sort, width and visibility as workflow state.
- Represent macro target states explicitly: resolved, unavailable, broken, disabled, learning, conflict.
- Keep expensive work out of paint; use caches/workers and message passing.
- Treat 1280×720 as a first-class target; test empty, loading, failure, missing-audio, script-fault, and narrow-window states.
- Keep the output window visually independent from the control surface.

Start with [UI_DESIGN.md](UI_DESIGN.md), [UI_IMPLEMENTATION_PLAN.md](UI_IMPLEMENTATION_PLAN.md), [UI_MOCKUP_WORKFLOW.md](UI_MOCKUP_WORKFLOW.md), and the [four-direction HTML mockup](mockups/strobe-perform-directions.html). Choose the topology before implementing a broad redesign.

### P1 — SSF/script conformance

Known gaps should be closed with small fixtures and explicit compatibility claims:

- Script writes to `meta/` controls are currently dropped.
- `textureFinalPass` returns black; implement only with asynchronous/delayed readback (e.g. PBO ring) so the render thread never waits for a GPU readback each frame.
- Script-created uniforms that appear after initial preparation can trigger a logged recompile; preserve the invariant that normal playback does not compile shaders.
- The benchmark does not execute scripts. Keep this limitation explicit and ensure every benchmark scene renders without script side effects.
- Expand GLES translation tests for desktop-only SSF idioms before advertising broad compatibility.

Acceptance: a fixture per behavior, tested against both supported shader dialects where applicable, plus a clear distinction between supported, degraded, and unsupported SSF features.

### P1 — scene assets and Shadertoy

- Replace the black `syn_Media` placeholder with a documented media input path and meaningful `syn_MediaType`.
- Load cubemaps, validate asset paths and report missing/invalid files without crashing the show.
- Complete Shadertoy import end to end: multipass dependencies, texture/channel assets, live audio channel, conversion diagnostics, attribution and license metadata, plus userscript/drop-folder workflow.
- Never assume a license from the shader's popularity or from a site default; preserve source metadata and let the user inspect it.

Acceptance: representative single-pass and multipass imports render correctly in native GL and ANGLE, missing assets fail visibly, and source/license attribution survives round-trip export.

### P2 — playlists and show control

- Complete playlist inheritance and validation for defaults → section → cue.
- Ensure precompilation occurs before a cue becomes active; test transition timing, beat-locked duration, wall-clock anchors, pause/resume, and deterministic scheduling.
- Wire presets, MIDI, OSC and hotkeys into the same ControlBank semantics. Do not duplicate toggle, bang, smoothing or normalization logic.
- Add a documented mapping/learn workflow only after basic mapping correctness is tested.

Acceptance: deterministic scheduler tests cover boundaries and clock changes; integration tests prove external input and UI target the same controls.

### P2 — release readiness

- Run a real tag-triggered release and inspect the published zip, ANGLE DLL provenance, PDB symbols archive, SBOM, checksums, and attestations.
- Verify the installed app can save diagnostic output without a console and that a crash report can be symbolicated with the matching PDB.
- Document install/upgrade/uninstall and a clean-machine smoke test.
- Keep platform claims honest: macOS renderer support still needs a suitable context provider; portable crate CI is not equivalent to a working macOS app.

## Regression traps for future agents

- `FrameInputs` and `strobe_scene::audio::SCALARS` are the renderer/runtime contract. Avoid parallel representations and unnecessary required fields that break struct-literal callers.
- Audio, scripts, playlist and input crates must not depend on `strobe-gfx`; keep GL ownership in the graphics crate.
- Do not perform synchronous GPU readbacks, blocking device enumeration, shader compilation, or heavy I/O in the frame/paint path.
- Keep native GL and ANGLE behavior under separate tests; “works under llvmpipe” does not prove Windows driver behavior.
- Keep benchmark frames and timing metadata together. llvmpipe CPU timings are useful for regressions and output comparison, not for estimating integrated-GPU bandwidth.
- Keep the release PDB matched to the exact binary. A symbols archive from another build is not useful for crash analysis.

## Handoff sequence

1. Read [ROADMAP.md](ROADMAP.md) and [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).
2. Reproduce or inspect the relevant baseline before editing.
3. Make one bounded change with a regression test.
4. Run `cargo xtask ci` and the smallest relevant hardware/backend test.
5. Update the roadmap and decision record in the same change.
6. Record evidence and remaining uncertainty; do not convert “compiled” into “verified”.
