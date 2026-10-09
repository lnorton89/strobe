# Benchmarks

Measured with `strobe-bench`, which renders every scene in `scenes/` on
a headless or window-owned context and times each frame with `glFinish`. Frames are
deterministic: fixed `dt`, and synthetic 120 BPM audio from
`strobe_scene::synthetic`.

```bash
cargo run --release -p strobe-bench -- --size 1280x720 --frames 30 --warmup 90
#   --half      RGBA16F instead of RGBA32F for FLOAT passes
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

- [ ] A real-GPU run on the owner's ThinkPad T480 (Intel UHD 620, and
  the MX150 if fitted), native GL vs ANGLE, 32F vs 16F, at 1280×720 and
  1920×1080. This needs the windowed context provider, which is in
  progress.
- [ ] Render-scale sweep (1.0 / 0.75 / 0.5) once render scale is in the
  app.
