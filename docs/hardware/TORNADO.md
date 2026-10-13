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

## 65 real Synesthesia scenes (2026-10-11, commit 963b535, Task T2)

These are the owner's scenes, kept local on this machine; none of them is in
the repo. Run on 1920x1080, with a window context, at release.

- **Compiles:** 47 of 65 on native, 35 of 65 on ANGLE. Before D192, ANGLE
  managed 8. `hello_world_start` (`vec4(1,0,0,0)`) now renders red on both
  backends.
- **Native failures (18):**
  - 8: the user-media library is missing (`syn_UserImage` and friends);
  - 3: `MEDIA` written as a list of slots;
  - 4: a control with no `TYPE`;
  - 2: a pass sized 0;
  - 1: the scene's own bug (it redeclares `fragColor`).
  D193 fixes the shape problems; the media library is still to do.
- **ANGLE-only failures (12):**
  - 10: implicit int to float;
  - 2: `#if` on a name that is not a macro;
  - 1: a CRLF `\` continuation, now fixed by D193. LF copies proved it,
    and they also fixed the error line numbers that came out as 2n-1.
- **Speed:** the slowest at 1080p were `wet_stone_1` 37.7 ms, `kifs_walls_`
  17.9, `wet_stone` 17.8 and `abstract_terrain_objects` 14.1. ANGLE runs
  within about 4% of native.
- **Scripts.** 50 scenes have `script.js`, but only 9 are non-empty. All 40
  that pass on native loaded in the app with no crash or SCRIPT fault.
  Script uniforms (`script_time`, `spectrum_0..3`, `BPMTri*` and others) are
  discovered and compiled in. A template script that names 8 controls its
  scene lacks now logs warnings (D193). No host function is missing.
- **Answers:**
  - Q24 is resolved as R7: pass sizes are absolute (D194).
  - Q22: real `MEDIA` is a list of slots; no cube maps.
  - Q21/Q23: the script API in use is listed in OPEN_QUESTIONS.
- **D191 check:** 07 at 1080p and 2160p shows the same spot size and
  layout.

## The 65 scenes again (2026-10-13, commit 40ee040, Tasks T4 and T5)

The same scenes at release with a window context, after D193 to D198.
Built from a detached worktree of `origin/master` under `target\tornado\`.

- **Compiles:** 55 of 65 on native (49 at T3) and 38 of 65 on ANGLE (no
  change). Newly passing on native: boreal, cinders, diffusion, dust,
  exoplanet_dup and panspermia. Their tiles look right, except diffusion,
  which is black because it is built from a user image the bench does not
  have.
- **Native failures (10):**
  - 4 fail only on script-created uniforms, because the bench runs no
    scripts. `cargo xtask corpus` runs them.
  - 2: `syn_pass_edgeDetectSobel` returns a `float` in Synesthesia, and
    the scenes assign it to one. Ours returned `vec4`.
  - 1: the one-argument `_correctImageCoords(textureSize(img, 0))`.
  - 1: `float _ratioFix(float, float)`, called with the frame-to-image
    ratio of each axis.
  - 2: the scenes' own bugs (a missing `return`, a redeclared
    `fragColor`).
- **ANGLE-only failures (17):**
  - 14: implicit int to float;
  - 2: `#if` on a plain `float` global (`TUBE_SHAPE`, `SHAPE`), which
    NVIDIA reads as 0;
  - 1: `uniform vec2 mousePos = ...;`, which GLSL ES rejects.
- **T5, call sites and the install** (read-only):
  - Synesthesia's own release scenes document `syn_pass_edgeDetectSobel`
    as `@returns {float}`.
  - `_hueSaturationContrastLEGACY` is only called, never defined or
    documented: `vec4 (vec4, float, float, float)`, from the call.
  - `syn_PrimeBang` is read only by scripts, as a number compared with 0.5.
  - The install's `lib\` documents none of these.
- **Fixed by D199:** all three ANGLE causes and the four library forms.
  **Not yet re-run.**
- **Note:** ANGLE reported the `#if` errors at lines 122 to 224 where the
  source has them at 243 to 593. Check this when re-running: a line offset
  from an `#include` would be a bug in the `#line` mapping.

## Pending on this machine

- [x] Re-run 07 at 1080p and 2160p after D191: same size (T2).
- [x] Re-run the 65 scenes after D193/D194 (T4): 55 native, 38 ANGLE.
- [ ] Re-run them after D199, native and ANGLE, plus `cargo xtask corpus`.
- [ ] The app fullscreen at 2560x1440 with a playlist show, transitions and
  the post stage, against the T480's numbers.
- [ ] A larger set of real Shadertoy shaders through NVIDIA's compiler
  (owner-supplied, kept local), to find NVIDIA-only acceptance.
- [ ] The Vulkan / wgpu compiler spike from
  [GFX_API_RESEARCH.md](../GFX_API_RESEARCH.md), stage 2, when it starts.
