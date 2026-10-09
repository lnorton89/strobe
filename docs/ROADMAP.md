# Roadmap and status

**Update this file whenever you finish something.** It's the first thing
the next agent reads. Keep "Now", "Next", and the phase tables current.
Design detail lives in [PLAN.md](PLAN.md) and decisions in
[DECISIONS.md](DECISIONS.md).

_Last updated: 2026-10-08._

## Now

**Goal set by the owner:** get a working copy running in the cloud
environment that can be tested, verified, and benchmarked, with a
10-scene library spanning GPU load as both the benchmark and the base
scene library. Then improve performance from measured data.

The following is done and verified headless:

- Workspace, conventions, and CI: `strobe-conventions` checks headers,
  the unsafe boundary, and the toolchain pin.
- `strobe-scene`: SSF parsing, validation, controls runtime, and shader
  assembly. 43 tests.
- `strobe-ssf-lib`: every documented SSF function. Compiles on GL 3.3
  core and GLES 3.0.
- `strobe-gfx`: the GL boundary, headless EGL contexts, and the scene
  player (multipass, ping-pong, `syn_FinalPass`, images). Seven render
  tests in both dialects.
- `strobe-bench`, plus the 10-scene library in `scenes/`. The baseline
  is in [BENCHMARKS.md](BENCHMARKS.md).
- `strobe-diag` (branch `agent/strobe-diag`): logging with a session
  file and UI ring, session context, structured failures, a panic hook,
  out-of-process native crash capture with minidumps, and GL driver debug
  output (`strobe-gfx`'s `gl::debug`). Crash pipeline verified end to end
  on Linux; the Windows half is compile-checked only until the T480
  session runs it. See [DEBUGGING.md](DEBUGGING.md) and decisions D40 to
  D45. **Not yet wired into an app**: `strobe-app` must follow the
  integration steps in the `strobe_diag` crate docs.

- Repo and release plumbing (branch `agent/repo-ci`): `cargo xtask ci |
  changelog | bench`, a generated `CHANGELOG.md`, a tag-triggered release
  workflow (Windows zip with ANGLE and scenes, separate PDB symbols
  archive, SBOM, checksums, provenance attestations), a macOS job (portable
  crates blocking; `strobe-gfx` awaits a CGL context), bench frames
  uploaded from CI, issue forms, dependabot, CODEOWNERS and SECURITY.md.
  Decisions D50-D59. Untested until the first push: the release workflow
  itself, which only runs on a tag.

### Merged 2026-10-09

The parallel agents' branches were reviewed, gated, and merged:
`strobe-audio` (analysis, capture, WAV harness), `strobe-script`
(QuickJS host), `strobe-diag` (logging, crash reports, minidumps through
a monitor process, GL debug output), `strobe-app` (control and output
windows sharing one context, with native/ANGLE fallback), and repo/CI
alignment (`cargo xtask ci`, release workflow, a generated changelog,
issue forms, and frames from the bench as CI artifacts).

### In flight

- T480 real-hardware pass: app smoke test on native GL and ANGLE,
  per-scene fps, and the crash-capture drill on Windows. Results go into
  `docs/hardware/T480.md`.

Done since: app integration (D60-D65). The app now has strobe-diag
logging, crash capture and `--diagnose`; live audio capture with a device
picker and a synthetic fallback; scene scripts (see
`scenes/11-script-orbits`); GPU timer queries driving adaptive
resolution; and a control window that opens maximized.

Also done: `strobe-bench --context window|headless|auto` and `--gl
native|angle|auto` (D70-D72). The bench now runs on a hidden-window
context where the driver has no EGL device (Intel on Windows). Window
mode renders the same images as headless, pixel for pixel, on llvmpipe.

## Next (in order)

0. **Survive a lost or unusable GL context** (T480 bug 1): detect it
   (`make_current` or `swap` failure, `GetGraphicsResetStatus`),
   recreate the context, fail over to ANGLE if native keeps failing,
   reload the scene, and record a structured failure. A projector must
   never go dark because one driver call failed.

1. **`strobe-app` window shell.** A winit event loop, one GL context with
   two surfaces (the egui control window and the fullscreen output
   window), a scene browser over `scenes/`, control widgets per control
   type, and an error console. Verify it under Xvfb with screenshots.
2. **Windowed context providers in `gl/context.rs`.** Native first, then
   ANGLE: try native GL 3.3 and fall back to ANGLE on failure or by
   flag. Ship ANGLE's `libEGL.dll`/`libGLESv2.dll` beside the exe.
   Decide where those come from (build from source? Chromium/Edge
   redistributable?) and log it in DECISIONS.md.
3. **Performance pass, driven by the bench.** Add render scale and
   adaptive resolution (drop the scale when frame time exceeds budget,
   raise it when there's headroom). Make the final pass optionally
   render at lower scale with an upscale blit, and decide the float
   pass precision default on real GPU data. Note that llvmpipe can't
   measure bandwidth, so RGBA16F needs a real GPU run.
4. **Phase 2: JavaScript API (`strobe-script`, rquickjs).**
5. **Phase 3: audio (`strobe-audio`).** This replaces the bench's
   synthetic audio with real analysis behind the same `SCALARS` contract.
6. **Port rustaman's UI system** (D5) into the app.

## Phases

| Phase | Scope | State |
|---|---|---|
| 0 | Workspace, conventions, CI; GL context spikes | Workspace, CI, headless and windowed (native, ANGLE) contexts done. |
| 1 | Renderer, SSF scene loading, multipass, controls UI, output window | Renderer, loading, multipass, controls runtime and a minimal desktop app (two windows, scene browser, controls, console, preview) done. Images done; cubemaps and media are placeholders. The Perform-first UI is not built. |
| 2 | JavaScript API | `strobe-script` built and wired into the app (D62): events, `update`, controls, `setUniform` compiled in at load, console, faults shown. Meta controls and `textureFinalPass` pending. |
| 3 | Audio analysis | `strobe-audio` built and wired into the app (D61): capture with synthetic fallback, textures uploaded, settings UI. The bench still uses the synthetic stand-in and does not upload textures. |
| 4 | Shadertoy import | Not started |
| 5 | Playlists | Not started |
| 6 | Presets, MIDI, OSC | Not started |
| 7 | NDI, media input, song-change transitions | Not started (`HARD`/`SMOOTH_TRANSITIONS` uniforms are declared and initialised, but never change) |

## Known gaps in what exists

- In the bench, `syn_Spectrum` and `syn_LevelTrail` are silent placeholder
  textures (the app uploads real or synthetic ones every frame).
- `syn_Media` is a black 1×1 texture, and `syn_MediaType` is 0.
- Cubemaps are declared with a unit reserved, but never loaded.
- The bench does not run scripts, so a scene whose shader needs a
  script-created uniform does not compile there; `11-script-orbits` drives
  controls instead (D62).
- `textureFinalPass` returns black in scripts (Q14); script `meta/` control
  writes are dropped.
- The GLES dialect doesn't rewrite desktop-only SSF idioms (Q8 in
  OPEN_QUESTIONS.md). The base library is written ES-clean, so the
  bench's `--api gles` run passes.
