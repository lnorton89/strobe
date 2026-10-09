# Architecture

This document covers how Strobe is put together and why the boundaries
sit where they do. For *what's built so far*, see [ROADMAP.md](ROADMAP.md).

## Crates

```
crates/
├── strobe-scene        SSF model: scene.json → SceneDef, controls runtime, shader assembly
├── strobe-ssf-lib      The SSF GLSL function library (one .glsl file, include_str!)
├── strobe-gfx          GL boundary (only unsafe), context providers, scene player
├── strobe-audio        FFT, bands, onsets, BPM → the syn_* values; cpal capture thread (built)
├── strobe-bench        Headless benchmark + visual regression frames
├── strobe-conventions  The house rules as tests (headers, unsafe boundary, toolchain pin)
├── strobe-diag         Logger, session context, panic hook, native crash capture
│
│   planned (see ROADMAP.md):
├── strobe-app          winit event loop, control window (egui) + output window
├── strobe-script       QuickJS host for the Synesthesia JavaScript API
├── strobe-shadertoy    Shadertoy JSON → scene conversion
├── strobe-playlist     Playlist model, TOML, scheduler, generator
└── strobe-io           MIDI, OSC, hotkeys, localhost import listener
```

### Dependency direction

```
strobe-ssf-lib ◀── strobe-scene ◀── strobe-gfx ◀── strobe-app
                        ▲   ▲                        │
                        │   └── strobe-script ◀──────┤
                        └────── strobe-audio  ◀──────┤
                                strobe-playlist ◀────┘
```

- `strobe-scene` knows nothing about GL, threads, or clocks: `dt` is
  passed in. Every SSF rule (control types, smoothing, pass layout, name
  matching, shader text) is tested there with no GPU.
- `strobe-gfx` knows nothing about audio capture, scripts, or playlists.
  It renders a scene given a `FrameInputs` (time, audio values, controls).
- `strobe-audio` and `strobe-script` must not depend on `strobe-gfx`.
  They produce values, and the app hands those values to the player.
- `strobe-diag` depends on no other Strobe crate, so every crate may
  depend on it (for `log` output, `context::set`, `failure::report`).
  `strobe-gfx` itself only uses the `log` facade; the app is what installs
  the backend.

## The GL boundary

All `unsafe` lives in `strobe-gfx/src/gl/`, with one exception:
`strobe-diag/src/native.rs` holds the single call that installs the native
crash handler (D41). `strobe-conventions` allows exactly those two places.
Within the GL module:

- `gl/context.rs` creates contexts. It is the only place that makes a
  context current, and the only place a `Gl` is constructed, immediately
  afterwards.
- `gl/mod.rs` holds `Gl`, the safe wrappers (one GL call each), and the
  owning resource types (`Program`, `Texture`, `Framebuffer`,
  `VertexArray`, `Shader`), each released on `Drop`.

`Gl` is `!Send`. A GL call is only defined on the thread where the
context is current, so the type system keeps `Gl` on that thread.

### Context providers

| Provider | Where | Dialect |
|---|---|---|
| `Headless::new(Api::DesktopCore33)` | Linux CI and the bench (Mesa llvmpipe via an EGL device display) | GLSL 330 core |
| `Headless::new(Api::Gles30)` | the same, for testing the ANGLE dialect | GLSL ES 300 |
| Windowed native (planned) | glutin: WGL on Windows, GLX/EGL on Linux, CGL on macOS | GLSL 330 core |
| Windowed ANGLE (planned) | glutin EGL with ANGLE's `libEGL.dll`/`libGLESv2.dll` beside the exe | GLSL ES 300 |

## A frame (target design; the current state is in ROADMAP.md)

```
1. drain input (UI, MIDI, OSC, hotkeys)            → control targets
2. snapshot audio analysis                         → syn_* values, spectrum textures
3. playlist tick                                   → start/finish transitions
4. per active scene (1, or 2 during a transition):
     a. JS event handlers, then update(dt)         (strobe-script)
     b. ControlBank::tick(dt)                      (strobe-scene)
     c. ScenePlayer::render(&FrameInputs)          (strobe-gfx)
5. composite transitions, meta post (brightness, hue, mirror…)
6. present: output window full-res, control window egui + preview
```

## Scene player internals (`strobe-gfx/src/player.rs`)

- **Programs.** By default there is one program per pass, with
  `const int PASSINDEX = n;`. The alternative is one shared program with
  `uniform int PASSINDEX`. Uniform locations and sampler units are
  resolved when the program links and stored in `Compiled`.
- **Targets.** Each listed pass gets one texture plus FBO, or two when
  `DOUBLE_BUFFER` is set (ping-pong). The final pass always ping-pongs,
  so `syn_FinalPass` is the previous frame. Every new target is cleared
  to `vec4(0)`.
- **Read rule.** When pass `i` samples target `j`: for `j < i` it reads
  this frame's output; for `j >= i` it reads last frame's.
- **Sampler units.** Units are assigned in a fixed order: `syn_FinalPass`,
  `syn_Media`, `syn_Spectrum`, `syn_LevelTrail`, pass targets, images,
  cubemaps. Each sampler gets its own unit even when the driver optimised
  it out, so a `samplerCube` never shares a unit with a `sampler2D`,
  which GL rejects at draw time.

## Shader assembly (`strobe-scene/src/assemble.rs`)

```
#version 330 core | #version 300 es (+ precision statements)
uniforms: standard, audio scalars, syn_Spectrum/LevelTrail, controls,
          pass targets, images, cubemaps, transition uniforms, script extras
const float PI; vec2 _xy, _uv, _uvc; out vec4 strobe_FragColor;
#define texture2D texture
<strobe-ssf-lib>
#line 1 1        ← user code is source string 1
<main.glsl>
#line 1 2
void main() { _xy/_uv/_uvc setup; strobe_FragColor = renderMain(); }
```

`map_error_line` reads Mesa, NVIDIA and ANGLE/AMD/Intel log formats, so
an error points at `main.glsl:N`.
