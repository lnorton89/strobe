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

## After D199 (2026-10-13, commit bc4668e, Task T6)

- **No regressions:** every scene that compiled in T4 still compiles on
  both backends.
- **Bench:** 56 of 65 on native (T4: 55) and 56 of 65 on ANGLE (T4: 38).
  The two backends now fail the same 9 scenes:
  - 7 fail only on script-created uniforms, because the bench runs no
    scripts;
  - 2 are the scenes' own bugs (a missing `return`, a redeclared
    `fragColor`).
- **`strobe-corpus` on Windows** (a hidden window, native GL 3.3 and ANGLE
  ES 3.0) runs the scripts: 59 of the 62 top-level scenes pass in both
  dialects. The three left:
  - the two scene bugs;
  - `circuit_bending_dup`, whose `images/` holds `...dup2-01.png` while
    `scene.json` names `...dup2.png`. D201 now plays it with that picture
    black.

  With the three scenes in subfolders, 62 of 65 pass.
- **Tiles:** 8 scenes compared across the two backends, 7 identical.
  `magnetismic` is brighter on ANGLE, with its smoke further right. It is a
  feedback scene with a 10x10 data pass: likely precision or format in the
  persistent passes. Open.
- **The halved line numbers** came from the files: 38 of the 65
  `main.glsl` end their lines `\r\r\n`. Drivers count the `\n`; editors
  that treat a lone `\r` as a line count double. D201 normalises every
  line-ending form to one `\n`.

## magnetismic, native against ANGLE (2026-10-13, commit 54a9b19, Task T7)

- **The scene.** Pass `var` is 10x10, `FLOAT`, single-buffered. It reads
  its own target and adds `syn_Intensity`: an accumulator. Every pass reads
  `var` to speed up its clock.
- **The difference grows with frames.** Mean difference per 8-bit channel,
  native against ANGLE:

  | Frame | Mean difference |
  |---:|---:|
  | 1 | 0.00 |
  | 2 | 0.44 |
  | 10 | 3.38 |
  | 60 | 10.35 |
  | 100 | 19.25 |

  - **Formats do not matter:** 16F and 32F give the same gap.
  - **ANGLE's accumulator never grows:** its 16F and 32F runs stay
    identical.
- **The cause** is a feedback loop, which GL leaves undefined. NVIDIA reads
  the previous contents, so the value accumulates. D3D11 cannot bind one
  texture as both shader input and render target, so ANGLE reads zeros and
  the picture barely moves.
- **The proof:** with `DOUBLE_BUFFER` added to `var`, native and ANGLE
  match (0.00 at 10 and at 100 frames). Double-buffered native is
  bit-identical to single-buffered native.
- **The fix** is D203, confirmed in T8.

## D203 confirmed, and a full re-run (2026-10-13, commit 1199b76, Task T8)

- **`magnetismic`, native against ANGLE:** 0.00 mean (max 3) at both 10
  and 100 frames. T7 measured 3.38 and 19.25. ANGLE is bit-identical to
  T7's hand-made `DOUBLE_BUFFER` copy.
- **Passes D203 double-buffers** (the same on both backends):
  `cinders/backbuffer`, `diffusion/backbuffer`, `dust/backbuffer` and
  `magnetismic/var`. ANGLE changed only for those scenes, and moved
  towards native (`dust` 1.72 to 0.57, `cinders` 0.75 to 0.24). Native is
  bit-identical to T6 for all 53 scenes that compile.
- **Unchecked:** the 9 scenes the bench cannot compile never link, so
  nothing is known for them. A text search suggests `fluid_body_dup`,
  `mana` and `vein_melter_dup` sample their own targets too.
- **Counts:** bench 56 and 56 of 65, as in T6, with no regressions. The
  corpus harness, scripts on, passes 60 of the 62 top-level scenes in each
  dialect: `circuit_bending_dup` now plays with its missing picture black
  (D201). The 2 left are the scenes' own bugs. With the 3 scenes in
  subfolders, 63 of 65.
- The bench installed no logger, so D203's log line went nowhere. It now
  logs to stderr.

## Two backend differences that are the scenes' own (2026-10-13, commit 3533e8b, Task T9)

Both scenes are single-pass with no feedback, and the difference is there
from frame 1 without growing.

- **`materialmakertest2`** (18.58 mean, max 239):
  - **Cause:** Material Maker's generated code computes `pow(pow(x, y), z)`
    with inputs that are exactly 0 on half the floor stripes. GLSL leaves
    `pow(0, 0)` undefined. NVIDIA gives NaN, which is written as black.
    D3D11 under ANGLE gives 0, so those stripes shade as dark grey with a
    highlight.
  - **Evidence:** forcing `0^0 = 0` makes the backends agree, and native
    then equals the original ANGLE image bit for bit.
  - **Verdict:** no fix. The value the scene "wants" is a NaN.
- **`warped_extruded_skewed_grid`** (4.97 mean, max 250):
  - **The blinking blocks** (about 4.4 of the 5) come from a
    `fract(sin(dot(p, k)) * 43758.5453)` hash. `sin` of a large argument
    differs in its low bits between NVIDIA's GL and D3D's compiler, and
    the scale and `fract` amplify that.
  - **About 0.5** is a `texture()` call with implicit LOD inside a
    raymarch loop. Derivatives are undefined in non-uniform control flow,
    so the backends pick different mip levels.
  - **Evidence:** making both hashes constant and using `textureLod`
    makes the backends agree exactly (0.00).
  - **Verdict:** no fix. A precise-`sin` or `textureLod` rewrite would cost
    performance or change filtering that other scenes rely on, and the
    scene looks the same apart from which blocks glow.

Hashes built on `fract(sin(...))` differ across GPU vendors in every
engine. A scene that needs identical noise everywhere should use an
integer hash.

## Fullscreen show soak at 2560x1440 (2026-10-13, Task T10)

The transition demo playlist ran as a 10-minute fullscreen show on the
2560x1440 panel (release build, native GL, the GTX 1080 Ti).

| | Result |
|---|---|
| Show | 140 cues and 104 transitions in 11.6 min, all finished; no shader compiled after load |
| Warnings | Only the expected `ghost-scene` skip (once per loop), and one audio buffer error at about 3 min that recovered by itself |
| Fullscreen | F11 at 15 s; no resize after that, and no occlusion lines (winit reports none on Windows) |
| Memory | Flat: working set 144-145 MB, private 290-305 MB with no trend |
| GPU | 1.8-2.8 ms a frame (the T480: 1.9-2.6 ms) |
| Frame rate | **48-56 fps** from the status bar every 30 s, once 26 just after the audio error |
| Close | 0.15 s, no process left |

The frame rate is the finding: the GPU is nearly idle, so the time goes
elsewhere, and the log had nothing to say where. Strobe's output window is
the only vsynced one, and the control window (vsync off, 30 Hz) was fully
covered by the fullscreen output on the same monitor. An unsynced swap on a
covered window can still wait for the compositor's vertical blank on
NVIDIA, which at 30 Hz would cost about every other output frame its blank.
That is the suspect, not yet proved. D206 adds what proves or clears it: a
`perf:` log line every minute (late frames, the worst gap, the control
window's redraw cost with its swap), and a 2 Hz control window while the
output covers it.

## The perf line, fullscreen and windowed (2026-10-13, commit 14b1ea5, Task T11)

**Run A, output fullscreen over the control window, 6 min:** after the
first minute every `perf:` line read **59.9 fps with 2 to 3 late frames a
minute** (under 0.1%). The first minute's 196 late frames include start-up
and the 8 s before fullscreen. The status bar still said 56 fps at the end:
it is painted by the control window, and Windows hardly repaints a covered
window. The control window redrew **0 to 2 times a minute**, whatever it
asked for. So **T10's 48-56 fps was a stale status bar, not the show**:
the show was at 60. The `control window covered` line was logged as
designed, but on Windows the 2 Hz throttle changes nothing, because the
paints were not arriving anyway.

**Run B, output windowed beside the control window (one monitor), 6 min:**
for 4 minutes the control window redrew about 25 times a second at **about
18 ms each** (worst 450 ms), and **about 37% of output frames were late**,
though the fps column still read 59.5-59.9. Then, with nothing in the log
to mark it, the redraws fell to 2-3.5 ms and the late frames to 117, then
4, a minute. An 18 ms redraw is one refresh: the control window waiting
for a vertical blank somewhere, although its swap interval is 0, and
each such wait makes the next output frame late. A GPU clocking up after
minutes of load would also fit the drop. The perf line now times the
control window's swap separately, to say which (T12).

## The swap is not the wait (2026-10-13, commit 7a1ab45, Task T12)

T11's run B (output windowed beside the control window) twice, back to
back, 6 minutes each, with the swap timed on its own. Every minute of both
runs: the control window redrew 26-29 times a second at **16.9-17.6 ms
each**, **34-42% of output frames were late**, and fps read 59.5-59.9. The
swap call itself returned in **0.03 ms** (worst 3.9 ms). T11's drop to
2-3 ms after 4 minutes did not come back in either run: it was a one-off.

So the control window waits for about one refresh somewhere else in its
redraw, before the swap: making the context current on its surface, or
egui's paint. NVIDIA's swap only queues the present, and a forced global
"Vertical sync" in the driver's settings would override the app's swap
interval 0 and surface the wait at the next call that needs a back buffer.
The perf line now splits the redraw into make-current, layout, paint and
swap (T13). The driver's global settings could not be read without taking
over the desktop; the owner can see them under NVIDIA Control Panel >
Manage 3D settings > Global.

## Pending on this machine

- [x] Re-run 07 at 1080p and 2160p after D191: same size (T2).
- [x] Re-run the 65 scenes after D193/D194 (T4): 55 native, 38 ANGLE.
- [x] Re-run them after D199, native and ANGLE, plus `cargo xtask corpus`
  (T6): 56 and 56, 62 with scripts.
- [x] Why `magnetismic` differs between native and ANGLE (T7): a pass
  samples its own target. Fixed by D203, confirmed in T8.
- [x] `materialmakertest2` and `warped_extruded_skewed_grid` differ
  between the backends (T9): undefined behaviour in the scenes, with no
  Strobe fix.
- [x] The app fullscreen at 2560x1440 with a playlist show and transitions
  (T10): clean and flat, but 48-56 fps with the GPU idle.
- [x] T10 again after D206 (T11): fullscreen was 60 all along (the T10
  number was a stale status bar); windowed, a visible control window cost
  late frames for 4 minutes.
- [x] T12: the swap returns in 0.03 ms; the wait is elsewhere in the
  redraw, and the 4-minute drop does not recur.
- [ ] T13: the same run with the redraw split into make-current, layout,
  paint and swap.
- [ ] A larger set of real Shadertoy shaders through NVIDIA's compiler
  (owner-supplied, kept local), to find NVIDIA-only acceptance.
- [ ] The Vulkan / wgpu compiler spike from
  [GFX_API_RESEARCH.md](../GFX_API_RESEARCH.md), stage 2, when it starts.
