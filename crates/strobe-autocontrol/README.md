# strobe-autocontrol

Auto-generated controls. Most Shadertoy shaders, and many community SSF
scenes, have no controls: their character is baked into literal numbers. This
crate finds those numbers, turns the ones that matter into `CONTROLS`, and
proves they matter by rendering the scene with each one moved. An import is
performable the moment it lands.

It finds candidates by static analysis of the GLSL, rewrites them into
uniforms in a copy of the scene (no line moves), compiles that once, renders a
few fixed frames headless per candidate with its value nudged down and up, and
scores the picture difference. Dead and blacked-out candidates are dropped,
the strongest few are kept, named, grouped (Motion, Shape, Colour, Quality)
and written as ordinary SSF controls, so the result survives export and runs
anywhere. The same scene always gives the same controls (D126).

## Role in the workspace

- **Depends on:** `strobe-scene` (control model, name rules, `SceneFolder`),
  `strobe-shadertoy` (the lossless `glsl` tokenizer and the converter's
  tab-marker constants only), `strobe-gfx` (`ScenePlayer` for scoring, and
  `offscreen` for the CLI's context), `strobe-ssf-lib` (library identifiers a control
  must not shadow), plus serde, serde_json, thiserror, clap, anyhow, log.
- **Used by:** the `strobe-autocontrol` binary here, and the app's "Generate
  controls" (D202). The app runs it in a worker process of its own executable
  (`strobe_app::generate`), so the show never waits on scoring.
- **Does not:**
  - touch the frame loop: scoring runs at import time only;
  - edit a scene in place: the result is a new folder;
  - touch `strobe-gfx`'s dependencies: `strobe-gfx` must never depend on this
    crate;
  - apply an audio binding unless asked.

## Main types and entry points

| Item | What it does |
|---|---|
| `analyze(&Sources) -> Analysis` | Pure. Finds `Candidate`s: `#define`/`const` values, `TIME`/`iTime` multipliers, loop bounds, `vec3` colours, coordinate scales and frequencies |
| `Candidate`, `Site`, `Kind`, `Group` | Where a literal is (file, line, column, byte span, text), what it was doing, its suggested name, group and range |
| `rewrite::apply`, `control_line` | Replaces chosen sites by uniform expressions without moving a line; writes the matching `CONTROLS` entry |
| `score::Rig`, `measure` | Compiles once; renders a probe with chosen controls moved; mean image difference; black detection |
| `generate(&Gl, &Caps, &SceneFolder, &Options) -> Generated` | The whole pipeline: analyse, score, keep, name, rewrite, verify |
| `Report` | Every candidate with score and verdict (`Kept`, `Cut`, `Dead`, `Broken`, `DoesNotCompile`, `NotScored`), the kept names, `fidelity`, and the audio `suggestions`; `to_json`, `summary`, `detail` |
| `write_scene` | Copies the source folder, overwrites the rewritten GLSL, splices `CONTROLS` into `scene.json` as text, renames into place |
| `suggest::suggest` | Shape to bass, colour to highs, speed to loudness; never quality |
| `naming::assign` | Unique snake_case names that avoid controls, reserved names, GLSL words and identifiers in the code |

## Usage

```bash
# Measure and write scenes/12-warm-auto.synScene next to the source.
cargo run --release -p strobe-autocontrol -- scenes/12-warm.synScene

# Options:
#   --context auto|headless|window   where the GL context comes from (default auto: headless if an EGL device exists, else a hidden window)
#   --gl auto|native|angle           window backend; --api must not contradict it
#   --show-window                    start with the tiny off-screen window instead of a hidden one
#   --out <DIR>        library to write <name>-auto.synScene into (default: beside the source)
#   --keep <N>         at most N controls (default 8)
#   --report           print every candidate with scores; write <name>-auto.report.json
#   --force-merge      add to a scene that already has authored controls (kept; new names only)
#   --replace          overwrite an earlier result
#   --dry-run          measure and report, write nothing
#   --analyze-only     list the candidates found by static analysis; needs no GPU
#   --audio-bindings   write the suggested audio mappings into the GLSL (amounts default to 0)
#   --api desktop|gles measure in GL 3.3 core (default) or GLES 3.0; with --context window the backend decides
#   --size WxH, --frames N, --threshold F, --max-scored N
```

On a machine with no EGL device (Intel on Windows) `--context auto` opens a
hidden window, the same way `strobe-bench` does (D70-D72, D140). The window
owns a context and nothing else, and the scores and kept controls match the
headless ones:

```powershell
strobe-autocontrol.exe target\t480\ac\05.synScene --context window --report --dry-run
```

A scene that already has authored controls is refused unless `--force-merge`
is given. The five "Strobe" controls of a Shadertoy import do not count as
authored. From Rust:

```rust
use strobe_autocontrol::{Options, generate};
# // The caller supplies the context: the app's own, or strobe_gfx::offscreen::run's.
# fn run(gl: &strobe_gfx::gl::Gl, caps: &strobe_gfx::Caps, folder: &strobe_scene::SceneFolder)
#     -> Result<(), Box<dyn std::error::Error>> {
let generated = generate(gl, caps, folder, &Options::default())?;
println!("{}", generated.report.summary());
// generated.folder is the scene as it would play; generated.control_lines are
// the CONTROLS entries to add; strobe_autocontrol::write_scene writes it out.
# Ok(()) }
```

How a literal becomes a control:

| Found | Becomes | Range |
|---|---|---|
| `#define GLOW 0.7`, `const float K = 1.5;` | `glow`, `k_amount` slider | `[0, 2x]`, symmetric if negative |
| `TIME * 0.3`, `iTime * k` | `speed`, a `slider speed` that replaces the whole product | `[0, 2k]` |
| `for (.. i < 64 ..)` | `steps` (`soft_shadow_steps` in a helper), integer | `[n/4, 2n]`, never over 256 unless already |
| `vec3(0.9, 0.4, 0.1)` | `color` / `fog_col`, a `color` | 0..1 |
| `uv * 3.0`, `p *= 2.0`, `sin(x * 10.0)` | `scale` / `zoom` / `freq` slider | `[0, 2x]` |

## Testing

```bash
cargo test -p strobe-autocontrol
STROBE_REQUIRE_HEADLESS_GL=1 cargo test -p strobe-autocontrol   # fail instead of skipping without EGL
STROBE_BLESS=1 cargo test -p strobe-autocontrol --test golden   # regenerate the goldens

# the window half of tests/window.rs needs a display; without one it skips
STROBE_REQUIRE_HEADLESS_GL=1 STROBE_REQUIRE_WINDOW_GL=1 \
    xvfb-run -a -s "-screen 0 1280x720x24" cargo test -p strobe-autocontrol --test window
```

- **Unit tests** beside each module: the tokenizer view, every detector with
  its false-positive traps (`* 2.0 - 1.0`, `vec3(0.5)`, `dot(c, vec3(..))`,
  `a / TIME * k`, `case MODE`, array sizes, hash functions), naming, the
  rewrite (every kind, line preservation across a multi-line span), the
  scene.json splice, ranges and the loop cap.
- **`tests/golden.rs`** pins the candidates found in five converted fixtures
  and the exact rewritten GLSL, byte for byte. No GPU.
- **`tests/engine.rs`** runs headless in both dialects (it skips without EGL,
  unless `STROBE_REQUIRE_HEADLESS_GL=1`). It converts every Shadertoy
  fixture and strips the `CONTROLS` of every library scene (baking each
  default in as a `#define`), then requires, for each: it compiles; a control
  is kept or the reason is documented; each kept control scores above the
  threshold and moves the picture again on the finished scene; the rewrite
  reproduces the original at its defaults; a second run is identical. It also
  covers the authored-controls guard and merge, the writer (the source is
  untouched, supporting files and `SOURCE` come along), the opt-in audio
  binding being inert at 0, a candidate that breaks the compile being dropped,
  and a scene that does not compile at all. It writes a results table to
  `target/tmp/`.
- **`tests/window.rs`** spawns the real binary (winit allows one event loop
  per process, on the main thread, which a test thread is not). It scores one
  scene with `--context headless` and `--context window` and requires the
  same printed report; it also checks that an `--api`/`--gl` contradiction is
  refused in every mode, that `--analyze-only` runs with no display, and that
  a window with no display says so. The window comparison skips without a
  display unless `STROBE_REQUIRE_WINDOW_GL` is set (CI's Xvfb step sets it).
- **Fixtures** (`tests/fixtures/`) are original work in exact Shadertoy export
  shape, MIT like the repo: a ring tunnel, a raymarched field, a feedback
  swirl with a `Common` tab, voronoi cells and colour bars. The converter's
  nine fixtures are reused from `strobe-shadertoy`.

## Design notes

- **A crate, not a module (D123).** SSF scenes need it without the importer.
- **Conservative patterns (D124).** A false positive costs more than a miss.
  Nothing at global scope, nothing in the Shadertoy shim, `const` becomes a
  `#define`, and a rewrite that does not compile is dropped, not fatal.
- **Speed is an integrator (D125).** `TIME * k` becomes a `slider speed`
  output, so moving it does not jump the phase.
- **Measured, not guessed (D126).** One compile, two nudges per candidate,
  fixed frames and synthetic audio, scores rounded to five decimals.
- **The library takes a context, only the CLI picks one (D140).** `generate`
  is given `&Gl` and `&Caps`, so the app can score on its own context. The CLI
  uses `strobe_gfx::offscreen`, shared with the bench.

## Status and known gaps

- **Verified:** llvmpipe (Mesa) on desktop GL 3.3 core and GLES 3.0. Not a real
  GPU, not the T480, not ANGLE itself.
- **The app's review** (D202, D219, D246) keeps or drops each control,
  re-ranges a slider and renames a control before the copy is written again;
  a name is checked against the scene the way generated names are.
- **Window mode on Windows is unverified.** `--context window` agrees with
  `--context headless` byte for byte on llvmpipe under Xvfb, but it has not
  run on the T480 yet (the check is in `docs/hardware/T480.md`).
- **Speed controls follow `st_speed` and `st_audio_time`** in a Shadertoy
  import because the app ticks them at the clock's rate
  (`strobe_shadertoy::convert::clock_rate`, D246). Tools that score scenes
  run at the defaults, where that rate is 1.
- **Not generated:** `xy` pads (a centre, an offset), dropdowns (a mode
  switch), toggles, or anything in a `const` statement that shares its line.
- **Heuristics are tuned on Strobe's own library and original fixtures.**
  Community scenes will show what they miss.
- **Audio bindings** are suggestions; only float and colour sites can be
  written into GLSL, and a speed suggestion is data.
- **A 10-frame window** can miss an effect that only appears later.
