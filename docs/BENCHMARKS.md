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
| 3 | 05-domain-warp, 08-bloom-orbs, 13-chromatic-rise | Heavy ALU (before D310: about 60 noise evaluations per pixel; a 32-light loop plus a quarter-resolution blur chain; 13 a full-resolution feedback pass plus an emboss and posterise pass, about 1.35x 05). Since D310 05 evaluates its warp fields at a third of the size, 08 its lights at half, and 13 its broad noise at a quarter |
| 4 | 04-voronoi-glass, 09-sdf-cathedral | 3×3 + 5×5 cell search; about 110 march steps plus a soft shadow and AO |
| 5 | 10-volume-clouds | Volumetric raymarch, up to 56 steps × 6 density samples × 4-octave 3D noise |

The tiers are ranked by measured cost, not by intent. Voronoi turned out
heavier than domain warping on llvmpipe, so check the ranking on real
GPUs.

## Results

### 2026-10-15: venue-GPU pass on 04, 05, 08, 13, 14 and 16 (D310), Mesa llvmpipe

Target: the venue mini PC's Vega 3 at 1920x1080, which measured 04 17.1 ms,
05 21.2, 08 14.2, 13 13.7, 14 16.8 and 16 12.6 (60 fps needs under 16.7).
Aim: about 10 ms or less without a visible change. Container as above
(Mesa llvmpipe 25.2.8, headless EGL, desktop GLSL 3.30), but **one
llvmpipe thread** (`LP_NUM_THREADS=1`) because other jobs load the shared
cores, 1280x720, 60 timed frames after 90 warm-up, the lower of two runs.
Absolute times are CPU rasterisation; the ratio is the number to read.

| Scene | Before ms | After ms | After / before | Vega 3 ms projected by ratio (measured) | Image change (MAD of 255, share of pixels off by >16) |
|---|---:|---:|---:|---:|---|
| 04-voronoi-glass | 160.0 | 83.4 | 0.52 | 8.9 (17.1) | 0.00%, 0.000% (identical) |
| 05-domain-warp | 111.9 | 48.2 | 0.43 | 9.1 (21.2) | 0.34%, 0.11% |
| 08-bloom-orbs | 124.7 | 46.8 | 0.38 | 5.3 (14.2) | 0.03%, 0.00% |
| 13-chromatic-rise | 141.2 | 98.2 | 0.70 | 9.5 (13.7) | 0.26%, 0.31% |
| 14-electric-bloom | 161.0 | 94.4 | 0.59 | 9.9 (16.8) | 0.10%, 0.30% |
| 16-acid-mirror | 114.6 | 107.3 | 0.94 | 11.8 (12.6) | 1.51%, 7.9% |

- Frames are a pure function of time, so before and after are compared
  frame for frame (same scene time, same warm-up). `--api gles` compiles
  all six and renders the same pixels as desktop (RMSE 0).
- **04** is exact: a cell is rejected before it is hashed when its box is
  already farther than the best point (nearest search), or when it cannot
  bring the border distance below `edge + 0.02` (border search, the largest
  value that changes the lead). 34 hashed cells per pixel become about 10.
- **05**: the two inner warp fields (4 of the 5 fbm calls) run in a
  `$WIDTH/3` pass and the output pass keeps the full-size outer fbm.
  Divisors 2, 3, 4 measured 0.57, 0.45, 0.41 of the original CPU time and
  0.17%, 0.34%, 0.47% mean error, so 3.
- **08**: the light pass runs at `$WIDTH/2`; `d*d` is a dot product.
  (Tried and dropped: a 32x2 texture of per-light data, which was exact but
  slower on llvmpipe and costs RGBA32F fetches per light per pixel on a
  small GPU.)
- **13**: the broad noise (flow, tint islands, height islands) moved into a
  `$WIDTH/4` pass; bubbles do their work only in the cells born this frame;
  the output pass reads the field with point fetches. 11 noise lookups per
  pixel become 2. llvmpipe is bound by the field taps here, so its ratio
  understates the ALU saving.
- **14**: back and front layers drawn into `$WIDTH/2` buffers (premultiplied),
  the sharp middle layer, pollen and the stack at full size; the middle
  layer is skipped under opaque front petals; candidate cells and pollen
  cells are rejected before hashing and pollen before the trig. One program
  per layer instead of a loop over three.
- **16**: the three broad noise fields (ink mask, accent, dye lines) moved
  into a `$WIDTH/4 x $HEIGHT/2` pass; the palette is a sum of steps instead of
  a chain of mixes and the final pass squares instead of calling `pow`. This
  one is a **feedback** scene, so any change in the injected paint
  compounds: the layout and palette match, the fine colour detail does not
  match pixel for pixel (1.5% mean error at 720p). llvmpipe barely sees the
  gain (it is bound by the fp16 bilinear taps and trig, not the noise); on a
  GPU the noise was about 70% of the feedback pass's arithmetic, so expect
  more there than the ratio shows. **Not measured on the Vega 3.**
- The "projected" column is the llvmpipe ratio applied to the measured
  Vega 3 times; it is a projection, and the real gain depends on ALU versus
  texture balance. Re-measure on the venue machine before ticking anything
  in STATUS_AUDIT.

### 2026-10-15: scenes 13-16 after the pre-merge review, Mesa llvmpipe

The review's changes (D276-D279 "Revised" notes) measured against the
scenes as cherry-picked, on the same container: Mesa llvmpipe 25.2.8,
headless EGL, desktop GLSL 3.30, half-float buffers, scene time 2 s plus 60
warm-up frames, 40 timed frames at 720p (30 at 1080p), synthetic 120 BPM
audio, 4 shared cores (load average 3.5 to 4.5). Ratios are to
`05-domain-warp` in the same run; absolute times are CPU rasterisation.

| Scene | Before, 720p ms (ratio) | After, 720p ms, two runs (ratio) | After, 1080p ms (ratio) |
|---|---:|---:|---:|
| 05-domain-warp | 25.12 (1.00) | 24.70 / 24.66 (1.00) | 55.46 (1.00) |
| 13-chromatic-rise | 34.64 (1.38) | 34.49 / 33.24 (1.37) | 72.76 (1.31) |
| 14-electric-bloom | 43.79 (1.74) | 36.54 / 36.76 (1.48) | 80.74 (1.46) |
| 15-neon-ribbon-flow | 31.91 (1.27) | 31.97 / 31.83 (1.29) | 69.99 (1.26) |
| 16-acid-mirror | 25.20 (1.00) | 27.44 / 26.89 (1.10) | 60.85 (1.10) |

- **13** holds its cost while adding a clock pass, shear lanes and a
  palette lookup: the curl potential dropped from two noise octaves to one
  (37.3 -> 32.0 ms in an A/B pair, 15%), and the cross-flow fix reuses the
  seed-check read, so the field pass still takes six taps.
- **14**: skipping the petal rings past the longest petal saved 9% (40.2
  -> 36.7 ms in an A/B pair). What remains is the 3 layers x 4 cells of
  hashes and glow every pixel pays (27 ms with petals and pollen both off)
  and the pollen (5.5 ms).
- **16** pays about 10% for the hairline streaks (one more `fwidth` and the
  streak line), less one tap the final pass no longer repeats.
- Projected on the venue's Vega 3, where 05 ran about 23 ms at 1920x1200 on
  ANGLE: 13 about 30 ms, 14 about 34, 15 about 29, 16 about 25, so each
  settles near the same 0.7 render scale 05 does. **Not measured on real
  hardware**; llvmpipe under-weights the bandwidth of 13's and 16's
  full-resolution float feedback, which matters more on a shared-memory
  iGPU.
- `cargo xtask corpus -- --library scenes --filter 1`: 8 of 8 ok in desktop
  and GLES 3.0; 640x360 frames of 13-16 at 5 s are identical in the two
  dialects (RMSE 0). 13 and 16 rendered at 60, 180 and 300 s show no
  blow-out, fade or NaN; all four still move and fill the frame with
  reactivity 0.

### 2026-10-14: Chromatic Rise rebuilt as a feedback loop (D276), Mesa llvmpipe

Cloud container, no GPU, headless EGL, 4 cores shared with other jobs
(load average 5 to 7, so absolute numbers wobble by up to 30%). Interleaved
runs of `strobe-bench --filter 05` / `--filter 13`, 1280x720, 40 timed
frames after 90 warm-up, per-pass programs, half-float buffers. The rebuild
is two passes: a full-resolution double-buffered `field` pass (about 14 noise
reads, 5 taps) and a post pass (5 taps, no noise but the grit hash). Budget:
no more than 2x 05-domain-warp.

| Scene | Run 1 ms | Run 2 ms | Run 3 ms | Mean ms | Ratio to 05 |
|---|---:|---:|---:|---:|---:|
| 05-domain-warp | 20.83 | 21.98 | 21.85 | 21.55 | 1.00 |
| 13-chromatic-rise (rebuilt) | 29.16 | 29.74 | 28.56 | 29.15 | 1.35 |

- `--api gles` (GLSL ES 3.0, one run each): 05 39.7 ms, 13 53.3 ms (1.34x).
- 1920x1080 interleaved runs were too disturbed by the load to quote
  (13 ran from 0.8x to 1.4x of 05 across three pairs); the 1.35x figure is
  the one to trust. The cost scales with pixels, so expect about 2.25x the
  720p time at 1080p for both scenes.
- The first draft (stateless, one pass) cost 0.44 ms on the RX 580. The
  rebuild adds a full-resolution float read-modify-write of a 16-bit RGBA
  buffer per frame, which is cheap on that card and the first thing to
  halve (`$WIDTH/2`) if the HD 4000 or Vega 3 cannot hold 60 fps.
  **Not measured on real hardware yet.**
- `cargo xtask corpus -- --library scenes --filter 13`: desktop ok,
  GLES 3.0 ok. Renders at 2 s, 10 s and 30 s showed no blow-out or fade to
  black; a run with every audio uniform forced to 0 still flows.

### 2026-10-13: AMD Radeon RX 580 (AMD desktop), commit 26576b0

Driver 31.0.21925.1001, Windows 10 19045. Native GL reports `3.3.0 Core
Profile Context 26.5.2.260413`; ANGLE is Electron 38.2.0's, on D3D11.
`--context window`, as above. Details are in
[hardware/AMD.md](hardware/AMD.md). Mean / p95 ms:

| Scene | Native 720p | ANGLE 720p | Native 1080p | ANGLE 1080p |
|---|---:|---:|---:|---:|
| 01-aurora-gradient | 0.13 / 0.14 | 0.09 / 0.10 | 0.16 / 0.19 | 0.13 / 0.15 |
| 02-plasma-pulse | 0.14 / 0.14 | 0.12 / 0.14 | 0.19 / 0.20 | 0.16 / 0.19 |
| 03-kaleido-tunnel | 0.16 / 0.17 | 0.12 / 0.12 | 0.25 / 0.25 | 0.16 / 0.17 |
| 04-voronoi-glass | 0.65 / 0.67 | 0.64 / 0.76 | 1.48 / 2.59 | 1.36 / 1.49 |
| 05-domain-warp | 0.71 / 0.76 | 0.69 / 0.70 | 1.63 / 2.73 | 1.49 / 1.66 |
| 06-feedback-trails | 0.14 / 0.17 | 0.14 / 0.15 | 0.17 / 0.18 | 0.16 / 0.17 |
| 07-reaction-diffusion | 0.34 / 0.34 | 0.32 / 0.33 | 0.37 / 0.38 | 0.36 / 0.40 |
| 08-bloom-orbs | 0.54 / 0.55 | 0.51 / 0.52 | 1.40 / 2.52 | 1.19 / 1.48 |
| 09-sdf-cathedral | 1.62 / 2.73 | 1.53 / 1.72 | 3.39 / 4.40 | 3.20 / 3.40 |
| 10-volume-clouds | 12.69 / 13.35 | 11.43 / 11.65 | 26.96 / 27.30 | 25.28 / 25.62 |
| 11-script-orbits | 0.17 / 0.18 | 0.13 / 0.14 | 0.25 / 0.27 | 0.22 / 0.26 |
| 12-silicon-atlas | 2.71 / 3.74 | 2.49 / 2.64 | 5.49 / 5.68 | 5.56 / 5.87 |

- **Relative cost:** an RX 580 takes about twice a 1080 Ti's time per frame.
- **The one heavy scene:** `10-volume-clouds` is the only scene under
  60 fps at 1080p (37 fps), and adaptive resolution would carry it.
- **ANGLE against native:** ANGLE is level with native or faster per
  frame, as on NVIDIA.
  - Native's p95 rises to 2.5-2.7 ms on 04, 05 and 08 at 1080p, where
    ANGLE stays flat.
  - ANGLE compiles 12-silicon-atlas in 1588 ms against 8.8 ms native.
- `11-script-orbits` runs without its `script.js` here (C-05).

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

### 2026-10-13: Chromatic Rise, Windows RX 580

Native GL 3.3 (driver 26.5.2.260413), hidden window, 1280x720,
per-pass programs, half-float buffers, 60 timed frames after 90 warm-up.
Before: `cargo xtask bench -- --size 1280x720`, twelve scenes.
After: same settings with `--out target/bench/chromatic-rise-final`,
thirteen scenes. Existing renderer code and scenes were unchanged.

| Scene | Before mean ms | After mean ms | Delta ms |
|---|---:|---:|---:|
| 01-aurora-gradient | 0.13 | 0.14 | +0.01 |
| 02-plasma-pulse | 0.16 | 0.16 | 0.00 |
| 03-kaleido-tunnel | 0.21 | 0.17 | -0.04 |
| 04-voronoi-glass | 0.68 | 0.72 | +0.04 |
| 05-domain-warp | 0.74 | 0.73 | -0.01 |
| 06-feedback-trails | 0.18 | 0.16 | -0.02 |
| 07-reaction-diffusion | 0.37 | 0.36 | -0.01 |
| 08-bloom-orbs | 0.64 | 0.57 | -0.07 |
| 09-sdf-cathedral | 1.57 | 1.54 | -0.03 |
| 10-volume-clouds | 12.28 | 12.15 | -0.13 |
| 11-script-orbits (no script execution) | 0.18 | 0.16 | -0.02 |
| 12-silicon-atlas | 2.59 | 2.52 | -0.07 |
| 13-chromatic-rise | absent | 0.44 | new scene |

Existing-scene differences are run variation, not an engine speedup.
Chromatic Rise's first draft cost 0.65 ms; reducing nested noise octaves
and tuning its defaults reduced it to 0.44 ms (-0.21 ms, approximately
32%), with a more readable visual. Final p95: 0.47 ms native.
The final shader also compiles and renders on ANGLE GLES 3.0 / D3D11
(ANGLE 2.1.25847), mean 0.35 ms, p95 0.36 ms, same frame settings.
These synchronized benchmark frame times are not app presentation FPS
or evidence for HD 4000 performance.

Local reports/frames: `target/bench/before-chromatic-rise.md`,
`target/bench/chromatic-rise-final/`,
`target/bench/chromatic-rise-angle-final/`. The scene thumbnail preserves
the inspected native rendered frame.

## 2026-10-14: Desktop remote integration on the updated PR base, Windows RX 580

Before and after `cargo xtask bench -- --size 1280x720` on upstream
`5507a13`, native GL 3.3 26.5.2.260413, hidden window, half-float
buffers, per-pass programs, 90 warmup / 60 timed frames. Reports:
`target/bench/desktop-remote-pr-base/results.md` and
`target/bench/desktop-remote-pr-after/results.md`.

| Scene | Before mean ms | After mean ms | Delta ms |
|---|---:|---:|---:|
| 01-aurora-gradient | 0.14 | 0.13 | -0.01 |
| 02-plasma-pulse | 0.16 | 0.16 | 0.00 |
| 03-kaleido-tunnel | 0.17 | 0.17 | 0.00 |
| 04-voronoi-glass | 0.67 | 0.66 | -0.01 |
| 05-domain-warp | 0.72 | 0.73 | +0.01 |
| 06-feedback-trails | 0.16 | 0.17 | +0.01 |
| 07-reaction-diffusion | 0.36 | 0.39 | +0.03 |
| 08-bloom-orbs | 0.55 | 0.55 | 0.00 |
| 09-sdf-cathedral | 1.56 | 1.53 | -0.03 |
| 10-volume-clouds | 12.28 | 12.23 | -0.05 |
| 11-script-orbits * | 0.16 | 0.16 | 0.00 |
| 12-silicon-atlas | 2.58 | 2.54 | -0.04 |
| 13-chromatic-rise | 0.43 | 0.42 | -0.01 |

Scripts are not run by this bench. Only the thirteen committed base scenes
were included; unrelated local scene work is excluded from this comparison.
No renderer or base shader changed in this feature. Changes range from
-0.05 ms to +0.03 ms and are run variation, not evidence of an optimization.
The bench does not link the app UI stream and cannot measure enabled
desktop-streaming overhead.

Desktop capture is capped at five frames per second and skipped without an
active viewer. Compression, socket transfer and decoding run on workers;
active capture reads the control window into a pixel-pack buffer behind a
fence and collects it on a later frame, so it never waits for the GPU
(review hardening, D275). Native RX 580
GL connection/viewer fixtures are committed in `docs/images/perform/`.
Loopback tests exercise host input/edit/save paths. Two physical desktops,
Wi-Fi loss/sleep, ANGLE capture and older-GPU CPU/frame-time/memory soak
remain unverified (STATUS_AUDIT.md).
### 2026-10-15: Electric Bloom, Neon Ribbon Flow, Acid Mirror, llvmpipe

Rebuilt from Codex drafts (D277-D279). Mesa llvmpipe 25.2.8 (CPU), headless
EGL, native GL 3.3, half-float buffers, 1920x1080, 30 timed frames after 60
warm-up at scene time 5 s, synthetic 120 BPM audio, two runs each. The
target was no more than about 2x `05-domain-warp` at 1080p. Absolute times are
CPU rasterisation; the ratio is the evidence. None of this is real-GPU
(Vega 3, HD 4000) data.

| Scene | Mean ms (run 1 / run 2) | Ratio to 05-domain-warp | Passes |
|---|---:|---:|---|
| 05-domain-warp | 54.3 / 54.0 | 1.00 | 1 |
| 14-electric-bloom | 97.6 / 97.5 | 1.80 | clock + final |
| 15-neon-ribbon-flow | 68.4 / 69.8 | 1.28 | clock + final |
| 16-acid-mirror | 57.5 / 58.3 | 1.07 | feedback + clock + final |

Both dialects render identical pixels (a GLSL ES 3.0 render at 640x360 and
the desktop one compared with RMSE 0 for 14 and 15; `strobe-corpus` reports
8 of 8 scenes ok in both for the 01 and 10-16 filter). The Codex drafts were cheaper but
flat; the rebuilds spend the budget on depth layers (14), five translucent
layers with glow (15) and a flow-advected feedback buffer at half width (16).
Optimisations that mattered: early-outs for empty cells and for pixels far
from a ribbon's glow (14: 108 -> 98 ms at first; 15: 103 -> 68 ms), and
colour work done only where a petal ring covers the pixel.
Silence is cheaper than the audio-on numbers above (bloom 42, ribbon 29,
acid 26 ms at 1280x720 against 114, 67 and 53 with audio) because the
synthetic track's kick drives larger flowers and wider glow.

## Wanted

- [x] A real-GPU run on the T480: native vs ANGLE, 32F vs 16F, 720p and
  1440p (2026-10-09, above). This machine has no discrete GPU.
- [ ] Render-scale sweep (1.0 / 0.75 / 0.5) once render scale is in the
  app.
