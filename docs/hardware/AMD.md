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
  - **neon_city:** a striped plane with no buildings. A Strobe bug,
    found in A8 and A9 below: adaptive resolution lowers the render
    scale, and the scene's literal 1920x1080 passes are read with
    `texelFetch(passN, ivec2(_xy))`, so a smaller main pass reads only
    their corner.
  - **beamers:** a starburst, but with blue, orange and violet beams
    where the thumbnail's are white. Colour and time (A8).
  - **PLUS:** masked by media, and Synesthesia feeds `demo_media.mp4`
    by default where Strobe had none.
  - **canvas** and **turbo_encabulator_fluid:** the thumbnails match
    ANGLE's render (A4, A5), not AMD native's.
- **Inconclusive (2):** churning (its feedback has not formed by 3 s) and
  hue_review (no thumbnail).

Synesthesia's renderer is desktop GL 3.3 (GLFW, WGL). The authors' GPUs
therefore draw these `fract(sin(…))` hashes the way ANGLE/D3D11 does here,
and AMD's native driver is the one that differs.

## neon_city and beamers at 30 s, and neon_city at fixed scales (2026-10-14, commit 3fa3ab6, Tasks A8 and A9)

**A8.** Both scenes were run with `--frames 1800 --audio synthetic`,
native and ANGLE.
- **beamers:** a starburst on both backends, close to the thumbnail. The
  beams are tinted rather than white, and the focus is right of centre.
- **neon_city:** still the striped plane on both backends. Adaptive
  resolution logged 1.00 -> 0.50, and the output was 240x135.

**A9.** neon_city native, 180 frames:

| Run | Output | Picture |
|---|---|---|
| `--no-adaptive --render-scale 1.0` | 960x540 | buildings, a street canyon |
| `--no-adaptive --render-scale 0.5` | 480x270 | the bottom-left quarter of the 1.0 picture, magnified |
| adaptive (1.00 -> 0.50 -> 0.25) | 240x135 | the bottom-left sixteenth: the "striped plane" |

- **Cause:** the passes are a literal 1920x1080, absolute under D194, and
  the shader reads them with `texelFetch(passN, ivec2(_xy))`. Any main
  pass smaller than the passes reads only their corner.
- **Adaptive gains nothing here:** GPU time is 59.4 ms at 1.0 and 59.5 ms
  at 0.5, because the heavy passes do not shrink. Fixed by D280; see
  the section below.

## neon_city after D280 (2026-10-14, llvmpipe under Xvfb, not the RX 580)

The fix for A9 (D280): a scene with a root `WIDTH` x `HEIGHT`, a literal
buffer pass of that size, and a shader that reads such a pass with
`texelFetch` renders its final pass at that size. Run on the
private corpus, native GL, Xvfb at 1600x900, `--window-size 1280x720
--frames 180 --audio synthetic --no-panes`:

| Run | Before (A9, RX 580) | After (llvmpipe) |
|---|---|---|
| `--no-adaptive --render-scale 1.0` | 960x540, a quarter of the buffers | 1920x1080, the whole frame |
| `--render-scale 0.5` | 480x270, the bottom-left quarter | 1920x1080, the same picture byte for byte |
| adaptive on | 240x135, a sixteenth ("striped plane") | 1920x1080, the same picture; the log says "this scene renders at 1920x1080 whatever the render scale (D280), so the scale stays at 1.00" |

All three show the whole street of towers, noisy as a 180-frame path trace
is, with `scene.png`'s framing. Open: the same run on the RX 580, where GPU
time will show what the heavy passes cost at 1080p (59 ms at A9).

Re-checked after the rule was narrowed to texel reads (same setup, `--frames
30`, render scale 0.5): `neon_city` still draws 1920x1080 with the whole
street, and `entombed_silicon_dreams` (adaptive on) 1920x1080 with the whole
landscape; the log says the scale stays put (D280). `neuron_proximitors` and
`bouncy_hexagons`, no longer pinned, draw 480x270 at 0.5 and 960x540 at 1.0,
the same whole picture (RMSE 0.4% and 6.1% of full scale after upscaling,
the second from hexagon edges resolved at the higher size).

## D280 on the RX 580 (2026-10-14, commit 62204ed, Task A10)

**neon_city**, native, 180 frames:
- All four A9 runs (no adaptive at 1.0, at 0.5, at 1.0 with a 1920x1080
  window, and adaptive on) now give a 1920x1080 picture of the whole city.
- With adaptive on, the log says once that the scene renders at 1920x1080
  whatever the render scale (D280), and the scale stays at 1.00.

**entombed_silicon_dreams**, the other pinned scene, adaptive on: 1920x1080
and the whole scene, with the same log line.

**bouncy_hexagons**, adaptive on, 1800 frames:
- Not pinned, so it is drawn at 960x540 and correct.
- The probe lowered it 1.00 -> 0.80 twice. Both times GPU time barely
  moved (17.9 -> 17.7 ms, then 17.9 -> 17.8 ms), so it went back up, then
  locked lowering for 1800 frames.
- That was the right call: the scene's heavy pass is a literal 1920x1080
  FLOAT pass, absolute under D194, so the render scale cannot reduce it.

**Corpus:** 90/90 in both dialects. Frame ms (desktop/GLES): neon_city
60.3/45.6, bouncy_hexagons 19.2/17.0, entombed_silicon_dreams 14.6/15.2.

## Desktop remote control, host side (2026-10-15, commit d19cc24, Task A11, D275)

The RX 580 was the host at 10.1.10.206:47812, on a Private Wi-Fi network,
with the T480 controlling (T480 Task 45). No firewall prompt appeared on
either machine.

- **Approval.** The pairing banner read "Full desktop: Strobe desktop at
  10.1.10.30 wants full control, files included", and the pairing was
  approved.
- **(a)** The title bar showed "Desktop controlled remotely · 1
  connected".
- **(b)** Remote Next loaded 02-plasma-pulse on the host. The controller
  also opened the Editor and loaded more scenes.
- **(c)** A remote "Pair a device" was refused: `refused
  OpenRemotePairing; it is host-only`, with the host-only banner shown.
- **(d)** Streaming had no measurable cost on the output: 59.9 fps,
  1-2 late frames a minute, control redraw 2.2-2.4 ms against 2.0-2.2 ms
  idle. Worst gaps of 72-92 ms came with scene loads and the Editor
  opening, and a local load costs as much (111 ms). This was not a clean
  same-scene comparison.
- **(e)** Switching desktop control off revoked the controller (`1
  desktop clients revoked`), and the viewer dropped within about 0.6 s.
- **(f)** A reconnect with no code was refused. A fresh code and a fresh
  approval gave a new client id.
- **Automation note, not a product issue:** posted mouse input lands only
  when the move, down and up messages are posted back to back.

## The rebuilt scenes 13-16 on the RX 580 (2026-10-15, commit 469c8a8, Task A12)

strobe-bench `--context window` at 1920x1080, mean ms native / ANGLE:

| Scene | Native | ANGLE |
|---|---|---|
| 05-domain-warp (reference) | 1.54 | 1.48 |
| 13-chromatic-rise | 0.99 | 0.95 |
| 14-electric-bloom | 1.48 | 1.49 |
| 15-neon-ribbon-flow | 0.98 | 1.33 |
| 16-acid-mirror | 0.96 | 0.85 |

- **Cost:** all four run at 0.6-1.0x domain-warp on real hardware. llvmpipe
  overstated them (1.1-1.5x).
- **ANGLE:** compiles take 83-278 ms (native 38-72 ms). Neon Ribbon Flow is
  the only scene slower on ANGLE, by 0.35 ms.
- **Corpus:** 8/8 in both dialects.
- **Stability:** 1800 frames in the app, with no WARN or ERROR and no
  render-scale changes. All four are stable and vivid, with no fade or
  blow-out.
- **Minor, in Acid Mirror:** small mirrored dash fragments just below mid
  height, and hard, slightly aliased edges on the black voids.

## Linux under WSL2 (2026-10-15, Task A14, D295)

Debian 13 on WSL2 (WSLg), on the RX 580, running `scripts/check-linux.sh`.
`/dev/dxg` and Mesa's `d3d12_dri.so` were present.

| Step | Result |
|---|---|
| apt packages | SKIP (done beforehand) |
| Rust toolchain | PASS |
| Build (`cargo build --release -p strobe-app -p strobe-bench`) | PASS, 289 s |
| `strobe --help` | PASS |
| Headless bench (EGL device) | PASS, on llvmpipe |
| Bench on a window context | PASS |
| App window, default backend: screenshot | PASS |
| App window, X11 forced: screenshot | PASS |
| App window, Wayland forced: screenshot | PASS |
| MIDI | SKIP (no `/dev/snd/seq` on WSL2) |

- **Renderer.** Mesa chose llvmpipe by default. With `GALLIUM_DRIVER=d3d12`
  it is "D3D12 (Radeon RX 580 Series) | 4.2 Core", and the bench ran about
  3x faster at twice the pixels. The script now sets it when `/dev/dxg`
  exists (opt out with `STROBE_CHECK_NO_D3D12=1`).
- **Bench on D3D12 at 1280x720, ms per frame:** 05 0.90, 10 12.32, 12 2.84,
  16 0.69.
- **Audio was synthetic.** cpal opens ALSA `default`, which does not exist
  under WSLg (`arecord -D default`: "Unknown PCM default"), while
  `arecord -D pulse` works with WSLg's RDPSource. Fixed two ways: an
  `~/.asoundrc` with `pcm.!default pulse` and `ctl.!default pulse` (README,
  WSL2), and Strobe now tries an input named `pulse`, then `pipewire`, when
  the default will not open. The second is not yet re-run on this machine.
- **Not covered:** a real Linux GPU other than D3D12 under WSL2, and a
  physical audio input.
