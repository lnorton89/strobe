# strobe-forge

The scene forge (D562). It does two jobs for the scene-from-image skill
(`.claude/skills/scene-from-image/`), which turns a photographed artwork
into a `.synScene`:

- **`check`** renders a scene the way the skill's step 4 asks and judges it:
  frames at chosen times, a sheet with the reference photo beside them, the
  frame time at 1080p, the quality gates, both GLSL dialects, and every
  control moved to its extremes.
- **`make`** runs the skill as a reproducible loop with a model: the picture
  and the skill go in, the model answers with `main.glsl` and `scene.json`,
  and each answer is checked, repaired and critiqued in turn.

## Role in the workspace

- **Depends on:** `strobe-gfx` (the scene player and `offscreen`'s headless
  or hidden-window context, as `strobe-bench` uses), `strobe-scene` (the
  folder, controls, the synthetic audio track), `strobe-autocontrol` (its
  image difference and the uniforms a `script.js` declares), `strobe-diag`
  (the stderr log), plus clap, anyhow, image, serde, serde_json and base64.
  The default `llm` feature adds ureq with rustls, the HTTP client `make`
  needs; `--no-default-features` builds without it and without C, which is
  how the Windows cross-check builds it, as it does `strobe-shadertoy`
  without `net`.
- **Used by:** nothing links it; it is a binary, run by hand and by agents
  following the skill.

## `check`

```text
cargo run --profile release-fast -p strobe-forge -- check <scene-dir> [--reference <photo>]
    [--out <dir>] [--size 1280x720] [--times 2,7,15,40,300] [--gl native|angle|auto]
    [--context auto|headless|window] [--no-dialect] [--no-timing]
```

Output goes to `--out`, by default `target/forge/<slug>/` (the folder name
without `.synScene`):

- `frame-<t>s.png` at each time. A frame "at" a time is the last of 1.5 s of
  frames at a fixed 1/60 s ending there, with the bench's synthetic 120 BPM
  audio, so a clock pass has history and two runs give identical pictures.
- `sheet.png`: the reference, turned upright by its EXIF orientation and
  letterboxed into the first cell, then the frames, three across. The
  photo itself is never copied into the output folder, but the sheet holds
  a scaled copy of it, so keep `sheet.png` local like the photo
  (`target/` is gitignored).
- `controls.png`: one row per probed control, its default render first,
  then each probe (sliders, knobs, toggles and pads at MIN and MAX, every
  other dropdown value, one bang press 1.25 s before the frame, since a
  smooth bang shows its press only after about 0.28 s; colours are
  skipped). There is no font in the workspace, so `report.md` lists the
  rows in order.
- `report.md` and `report.json`: title, control count, compile time and
  log, both dialects, the timing at 1920x1080 with the renderer (three
  batches of 30 frames; the lowest batch mean and its p95 are reported,
  since other GPU work only adds time, with a warning when the batches
  differ by more than 2x), each gate with its measured number and limit,
  each control's change from the default render, warnings and failures.

The gates are the website's (`site/src/lib/play/image/gate.ts`, D560),
ported: almost black, one flat colour, blown out, frozen (no frame differs
from the first) and the WCAG 2.3.1 / ITU-R BT.1702 flash rule (more than
three luminance flashes a second over a quarter of the screen), measured
over four seconds sampled at 30 fps.

The exit status is non-zero only for a hard failure: a compile error in
either dialect, black, frozen or flashing. Flat, blown out, a control that
does little and an extreme that blacks or blows out the picture are
warnings in the report. A control does little when every probe is small
both over the whole frame (under `strobe-autocontrol`'s visibility
threshold, 0.003 of full scale) and locally (the largest tile of a 16x16
grid under 0.02), so a control that moves only a pupil or a sparkle
is not flagged; the report gives both numbers and the share of pixels
that changed.

On Windows a running `strobe-forge.exe` is locked, so `cargo run` fails
with "Access is denied" while another check is going (cargo cannot replace
the file). Run the built binary directly instead:
`target/release-fast/strobe-forge.exe check <scene-dir> ...`.

The second dialect needs a second context, and winit allows one window
event loop per process, so `check` runs itself again with a hidden
`dialect` subcommand: headless GLES when the check ran headless, an ANGLE
window when it ran on native GL (or native when it ran on ANGLE). Where
that context cannot be had, the report says "not checked" and the check
does not fail. A child that takes longer than four minutes is stopped and
counted the same way.

## `make`

```text
cargo run --profile release-fast -p strobe-forge -- make <photo> --out <scene-dir>
    [--model claude-opus-5-5] [--rounds 2] [--skill .claude/skills/scene-from-image]
```

1. The system prompt is the skill: `SKILL.md`, then every file in
   `reference/` in name order (the Markdown guides and the template's GLSL
   and JSON), read **at run time** from `--skill`, so an edit to the skill
   reaches the next run without a rebuild. A short contract follows it: no
   tools, answer with two tagged blocks.
2. The picture goes as a JPEG, resized to at most 1568 pixels on its long
   side.
3. The answer must hold exactly `<file name="main.glsl">` and
   `<file name="scene.json">`. Any other file (`script.js` included), and
   any `scene.json` key outside TITLE, CREDIT, DESCRIPTION, WIDTH, HEIGHT,
   PASSES, CONTROLS, TAGS, HARD_TRANSITIONS and SMOOTH_TRANSITIONS (`IMAGES`,
   `CUBEMAPS`, `MEDIA` and `PASS_SOURCES` by name), is refused, as is a
   `scene.json` the scene format rejects.
4. A refused or non-compiling answer goes back with the reason or the
   compile log, up to three repairs a round. A scene that compiles is
   checked; the report and the sheet go back with the skill's critique step
   for `--rounds` rounds. A revision that fails a hard gate does not replace
   a scene that passed, and the model is told so.
5. Every round is written to `<out>/rounds/<n>/` with its check in
   `check/`; the scene that stands is written to `<out>/` with
   `forge-log.md`.

The one provider is the Anthropic Messages API, called over HTTPS (there is
no Rust SDK). The model is `claude-opus-5-5` unless `--model` names
another; requests ask for effort `high`, leave thinking at the model's
adaptive default, and on the models that take it turn on the server-side
fallback (`fallbacks: "default"`) so a classifier's false positive is rerun
rather than ending the run. Replies are replayed unchanged, so the
conversation stays append-only. The `Author` trait is the provider-neutral
boundary (`docs/LLM_SHADER_CREATION.md`).

**Privacy.** `make` sends the picture, the skill's text and every rendered
sheet to the provider, with your own key from `ANTHROPIC_API_KEY`; it
refuses to start when the variable is unset. The key is sent only as a
request header and is never logged or written. Photos of other people's
art stay private: write the output under `user-scenes/` (D526, D532).
`check` sends nothing anywhere.

## Status and known gaps

Open work for this crate is tracked in
[`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md) under the
IDs given here; a gap with no ID is by design, with its decision named.

- **Verified:** `check` on LIGHTNING's RX 580 through a hidden window (no
  headless EGL on Windows), on the skill's template and on a library scene.
  The loop, the reply parser, the prompt and the Anthropic request and
  response shapes are unit-tested with fakes; no test needs a key, a
  network or a GPU, and the one test that renders skips without a headless
  context unless `STROBE_REQUIRE_HEADLESS_GL` is set.
- **`make` has not run against the real API yet**, and OpenAI and Google
  providers are not built; requests are not streamed (backlog I-07).
- Not built for macOS yet: `strobe-gfx` has no CGL provider (backlog G-07).
