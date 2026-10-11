# Test machine: TORNADO (owner's desktop, NVIDIA)

This is the owner's main machine, used for **NVIDIA-specific** checks: the
driver's GLSL compiler, ANGLE on a discrete card, and the top of the
performance range. The T480 ([T480.md](T480.md)) stays the reference for
the old-laptop target. A Remote Control session runs here; its working
copy is `C:\Users\Lawrence\documents\dev\strobe`. Agents write only under
`target\tornado\` and set `STROBE_DIAG_DIR=target\tornado\diag`.

## Hardware (reported 2026-10-11)

| | |
|---|---|
| GPU | NVIDIA GeForce **GTX 1080 Ti** (primary; every run below), plus a GTX 1070 that sits idle |
| Driver | 582.78 (`32.0.15.8278`) |
| GL string | `NVIDIA GeForce GTX 1080 Ti/PCIe/SSE2`, `3.3.0 NVIDIA 582.78` |
| ANGLE | `ANGLE (NVIDIA, ... Direct3D11 vs_5_0 ps_5_0, D3D11-32.0.15.8278)`, OpenGL ES 3.0.0 (ANGLE 2.1.25847, from Electron 38.2.0) |
| OS | Windows 10 Pro 19045 |
| Display | one 2560x1440 panel at 100% |
| Toolchain | rustc 1.98.0, `x86_64-pc-windows-msvc` |

## Baseline (2026-10-11, commit 862ad2e, Task T1)

- **Build.** `cargo build --release` of the app, bench, importer and
  autocontrol: clean, no warnings, 2 min 45 s.
- **Scenes on native GL.** All 11 compiled and rendered correctly at 1080p
  and 2160p; the contact sheets have nothing black or corrupted. The
  timings are in [BENCHMARKS.md](../BENCHMARKS.md). `10-volume-clouds` is
  the only scene not effortless: 13.3 ms at 1080p, 52.9 ms at 2160p.
- **ANGLE (D3D11).** Every scene renders the same, and frame times are
  close to native or slightly faster. Compile times are higher
  (`10-volume-clouds` 49.6 ms native against 294.4 ms on ANGLE, because of
  D3D shader translation). Shows precompile at load (D151), so this costs
  load time, not frames.
- **One program per scene (`--shared`).** Frame times track the per-pass
  programs; compile times are lower (09: 57.2 ms against 39.3 ms). On a
  fast GPU, D-decision "one program per pass" costs compile time and wins
  nothing visible, unlike on Intel (see the T480 numbers).
- **Driver debug output** (`STROBE_GL_DEBUG=1`, three scenes, 300 frames
  each). No compile, recompile or pipeline-state warnings. Two kinds of
  message appeared:
  - `id=131185` "Buffer detailed info ... will use VIDEO memory": NVIDIA's
    informational notice on buffer use, hundreds a second, all collapsed by
    the log's repeat suppression. It is noise, not a problem.
  - `id=131154` "Pixel-path performance warning: Pixel transfer is
    synchronized with 3D rendering": twice per run, at the screenshot's
    `glReadPixels`, which is expected in `--screenshot` mode only.
- **The nine Shadertoy fixtures.** All imported, compiled and rendered on
  native NVIDIA GL and on ANGLE, and the contact sheets match. No
  NVIDIA-only leniency or strictness showed up on this set
  (`missing_texture` reads black, as designed).
- **Auto-controls on the window context.** For `05-domain-warp` with
  `--force-merge`, native and ANGLE kept the same four controls (`speed`,
  `color_1..3`), with scores equal to four or five decimals, and the same
  suggestions.
- **Found: `07-reaction-diffusion` looked different at 2160p.** Its
  simulation grid followed the output size (D11), so the Gray-Scott spots,
  a fixed number of cells across, came out half the size on screen. Fixed
  by `FIXED_SIZE` passes (D191). To be re-checked here.

## Pending on this machine

- [ ] Re-run 07 at 1080p and 2160p after D191: the pattern should be the
  same size on screen.
- [ ] The app fullscreen at 2560x1440 with a playlist show, transitions and
  the post stage, against the T480's numbers.
- [ ] A larger set of real Shadertoy shaders through NVIDIA's compiler
  (owner-supplied, kept local), to find NVIDIA-only acceptance.
- [ ] The Vulkan / wgpu compiler spike from
  [GFX_API_RESEARCH.md](../GFX_API_RESEARCH.md), stage 2, when it starts.
