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
├── strobe-bench        Benchmark + visual regression frames (headless or hidden-window context)
├── strobe-conventions  The house rules as tests (headers, unsafe boundary, toolchain pin)
├── strobe-diag         Logger, session context, panic hook, native crash capture
├── strobe-shadertoy    Shadertoy export JSON → scene folder (id parser, model, dialect rewrite, converter, strobe-import)
├── strobe-autocontrol  Auto-generated controls: finds tunable literals, rewrites them to uniforms, keeps those that measurably change the image
├── strobe-app          winit event loop, control window (egui) + output window, wiring
├── strobe-playlist     Playlist model, TOML, generator, timeline, playback clock (pure)
│
│   planned (see ROADMAP.md):
└── strobe-io           MIDI, OSC, hotkeys, localhost import listener
```

### Dependency direction

```
strobe-ssf-lib ◀── strobe-scene ◀── strobe-gfx ◀── strobe-app
                        ▲   ▲                        │
                        │   └── strobe-script ◀──────┤
                        └────── strobe-audio  ◀──────┤
                                strobe-playlist ◀────┘

strobe-scene ◀── strobe-autocontrol ──▶ strobe-gfx   (nothing depends on it yet;
strobe-shadertoy ◀──┘  (tokenizer only)                 scoring needs a GL context)

strobe-scene ◀── strobe-shadertoy         (nothing depends on it yet;
                                           dev-dependency: strobe-gfx, for
                                           the render tests only)
```

- `strobe-scene` knows nothing about GL, threads, or clocks: `dt` is
  passed in. Every SSF rule (control types, smoothing, pass layout, name
  matching, shader text) is tested there with no GPU.
- `strobe-gfx` knows nothing about audio capture, scripts, or playlists.
  It renders a scene given a `FrameInputs` (time, audio values, controls).
- `strobe-audio` and `strobe-script` must not depend on `strobe-gfx`.
  They produce values, and the app hands those values to the player.
- `strobe-shadertoy` depends on `strobe-scene` only, to validate what it
  generates with `SceneDef::parse`. `strobe-gfx` must never depend on it;
  its render tests take `strobe-gfx` as a dev-dependency instead.
- `strobe-autocontrol` depends on `strobe-scene`, `strobe-ssf-lib`, the
  tokenizer and tab markers of `strobe-shadertoy`, and `strobe-gfx` (to
  render for scoring). `strobe-gfx` must never depend on it, and the frame
  loop never calls it: scoring runs at import time (D123).
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
| `offscreen::run` | The bench and `strobe-autocontrol` (D140): `Headless` when an EGL device exists, else one hidden window's `WindowedContext` (Intel on Windows has no EGL device). Runs a job with `&Gl` and `&Caps`; main thread, once per process | whichever it opened |

## A frame

The **scene frame** (`strobe-app/src/app.rs`, `scene_frame`) is the frame. It
runs on the `next_output` schedule whether or not the output window can be
seen, because the control window's preview paints the scene's output texture
(D96). The output window starts hidden, and a hidden window may never get a
`RedrawRequested`, so `Running::schedule` decides who calls it:

- **output shown**: it requests a redraw (up to 144 Hz apart) and the redraw
  runs `scene_frame` on the output surface, then steps 5 and 8 below. The
  swap's vsync paces the loop.
- **output hidden**: `schedule` (from `about_to_wait`) runs `scene_frame`
  itself on the *control* surface, one refresh period of the output's monitor
  apart (60 Hz if unknown), and skips steps 5 and 8. Nothing is swapped, so
  nothing paces the CPU against the GPU; instead the frame is skipped while
  all four GPU timer slots are still waiting (`Performance::backlogged`),
  which bounds the queue to four frames when the context has a timer.

Everything below runs on the event-loop thread, in this order; worker
threads only ever hand values *to* it.

```
0. fault check (STROBE_FAULT), then make_current on the output surface (shown) or
   the control surface (hidden); a failure skips the frame and goes to the
   recovery policy (see "Context loss"). Hidden only: poll the timer ring and
   stop here if it is full
1. dt from the clock (fixed in screenshot mode); window and render-size housekeeping
2. mouse uniforms from the pointer over the output window        (app::mouse)
3. GpuTimer::begin                                               (strobe-gfx::timing)
3b. With a playlist loaded and compiled (Show::frame, app::show::run):
     a. beat = BeatReader::read(audio frame's scalars)            (show::beat)
     b. PlaylistClock::tick(dt, &beat); drain_events into a buffer
        kept between frames; for each event:
          CueStarted        swap the cue's session into Stack::session (the old
                            one becomes `outgoing`); a warmed scene is grown
                            with resize_keeping, a cold one is resized; control
                            defaults, then the cue's overrides
          WarmUp            take the next scene from the cache, apply its
                            overrides, render it at half each edge
          TransitionStarted pick the Blend (crossfade, fade-through-black or a
                            custom transition compiled at load)
          CueSkipped, HoldChanged, PlaylistEnded   console, banner
          BlackoutChanged   the app's blackout flag
     c. step 4 below for the live scene, the outgoing scene and the warming
        scene, each with its own clock
     d. while clock.transition() is Some: Compositor::render(outgoing, live,
        blend, progress) into the compositor's own target
     e. a finished transition parks the outgoing scene (targets cut to 1x1)
     f. shader-compile counter check; SaveTimer (resume point to its thread);
        file watcher poll (once a second)
   With no playlist this step is absent and the live scene is advanced by step 4.
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
5. ScenePlayer::present (or Compositor::present while a transition runs):
   letterboxed blit to the output window (shown only)
6. GpuTimer::end, then poll: results of earlier frames that are ready (never
   waited for) go to AdaptiveScale; a new scale triggers a resize next frame
7. reset-status health check, at most once a second, on a robust context only
8. swap (shown only; vsync paces the loop); a failure asks the driver for the
   reset status and tells the recovery policy, a success tells it the window
   presented
```

The GPU timer brackets steps 3b to 5, so hidden it brackets the scene alone;
the budget it is judged against is the same one refresh period, so adaptive
resolution behaves the same either way. In screenshot mode a shown output is
read back from its own back buffer and a hidden one from the scene's output
texture.

The control window redraws at 30 Hz in its own event, never inside the output
frame: it refreshes the console from `strobe-diag`'s ring (only if the record
count moved), reads the audio status (at most twice a second), runs one egui
pass over a `Model` and applies the `Action`s it returns, and paints the
scene's output texture as the preview.

Not yet in the frame, from the original design: MIDI and OSC (step 1), audio
events that end a cue (`clock.audio_event()`), and the meta post stage
(brightness, hue, mirror).

Loading a playlist is not in the frame at all. `Running::compile_show_step`
runs once per control-window frame, after that frame is painted, and compiles
one scene (or one custom transition) with the context current on the control
surface; the scene frame does not tick the clock until every step is done.
See D150-D158.

## The control window's UI (`strobe-app`)

**Contextual Perform** (D120, built in stage 1 as D130 to D138). The UI is a
pure function of a borrowed `ui::Model` that returns `Action`s; the app owns
every piece of state and carries the actions out after the frame is painted.

```
rail | browser (optional) | preview               | inspector / diagnostics / settings (optional)
-------------------------------------------------------------------------------------------
title bar: brand, crumb, health readouts, window buttons (above all of it)
transport strip: prev, hold (unavailable), next, NOW and NEXT, time, BPM, blackout
macros: the playing scene's first controls
```

Layers (UI_IMPLEMENTATION_PLAN section 3), and where they are:

| Layer | Code |
|---|---|
| product state | `app.rs` (`Running`), `session.rs`, `audio.rs`; per-scene `library::Health` |
| presentation state | `ui::state::UiState`, saved by `prefs` (a debounced writer thread, D133) |
| adapters | `ui::model`: `Model` and its panels, borrowed for one frame |
| commands | `ui::model::Action`, and `commands` (the keyboard table and focus rule, D134) |
| rendering | the preview paints the scene's output texture as a native egui texture; there is no readback |

Portable modules (no egui, unit-tested anywhere): `color`, `palette`,
`motion`, `icon`, `workspace` (the pane model and how it fits a window),
`prefs`, `commands`, and `library` (search, filter, health and stepping). The
egui layer is `ui/`: `theme`, `motion` and `icon` bind the portable ones;
`widgets` and `modal` are the shared vocabulary; `shell` composes `chrome`,
`perform`, `transport` and `macros` with the panes `browser`, `inspector`
(with `controls`), `diagnostics`, `import` and `settings`; `keys` turns key
events into commands; `host` runs egui on the control window.

What a control-window frame does (30 Hz, off the output frame's path):

1. refresh the console and the audio status (cheap, as before);
2. if a load or a preview was asked for last frame, remember it (the UI shows
   "preparing" this frame);
3. advance the browser's preview session, if there is one;
4. register the preview textures, and build the `Model` (no allocation beyond
   the model's own borrows);
5. one egui pass: `shell::show` (hashing `UiState` to notice a change, reading
   key events, drawing), then paint;
6. `UiState::tick` (debounced save), the `--screenshot` capture if due,
   present, then apply the actions;
7. only now, run the blocking load or preview preparation, so the frame that
   said "preparing" has been on screen.

New in the frame path, all in `app.rs`:

- **Scene stepping** (`library::step`: wraps, skips scenes known to fail).
- **Blackout**: when the output is shown, `scene_frame` clears it to black
  instead of calling `present`. The scene keeps rendering.
- **Two-phase load** and a **per-scene health** vector, so a failed load
  leaves the old scene live and marks the row.
- **A second `Session` for Preview**, kept in the `Stack` before the context
  in drop order, rendered at 384 px and dropped on a context rebuild.
- **A notice** after a context rebuild ("rebuilt on ANGLE in 181 ms; the scene
  is playing again").
- **The title bar**: `Layout::initial` opens the control window undecorated
  when `custom_title_bar` is set, and `Action::SetCustomTitleBar` flips it
  live (D132).

The rules that keep it cheap and honest, each a test: the drawing modules
hold no colour literal, hand-picked gap, hand-rolled animation or font icon;
no fixed panel has a dead resize handle; nothing in them touches the disk,
the driver or a socket; `shell_tests` draws every state at six window sizes
and asserts it fits; `palette` checks every theme's contrast. The per-frame
cost is measured and shown (`Control UI` in the diagnostics: the egui layout pass, about 1 ms in a
debug build; the GL paint is the driver's and the T480 number is open).

## Context loss

A projector must never go dark because one driver call failed (D66).

- **Detection** (`strobe-gfx`): both backends request a robust
  (`RobustLoseContextOnReset`) context and fall back to default attributes if
  the driver refuses, logging which they got (`WindowedContext::robust`).
  `Gl::reset_status()` loads `glGetGraphicsResetStatus` (plain, `ARB`, `KHR` or
  `EXT` name, chosen by version and extensions) through the glow loader. It is
  a driver round trip: it runs after a failed swap and as a once-a-second
  health check, never per frame, and never after a failed `make_current`.
- **Lost flag**: `Gl::mark_lost()` sets a flag shared by every clone; every
  owning type (`Program`, `Texture`, `Framebuffer`, `Shader`, `TimerQuery`,
  `VertexArray`) skips its delete on drop when it is set, and egui's painter is
  abandoned rather than destroyed. The driver frees a destroyed context.
- **Policy** (`strobe-app/src/recovery.rs`): a pure state machine, fed
  failures, reset statuses, successes, rebuild results and time. Three failed
  frames on a window, or any reset report, mean rebuild; a second native
  rebuild within 60 s, or a failed native rebuild, means ANGLE (only when
  `--gl auto`); ANGLE failing too, or more than four rebuilds in 60 s, means
  give up through the normal failure dialog.
- **Rebuild** (`Running::recover`, run from `App::settle` where the
  `ActiveEventLoop` is available): mark lost, drop the session, timers, egui,
  **both windows** and the context, in that order (WGL sets a window's pixel
  format once, so windows are never reused), release the context from the
  thread (glvnd refuses an EGL context while a GLX one is current), build a new
  `Stack` on the chosen backend, restore fullscreen, monitor, maximized and
  shown/hidden state (the output window is recreated with the same
  visibility), resume the adaptive render scale, and reload the scene at its clock
  with its control values. Audio, library, console and screenshot state are
  not GL state and carry over untouched.
- **Records**: each loss and rebuild is a `CONTEXT` failure record, plus an
  `INFO` line the console shows. `STROBE_FAULT` injects a loss for testing
  (DEBUGGING.md).

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
  `syn_Media`, `syn_Spectrum`, `syn_LevelTrail`, `strobe_ShadertoyAudio`,
  pass targets, images,
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

## Playlists (`strobe-playlist`)

Pure: no GL, no threads, no system clock, no I/O except `file::load` / `save`
and `ResumePoint::save` / `load` (both atomic). Depends on serde, toml and
thiserror only; it does not depend on `strobe-audio` or `strobe-scene`
(the clock takes a `BeatInfo`, overrides use its own `ControlValue`).

| Part | Module | Entry points |
|---|---|---|
| model | `model`, `duration` | `Playlist`, `Section`, `Cue`, `CueSettings`, `DurationSpec` (`90s`, `1:30`, `32 bars`, `rand(a, b)`) |
| inheritance | `resolve` | `resolve()` to `ResolvedCue`, `identity()`, `distinct_scenes()` |
| file | `parse`, `write`, `file` | `parse::parse`, `write::to_toml`, `file::load` / `save`; errors carry line and column |
| generator | `generate`, `rng` | `generate(&LibrarySnapshot, &GenerateSpec)` to a `Section`; `SplitMix64` |
| timeline | `timeline`, `schedule` | `Timeline::from_playlist`, `total_runtime`, `locate`, `place`, `wall_time` |
| playback | `clock`, `tempo` | `PlaylistClock::tick(dt, &BeatInfo)`, `apply(Command)`, `drain_events()` |
| resume | `resume` | `PlaylistClock::resume_point()` / `restore()`, `ResumePoint` TOML |

The playlist tick sits where the frame section above says it will (step 3b):
after input, before the scene renders. It lives in `strobe-app`'s `show`
module, which is the glue and the only place the pure crate meets a GPU:

| Module | What |
|---|---|
| `show::run` | `Show`: the precompile (one step per control frame), the frame, warm-up, transitions, commands, the compile counter. Lent an `Env` (context, the live scene's slot, the compositor, audio) per call. |
| `show::cache` | `SceneCache<T>`: a slot per distinct scene, progress, take and put back. Each slot holds a parked `Session` (targets at 1x1, D151). |
| `show::resolve` | Scene references to folders (exact library names, paths beside the playlist); control overrides to `ControlBank` writes, with a reason for each refusal. |
| `show::start` | Wall clock, then resume point, then the first cue. |
| `show::resume`, `show::watch` | The resume file on its own thread, every 5 s; the file watcher (a `stat` a second). |
| `show::view`, `show::generate` | Timeline rows for the LIST pane; the Generate form and the library snapshot. |
| `ui::playlist` | The LIST pane. |

Where a scene is at any moment: the cache (parked, 1x1), `Stack::session` (the
clock's current cue, full size), `Show::outgoing` (during a transition, full
size) or `Show::warming` (from `WarmUp` to its cue, half each edge). A scene is
in exactly one of them. `strobe-gfx`'s `Compositor` mixes live and outgoing at
the clock's `progress`; its program is compiled once when the context opens.
`Gl::shader_compiles()` is how a test or the show itself proves nothing
compiled after the load. See D73-D77 for the clock's rules and D150-D158 for
the app side.

## Shadertoy import (`strobe-shadertoy`)

Four layers, kept apart (AGENT_RESEARCH section 9). Only (a), (b), (c) and
the CLI exist; acquisition is a later layer that ends in the same JSON.

```
(a) id        ShaderId::parse        URL / embed link / bare id          id.rs
(b) model     Shader::from_json      API {"Shader":..} or bare export     model.rs
    [ acquisition: REST API (path B), localhost listener (path A) ]       not built
(c) convert   convert(&Shader, &Options) -> Converted                    convert/
      plan      order buffers A..D, bind channels, decide the clock       plan.rs
      dialect   token-level rewrite of each tab's code (D94)              dialect.rs, glsl.rs
      shim      per-pass GLSL: shim + Common + tab + entry (D91)          shim.rs
      scene_json  scene.json from Serialize structs (deterministic)       scene_json.rs
      write     hidden staging folder, one rename                         write.rs
(e) strobe-import <file.json> [--out DIR] [--name N] [--assets DIR] [--strict] [--force]
```

What it writes, and how the player runs it:

```
<name>-<id>.synScene/
  scene.json            PASSES [st_clock?, st_buf_a..d], PASS_SOURCES, IMAGES,
                        CONTROLS (the five "Strobe" controls), SOURCE
  main.glsl             the Image pass (the final PASS_SOURCES entry)
  passes/*.glsl         st_clock (when a pass reads time), buffer_a..buffer_d
  images/               fetched textures, st_black.png, pending-assets.json
```

- The player builds one program per pass from that pass's own file
  (`SceneFolder::shader_for`), names the file in compile errors
  (`file_for`), and ignores a request for a shared program (D90).
- `st_clock` is an ordinary one-pixel float pass; nothing in the frame loop
  knows about it (D91).
- Audio inputs sample `strobe_ShadertoyAudio`, filled inside
  `upload_audio` only when a pass samples it; `iDate` is `strobe_Date`
  (D93).
- Every fixture under `crates/strobe-shadertoy/tests/fixtures/` is original
  and in exact export shape; the render tests convert, write, load and
  render them on both dialects.

## Auto-generated controls (`strobe-autocontrol`)

Pure analysis, a rewrite that keeps line numbers, and one stage that needs a
GL context. See D123-D126 for the rules. The library takes the context from
its caller (`generate(&Gl, &Caps, ..)`); the CLI gets one from
`strobe_gfx::offscreen`, headless or a hidden window (D140).

```
Sources (per-pass GLSL)          source.rs     Scan: tokens + offsets + depth;
   │                                           Shadertoy shim/entry excluded
   ▼
analyze()  -> Candidate[]        analyze/      consts (#define/const), expr
   │                                           (time, loops, colour, scale/freq)
   ▼ rewrite::apply (ALL as uniforms, placeholder names)
   ▼ score::Rig::new  (compile once)
   │   per probe: reset history, fresh ControlBank, 10 frames @160x90,
   │   synthetic audio; score = mean |diff| to baseline, nudged down and up
   ▼
dead / black / does-not-compile dropped, top K kept   generate.rs
   ▼ naming::assign, rewrite::apply (only the kept), verify compile + fidelity
Generated { report, sources, control_lines, folder }
   ▼ output::write_scene           <name>-auto.synScene (copy, never in place)
```

- `Rig` compiles the scene once; probes only change uniform values.
- The original scene is compiled and rendered first, so a broken scene is
  reported as such, and the finished scene's default picture is compared with
  the original (`Report::fidelity`).
- The app is expected to call `generate` with its own `Gl` and `Caps`
  (the Import and Inspector review panel is not built).
