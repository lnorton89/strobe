# strobe-bench

The performance baseline and the way to see a renderer change without a
window. `strobe-bench` renders every scene folder in a library (by default
`scenes/`), times each one, and writes a PNG per scene, a contact sheet and a
`results.md` table. Frames are a pure function of time (fixed `dt`, the
synthetic 120 BPM audio from `strobe_scene::synthetic`), so two runs on one
machine produce identical images, which makes the PNGs usable as a visual
regression check.

## Role in the workspace

- **Depends on:** `strobe-gfx` (contexts and the scene player) and
  `strobe-scene` (loading, controls, the synthetic audio), plus `clap`,
  `anyhow` and `image`. The window context is `strobe_gfx::offscreen`, so it
  needs no `winit` of its own.
- **Depended on by:** nothing. It is a binary; `cargo xtask bench` wraps it and
  CI runs it.
- **Does not:** use `strobe-diag`, run scripts, upload real audio textures or
  swap a window. Scenes render into the player's offscreen targets, so a scene
  whose shader needs a script-created uniform does not compile here (D62), and
  `syn_Spectrum` / `syn_LevelTrail` are silent placeholders.

## Main types / entry points

This is a binary (`src/main.rs`). The pieces worth knowing:

| Item | What |
|---|---|
| `Cli` (clap) | The flags below. |
| `run` | The per-scene loop used by both context modes: load and compile (timed), untimed warm-up, timed frames each followed by `glFinish`, PNG readback. |
| `strobe_gfx::offscreen::run` | Picks headless or window (D70-D72) and runs the same job on either: one hidden winit window through `WindowedContext`, inside the event loop. Shared with `strobe-autocontrol` (D140). |

## Usage

```bash
cargo run --release -p strobe-bench -- --size 1280x720
cargo xtask bench -- --size 1280x720      # the same, via the xtask alias
```

Every flag is defined in the clap `Cli`:

| Flag | Default | Meaning |
|---|---|---|
| `--library <dir>` | `scenes` | Folder of scene folders. |
| `--size WxH` | `640x360` | Render resolution. |
| `--frames <n>` | `60` | Timed frames per scene. |
| `--warmup <n>` | `90` | Untimed frames first, so feedback scenes have history. |
| `--start <seconds>` | `2.0` | Scene time at the first frame. |
| `--context auto\|headless\|window` | `auto` | An EGL device with no window system, one hidden window, or headless if a device exists and the window otherwise. |
| `--gl auto\|native\|angle\|angle-d3d11\|angle-vulkan\|angle-gl\|angle-metal` | `auto` | Backend for `--context window`; `angle-*` names ANGLE's backend too (D485). |
| `--show-window` | off | Start with the tiny off-screen window instead of a hidden one. |
| `--window-size WxH` | `16x16` | Window contexts: the stand-in window's client size. Renders never depend on it; it is a bisection tool for drivers that draw black above some size (D296). Windows caps it at about the screen size; the report's context line gives the size the window got. |
| `--probe` | off | Instead of benchmarking, clear, draw into and read back targets at a ladder of sizes plus `--size`, and print the viewport the driver keeps. Writes `probe.md` (D296). |
| `--api desktop\|gles` | unset | Dialect for headless (desktop by default). In window mode the backend decides, and a contradiction with `--gl` is an error (D72). |
| `--full` | off | RGBA32F instead of the default RGBA16F for `FLOAT` passes. |
| `--shared` | off | One shared program per scene with a `PASSINDEX` uniform, instead of one specialised program per pass. |
| `--filter <text>` | none | Only scenes whose folder name contains the text. |
| `--out <dir>` | `target/bench` | Output folder: `<scene>.png`, `contact-sheet.png`, `results.md`. |

Environment: `STROBE_LOG` filters the renderer's log lines on stderr;
`STROBE_GL_DEBUG=1` shows the driver's own messages; `STROBE_GL_TRACE=1` records
the framebuffer, viewport and size of every bind, clear, draw, blit and read, and
prints the distinct set at the end (D296, [`DEBUGGING.md`](../../docs/DEBUGGING.md)).
`STROBE_REQUIRE_HEADLESS_GL` is read by tests, not by this binary.

On a Windows laptop whose driver exposes no EGL devices (the T480's Intel UHD
620):

```powershell
cargo run --release -p strobe-bench -- --context window --size 1280x720
powershell -ExecutionPolicy Bypass -File scripts\fetch-angle.ps1   # once, for ANGLE
cargo run --release -p strobe-bench -- --context window --gl angle --size 1280x720
```

Use a different `--out` per run to keep the frames side by side. CI runs the
bench twice (desktop and `--api gles`) at 640x360 and uploads both output
folders as artifacts (D56). Numbers and method are in
[`BENCHMARKS.md`](../../docs/BENCHMARKS.md).

## Testing

The crate has no tests of its own. Running it is the test:

```bash
apt-get install -y libegl1 libegl-mesa0 mesa-utils      # headless Mesa llvmpipe
cargo run --release -p strobe-bench -- --size 640x360 --frames 10 --warmup 60
cargo run --release -p strobe-bench -- --size 640x360 --frames 10 --warmup 60 --api gles
```

Open `target/bench/contact-sheet.png` to check the visuals. On a machine with
no GPU the context is llvmpipe: the timings are CPU rasterisation, but the
ranking between scenes and the before/after delta of a change are meaningful.
It has no `#[test]` functions, so `cargo test -p strobe-bench` only compiles it.

## Design notes

- **A window context rather than a different GL path** (D70): the bench
  measures the same native (WGL) and ANGLE contexts the show runs on. The
  window owns a context and nothing else; nothing is ever swapped. The code
  is `strobe_gfx::offscreen` since D140, shared with `strobe-autocontrol`;
  the bench's flags and output did not change, and `--context window` writes
  PNGs byte-identical to `--context headless` (checked under Xvfb).
- **Hidden window first, tiny off-screen window as the fallback** (D71).
- **`--api` is optional** (D72), because in window mode the backend decides
  the dialect.
- **Float passes default to RGBA16F** (D13) and **per-pass programs stay the
  default** (D14), both from bench runs on the T480; `--full` and `--shared`
  exist to re-measure the alternatives.
- **Original scenes** (D10): the library being timed is `scenes/`, written to
  span five load tiers.

## Status and known gaps

Open work for this crate is tracked in
[`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md) under the
IDs given here; a gap with no ID is by design, with its decision named.

- Real-GPU numbers are in [`BENCHMARKS.md`](../../docs/BENCHMARKS.md): the
  T480 (Intel UHD 620, native and ANGLE, 720p and 1440p), the AMD
  desktop's RX 580, TORNADO's GTX 1080 Ti (ANGLE's backends included) and
  the venue mini-PC's Vega 3. The other baseline is llvmpipe in a cloud
  container. A render-scale sweep is still wanted (backlog D-06).
- **A failed scene fails the run.** A scene that fails to compile is printed
  as `FAILED`, recorded in `results.md` and left out of the contact sheet.
  After every frame and the report are written, the process exits non-zero
  with `N of M scenes failed: <names>`, which is what fails CI's bench step
  (D56).
- No scripts run, by design (D239): `11-script-orbits` renders as a still
  with default controls, and `results.md` says so and marks such scenes
  `*`; `cargo xtask corpus` runs scripts. The audio textures are
  placeholders (backlog D-06).
- Not built for macOS yet: `strobe-gfx` has no CGL provider (D55; backlog
  G-07).
