<div align="center">

# Strobe

**A Synesthesia-style VJ engine in Rust, built for old laptops.**

</div>

Strobe plays audio-reactive GLSL scenes fullscreen on a projector or LED
wall. It runs Synesthesia-format scenes (`.synScene`) with their
JavaScript control API intact, imports Shadertoy shaders in one click, and
builds long shows from playlists that take minutes to author instead of
an afternoon of per-item data entry.

> **Status: early development.** The scene model, the SSF function
> library and the renderer work, and are tested headless against real
> GLSL compilers in both dialects. There's no window yet. See the
> [roadmap](#roadmap) and [`docs/ROADMAP.md`](docs/ROADMAP.md) for live
> status.

- **Scenes run as-is.** `main.glsl`, `scene.json`, `script.js`: the
  Synesthesia Shader Format, its uniforms, its function library, and its
  `setup` / `update` / `setUniform` / `setControl` / event-handler API.
- **Shadertoy in one click.** A "Send to Strobe" button on any shader
  page. Buffers, textures and the audio channel come along with it, and
  the audio input is your live sound instead of a music file. A
  generated control group gives you speed, audio-driven time, bass flash,
  zoom and hue on every imported shader without editing it.
- **Playlists by inheritance.** A cue that only names a scene is
  complete, because duration, advance mode and transition are inherited
  from the section and the playlist. You can generate a 4-hour show from
  tags, write it as TOML by hand, or both.
- **Fast on old hardware.** The floor is OpenGL 3.3, roughly a 2012
  Intel HD 4000. Render scale and adaptive resolution keep heavy shaders
  at frame rate. Nothing compiles during a show, and a runaway script
  can't freeze the output.

---

## How it differs from Synesthesia

| | Synesthesia | Strobe |
|---|---|---|
| Scene format | SSF | SSF, read as-is, plus one namespaced `X_STROBE` key for extensions |
| Shadertoy | Manual porting | One-click import, multipass, live audio channel |
| Playlists | Every item carries its own duration | Defaults → section → cue inheritance, generator, beat-locked durations, wall-clock anchor |
| Broken GL driver | Doesn't open | Falls back to ANGLE (GLES on Direct3D 11) |
| Runaway `while(true){}` in a script | Hangs | Interrupted within a per-frame budget; the scene keeps rendering |

---

## Architecture

A Cargo workspace. Each crate owns one concern, and the boundaries are
what keep the per-frame path fast and the portable parts testable on any
machine.

| Crate | Owns |
|---|---|
| `strobe-bench` | Headless benchmark: renders `scenes/`, times each, writes frames and a contact sheet |
| `strobe-conventions` | The house rules as tests: file headers, the `unsafe` boundary, the toolchain pin |
| `strobe-app` | Window, event loop, egui control surface, wiring |
| `strobe-gfx` | The GL renderer: context providers (native GL, ANGLE), programs, passes, compositor, post |
| `strobe-scene` | SSF model: `scene.json`, controls and their runtime, presets, shader assembly |
| `strobe-ssf-lib` | The SSF GLSL function library and preambles |
| `strobe-script` | QuickJS host for the Synesthesia JavaScript API |
| `strobe-audio` | Capture, FFT, bands, onsets, BPM, audio textures |
| `strobe-shadertoy` | Shadertoy JSON → scene conversion and asset fetch |
| `strobe-playlist` | Playlist model, TOML I/O, scheduler, generator |
| `strobe-io` | MIDI, OSC, hotkeys, the localhost import listener |

### Graphics: native OpenGL, with ANGLE as the fallback

Scenes and Shadertoy shaders are GLSL, and native GL hands GLSL straight
to the driver's compiler. That's the same model Synesthesia uses, and
it's why Strobe doesn't go through wgpu: naga's GLSL frontend rejects
too much real Shadertoy code.

On Windows, the native GL (WGL) path is the one that breaks on
hybrid-graphics laptops and old drivers. So context creation is a
provider, not an assumption. Strobe tries native GL 3.3+ first. If that
fails, or you force it, Strobe opens a GLES 3.0 context through
[ANGLE](https://chromium.googlesource.com/angle/angle) on Direct3D 11.
That's the same path Chrome takes to run Shadertoy itself on Windows.
The renderer above the context is identical in both cases.

| Context | Shadertoy (`#version 300 es`) | SSF scenes |
|---|---|---|
| GL 4.3+ core | Compiled as-is (`ARB_ES3_compatibility`) | `#version 330 core` |
| GL 3.3–4.2 core, macOS 4.1 | Header rewritten to desktop GLSL | `#version 330 core` / `410 core` |
| GLES 3.0 (ANGLE) | Compiled as-is | Translated to GLSL ES 3.00 |

---

## Scene library

`scenes/` holds ten original scenes. They're the base library, and also
the benchmark set, written to span five tiers of GPU load: from a
trivial gradient, through feedback, voronoi, domain warping, a
four-step reaction-diffusion simulation and a bloom pipeline, up to a
raymarched cathedral and volumetric clouds.

![The base scene library](docs/images/contact-sheet.png)

*Rendered headless by `strobe-bench`. Baseline timings are in [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md).*

---

## Requirements

- **Windows 10** or later, 64-bit, with a GPU that supports OpenGL 3.3, or
  Direct3D 11 for the ANGLE fallback. macOS and Linux come later; the
  portable crates already build and test on both.
- To build: the Rust toolchain pinned in `rust-toolchain.toml`. rustup
  installs it automatically. Windows builds use the MSVC toolchain.

## Building

```bash
cargo test --workspace                   # every crate's tests
cargo run --release -p strobe-bench      # render + time every scene, headless
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

On Linux, the renderer tests compile real shaders on a headless Mesa
context (`libegl1 libegl-mesa0`), against both desktop GL 3.3 core and
GLES 3.0. That covers the two dialects the native and ANGLE paths take.

---

## Roadmap

| Phase | Scope | State |
|---|---|---|
| 0 | Workspace, conventions, CI; GL context spikes (native + ANGLE) | Workspace, CI and headless contexts done; windowed contexts next |
| 1 | Renderer, SSF scene loading, multipass, controls UI, output window | Renderer and scene model done; window and UI next |
| 2 | JavaScript API (`rquickjs`, interrupt budget, conformance suite) | Planned |
| 3 | Audio analysis: bands, hits, BPM, spectrum textures, tap tempo | Planned |
| 4 | Shadertoy import: userscript, API, drop folder, batch | Planned |
| 5 | Playlists: inheritance, generator, transitions, resume | Planned |
| 6 | Presets, MIDI, OSC | Planned |
| 7 | NDI out, media input, song-change transitions | Planned |

---

## Licensing of imported shaders

Every Shadertoy shader carries its author's license, and most are
non-commercial (CC BY-NC-SA 3.0 by default). Strobe records the source
and author of every import. Before you show an imported shader in a
paid or commercial setting, check its license. Synesthesia's own
marketplace and built-in scenes are covered by its EULA. Strobe reads
scene folders you point it at and doesn't ship any.

## License

MIT. See [LICENSE](LICENSE).
