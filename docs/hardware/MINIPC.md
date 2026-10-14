# JaJa PDX projector mini-PC

The venue machine that drives the projector. Reached through a Claude Code
bridge session that runs as a Windows **service** (session 0), so anything
it launches directly lands on an invisible desktop; real-display runs go
through a one-shot scheduled task in the signed-in user's session 1,
created and deleted per run with the owner's approval.

| | |
|---|---|
| Machine | AZW SER3 mini-PC |
| CPU | AMD Ryzen 3 3200U, 2 cores / 4 threads |
| RAM | 14 GB usable |
| GPU | AMD Radeon Vega 3 (integrated), driver 31.0.21925.1001 (2026-05-19); GL `3.3.0 Core Profile Context 26.5.2.260413` |
| OS | Windows 11 Pro 26H2, build 26300.9711 (updated from 25H2 26200 during a reboot on 2026-10-14) |
| Projector | Sony (EDID SNY8203), 1920x1200 @ 60 Hz, the only display at the console |
| Power | Balanced plan; display-off set to never (owner's request). System sleep is AC 15 min / DC 10 min, and it did idle-sleep once during testing |
| Tools | git, Rust 1.98, Visual Studio 2022; no `gh` |

## The released v0.1.0 on a clean install (2026-10-14, Task M2, R-03)

Unzipped from the published archive (SHA-256 verified) into
`%USERPROFILE%\strobe-test\`, against a fresh `%LOCALAPPDATA%\strobe`.

- **Native GL:** Vega 3, OpenGL 3.3 core. The only WARN is the known one:
  `WGL_ARB_create_context_robustness` is not supported, so a driver reset
  would not be reported. There are no ERRORs.
- **ANGLE:** starts in session 1, ES 3.0 on D3D11, with no robustness
  WARN. In session 0 it fails at surface creation (`EGL 0x300e`, context
  loss): a session with no presented desktop, not the GPU.
- **Scripts:** 11-script-orbits runs cleanly in session 1. One interrupt
  of the 4 ms per-frame script budget in session 0 came from contention
  with a Windows Update worker, and recovered on its own.
- **Shadertoy listener:** `--listen` binds 127.0.0.1:47811 only, and
  writes its token.
- **Relaunch over an existing data folder:** works. Frames are
  byte-identical across runs.
- **First run:** creates `logs\` and `run\`. Screenshot mode saves no
  `ui.toml`, by design.
- **Found and fixed on master after the release:**
  - A screenshot run never ended when the control window could not paint
    (session 0, or RDP). It now gives up after 10 s.
  - A fatal startup error raised a message box on an invisible desktop and
    hung. Unattended runs and service sessions now only log.
  - strobe-bench reported timings for all-black frames in session 0. It
    now fails when every frame is black.
- **Not fixed:** killed runs leave stale `run\strobe-crash-<pid>.sock`
  files, which nothing cleans up.

## The 12 shipped scenes on the projector (2026-10-14, Task M2 step 7)

The runs were in session 1, with `--show-output` and the output window
moved over the whole 1920x1200 desktop (client area 1904x1161, scale 1.0
before adaptive). Each ran 66 s with synthetic audio and a 60 Hz budget.

| Scene | fps | GPU ms | Late frames | Worst gap | Settled scale |
|---|---|---|---|---|---|
| 01-aurora-gradient | 60.0 | 3.90 | 0 / 3603 | 23.6 ms | 1.00 |
| 02-plasma-pulse | 60.0 | 4.92 | 4 / 3599 | 39.8 ms | 1.00 |
| 03-kaleido-tunnel | 59.8 | 6.47 | 11 / 3586 | 61.7 ms | 1.00 |
| 04-voronoi-glass | 59.2 | 12.73 | 45 / 3551 | 36.9 ms | 0.80 |
| 05-domain-warp | 59.1 | 12.24 | 36 / 3547 | 39.7 ms | 0.70 |
| 06-feedback-trails | 60.0 | 5.84 | 1 / 3599 | 25.4 ms | 1.00 |
| 07-reaction-diffusion | 59.9 | 7.15 | 2 / 3597 | 34.6 ms | 1.00 |
| 08-bloom-orbs | 59.5 | 13.12 | 8 / 3568 | 42.2 ms | 0.90 |
| 09-sdf-cathedral | 57.1 | 13.91 | 29 / 3429 | 72.6 ms | 0.50 |
| 10-volume-clouds | **27.7** | 33.36 | 1660 / 1661 | 554.6 ms | **0.25 (minimum)** |
| 11-script-orbits | 60.0 | 7.03 | 2 / 3600 | 25.1 ms | 1.00 |
| 12-silicon-atlas | 56.6 | 14.14 | 24 / 3399 | 119.4 ms | 0.40 |

**ANGLE**, the three heaviest:
- 09: 56.1 fps at 0.50.
- 10: 28.8 fps at 0.25.
- 12: 56.6 fps at 0.40.

These are within about 1 fps of native. 12-silicon-atlas takes
**6.5 s to load on ANGLE**, against 1.4 s native, because of D3D shader
compilation.

**Summary:**
- Six scenes hold 60 fps at full scale.
- 04, 05 and 08 hold 60 fps by scaling down.
- 09 and 12 are marginal.
- 10-volume-clouds cannot hold 30 fps even at the minimum scale. It needs
  a cheaper path on this class of GPU.

## Still to do here

- strobe-bench in session 1. In session 0 it renders black.
- The JaJa test card on the projector at native 1920x1200, with
  `--output-monitor 1 --fullscreen --render-scale 1 --no-adaptive`. The
  flags landed after the run above.
- The owner's call on system sleep for show use.
