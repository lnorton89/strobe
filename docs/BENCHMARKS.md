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
| 4 | 04-voronoi-glass, 09-sdf-cathedral | 3×3 + 5×5 cell search (since D310 cells that cannot change the pixel are skipped before hashing); about 110 march steps plus a soft shadow and AO |
| 5 | 10-volume-clouds | Volumetric raymarch, up to 56 steps × 6 density samples × 4-octave 3D noise |

The tiers are ranked by measured cost, not by intent. Voronoi turned out
heavier than domain warping on llvmpipe, so check the ranking on real
GPUs.

## Results

### 2026-10-16: the presented rate against the GPU mean, venue mini PC (Vega 3), Task M16

In the app, not the bench: `13-chromatic-rise` held fullscreen at 1920x1080,
vsync at 60 Hz, numbers from the per-minute `perf:` lines (full table in
[hardware/MINIPC.md](hardware/MINIPC.md), M16).

| Setup | fps | GPU ms | Late frames |
|---|---|---|---|
| Adaptive on | 42.6-44.6 | 12.41-13.62 | 7.0-10.2% |
| `--no-adaptive --render-scale 1` | 42.2 | 13.6 | about 9% |

- **What it changed:** a mean GPU time under budget is not a smooth show.
  At 0.82 of the frame, about one frame in eleven still missed vertical
  blank, and the adaptive controller, which watched only the mean, never
  lowered the scale. Scenes 04, 05, 08, 14 and 16 (12.6-13.8 ms) did the
  same at about 47 fps. D314 adds the missed-refresh rule: the controller
  now steps the scale down when about 4% or more of the presented frames miss
  for four seconds with the GPU at 60% of the cadence or more, and the perf
  line carries the scale. Before and after on the projector is Task M18.
- **For reading GPU times in this file:** on a vsynced display, a scene needs
  real headroom under 16.7 ms, not just a mean under it, to present every
  frame. On this machine 12.4-13.8 ms was not enough.

### 2026-10-15: venue-GPU pass on 04, 05, 08, 13, 14 and 16 (D310), Mesa llvmpipe

Target: the venue mini PC's Vega 3 at 1920x1080. The owner's figures for
it were 04 17.1 ms, 05 21.2, 08 14.2, 13 13.7, 14 16.8 and 16 12.6 (60 fps
needs under 16.7). Aim: about 10 ms or less without a visible change.

**Method.** The container as above: Mesa llvmpipe 25.2.8, headless EGL,
desktop GLSL 3.30. Other jobs loaded the four shared cores (load average 6
to 12), so the runs used **one llvmpipe thread** (`LP_NUM_THREADS=1`).
Each run was 1280x720, 60 timed frames after 90 warm-up. Master's scenes and
the branch's alternated, twice each; the table gives the lower mean.
Absolute times are CPU rasterisation; read the ratio.

| Scene | Master ms | D310 ms | D310 / master | Vega 3 ms if the ratio carried over (owner's figure) | Image change at 720p (mean abs diff of 255; pixels off by >16) |
|---|---:|---:|---:|---:|---|
| 04-voronoi-glass | 144.5 | 78.2 | 0.54 | 9.2 (17.1) | 0.000%; 13 pixels off by 1 level |
| 05-domain-warp | 98.7 | 41.0 | 0.42 | 8.9 (21.2) | 0.34%; 0.08% |
| 08-bloom-orbs | 115.3 | 41.6 | 0.36 | 5.1 (14.2) | 0.03%; 0% |
| 13-chromatic-rise | 137.2 | 95.8 | 0.70 | 9.6 (13.7) | 0.27%; 0.30% |
| 14-electric-bloom | 153.2 | 126.1 | 0.82 | 13.8 (16.8) | 0.000%; max 1 level |
| 16-acid-mirror | 108.0 | 109.6 | 1.01 (noise) | 12.6 (12.6) | 0.000%; max 1 level |

**The fourth column is a projection, not a measurement.** It applies an
llvmpipe ratio to the owner's Vega 3 figures. llvmpipe is a CPU, bound by
texture sampling and transcendentals; a GCN GPU's balance of ALU against
texture is different, so the real ratios will differ. Re-measure on the
mini PC (`hardware/MINIPC.md`, "Re-measuring the D310 scene pass") before
ticking anything in STATUS_AUDIT.

**Measured on the Vega 3 (2026-10-16, MINIPC.md Task M17),** 1920x1080,
native WGL, `--show-window --window-size 1920x1080`, before (6923fa5) and
after (b8789bf) alternating. Image difference on a 4-pixel grid, per channel
of 255.

| Scene | Before ms (p95) | After ms (p95) | After / before | Image change |
|---|---|---|---|---|
| 04-voronoi-glass | 21.00 (28.75) | 10.68 (12.57) | 0.51 | identical |
| 05-domain-warp | 25.62 (33.10) | 9.96 (11.88) | 0.39 | mean 0.37, max 16 |
| 08-bloom-orbs | 17.50 (25.78) | 7.71 (13.22) | 0.44 | mean 0.04, max 5 |
| 13-chromatic-rise | 16.04 (18.23) | 8.51 (12.50) | 0.53 | mean 0.66; isolated pixels flip between posterise bands |
| 14-electric-bloom | 22.17 (30.28) | 18.74 (26.88) | 0.85 | mean 0.01, max 4 |
| 16-acid-mirror | 15.51 (24.11) | 14.92 (17.83) | 0.96 | identical |

The GPU ratios came out better than llvmpipe's for 05, 08 and 13 and about
the same for the rest. Compile times fell too (14 729 → 63 ms, 13 428 →
121 ms).

**Per scene:**
- **04** is exact. A cell is skipped before it is hashed when its box is
  already farther than the best point (nearest search), or when it cannot
  bring the border distance below `edge + 0.02`, the largest value that
  changes the lead (border search). 34 hashed cells per pixel become about
  10.
- **05:** the two inner warp fields (4 of the 5 fbm calls) run in a
  `$WIDTH/3` pass, stored as `q - 0.5` and `r - 0.5` in RGBA16F. In the first pass, divisors 2,
  3 and 4 measured 0.57, 0.45 and 0.41 of master's time, with 0.17%, 0.34%
  and 0.47% mean error, so 3. At 3x zoom the finest dark creases are a
  little softer; at 1x they read the same.
- **08:** the light pass runs at `$WIDTH/2`, and `d*d` is a dot product. A
  32x2 texture of per-light data was tried and dropped: it was exact, but
  slower on llvmpipe, and it costs RGBA32F fetches per light per pixel on a
  small GPU.
- **13:** the broad noise (flow, tint islands, height islands) moved into a
  `$WIDTH/4` pass: 11 noise lookups per pixel become 2. Bubbles do their work
  only in cells born this frame, which is exact (a mix by weight 0 returns
  `h` unchanged). The output pass reads the field with `texelFetch`, which is
  exact because the field is always the final pass's size (`$WIDTH` scales by
  1); the edge taps wrap as the repeat sampler did.
- **14:** the first version drew the back and front layers into half-size
  buffers. That blurred the beaded centres and their rings on every back and
  front flower; at 720p the large front flower's centre turned to mush. All
  three layers are back at full size. What remains is exact: the layers are
  drawn front to back and a layer is skipped where those in front are opaque
  (to within 0.05%). Candidate and pollen cells are rejected before hashing.
  Without the back layer at all the scene measured 86 ms, so a half-size back
  layer would bring it to about 0.65 of master, if the owner accepts softer
  far-layer centres.
- **16:** the first version moved the ink-mask, accent and dye-line noise
  into a quarter-size pass. That measured 0.94 here and moved the feedback
  picture (1.5% mean error, 7.9% of pixels off by more than 16), so it was
  reverted. What remains is exact and in the final pass only: the palette is
  a sum of steps instead of a chain of mixes, the three `pow` calls are
  squarings, and a streak's colour is only looked up where it has a line. On
  llvmpipe the gain is within the noise.

**Checks:**
- `--api gles` renders the same pixels as desktop for all six (max 0).
- At 160x90: 04, 14 and 16 are identical to master and 08 is close (0.34%).
  05's warp fields are then 53x30, and the picture is visibly blockier (2.9%
  mean error). 13 is equivalent in kind but differs pixel for pixel (4.5%),
  as its feedback loop amplifies a 40x22 flow field's error. Render scales
  that shows use (1080p at 0.5 and up) keep the reduced passes at 320x180 or
  more.
- 1800 warm-up frames (30 s of scene time) at 720p: 13 is 0.30% from master
  and neither blows out nor dies to black. 16 matches master to 1 level.

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

### 2026-10-19: Blacklight Carpet, GTX 1080 Ti (TORNADO)

New scene (D439). TORNADO, driver 582.78, hidden-window context,
half-float buffers, 1920x1080, 300 timed frames after 90 warm-up at scene
time 5 s, synthetic 120 BPM audio, two runs each. Not llvmpipe (no EGL
device on Windows) and not old-GPU data.

| Scene | Native GL 3.3 mean ms (run 1 / run 2) | Ratio | ANGLE GLES 3.0 mean ms | Ratio |
|---|---:|---:|---:|---:|
| 05-domain-warp | 0.39 / 0.34 | 1.00 | 0.33 / 0.29 | 1.00 |
| 15-neon-ribbon-flow | 0.42 / 0.40 | 1.12 | 0.50 / 0.50 | 1.61 |
| 17-blacklight-carpet | 0.83 / 0.81 | 2.25 | 0.77 / 0.80 | 2.53 |

The first draft was 1.37 ms (3.6x). Turning one layer off at a time put
0.35-0.45 ms on each of sprinkles, small confetti, streamers and the
per-layer fibre texture. Applying the texture once per layer only where a
piece covers the pixel, one cell per sprinkle instead of four, a bound test
before each piece's wobble and one pile octave brought it to 0.81 ms; what
remains is spread at 0.1-0.17 ms per layer. Native and ANGLE frames differ
by at most 1/255.

### 2026-10-26: ANGLE's backends, GTX 1080 Ti (TORNADO)

`--gl angle-*` (D485, G-05 and G-06). TORNADO, driver 582.78, Electron
38.2.0's ANGLE (a8c8a6febe63) and Vulkan loader beside a **debug**
`strobe-bench`, hidden-window context, half-float buffers, 1280x720, 240
timed frames after 30 warm-up, the 17 library scenes. GPU times from timer
queries, so the debug build costs CPU, not these numbers; still one run
each, and on a GPU this fast every scene is far inside its budget. The
renderers:

- native: `NVIDIA GeForce GTX 1080 Ti/PCIe/SSE2 (3.3.0 NVIDIA 582.78)`
- `angle-d3d11`: `ANGLE (NVIDIA, ... Direct3D11 vs_5_0 ps_5_0, D3D11-32.0.15.8278)`
- `angle-vulkan`: `ANGLE (NVIDIA, Vulkan 1.4.312 (NVIDIA NVIDIA GeForce GTX 1080 Ti (0x00001B06)), NVIDIA-582.78.0.0)`, OpenGL ES 3.1
- `angle-gl`: `ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1080 Ti/PCIe/SSE2, OpenGL ES 3.2 NVIDIA 582.78)`
- `angle-metal`: refused, "asked ANGLE for Metal, and it started on another backend (... D3D11 ...)".

ANGLE-Vulkan did not start with the loader the NVIDIA driver installs in
System32 (`VK_ERROR_INITIALIZATION_FAILED` in `vk::Renderer::initialize`);
with Electron's `vulkan-1.dll` beside it, it did. The release now ships it.

Frame time, mean (p95) ms:

| Scene | native mean (p95) | angle-d3d11 mean (p95) | angle-vulkan mean (p95) | angle-gl mean (p95) |
|---|---:|---:|---:|---:|
| 01-aurora-gradient | 0.08 (0.10) | 0.07 (0.09) | 0.10 (0.17) | 0.07 (0.09) |
| 02-plasma-pulse | 0.09 (0.11) | 0.07 (0.08) | 0.11 (0.18) | 0.07 (0.09) |
| 03-kaleido-tunnel | 0.11 (0.13) | 0.07 (0.08) | 0.15 (0.22) | 0.08 (0.10) |
| 04-voronoi-glass | 0.26 (0.30) | 0.28 (0.39) | 0.26 (0.33) | 0.22 (0.24) |
| 05-domain-warp | 0.23 (0.30) | 0.18 (0.31) | 0.20 (0.27) | 0.22 (0.29) |
| 06-feedback-trails | 0.11 (0.13) | 0.07 (0.09) | 0.11 (0.16) | 0.08 (0.10) |
| 07-reaction-diffusion | 0.25 (0.37) | 0.17 (0.20) | 0.21 (0.28) | 0.20 (0.26) |
| 08-bloom-orbs | 0.19 (0.22) | 0.14 (0.17) | 0.18 (0.25) | 0.19 (0.29) |
| 09-sdf-cathedral | 0.75 (0.87) | 0.61 (0.72) | 0.63 (0.72) | 0.62 (0.76) |
| 10-volume-clouds | 6.51 (7.04) | 5.53 (6.10) | 5.54 (5.72) | 6.45 (7.01) |
| 11-script-orbits * | 0.10 (0.13) | 0.08 (0.10) | 0.12 (0.17) | 0.09 (0.12) |
| 12-silicon-atlas | 1.49 (2.14) | 1.36 (1.66) | 1.45 (1.54) | 1.49 (2.12) |
| 13-chromatic-rise | 0.23 (0.32) | 0.18 (0.27) | 0.21 (0.28) | 0.21 (0.25) |
| 14-electric-bloom | 0.44 (0.56) | 0.41 (0.51) | 0.48 (0.56) | 0.44 (0.52) |
| 15-neon-ribbon-flow | 0.24 (0.28) | 0.26 (0.36) | 0.23 (0.30) | 0.26 (0.31) |
| 16-acid-mirror | 0.31 (0.44) | 0.23 (0.26) | 0.29 (0.37) | 0.28 (0.34) |
| 17-blacklight-carpet | 0.46 (0.58) | 0.42 (0.52) | 0.45 (0.54) | 0.43 (0.52) |

Compile ms as each backend reports it. ANGLE-Vulkan and ANGLE-GL return
from the link far sooner than ANGLE-D3D11 (12-silicon-atlas: 2155 ms
against 16 ms), but a Vulkan driver can defer pipeline work to the first
draw, so this column says when `link` returned, not what the first frame
cost:

| Scene | native compile | angle-d3d11 compile | angle-vulkan compile | angle-gl compile |
|---|---:|---:|---:|---:|
| 01-aurora-gradient | 46.7 | 20.9 | 9.2 | 10.6 |
| 02-plasma-pulse | 22.0 | 21.4 | 8.4 | 10.4 |
| 03-kaleido-tunnel | 25.3 | 26.7 | 8.5 | 10.7 |
| 04-voronoi-glass | 29.9 | 72.6 | 9.6 | 11.7 |
| 05-domain-warp | 12.5 | 102.2 | 23.1 | 20.0 |
| 06-feedback-trails | 23.4 | 23.8 | 8.4 | 11.1 |
| 07-reaction-diffusion | 134.8 | 115.3 | 43.2 | 46.1 |
| 08-bloom-orbs | 91.1 | 63.7 | 37.8 | 42.9 |
| 09-sdf-cathedral | 45.4 | 108.8 | 10.5 | 14.1 |
| 10-volume-clouds | 41.7 | 243.4 | 9.3 | 11.8 |
| 11-script-orbits * | 24.8 | 38.0 | 8.7 | 10.9 |
| 12-silicon-atlas | 124.6 | 2154.7 | 15.8 | 22.4 |
| 13-chromatic-rise | 242.6 | 270.2 | 55.4 | 58.1 |
| 14-electric-bloom | 473.8 | 1058.5 | 33.7 | 42.7 |
| 15-neon-ribbon-flow | 16.5 | 119.8 | 23.1 | 27.1 |
| 16-acid-mirror | 225.6 | 358.6 | 43.2 | 49.7 |
| 17-blacklight-carpet | 31.7 | 781.5 | 34.1 | 40.8 |

Images against ANGLE-D3D11, mean absolute difference over 255 / pixels
off by more than 16:

| Scene | native vs D3D11 | angle-vulkan vs D3D11 | angle-gl vs D3D11 |
|---|---:|---:|---:|
| 01-aurora-gradient | 0.00 / 0.0% | 0.00 / 0.0% | 0.00 / 0.0% |
| 02-plasma-pulse | 0.00 / 0.0% | 0.00 / 0.0% | 0.00 / 0.0% |
| 03-kaleido-tunnel | 0.01 / 0.0% | 0.01 / 0.0% | 0.01 / 0.0% |
| 04-voronoi-glass | 0.00 / 0.0% | 0.17 / 0.3% | 0.00 / 0.0% |
| 05-domain-warp | 0.00 / 0.0% | 0.62 / 0.5% | 0.00 / 0.0% |
| 06-feedback-trails | 0.00 / 0.0% | 0.00 / 0.0% | 0.00 / 0.0% |
| 07-reaction-diffusion | 1.38 / 2.6% | 1.87 / 2.9% | 1.38 / 2.6% |
| 08-bloom-orbs | 0.00 / 0.0% | 0.00 / 0.0% | 0.00 / 0.0% |
| 09-sdf-cathedral | 0.00 / 0.0% | 0.00 / 0.0% | 0.00 / 0.0% |
| 10-volume-clouds | 0.00 / 0.0% | 0.09 / 0.0% | 0.00 / 0.0% |
| 11-script-orbits | 0.00 / 0.0% | 0.00 / 0.0% | 0.00 / 0.0% |
| 12-silicon-atlas | 0.00 / 0.0% | 0.24 / 1.1% | 0.00 / 0.0% |
| 13-chromatic-rise | 3.04 / 5.5% | 5.51 / 9.0% | 3.04 / 5.5% |
| 14-electric-bloom | 0.00 / 0.0% | 0.22 / 0.3% | 0.00 / 0.0% |
| 15-neon-ribbon-flow | 0.00 / 0.0% | 0.45 / 0.0% | 0.00 / 0.0% |
| 16-acid-mirror | 2.12 / 3.4% | 3.52 / 5.4% | 2.12 / 3.4% |
| 17-blacklight-carpet | 0.00 / 0.0% | 0.09 / 0.1% | 0.00 / 0.0% |

The scenes that differ (07, 13, 16) build on their own previous frames
and drift between native and D3D11 as much; elsewhere Vulkan differs from
D3D11 by under 1/255 on average.

### 2026-10-26: render-scale sweep, GTX 1080 Ti (TORNADO)

`--scales 1,0.75,0.5` (D496), the first run with the synthetic audio
textures uploaded inside each timed frame. TORNADO, driver 582.78, native
OpenGL 3.3 core, hidden-window context, `release-fast` build, half-float
buffers, 1280x720 and its 0.75 and 0.5 scales, 60 timed frames after 90
warm-up, wall time around a `glFinish` per frame. One run each; the 17
library scenes:

| Scene | 1 (1280x720) | 0.75 (960x540) | 0.5 (640x360) | Saved |
|---|---:|---:|---:|---:|
| 01-aurora-gradient | 0.09 | 0.09 | 0.12 | -24% |
| 02-plasma-pulse | 0.09 | 0.09 | 0.09 | 8% |
| 03-kaleido-tunnel | 0.14 | 0.11 | 0.14 | 1% |
| 04-voronoi-glass | 0.28 | 0.22 | 0.15 | 45% |
| 05-domain-warp | 0.20 | 0.16 | 0.15 | 26% |
| 06-feedback-trails | 0.11 | 0.11 | 0.09 | 21% |
| 07-reaction-diffusion | 0.24 | 0.24 | 0.22 | 9% |
| 08-bloom-orbs | 0.21 | 0.17 | 0.22 | -7% |
| 09-sdf-cathedral | 1.19 | 0.49 | 0.33 | 73% |
| 10-volume-clouds | 5.72 | 3.30 | 1.66 | 71% |
| 11-script-orbits | 0.15 | 0.11 | 0.09 | 38% |
| 12-silicon-atlas | 1.52 | 0.85 | 0.43 | 72% |
| 13-chromatic-rise | 0.24 | 0.21 | 0.20 | 17% |
| 14-electric-bloom | 0.44 | 0.35 | 0.21 | 52% |
| 15-neon-ribbon-flow | 0.22 | 0.16 | 0.12 | 45% |
| 16-acid-mirror | 0.27 | 0.25 | 0.16 | 43% |
| 17-blacklight-carpet | 0.45 | 0.32 | 0.20 | 55% |

On a GPU this fast the light scenes are fixed cost (well under a
millisecond whatever the scale, so their "saved" is noise, negative
included). Where the pixels are the cost, the raymarched and volumetric
scenes, half scale (a quarter of the pixels) saves 71-73%, close to the 75%
that pixel count predicts, which is the lever adaptive resolution pulls on
a slow GPU (D64). The T480 and llvmpipe sweeps are still wanted.

## Wanted

- [x] A real-GPU run on the T480: native vs ANGLE, 32F vs 16F, 720p and
  1440p (2026-10-09, above). This machine has no discrete GPU.
- [x] Render-scale sweep (1.0 / 0.75 / 0.5): `--scales` (D496), run on
  the GTX 1080 Ti (2026-10-26, above).
- [ ] The same sweep on the T480 and llvmpipe.
