# strobe-scene

The Synesthesia Shader Format (SSF) as data. This crate turns a
`.synScene` folder (`main.glsl`, `scene.json`, optional `script.js`) into
checked Rust values, runs the per-frame control logic (smoothing, speed
integration, bangs, counters), and assembles the GLSL text the driver
compiles. It owns every SSF rule that can be decided without a GPU.

## Role in the workspace

- **Depends on:** `strobe-ssf-lib` (the GLSL function library, which the
  assembler embeds), plus `serde`, `serde_json` and `thiserror`.
- **Depended on by:** `strobe-gfx`, `strobe-audio`, `strobe-script`,
  `strobe-app` and `strobe-bench`. It is the bottom of the dependency graph
  in [`ARCHITECTURE.md`](../../docs/ARCHITECTURE.md).
- **Does not:** touch GL, spawn threads, or read a clock (`dt` is always
  passed in). The only file access is in `load`. It does not parse
  JavaScript (that is `strobe-script`), analyse audio (`strobe-audio`), or
  compile a shader (`strobe-gfx`).

## Main types / entry points

| Item | What |
|---|---|
| `SceneFolder` (`load`) | `SceneFolder::load(dir)` reads and validates a scene folder: `def`, `shader`, optional `script`. `load::scan(library)` lists scene folders in a library directory. |
| `SceneDef` (`scene`) | The validated `scene.json`: controls, passes (always ending with the implicit final pass), images, cubemaps, transition uniforms. `SceneDef::parse(json, fallback_title)`. |
| `json` | `Raw*` structs that mirror the file as written, tolerant of a number where an array is allowed and of unknown keys. |
| `control::ControlDef`, `ControlKind` | A validated control and its parsed `TYPE` (slider, knob, toggle, bang, xy, color, dropdown, with the smooth / speed / counter modifiers each accepts). |
| `control::ControlBank` | The live values of one scene instance: `set`, `set_normalized`, `press`, `set_default`, `randomize`, `reset`, `tick(dt)`, and `uniforms()` for the renderer. |
| `assemble` | `assemble::fragment(&Request)` and `assemble::vertex(Dialect)` build shader text. `Dialect` is `Glsl330`, `Glsl410` or `Es300`; `PassIndex` is a constant or a uniform; `map_error_line` maps a driver's log line back to a line of `main.glsl`. |
| `audio::SCALARS` | The one list of `syn_*` scalar audio uniform names, shared by the assembler, the analyser and the script host. |
| `synthetic` | A deterministic 120 BPM stand-in for audio (`fill`, `bands`, `fill_textures`), a pure function of time. |
| `meta` | The meta controls (master, brightness, hue, mirror...) as a `ControlBank` built by `meta::bank()`, which a script's `meta/` writes reach (D160). `MetaValues::read(&bank)` snapshots it into a plain `Copy` struct with no allocation, and `post_is_neutral()` tells the renderer's output post stage (D180) it has nothing to do. |
| `SceneDef::media` (`MediaDef`) | The optional `MEDIA` key (a Strobe extension, D186): a still file or a folder of pictures for `syn_Media`, with `FPS`, `LOOP`, `WRAP`, `FILTER`, `VFLIP`. A path that leaves the scene folder is a validation error. `scene::is_folder_relative` is the shared path rule. |
| `fixups` | GLSL ES rewrites for desktop-only SSF idioms; today `PASSINDEX == 0.0` becomes `float(PASSINDEX) == 0.0` (D163). |
| `preset` | Presets (`presets/<name>.json` in the scene folder: capture, apply by name through `ControlBank`, a checked name that cannot escape the folder, `load_all`) and the eight global fav slots (D176). |
| `names` | Name matching used by the script API (case-insensitive, ignoring `_`, `-` and spaces) and the reserved-name list. |
| `SceneError` | Every way a scene can fail to load; messages name the control or pass at fault. |

## Usage

Load a scene from the repository's library, tick its controls and
assemble the fragment shader for desktop GL. This compiles against the
current API:

```rust
use std::path::Path;
use strobe_scene::SceneFolder;
use strobe_scene::assemble::{self, Dialect, PassIndex, Request};
use strobe_scene::control::ControlBank;

fn main() -> Result<(), strobe_scene::SceneError> {
    let folder = SceneFolder::load(Path::new("scenes/01-aurora-gradient.synScene"))?;
    let mut controls = ControlBank::new(folder.def.controls.clone())?;
    controls.tick(1.0 / 60.0);
    for (name, value) in controls.uniforms() {
        println!("{name} = {value:?}");
    }
    let glsl = assemble::fragment(&Request {
        scene: &folder.def,
        user_code: &folder.shader,
        dialect: Dialect::Glsl330,
        pass_index: PassIndex::Constant(0),
        extra: &[],
    });
    println!("{} bytes of GLSL", glsl.len());
    Ok(())
}
```

For the renderer side of the same flow, see `crates/strobe-gfx/tests/render.rs`.

## Testing

```bash
cargo test -p strobe-scene
```

The tests are unit tests inside the modules (control parsing, defaults and
clamping, smoothing and bang behaviour, scene validation, name collisions,
shader assembly, driver error-line mapping, the synthetic audio). They need
no GPU, window or audio device. Whether the assembled text actually
compiles is checked in `strobe-gfx`'s render tests, which need headless EGL
(`libegl1 libegl-mesa0`); set `STROBE_REQUIRE_HEADLESS_GL=1` to make a
missing driver fail rather than skip.

## Design notes

- **`#version 330 core` on every desktop context** (D6): real SSF scenes
  compare `PASSINDEX == 0.0` and write `float x = 2;`, which desktop GLSL
  accepts and GLSL ES does not. `Dialect::Es300` exists for the ANGLE path.
- **The final pass is implicit** (D8): `PASSES` lists buffer passes, and
  `SceneDef::passes` always ends with one more, untargeted pass, so
  `PASSINDEX == passes.len() - 1` there.
- **Pass sizes are fractions of the design resolution** (D11), so render
  scale shrinks every buffer in proportion.
- **Frame-rate-independent smoothing** (D9): `alpha = 1 - (1 - PARAMS)^(dt*60)`,
  identical to SSF's per-frame step at 60 fps.
- **`PassIndex::Constant` versus `Uniform`** (D7): one specialised program
  per pass is the default; the shared-program mode is kept for comparison.
- **Original scenes only** (D10): `scenes/` is original work, and the
  unit tests here use inline JSON rather than any scene folder.

## Status and known gaps

- Done and tested headless: parsing, validation, the controls runtime and
  shader assembly ([`ROADMAP.md`](../../docs/ROADMAP.md)).
- Cube maps and `syn_Media` stills and sequences are loaded by `strobe-gfx`
  (D185, D186); this crate only declares and validates them. The `MEDIA` key
  and `CUBEMAPS` shape are Strobe's reading of SSF (Q22).
- `HARD_TRANSITIONS` / `SMOOTH_TRANSITIONS` uniforms are declared and
  initialised but never change; song-change transitions are phase 7.
- `X_STROBE`, Strobe's planned extension block, is currently ignored along
  with any other unknown key (`json.rs` says "later phases" will read it).
- The ANGLE (GLSL ES) dialect does not rewrite desktop-only SSF idioms
  (Q8 in [`OPEN_QUESTIONS.md`](../../docs/OPEN_QUESTIONS.md)); the shipped
  scenes are written ES-clean.
- Unverified against real Synesthesia: the audio texture layout (Q1, Q10),
  `_click` (Q13), the bang-smooth envelope (Q4), among others in
  `OPEN_QUESTIONS.md`.
