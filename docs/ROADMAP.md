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

### In flight (parallel agents, 2026-10-08)

- `strobe-script` (Sonnet agent, worktree branch `agent/strobe-script`)
- `strobe-audio` (Sonnet agent, worktree branch `agent/strobe-audio`)
- `strobe-app` and the windowed native/ANGLE context (Sonnet agent,
  branch `agent/strobe-app`)
- `strobe-diag`: logging, panic and native crash reports, minidumps
  through an out-of-process monitor, GL debug output, `--diagnose`
  (Sonnet agent, branch `agent/strobe-diag`)
- Repo and CI alignment with the owner's rustaman/rustdirstat
  conventions: release workflow, issue forms, dependabot, `xtask`, a
  generated changelog, macOS CI, and bench artifacts in CI (Sonnet agent,
  branch `agent/repo-ci`)
- A real-hardware test session on the owner's ThinkPad T480 (remote
  bridge session in `C:\Users\ExaltedLaptop2\Development\strobe`):
  hardware report, a native MSVC build and tests, and later a real-GPU
  bench. Its report goes to `docs/hardware/T480.md`.

The coordinating session reviews each branch and merges it into
`claude/festive-ramanujan-oic192`.

## Next (in order)

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
| 0 | Workspace, conventions, CI; GL context spikes | Workspace, CI, and headless contexts done. Windowed native and ANGLE contexts not started. |
| 1 | Renderer, SSF scene loading, multipass, controls UI, output window | Renderer, loading, multipass, and the controls runtime done. Images done; cubemaps and media are placeholders. **No window or UI yet.** |
| 2 | JavaScript API | Not started |
| 3 | Audio analysis | Not started; the bench has a synthetic stand-in |
| 4 | Shadertoy import | Not started |
| 5 | Playlists | Not started |
| 6 | Presets, MIDI, OSC | Not started |
| 7 | NDI, media input, song-change transitions | Not started (`HARD`/`SMOOTH_TRANSITIONS` uniforms are declared and initialised, but never change) |

## Known gaps in what exists

- `syn_Spectrum` and `syn_LevelTrail` are silent placeholder textures.
- `syn_Media` is a black 1×1 texture, and `syn_MediaType` is 0.
- Cubemaps are declared with a unit reserved, but never loaded.
- Script-created uniforms (`Request::extra`) are plumbed through the
  assembler, but nothing produces them yet.
- The GLES dialect doesn't rewrite desktop-only SSF idioms (Q8 in
  OPEN_QUESTIONS.md). The base library is written ES-clean, so the
  bench's `--api gles` run passes.
