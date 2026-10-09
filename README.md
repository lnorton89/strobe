<div align="center">

# Strobe

**A Synesthesia-style VJ engine in Rust, built for old laptops.**

</div>

Strobe plays audio-reactive GLSL scenes fullscreen on a projector or LED wall. It targets Synesthesia Shader Format (SSF) compatibility, Shadertoy import, and long-form playlist playback.

> **Status: early development, with a working Windows desktop shell.** The renderer, scene model, native GL/ANGLE paths, audio capture, scene scripting, diagnostics, and adaptive resolution are integrated. The performer-facing UI, context-loss recovery, Shadertoy workflow, playlists, and several media/compatibility features remain incomplete. See [the live roadmap](docs/ROADMAP.md) and [the current gap audit](docs/STATUS_AUDIT_2026-10-09.md).

- **SSF scenes.** `main.glsl`, `scene.json`, and `script.js`, including implemented Synesthesia-style controls and script APIs.
- **Audio-reactive rendering.** Live device capture with synthetic fallback, analysis, and app audio textures.
- **Native GL with ANGLE fallback.** Native OpenGL where available; ANGLE/GLES over Direct3D 11 when required.
- **Bounded scene scripts.** QuickJS execution is budgeted; failures are logged and surfaced.
- **Measured performance work.** GPU timer queries drive adaptive render scale; the benchmark supports headless and window contexts.
- **Still in progress.** Shadertoy import, playlists, media/cubemaps, complete SSF edge-case compatibility, context-loss recovery, and final Perform UI.

---

## Architecture

A Cargo workspace. Portable crates produce values; `strobe-gfx` owns GL rendering, and `strobe-app` wires the runtime to desktop windows.

| Crate | Owns |
|---|---|
| `strobe-app` | Window/event loop, egui control shell, output window, runtime wiring |
| `strobe-gfx` | GL providers, programs, passes, compositor, GPU timer queries |
| `strobe-scene` | SSF model, controls runtime, shader assembly |
| `strobe-ssf-lib` | SSF GLSL helpers and preambles |
| `strobe-script` | QuickJS host for scene scripting |
| `strobe-audio` | Capture, analysis, audio textures |
| `strobe-diag` | Logging, structured failures, crash capture, GL diagnostics |
| `strobe-bench` | Benchmark rendering, timings and contact sheets |
| `strobe-shadertoy` | Shadertoy conversion/assets (workflow incomplete) |
| `strobe-playlist` | Playlist model and scheduler (app workflow incomplete) |
| `strobe-io` | MIDI, OSC, hotkeys, localhost import listener (integration incomplete) |
| `strobe-conventions` | Repository rules enforced as tests |

## Scene library

`scenes/` contains eleven original scenes spanning five GPU-load tiers, including a scripted orbit scene. They are both a starter library and benchmark workload.

![The base scene library](docs/images/contact-sheet.png)

*See [BENCHMARKS.md](docs/BENCHMARKS.md) and [hardware/T480.md](docs/hardware/T480.md) for measurements and device validation.*

## Requirements

- Windows 10 or later, 64-bit.
- GPU supporting OpenGL 3.3 or Direct3D 11 for the ANGLE fallback.
- Rust toolchain pinned in `rust-toolchain.toml`; rustup installs it automatically. Windows builds use MSVC.

## Building and testing

```bash
cargo xtask ci
cargo run --release -p strobe-bench
cargo xtask bench
```

Linux renderer tests use Mesa/EGL and compile desktop GL and GLES shader dialects. The benchmark supports window-context mode on machines where EGL is unavailable.

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md) for priorities and [docs/STATUS_AUDIT_2026-10-09.md](docs/STATUS_AUDIT_2026-10-09.md) for evidence-based acceptance criteria. Immediate priorities are Windows hardware validation, context-loss recovery, the Perform-first UI, SSF/script gaps, Shadertoy import, playlists, and media/show-control support.

## Licensing of imported shaders

Preserve source, author and license metadata when importing shaders. Check each shader's license before commercial use. Strobe does not ship third-party shader libraries by default.

## License

MIT. See [LICENSE](LICENSE).
