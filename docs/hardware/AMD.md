# Test machine: AMD desktop (DESKTOP-30KSOCM)

The owner's desktop, and Strobe's first AMD GPU. The T480 covers Intel's
driver and TORNADO covers NVIDIA's; this machine covers AMD's OpenGL
driver and ANGLE on AMD's D3D11 driver. A remote bridge session runs on it
(working copy `Documents\Development\strobe`), driven by the coordinating
cloud session. It writes only under `target\amd\`.

## Hardware and software (Task A1, 2026-10-13)

| | |
|---|---|
| OS | Windows 10 Pro 22H2, build 19045 |
| CPU | AMD Ryzen 5 5600X, 6 cores / 12 threads |
| RAM | 16 GB |
| GPU | AMD Radeon RX 580 Series, 8 GB |
| GPU driver | 31.0.21925.1001 (2026-05-19), CNVersion 23.19.25.01 |
| Native GL | `Radeon RX 580 Series \| 3.3.0 Core Profile Context 26.5.2.260413` |
| ANGLE | `ANGLE (AMD, Radeon RX 580 Series (0x000067DF) Direct3D11 ... D3D11-31.0.21925.1001)`, from Electron 38.2.0 via `scripts\fetch-angle.ps1` |
| Display | One, 1920x1080 @ 59 Hz |
| Toolchain | Rust 1.98.0 MSVC, Visual Studio 2022, Windows SDK 10.0.26100 (QuickJS builds) |

## Baseline (Task A1, commit 26576b0)

- **Build:** release, 0 warnings, 4 min 28 s.
- **Tests:** 1555 pass. 259 GL tests skip, because Windows has no EGL
  device, as on the T480.
  - Two or three failures came from a race between Shadertoy render tests
    importing the same fixture into one staging folder ("Access is
    denied", os error 5). With `--test-threads=1` they passed every time.
  - Fixed: each test now imports into its own folder.
- **Benchmark:** every scene renders on both backends at 720p and 1080p.
  Native and ANGLE contact sheets are pixel-identical (mean difference
  0.00, a few edge pixels in 03).
  - At 1080p the heaviest scenes, native/ANGLE, are 10-volume-clouds
    26.96/25.28 ms (the only one under 60 fps), 12-silicon-atlas
    5.49/5.56 ms and 09-sdf-cathedral 3.39/3.20 ms.
  - ANGLE is as fast as native or faster almost everywhere.
  - Native's p95 reaches about 2.6 ms on 04, 05 and 08 at 1080p.
  - ANGLE compiles 12-silicon-atlas in 1588 ms, against 8.8 ms native.
- **Corpus:** desktop 12/12, GLES 12/12.
- **Readback (D245):** all five checks pass on both backends. With the
  GPU busy, native dropped 87 of 90 reads (age 0) and ANGLE dropped 29
  (age 2.3).
- **App:** 158 s with three scenes, ending with a clean exit on `WM_CLOSE`
  and no crash files.
  - Perf lines: 59.9 fps, 3 late frames a minute, a worst gap of 43-49 ms,
    GPU 4-6 ms.
  - KHR_debug at the All level: 0 messages.

## Findings

- **No robust context on native WGL.** The driver refuses
  `WGL_ARB_create_context_robustness`, so a GPU reset on the native path
  is never reported here, and context-loss recovery (D66) cannot see one.
  ANGLE gets a robust context. Recorded in COMPATIBILITY.md. A reset drill
  on this machine is still to do.
- **Audio capture reopened about 70 times a second.** The microphone
  (High Definition Audio Device, 44.1 kHz) reported WASAPI buffer glitches
  ("A buffer underrun or overrun occurred"). Every stream error marked
  the stream failed, and the worker reopened at once with no backoff:
  11,200 reopens and 33,600 log lines in 158 s.
  - Fixed by D266: glitches are counted, not failures, and repeated quick
    failures back off.
  - **Re-checked in Task A2 (commit 2e678e4):** 188 s with 0 failures, 0
    stream errors and 0 glitches. One "capturing" line, and the stream was
    kept. A1's glitches came from the reopen loop itself.
  - Whether the levels follow sound is untested here. The microphone hears
    nothing (-47 dBFS with speech playing, because output goes to
    headphones), and the app has no input-level meter to show it. "Audio
    live, 120 BPM" looks the same on a dead microphone.

## Task A2 (commit 2e678e4)

- **Tests, in parallel:** 1559 passed, 0 failed; the strobe-shadertoy
  render tests are 14/14 with the per-test import folders. 266 GL tests
  skip, because Windows has no EGL device.
- **Benchmark tables:** in [BENCHMARKS.md](../BENCHMARKS.md).
- **App on ANGLE (`--gl angle`, 71 s):**
  - Robust context: "lose context on reset".
  - ANGLE 2.1.25847 on D3D11 31.0.21925.1001.
  - 05 loaded in 102 ms against 19 ms native.
  - 60.0 fps, 0 late frames, GPU 1.69 ms.
  - No WARN or ERROR lines.

## Synesthesia's 90 default scenes (Task A4, commit 776ddf3)

The owner's private corpus was cloned on this machine and never copied
into the repository.
- **Corpus run** (`cargo xtask corpus`, scripts on): 88 of 90 in both
  dialects, and all 48 scripted scenes ran.
  - The two failures were SkaiWalkerOG (its `#include` of Synesthesia's
    `hg_sdf.glsl`; no Synesthesia `lib` here) and hue_review
    (`_hueSaturationContrastLEGACY`). Both are fixed by D268.
- **Slowest frames, 160x90:**
  - neon_city: 57.7 ms native, 42.7 ms ANGLE.
  - bouncy_hexagons and entombed_silicon_dreams: 17-19 ms each.
  - The median is 1.76 ms native and 1.00 ms ANGLE.
- **Compile time:** 11.8 s native and 26.8 s on ANGLE in total; the worst
  single ANGLE compile is 2.08 s.
- **Black frames:** eight scenes are black, all of them media-driven
  (`_loadMedia`), and the harness supplies no media.
- **Native against ANGLE,** compared on the 47 scenes `strobe-bench` can
  render (the bench runs no scripts):
  - 25 frames are identical and 36 nearly so.
  - `PLUS` differs because of its own `pow` of a negative base, which is
    undefined in GLSL.
  - `canvas`'s feedback background differs: grey on native, black on
    ANGLE.
  - `turbo_encabulator_fluid` shows banding on ANGLE.
  - Four simulations diverge in detail (dynamical_flame, shrouded,
    gaussian_smoothlife, fuzzy_signal), and churning has a slight hue
    shift.

## The 90 again, and the banding (Task A5, commit e47c44f)

- **Corpus:** with `STROBE_GLSL_PATH` set to Synesthesia's `lib` (the
  corpus copy), the run gives 90 of 90 in both dialects, with no notes.
  - SkaiWalkerOG renders in 5.45 ms native and 3.81 ms ANGLE.
  - hue_review renders in 0.32 / 0.52 ms.
  - thresholder's "uniforms.freezes" note is gone.
- **`turbo_encabulator_fluid`'s banding:** it does not come from the
  render-target format.
  - With `--full` (RGBA32F), ANGLE shows the same striated ripples as
    with RGBA16F, and native is smooth in both.
  - The ES preamble is `highp` and the buffers are the scene's literal
    1600x900 FLOAT passes.
  - Likely cause: the same class as canvas. Two `fract(sin(…))` hashes
    in the scene, where D3D's `sin` precision differs from the native
    driver's and the fluid feedback amplifies the difference.
  - Not confirmed: that needs a local copy with an integer hash.
- **Timings at 640x360, mean/p95 ms:**
  - native: Half 1.32/1.61, Full 1.54/1.79;
  - ANGLE: Half 1.09/1.31, Full 1.45/1.75.

## Against Synesthesia itself (Task A6, commit e47c44f)

**Synesthesia 1.25.4.189** is installed at `C:\Program Files (x86)\Synesthesia`.
- Built-in scenes are in `release\` (87) and `default\` (welcome_scene).
- Includes are in `lib\`.
- There is no default folder for the user's own scenes. Those live in the
  folders listed under `sceneFolders` in
  `%LOCALAPPDATA%\Synesthesia\Data\vs.json`, with `PRIMARY_SCENE_FOLDER`
  and recursive search set in `preferences.json`. Both are empty on this
  machine.
- OSC input is a Pro-licence feature, off on this free install, so
  Synesthesia could not be driven. The comparison used Synesthesia's
  shipped thumbnails (the authors' own captures) against Strobe's output
  after 3 s.

Results for 17 scenes, all of which exit cleanly with no WARN or ERROR:
- **Same (3):** test_card, kifs_flythrough, terrain_lattice.
- **Close: colour, camera or time only (8):** SkaiWalkerOG, welcome_scene,
  alien_cavern, hexoplanet, glassier, stained_glass, fluid_body.
- **Different (5):**
  - **neon_city:** a striped plane with no buildings. Under
    investigation.
  - **beamers:** horizontal blue beams against a white starburst.
    Under investigation.
  - **PLUS:** masked by media, and Synesthesia feeds `demo_media.mp4`
    by default where Strobe had none.
  - **canvas** and **turbo_encabulator_fluid:** the thumbnails match
    ANGLE's render (A4, A5), not AMD native's.
- **Inconclusive (2):** churning (its feedback has not formed by 3 s) and
  hue_review (no thumbnail).

Synesthesia's renderer is desktop GL 3.3 (GLFW, WGL). The authors' GPUs
therefore draw these `fract(sin(…))` hashes the way ANGLE/D3D11 does here,
and AMD's native driver is the one that differs.
