# strobe-script

The QuickJS host for the Synesthesia JavaScript API. A scene may ship a
`script.js`; this crate runs it once per frame in a sandboxed interpreter
(`rquickjs`), exposes the frame's uniforms and the scene's controls as
globals, and hands back values: published uniforms, control writes, console
lines and a health status. A script that loops forever or throws cannot
freeze or crash the output.

## Role in the workspace

- **Depends on:** `strobe-scene` (the `ControlBank`, name matching, the audio
  uniform list, `ExtraType`), `rquickjs` (which compiles QuickJS's C sources)
  and `thiserror`. Dev-dependencies are `anyhow` and `serde_json`.
- **Depended on by:** `strobe-app`.
- **Does not:** touch GL, audio capture or a clock; `dt` and every input are
  passed in. It never recompiles a shader: it reports which uniforms the
  script has published and the app compiles them in (D62). It contains no
  `unsafe`, so native functions cannot hold `&mut ControlBank`; they queue
  writes that the host applies in order at the end of the frame.
- **Is not `Send`.** A `ScriptHost` is built and driven on the render thread.

## Main types / entry points

| Item | What |
|---|---|
| `ScriptHost` | One per scene instance. `new(source, &controls, Budget)` evaluates `script.js` and runs `setup()`; `frame(dt, &inputs, &mut controls)` runs one frame; `announce_extras()`, `last_fault()`, `is_disabled()`. |
| `ScriptInputs` | One frame's inputs: `time`, `frame_count`, `audio` scalars in `SCALARS` order, mouse (`mouse`, `click`, `muv`, `muvc`) and an optional 64x36 RGB8 `final_pass` downsample. `Default` is a silent room. |
| `FrameOutput` | `uniforms` (name, GLSL type, value), `new_uniforms`, `console` lines, `meta_writes`, `status`; `extras()` gives the `(name, type)` list the shader assembler takes. |
| `ScriptStatus` | `Running`, `Faulted { consecutive }` or `Disabled`. |
| `Budget` | Limits: 4 ms per frame, 250 ms to load, 64 MiB heap, 1 MiB stack, and a seed for `randomizeControl` / `randomizeGroup`. |
| `ScriptError` | Why `new` failed (load error, `setup()` error, interpreter errors). |

The API surface a script sees is `setUniform`, `setControl`,
`setControlNormalized`, `randomizeControl`, `defaultControl`,
`randomizeGroup`, `defaultGroup`, the event registrations `onChange`,
`onOffToOn`, `onOnToOff`, `whileOn` and `whileOff`, `textureFinalPass`,
`print` and `console`, plus the standard uniforms and the audio scalars as
globals.

## Usage

Load the shipped script scene, build the host and run one frame, as the app
does (this compiles against the current API):

```rust
use std::path::Path;
use strobe_scene::SceneFolder;
use strobe_scene::control::ControlBank;
use strobe_script::{Budget, ScriptHost, ScriptInputs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scene = SceneFolder::load(Path::new("scenes/11-script-orbits.synScene"))?;
    let mut controls = ControlBank::new(scene.def.controls.clone())?;
    let source = scene.script.as_deref().unwrap_or_default();
    let mut host = ScriptHost::new(source, &controls, Budget::default())?;

    let inputs = ScriptInputs::default();
    let out = host.frame(1.0 / 60.0, &inputs, &mut controls);
    controls.tick(1.0 / 60.0); // the script's control writes are already applied
    println!("{:?}, {} uniforms, {} console lines", out.status, out.uniforms.len(), out.console.len());
    Ok(())
}
```

The app's side of the contract (compiling the first frame's uniforms in at
load, then sending them each frame) is `crates/strobe-app/src/script.rs`,
`session.rs`, and `crates/strobe-app/tests/script_uniforms.rs`.

## Testing

```bash
cargo test -p strobe-script
```

The tests are a conformance suite in `src/conformance.rs`: each one writes a
small `script.js`, builds a real `ControlBank`, and drives `ScriptHost`
frame by frame, asserting on the harvested output and the bank rather than
pixels. No GPU is needed. Most tests use a relaxed time budget so a loaded
CI machine does not cause a spurious fault; the watchdog tests use the 4 ms
default on purpose. Building the crate needs a C compiler for QuickJS; the
Windows cross-check from Linux excludes it for that reason (D63), so only
the CI `windows` job and the T480 check it on Windows.

## Design notes

- **Containment.** Each host owns its interpreter, capped at 64 MiB and a
  1 MiB stack. A frame runs under a deadline (4 ms); `while (true) {}` is
  interrupted and the last good uniforms stay in force. Three faulted frames
  in a row disable the script until the host is rebuilt, which a reload does.
- **Two kinds of error.** A malformed call (a non-number, five components, a
  uniform named `TIME`) throws a JavaScript `TypeError`; a name that matches
  nothing logs one console error and carries on.
- **Script uniforms are compiled in at load** (D62): the app runs the
  script's first frame before building the player, so `setUniform` names
  from `setup()` or the first `update` are declared in every pass. A
  uniform published later costs one recompile, logged once.
- **Event threshold**: "on" is `value >= 0.5`. A handler takes the current
  value as its previous one on frame one, so edge handlers do not fire just
  because the scene started.
- **Name matching** is `strobe_scene::names::normalize`: case-insensitive,
  ignoring `_`, `-` and spaces.
- The shipped `11-script-orbits` drives its picture through `setControl`,
  because the bench runs no scripts (D62).

## Status and known gaps

- Built and wired into the app (phase 2, [`ROADMAP.md`](../../docs/ROADMAP.md)).
  On the T480, `11-script-orbits` ran with `script.js running` and
  script-written positions ([`T480.md`](../../docs/hardware/T480.md)).
- `textureFinalPass` returns black today: the app does not supply the
  previous frame's downsample, because a per-frame GPU readback would stall
  old hardware (Q14, D62).
- Writes to `meta/` controls are recorded in `FrameOutput::meta_writes` but
  the app drops them; meta controls do not exist yet.
- `_click` is ambiguous between `strobe-gfx` (buttons held) and this crate
  (last click and button state); the app feeds both readings (Q13).
- The conformance tests are written per documented API surface; Q13 and Q14
  in [`OPEN_QUESTIONS.md`](../../docs/OPEN_QUESTIONS.md) are waiting on a real
  script that exercises `_click` and `textureFinalPass`.
