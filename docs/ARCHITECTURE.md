# Architecture

This document covers how Strobe is put together and why the boundaries
sit where they do. For *what's built so far*, see [ROADMAP.md](ROADMAP.md).

## Crates

```
crates/
├── strobe-scene        SSF model: scene.json → SceneDef, controls runtime, shader assembly
├── strobe-ssf-lib      The SSF GLSL function library (one .glsl file, include_str!)
├── strobe-gfx          GL boundary (only unsafe), context providers, scene player
├── strobe-audio        FFT, bands, onsets, BPM → the syn_* values; cpal capture thread
├── strobe-script       QuickJS host for the Synesthesia JavaScript API
├── strobe-bench        Headless benchmark + visual regression frames
├── strobe-conventions  The house rules as tests (headers, unsafe boundary, toolchain pin)
├── strobe-diag         Logger, session context, panic hook, native crash capture
├── strobe-app          winit event loop, control window (egui) + output window, wiring
│
│   planned (see ROADMAP.md):
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
| Windowed native | glutin: WGL on Windows, GLX/EGL on Linux (CGL on macOS is not built) | GLSL 330 core |
| Windowed ANGLE | glutin EGL with ANGLE's `libEGL.dll`/`libGLESv2.dll` beside the exe | GLSL ES 300 |

## A frame

The output window's redraw (`strobe-app/src/app.rs`, `redraw_output`) is the
frame. Everything below runs on the event-loop thread, in this order; worker
threads only ever hand values *to* it.

```
1. dt from the clock (fixed in screenshot mode); window and render-size housekeeping
2. mouse uniforms from the pointer over the output window        (app::mouse)
3. GpuTimer::begin                                               (strobe-gfx::timing)
4. Session::advance:
     a. scene clock += dt
     b. audio frame at that time: live snapshot (one atomic load + copy) or
        the synthetic track                                       (app::audio)
     c. script, if the scene has one: events, then update(dt), then control
        writes and uniforms harvested                             (strobe-script)
     d. ControlBank::tick(dt)                                     (strobe-scene)
     e. ScenePlayer::upload_audio: spectrum and level-trail texels
     f. ScenePlayer::set_script_uniforms is called with step c's values
     g. ScenePlayer::render(&FrameInputs): every pass
5. ScenePlayer::present: letterboxed blit to the output window
6. GpuTimer::end, then poll: results of earlier frames that are ready (never
   waited for) go to AdaptiveScale; a new scale triggers a resize next frame
7. swap (vsync paces the loop)
```

The control window redraws at 30 Hz in its own event, never inside the output
frame: it refreshes the console from `strobe-diag`'s ring (only if the record
count moved), reads the audio status (at most twice a second), runs one egui
pass over a `Model` and applies the `Action`s it returns, and paints the
scene's output texture as the preview.

Not yet in the frame, from the original design: MIDI, OSC and hotkeys (step
1), the playlist tick, two active scenes during a transition, and the meta
post stage (brightness, hue, mirror).

## Diagnostics wiring

`main` follows `strobe_diag`'s documented order: register the notifier (the
same `dialog::show_error` the startup-failure path uses), run as the crash
monitor if asked, `strobe_diag::init()`, `--diagnose`, then
`crash::start_monitor()` held for the run. After the GL context exists its
vendor, renderer, version and backend go into the session context, driver
debug output is switched on (`gldebug`), and the scene name, render size and
frame counter are kept current. Shader compile failures, scene load errors,
script faults, resize failures and startup failures are `strobe_diag::failure`
records (`failures.rs`), so the log, the console and a crash report agree.

## Audio, scripts and performance

- **Audio** (`audio.rs`): one `AudioInput` for the run. Capture opens on a
  thread; synthetic audio plays until it is up and whenever capture is in
  `SilentFallback`. Settings (device, reactivity, speed, tap tempo) live in
  the app and are applied through the capture's atomics.
- **Scripts** (`script.rs`, `session.rs`): one `ScriptHost` per scene. Its
  first frame runs at load so the uniforms it creates are compiled into the
  shaders (`ScenePlayer::with_extras`); see D62.
- **Performance** (`perf.rs`): GPU time from a four-slot ring of timer
  queries feeds `AdaptiveScale`; budget is one refresh period of the output
  monitor; the render-scale slider is the ceiling; nothing changes in the
  first second after a scene load. See D64.

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
- **Audio textures.** `upload_audio(&AudioTextures)` quantises the spectrum
  and level-trail texels into staging buffers kept between frames and
  uploads only a texture some pass samples (1D on desktop, one-texel-high 2D
  on GLES).
- **Script uniforms.** `with_extras` declares the script's uniforms in every
  program and resolves their locations at link time; `set_script_uniforms`
  copies this frame's values into a reused buffer.
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
