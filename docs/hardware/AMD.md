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
