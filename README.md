<div align="center">

# Strobe

**A Synesthesia-style VJ engine in Rust, built for old laptops.**

</div>

Strobe plays audio-reactive GLSL scenes fullscreen on a projector or LED wall. It targets Synesthesia Shader Format (SSF) compatibility, Shadertoy import, and long-form playlist playback.

> **Status: early development, with a working Windows desktop shell.** The renderer, scene model, native GL/ANGLE paths, audio capture, scene scripting, diagnostics, and adaptive resolution are integrated. The performer-facing UI, context-loss recovery, Shadertoy workflow, playlists, and several media/compatibility features remain incomplete. See [the live roadmap](docs/ROADMAP.md) and [the current gap audit](docs/STATUS_AUDIT_2026-10-09.md).

- **SSF scenes.** `main.glsl`, `scene.json`, and `script.js`, including the implemented Synesthesia-style controls and script API.
- **Audio-reactive rendering.** Live device capture with a synthetic fallback, analysis, and audio textures in the app.
- **Native GL with ANGLE fallback.** Uses native OpenGL where available and ANGLE/GLES on Direct3D 11 when required.
- **Bounded scene scripts.** QuickJS execution is budgeted; script failures are logged and surfaced rather than silently ignored.
- **Measured performance work.** GPU timer queries drive adaptive render scale; the benchmark can render through headless or window contexts.
- **Still in progress.** Shadertoy import, playlists, full media/cubemap support, complete SSF edge-case compatibility, context-loss recovery, and the final Perform UI.

---

## How it differs from Synesthesia

| | Synesthesia | Strobe |
|---|---|---|
| Scene format | SSF | SSF compatibility with namespaced `X_STROBE` extensions |
| Shadertoy | Manual porting | Planned import workflow; not yet complete end to end |
| Playlists | Native show workflow | Planned inheritance, generator, and scheduler |
| Broken GL driver | Depends on system/driver | Native GL and ANGLE fallback; context-loss recovery remains open |
| Runaway script | Can stall the app | QuickJS execution budget interrupts runaway scripts |

---

## Architecture

A Cargo workspace. Portable crates produce values; `strobe-gfx` owns GL rendering, and `strobe-app` wires the runtime to the desktop windows.

| Crate | Owns |
|---|---|
| `strobe-app` | Window/event loop, egui control shell, output window, runtime wiring |
| `strobe-gfx` | GL contexts/providers, programs, passes, compositor, GPU timer queries |
| `strobe-scene` | SSF model, controls runtime, presets model, shader assembly |
| `strobe-ssf-lib` | SSF GLSL function library and preambles |
| `strobe-script` | QuickJS host for the Synesthesia JavaScript API |
| `strobe-audio` | Capture, FFT/bands/onsets/BPM analysis, audio textures |
| `strobe-diag` | Session logging, structured failures, crash capture and GL diagnostics |
| `strobe-bench` | Headless/windowed benchmark, frame output and contact sheet |
| `strobe-shadertoy` | Shadertoy JSON-to-scene conversion and asset fetching (workflow incomplete) |
| `strobe-playlist` | Playlist model, TOML I/O, scheduler and generator (app workflow incomplete) |
| `strobe-io` | MIDI, OSC, hotkeys, and localhost import listener (integration incomplete) |
| `strobe-conventions` | Repository rules enforced as tests |

### Graphics: native OpenGL with ANGLE fallback

Strobe feeds GLSL to the driver's compiler rather than relying on a GLSL frontend that rejects real-world shaders. On Windows, native GL is attempted first; ANGLE provides a GLES 3.0 path over Direct3D 11 when forced or needed. The renderer is shared across providers.

| Context | Shadertoy (`#version 300 es`) | SSF scenes |
|---|---|---|
| GL 4.3+ core | Compiled as-is with ES compatibility | `#version 330 core` |
| GL 3.3–4.2 core | Header rewritten to desktop GLSL | `#version 330 core` |
| GLES 3.0 (ANGLE) | Compiled as-is | Translated to GLSL ES 3.00 |

---

## Scene library

`scenes/` contains eleven original scenes spanning five GPU-load tiers, from simple gradients and feedback through reaction diffusion, bloom, raymarching, volumetric clouds, and a scripted orbit scene. They serve as both a starter library and benchmark workload.

![The base scene library](docs/images/contact-sheet.png)

*Rendered by `strobe-bench`. See [BENCHMARKS.md](docs/BENCHMARKS.md) for measurements and [hardware/T480.md](docs/hardware/T480.md) for real-device validation.*

---

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

The CI path includes formatting, clippy, workspace tests, and benchmark validation. Linux renderer tests use Mesa/EGL and compile both desktop GL and GLES shader dialects. The benchmark supports window-context mode for machines where EGL is unavailable.

## Roadmap

The project status and priority order are maintained in [docs/ROADMAP.md](docs/ROADMAP.md). The short version:

1. Verify Windows native/ANGLE behavior and crash capture on real hardware.
2. Implement context-loss recovery.
3. Decide and build the Perform-first UI using the [HTML topology mockups](docs/mockups/strobe-perform-directions.html).
4. Close script/SSF compatibility gaps.
5. Complete Shadertoy import, playlists, and show-control integration.
6. Implement media/cubemap support and transitions.
7. Validate the tag-triggered release end to end.

## Licensing of imported shaders

Imported shaders retain their author/source/license metadata. Check the individual shader license before using it in a paid or commercial setting. Strobe does not ship third-party shader libraries by default.

## License

MIT. See [LICENSE](LICENSE).
