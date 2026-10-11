# Benchmarks

Measured with `strobe-bench`, which renders every scene in `scenes/` on
a headless or window-owned context and times each frame with `glFinish`. Frames are
deterministic: fixed `dt`, and synthetic 120 BPM audio from
`strobe_scene::synthetic`.

```bash
cargo run --release -p strobe-bench -- --size 1280x720 --frames 30 --warmup 90
#   --full      RGBA32F instead of the default RGBA16F for FLOAT passes
#   --shared    one program per scene with a PASSINDEX uniform (Synesthesia's way)
#   --api gles  compile in the GLSL ES dialect the ANGLE path uses (headless)
```

### Which context: headless or window

| `--context` | Context | Use |
|---|---|---|
| `headless` | An EGL device display, no window system | CI, cloud containers, Linux with Mesa |
| `window` | One hidden winit window through the app's `WindowedContext` | Windows with Intel or other drivers that expose no EGL devices; any real GPU through its normal driver |
| `auto` (default) | Headless if an EGL device exists, else the window | Everywhere |

Window mode runs the identical per-scene loop (load, compile timing,
warm-up, timed frames with `glFinish`, PNG readback, contact sheet,
`results.md`). The window exists only to own a context: scenes render
into the player's offscreen targets, nothing is swapped, and the
surface's vsync is off. `results.md` records the context line (headless
or window, hidden or tiny, and the backend) next to the renderer. If a
platform will not give a hidden window a GL surface, the bench retries
with a 16x16 undecorated window parked off-screen; `--show-window`
starts there.

**On Windows** (the T480, any machine whose driver has no EGL devices):

```powershell
cargo run --release -p strobe-bench -- --context window --size 1280x720
# ANGLE (GLES 3.0 on Direct3D 11): fetch its DLLs once, then
powershell -ExecutionPolicy Bypass -File scripts\fetch-angle.ps1
cargo run --release -p strobe-bench -- --context window --gl angle --size 1280x720
```

`--gl native|angle|auto` picks the window's backend (`auto` is native,
then ANGLE). `--api gles` in window mode means ANGLE, and a `--gl` that
contradicts `--api` is an error. Use a different `--out` per run
(`--out target\bench-native`, `--out target\bench-angle`) to keep the
frames side by side. Window mode renders the same images as headless
(checked pixel for pixel on llvmpipe under Xvfb, native and GLES).

**Add a new section, at the top of "Results", for every run that
informs a decision.** Include the machine, the date, the commit, and
what the numbers changed. Don't overwrite old results.

## The scene library as a load ladder

| Tier | Scene | What it stresses |
|---|---|---|
| 1 | 01-aurora-gradient, 02-plasma-pulse, 03-kaleido-tunnel, 06-feedback-trails | Baseline cost per frame; `syn_FinalPass` bandwidth (06) |
| 2 | 07-reaction-diffusion | Five passes, four RGBA32F sim steps at half resolution |
| 3 | 05-domain-warp, 08-bloom-orbs | Heavy ALU: about 60 noise evaluations per pixel; a 32-light loop plus a quarter-resolution blur chain |
| 4 | 04-voronoi-glass, 09-sdf-cathedral | 3×3 + 5×5 cell search; about 110 march steps plus a soft shadow and AO |
| 5 | 10-volume-clouds | Volumetric raymarch, up to 56 steps × 6 density samples × 4-octave 3D noise |

The tiers are ranked by measured cost, not by intent. Voronoi turned out
heavier than domain warping on llvmpipe, so check the ranking on real
GPUs.

## Results

### 2026-10-11: NVIDIA GTX 1080 Ti (TORNADO), commit 862ad2e

Driver 582.78, Windows 10 19045. `--context window` (hidden window), 60
timed frames after 90 warm-up, as above. Details are in
[hardware/TORNADO.md](hardware/TORNADO.md).

1920x1080:

| Scene | Native compile ms | Native mean ms | Native p95 ms | ANGLE compile ms | ANGLE mean ms | ANGLE p95 ms |
|---|---:|---:|---:|---:|---:|---:|
| 01-aurora-gradient | 52.0 | 0.08 | 0.09 | 28.8 | 0.07 | 0.09 |
| 02-plasma-pulse | 19.3 | 0.09 | 0.10 | 17.3 | 0.08 | 0.09 |
| 03-kaleido-tunnel | 23.3 | 0.12 | 0.13 | 22.0 | 0.09 | 0.10 |
| 04-voronoi-glass | 38.0 | 0.70 | 0.78 | 44.7 | 0.53 | 0.54 |
| 05-domain-warp | 44.2 | 0.95 | 0.99 | 106.0 | 0.77 | 1.16 |
| 06-feedback-trails | 20.7 | 0.11 | 0.17 | 24.9 | 0.11 | 0.28 |
| 07-reaction-diffusion | 114.8 | 0.25 | 0.30 | 96.1 | 0.17 | 0.19 |
| 08-bloom-orbs | 102.2 | 0.50 | 0.54 | 50.7 | 0.36 | 0.38 |
| 09-sdf-cathedral | 57.2 | 1.44 | 1.50 | 103.5 | 1.20 | 1.64 |
| 10-volume-clouds | 49.6 | 13.33 | 13.46 | 294.4 | 11.97 | 13.52 |
| 11-script-orbits | 25.9 | 0.13 | 0.13 | 33.3 | 0.16 | 0.13 |

At 3840x2160 on native GL the costs scale with the pixel count;
`10-volume-clouds` goes from 13.33 ms to 52.91 ms (18.9 fps), the only
scene a 1080 Ti cannot run at 60 fps in 4K. On this card ANGLE matches or
beats native per frame but compiles up to six times slower, so it costs
load time, not frames. These 07 numbers predate D191 (`FIXED_SIZE`), so
its simulation then ran at a quarter of the output's pixels at each size.

### 2026-10-09: first real GPU, ThinkPad T480 (Intel UHD 620), commit b7b0b50

Intel UHD 620, driver 30.0.101.1122, Windows 11 26300, on AC power.
`--context window` (hidden window), 60 timed frames after 90 warm-up
frames. Times are mean ms per frame. Native means WGL with GLSL 330;
ANGLE means GLES 3.0 on D3D11 (ANGLE 2.1.25847, from Electron 38.2.0).
32F and 16F are the float-pass formats; "shared" is one program per
scene instead of one per pass.

| Scene | Native 720p 32F | Native 720p 16F | Native 720p shared | ANGLE 720p 32F | Native 1440p 32F |
|---|---:|---:|---:|---:|---:|
| 01-aurora-gradient | 0.80 | 0.74 | 0.69 | 0.71 | 1.64 |
| 02-plasma-pulse | 0.80 | 0.67 | 0.63 | 0.69 | 2.18 |
| 03-kaleido-tunnel | 1.03 | 0.84 | 0.95 | 0.86 | 3.17 |
| 04-voronoi-glass | 7.65 | 8.33 | 8.61 | 8.66 | 32.12 |
| 05-domain-warp | 11.77 | 10.67 | 10.79 | 10.40 | 42.67 |
| 06-feedback-trails | 1.20 | 1.16 | 1.13 | 0.83 | 3.24 |
| 07-reaction-diffusion | 4.47 | **1.59** | 3.98 | 3.79 | 15.49 |
| 08-bloom-orbs | 4.14 | 3.82 | 3.88 | 4.42 | 16.51 |
| 09-sdf-cathedral | 20.44 | 20.01 | 20.01 | 20.14 | 82.27 |
| 10-volume-clouds | 193.26 | 191.43 | 188.31 | 186.00 | 743.40 |

All 10 scenes compile and render identically on native and ANGLE, judged
from the contact sheets.

**What this run decided:**

- **RGBA16F is now the default for float passes (D13).** Reaction
  diffusion, the bandwidth-bound scene, is **2.8× faster** (4.47 → 1.59
  ms), and nothing is meaningfully slower. This is exactly the
  integrated-GPU bandwidth wall that llvmpipe couldn't show. Use
  `--full` for RGBA32F.
- **Per-pass specialisation is neutral on Intel (D14).** Frame time is
  within run-to-run noise either way. Unlike llvmpipe, Intel's compiler
  handles uniform branches well. Specialisation stays the default
  because it is a 35% win elsewhere and never a loss, but it costs
  compile time on multi-pass scenes (07: 154 ms specialised vs 57 ms
  shared). Precompiling playlists hides that cost.
- **ANGLE performs on par with native** (within 5–10%, sometimes faster).
  It is a full-quality fallback, not a degraded mode.
- **At the panel's native 1440p**, only the tier-1 scenes hold 60 fps
  with headroom. 07 and 08 sit at about 60, 04, 05 and 09 fall well
  short, and 10 runs at 1.3 fps. Cost scales almost exactly with pixel
  count, so render scale and adaptive resolution are the lever on this
  machine. Volume Clouds, the tier-5 stress scene, needs about scale 0.35
  to reach 30 fps at 720p.


### 2026-10-08: baseline, Mesa llvmpipe (no GPU), cloud container, commit f84b564

llvmpipe (LLVM 20.1.2, 256-bit), 4 vCPU, desktop GL 4.5 core context,
GLSL 330. 1280×720, 30 timed frames after 90 warm-up frames. Times are
mean ms per frame.

| Scene | Per-pass programs, RGBA32F | Per-pass, RGBA16F | Shared program, RGBA32F |
|---|---:|---:|---:|
| 01-aurora-gradient | 2.59 | 2.63 | 2.72 |
| 02-plasma-pulse | 3.95 | 3.52 | 3.55 |
| 03-kaleido-tunnel | 2.40 | 2.36 | 2.25 |
| 04-voronoi-glass | 34.13 | 33.50 | 32.48 |
| 05-domain-warp | 23.02 | 23.51 | 22.36 |
| 06-feedback-trails | 3.83 | 3.73 | 3.50 |
| 07-reaction-diffusion | 14.52 | 17.06 | 13.84 |
| 08-bloom-orbs | **27.18** | 26.55 | **41.95** |
| 09-sdf-cathedral | 58.11 | 58.07 | 57.82 |
| 10-volume-clouds | 319.41 | 318.87 | 337.04 |

GLES 3.0 dialect (the ANGLE path), 640×360: all 10 scenes compile and
render.

**What this run says:**

- **Per-pass specialisation (D7) pays off where it should.** Bloom Orbs
  pairs a 32-light loop pass with cheap blur passes, and runs **35%
  faster** with one program per pass. Single-pass scenes are within
  noise of each other, as expected, because there's nothing to
  specialise.
- **RGBA16F shows no gain on llvmpipe, and Reaction Diffusion is 17%
  slower.** That's expected on a CPU rasteriser: half floats get
  converted in software, and there's no memory-bandwidth wall to relieve.
  **This says nothing about integrated GPUs, where bandwidth is the
  wall. Measure on real hardware before choosing the default.**
- Compile and link time is 5–30 ms per scene (up to about 90 ms for the
  5-pass scene on GLES). Precompiling a 100-scene playlist takes a few
  seconds.
- llvmpipe's absolute numbers are CPU rasterisation, so use them only to
  rank scenes and to measure the delta of a change.

## Wanted

- [x] A real-GPU run on the T480: native vs ANGLE, 32F vs 16F, 720p and
  1440p (2026-10-09, above). This machine has no discrete GPU.
- [ ] Render-scale sweep (1.0 / 0.75 / 0.5) once render scale is in the
  app.
