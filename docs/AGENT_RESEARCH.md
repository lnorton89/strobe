# Agent Research and Integration Guide

_Last researched: 2026-10-08_

This document is the implementation-oriented research cache for future Strobe agents. It is intentionally more concrete than the roadmap: it records verified upstream behaviour, the contracts already present in the code, integration traps discovered by reading the current branch, and acceptance tests that prevent agents from rediscovering the same facts.

The current branch at the time of this research was `66a96767263ab780b1af55467881de106c5e1ed1`.

## 1. Current ground truth

Strobe is no longer at the "design a renderer" stage. The core scene, shader assembly, GL resource model, headless rendering, benchmark harness, ten-scene benchmark library, and strict repository conventions already exist.

The durable architecture is:

```text
                    ┌──────────────────┐
                    │   strobe-app     │
                    │ windows + event  │
                    │ loop + UI + I/O  │
                    └────────┬─────────┘
                             │
              ┌──────────────┼───────────────┐
              │              │               │
       strobe-script   strobe-audio   strobe-playlist
              │              │               │
              └──────────────┼───────────────┘
                             │ values/messages
                      ┌──────▼──────┐
                      │ strobe-gfx  │
                      │ ScenePlayer │
                      └──────┬──────┘
                             │
                      ┌──────▼──────┐
                      │strobe-scene │
                      │ SSF model + │
                      │ assembler   │
                      └──────┬──────┘
                             │
                      strobe-ssf-lib
```

The most important invariant is still the one in `docs/ARCHITECTURE.md`: portable crates produce values, and `strobe-gfx` renders them. Do not make audio, JavaScript, playlists, MIDI or OSC depend on the GL crate.

### What is actually implemented now

- `strobe-scene`: tolerant `scene.json` parsing, validation, controls, frame-rate-independent smoothing, SSF shader assembly.
- `strobe-ssf-lib`: documented SSF helper library.
- `strobe-gfx`: headless EGL contexts, safe GL wrapper boundary, multipass renderer, ping-pong targets, `syn_FinalPass`, image textures, program/link diagnostics.
- `strobe-bench`: deterministic headless benchmark and visual output.
- ten original benchmark scenes.
- adaptive-scale arithmetic exists, but it is not yet connected to GPU timer queries or the windowed player.
- ANGLE acquisition is now pinned to Electron 38.2.0 and hash-checked.

The missing product spine is still the runtime: windowed contexts, UI, script execution, real audio, input routing, playlists, Shadertoy import, transitions and persistence.

## 2. Read these contracts before coding

### 2.1 Frame data contract

`strobe-gfx::FrameInputs` is already the boundary between the runtime and the renderer:

- `TIME`
- `FRAMECOUNT`
- mouse state
- audio scalar array in `strobe_scene::audio::SCALARS` order
- `ControlBank`
- transition values

Do not add a second parallel representation of those values in the app.

`SCALARS` is the authoritative audio/script naming list. It currently contains 39 scalar uniforms, including levels, hits, time clocks, presence, beat state, BPM state and long-form values. An audio agent should write into that array order, not build a separate enum-to-name map that can drift.

### 2.2 Control contract

`ControlBank` already handles:

- normalized and raw writes
- per-component writes
- toggle press semantics
- bang press counting
- smooth and speed modifiers
- counter bangs
- reset
- case/space/`_`/`-` insensitive name lookup

It explicitly avoids per-frame allocation in `tick`.

The UI, JavaScript host, MIDI and OSC should all target the same control bank. They should not independently implement smoothing or toggle/bang semantics.

### 2.3 Scene contract

`SceneDef` is already validated before it reaches GL.

Important existing behaviour:

- the final output pass is implicit and is always appended;
- listed pass sizes are stored as fractions of design resolution;
- `FLOAT` defaults to true;
- `DOUBLE_BUFFER` defaults to false;
- image/cubemap sampler names are validated along with controls and pass targets;
- normalized name collisions are rejected;
- scene metadata is intentionally tolerant of unknown JSON keys.

Do not re-parse scene JSON in app, playlist, script or importer code.

### 2.4 Renderer contract

The renderer currently:

1. compiles all programs at scene load;
2. resolves uniform locations and sampler units at link time;
3. allocates targets at scene load;
4. renders every pass without per-frame driver queries;
5. uses one program per pass by default;
6. uses a shared program with a `PASSINDEX` uniform only in benchmark compatibility mode;
7. makes the final pass implicit;
8. reserves sampler units even if the driver optimizes a sampler out.

The renderer also currently has real limitations that future agents must not silently paper over:

- `syn_Spectrum` and `syn_LevelTrail` are placeholder textures;
- `syn_Media` is a black texture;
- cubemaps are only declared/reserved, not loaded;
- `ScenePlayer::resize` reallocates all targets and resets feedback to black;
- shader-created uniforms are assembled through `Request::extra`, but `ScenePlayer::new` currently passes an empty extra list and does not cache dynamic uniform locations.

These are integration points, not "minor TODOs."

## 3. JavaScript: what the host must actually emulate

The goal is compatibility with the Synesthesia scene scripting API, not a generic embedded JavaScript engine.

The official Synesthesia scripting documentation establishes:

- `setup()` runs once at scene load.
- `update(dt)` runs every frame.
- input uniforms are exposed to scripts as globals.
- scripts can call `setUniform`.
- `setUniform` accepts one to four numeric components, arrays, or objects.
- the output uniform is created automatically from the value arity.
- event handlers are registered from setup.
- control/event target names use the same case/space-insensitive matching rules as the scene controls.
- the legacy `inputs` and `uniforms` object APIs still exist and should be treated as compatibility surface, not silently omitted.

A subtle but important boundary: Synesthesia's JavaScript input globals are not pixel shader globals. The official scripting docs explicitly distinguish script-visible inputs from shader-only values such as `_uvc`, `PASSINDEX`, and `RENDERSIZE`. Do not expose shader implementation details merely because they exist in the fragment preamble.

### Required design

Use a persistent QuickJS runtime per active scene instance.

The runtime should own:

- `setup` function
- `update` function
- event registrations
- persistent JS values
- dynamic output uniform registry
- failure/fault state

The runtime should emit a small, typed host-side command/value buffer. The renderer should not call into QuickJS itself.

A good per-frame ordering is:

```text
snapshot inputs
→ invoke JS event handlers generated since previous frame
→ invoke update(dt)
→ apply returned/setUniform/control operations
→ tick ControlBank
→ render
```

Whether control writes happen before or after `update(dt)` must be fixed and covered by tests. The current architecture document places JS before control ticking; preserve that unless real Synesthesia evidence forces a change.

### Dynamic uniform trap

`strobe-scene::assemble::Request::extra` already has an `ExtraType` enum for float/vec2/vec3/vec4. That is a useful start, but it is not a runtime implementation.

The missing pieces are:

1. collect dynamic uniform declarations before program compilation;
2. assign stable types by the first successful declaration;
3. reject or diagnose a later write that changes a uniform's arity/type;
4. compile programs with the complete extra list;
5. resolve and cache locations during link;
6. upload dynamic values each frame without rebuilding shader programs.

Do not compile shaders every time a script creates a new uniform during playback. The intended architecture is "scene/script preparation first, zero shader compilation in the show."

### QuickJS safety

Current `rquickjs 0.14` exposes:

- `Runtime::set_interrupt_handler`
- `Runtime::set_memory_limit`
- `Runtime::set_max_stack_size`

The interrupt callback is invoked regularly while the interpreter is executing. Returning `true` raises an uncatchable exception and returns control to the host.

Use the interrupt handler as a watchdog, not as a promise of hard real-time execution. Record the frame deadline in host state and have the handler perform a cheap elapsed-time check. A script that overruns should be faulted for that frame and should not prevent rendering the scene.

Set explicit memory and stack limits. The current runtime defaults have changed over time, and explicit limits make failure predictable across platforms.

The runtime itself should stay on the render/event thread unless the chosen rquickjs feature set and ownership model deliberately make it movable. The event loop is not generally a worker object, and the GL context is intentionally thread-bound.

Source: https://docs.rs/rquickjs/latest/rquickjs/struct.Runtime.html

## 4. Audio: build it as a streaming system, not an FFT demo

The planned audio pipeline is sensible, but the critical requirement is that the capture callback never performs expensive analysis or allocates.

CPAL documents that audio callbacks are run by a dedicated high-priority thread. Treat the callback as a transport stage:

```text
audio callback
    ↓
small lock-free/ring buffer
    ↓
analysis worker
    ↓
stable AudioSnapshot
    ↓
render thread copies the latest snapshot
```

For the current Windows-first target, WASAPI is the primary backend through CPAL.

Current CPAL documentation identifies WASAPI as Windows' standard host API. The Windows stack also supports using an output device for capture/loopback. The `wasapi` crate exposes richer loopback and device-notification primitives if CPAL alone proves insufficient.

Do not make the renderer wait for the audio analyzer. A stale snapshot for one frame is preferable to blocking a frame.

### FFT implementation

The planned 2048-sample Hann window / 512-hop real FFT is a good starting point.

Current `realfft` 3.5.0 documents:

- reuse of `RealFftPlanner`;
- reusable input/output vectors;
- `process_with_scratch` specifically to avoid allocating scratch space on every transform.

Build the planner and all FFT/scratch buffers once per analysis worker.

Source: https://docs.rs/realfft/latest/realfft/

### Audio clock contract

The renderer should receive already-derived scalar values. Do not expose mutable analyzer internals to shaders.

The analyzer should own:

- ring-buffer indexing
- FFT
- windowing
- band integration
- running normalization
- hit envelopes
- presence followers
- onset detection
- BPM estimate/confidence
- beat phase
- the final `SCALARS` snapshot
- spectrum/history texture upload data

The render thread should only consume an immutable snapshot and upload/copy it.

### Device changes

CPAL explicitly warns that devices can become invalid when disconnected. The implementation needs a state machine:

```text
Running
  ↓ device error
Recovering
  ↓ enumerate + select
Running
  ↓ no device
SilentFallback
  ↓ device appears
Recovering
```

Never allow an input-device failure to terminate the render loop.

Source: https://docs.rs/cpal/latest/cpal/traits/trait.DeviceTrait.html

For Windows-specific loopback/device-notification problems, the current `wasapi` crate supports loopback capture and device/default-device notifications. Keep it as the escape hatch rather than coupling the entire application to WASAPI.

Source: https://docs.rs/wasapi/latest/wasapi/

## 5. Spectrum textures: do not guess the layout indefinitely

The code already declares `syn_Spectrum` and `syn_LevelTrail` in the shader preamble. On desktop they are `sampler1D`; on GLES they are a one-texel-high 2D fallback because GLSL ES 3.0 has no `sampler1D`.

The remaining real compatibility question is the exact data layout and coordinate semantics used by actual Synesthesia scenes.

Synesthesia 1.25.4's shipped scenes settle part of it: every call samples with a `float` coordinate (R10), and `audio_indicators` fetches the raw channel by `int(x*1024)`, so the spectrum is 1024 texels wide (D269). The frequency axes and the trail's direction are still open (Q10).

Until a real fixture establishes that layout:

- keep the abstraction in `strobe-scene::audio`;
- keep texture allocation/upload in `strobe-gfx`;
- make the texture width/layout configurable;
- do not spread texture-coordinate assumptions through scene, audio and app code.

The best test fixture is a real licensed/owner-supplied `.synScene` that samples the textures in known ways.

## 6. Windowing and the two-surface GL design

The planned "one context, two surfaces" model is technically sound on the current stack.

Current glutin/WGL supports making a context current against a surface. That means the app can retain one GL context and two window surfaces and switch the current surface before rendering each window.

Source: https://docs.rs/crate/glutin/latest/source/src/api/wgl/context.rs

The important consequence is that `Gl` must not carry a surface identity. The existing design already gets this right.

### Suggested frame ownership

Keep all window/context/ScenePlayer work on the event-loop thread:

```text
EventLoop thread
  ├─ input events
  ├─ audio snapshot read
  ├─ JS update
  ├─ playlist tick
  ├─ ScenePlayer render
  ├─ switch to control surface
  ├─ egui paint
  ├─ switch to output surface
  └─ output present
```

Worker threads may prepare data, but they should send data/messages into this thread. They should never touch the current GL context or Winit windows.

### Winit API

Current Winit 0.30 recommends the `ApplicationHandler` + `EventLoop::run_app` model. Window creation belongs in `resumed`, and redraws should be requested rather than implementing an uncontrolled busy loop.

Useful lifecycle facts:

- create windows during `resumed`;
- process input through `window_event`;
- request redraws from `about_to_wait`;
- render on `RedrawRequested`.

Sources:
- https://docs.rs/winit/latest/winit/changelog/v0_30/index.html
- https://docs.rs/crate/winit/latest/source/src/changelog/v0.30.md

The app agent should not resurrect the deprecated `EventLoop::run` architecture from older examples.

### egui/GL state

`ScenePlayer::render` deliberately establishes its own GL state before rendering. egui_glow is another GL consumer.

Render UI after the scene and treat the boundary between them as explicit state ownership. Do not rely on an accidental set of bindings surviving an egui paint. If an operation requires a known framebuffer/viewport/program/VAO state, set it explicitly at that boundary.

## 7. GPU timing and adaptive resolution

The repository already contains `AdaptiveScale`, but it currently receives an abstract millisecond value. It needs a real GPU timing source before it should drive production resolution changes.

OpenGL 3.3 exposes timer queries through `GL_TIME_ELAPSED`/timer-query functionality. Query results are GPU timestamps in nanoseconds.

Source: https://registry.khronos.org/OpenGL/extensions/ARB/ARB_timer_query.txt

### Do not block the render thread for the current query

The implementation should use delayed/ring-buffered query results:

```text
frame N: begin query
frame N: render
frame N: end query

frame N+1 or later:
  check previous query availability
  if available -> feed AdaptiveScale
  if unavailable -> skip sample
```

Do not call a blocking result read on the query just issued. The whole point is to avoid converting GPU timing into a CPU/GPU synchronization point.

### Existing adaptive controller implications

`AdaptiveScale` changes are quantized to 0.05 and its own comments correctly recognize that resizing reallocates targets.

But `ScenePlayer::resize` currently discards all feedback history. A resolution change therefore resets:

- all `DOUBLE_BUFFER` pass contents;
- the implicit final-pass history;
- simulation state in reaction-diffusion/feedback scenes.

This is not necessarily wrong, but it is visually significant. Adaptive resolution should therefore be:

- hysteretic, as the current controller is;
- disabled or rate-limited during critical transitions;
- observable in the diagnostics UI;
- tested against at least one feedback scene.

The current controller's rules are reasonable starting values, but they must be measured on real hardware.

### Critical benchmark correction

The headless benchmark currently calls `glFinish()` around each timed frame. That makes the benchmark useful for deterministic "how expensive was the whole frame" comparisons, but it is not representative of the asynchronous runtime.

Keep the existing benchmark. Add a separate real-window timing path for production adaptation rather than changing the current benchmark's semantics.

## 8. ANGLE: the existing pin is intentional

The repository pins Electron 38.2.0 as a source of `libEGL.dll`, `libGLESv2.dll`, and `d3dcompiler_47.dll`.

Do not casually upgrade the Electron source archive.

Electron 44, released in August 2026, changed ANGLE packaging: ANGLE is statically linked and those DLLs are no longer shipped. Therefore a newer Electron archive is not a drop-in replacement for the existing acquisition script.

Sources:
- https://www.electronjs.org/blog/electron-44-0
- https://www.electronjs.org/docs/latest/breaking-changes/

The current hash-pinned 38.2.0 arrangement should remain until a different ANGLE distribution is selected and its licensing/binary provenance is deliberately reviewed.

## 9. Shadertoy: separate acquisition from conversion

The importer should not conflate "download JSON" with "convert shader."

The Shadertoy export model is:

```text
Shader
 ├─ info
 └─ renderpass[]
      ├─ type
      ├─ code
      ├─ inputs[]
      │    ├─ type
      │    ├─ filepath
      │    ├─ channel
      │    └─ sampler
      └─ outputs[]
```

Public API access is not equivalent to web-site visibility. Current independent API clients document that the official API requires an API key and only exposes shaders that are made available through the API/privacy mechanism.

The most reliable REST path used by current community tooling is:

```text
GET https://www.shadertoy.com/api/v1/shaders/<ID>?key=<API_KEY>
```

Sources:
- https://github.com/dfranx/PluginShadertoy/blob/master/Shadertoy.cpp
- https://github.com/ivoleitao/shadertoy_api
- https://github.com/dwgx/ShadertoyTool/blob/master/README.md

### Importer layers

Implement the importer as four separable layers:

1. URL/ID parser.
2. acquisition client.
3. asset fetch/cache.
4. deterministic converter into Strobe scene files.

That separation makes the following possible without changing conversion code:

- API v1 acquisition;
- userscript/browser acquisition;
- pasted/exported JSON;
- an offline fixture directory.

### Asset and pass mapping

Do not flatten the renderpass array into one shader blob too early.

Preserve:

- original pass order;
- buffer/image/common pass type;
- channel numbers;
- filter/wrap/vflip/sRGB/internal hints;
- source URLs;
- original shader ID/author metadata.

The converter should then map these into Strobe's existing pass/image abstractions.

Current Shadertoy-compatible libraries confirm that exported JSON uses `info` + `renderpass`, and each pass can contain typed input channels and sampler settings.

Source: https://github.com/reindernijhoff/shadertoy-package

### Licensing

The importer should preserve author/source/license metadata even when the source does not declare a formal SPDX value. Never treat a successful download as permission for commercial use.

The repository's existing licensing warning is correct and should be preserved.

## 10. Shadertoy-to-SSF dialect work needs its own stage

The project currently has a strong reason for native GLSL: real Shadertoy and SSF code depends on constructs that are awkward for SPIR-V/naga conversion.

For the ANGLE path, however, the project still needs source-level compatibility handling.

Do not scatter one-off substitutions through the importer.

Create a deterministic "dialect normalization" stage:

```text
source
  ↓
Shadertoy canonical source
  ↓
Strobe semantic rewrite
  ↓
target dialect normalization
  ↓
assembled Strobe GLSL
```

The rewrite stage should have fixtures for:

- `PASSINDEX == 0.0`;
- integer-to-float assignment;
- `texture2D`;
- Shadertoy `i*` uniforms mapped to Strobe equivalents;
- buffer channels;
- common pass code;
- sampler declarations;
- precision qualifiers.

Do not claim ANGLE compatibility based only on the ten original scenes. Those were deliberately written to be ES-clean.

## 11. Playlist architecture: keep scheduling pure

The playlist system should be split into:

- data model;
- inheritance resolution;
- deterministic generator;
- timeline calculation;
- playback state machine;
- persistence.

The generator should be pure with respect to a seed. Given the same library snapshot, tag pool, duration rules, transition rules, seed and start time, it should produce the same cue sequence.

This is important because the UI needs to preview a generated section before committing it.

### Playback state

Treat wall-clock scheduling and scene-time playback as separate clocks.

A cue should be able to answer:

- which scene is active;
- elapsed cue time;
- next transition deadline;
- whether playback is held;
- whether a late wall-clock start should jump forward.

Do not make wall-clock scheduling mutate the underlying playlist representation.

### Precompile guarantee

The roadmap explicitly promises zero shader compilation during playback.

That means precompile must happen after all scene/script-generated uniforms are known. This is another reason dynamic script uniform discovery cannot be a lazy "compile on first use" feature.

## 12. Diagnostics: make failure actionable

The renderer already maps compiler logs back to `main.glsl:N`. Extend that philosophy through the application.

Every user-visible failure should carry:

- subsystem;
- scene or asset;
- path/URL if applicable;
- phase;
- root error;
- recovery action.

Examples:

```text
SHADER
scene=09-sdf-cathedral
phase=link
file=main.glsl
line=142
message=...
```

```text
AUDIO
device=Default Output
phase=capture
message=device disconnected
action=retrying in 500ms
```

```text
IMPORT
shader=4dX...
phase=asset
url=...
message=HTTP 403
action=left in needs-work
```

Do not let diagnostics mutate the renderer's hot path. Use a bounded log channel/ring and render the newest entries in egui.

## 13. Repository/CI findings

There is one current repository-level inconsistency future agents should know about:

- the current workflow's push trigger lists `main`;
- the only branch currently present is `claude/festive-ramanujan-oic192`;
- the Windows job attempts to build `strobe-app`, while that crate is still an in-flight branch.

A separate repo/CI agent is already assigned this area. This guide does not modify the workflow to avoid stepping on that work.

The correct long-term target is not merely "green CI." It is:

1. every intended trigger actually runs on the branch model being used;
2. Windows checks compile the shipping app;
3. Linux checks compile both desktop and GLES shader dialects;
4. the headless renderer remains required in CI;
5. cargo-deny and format/lint remain enforced;
6. benchmark artifacts are preserved from CI for performance regressions.

## 14. Integration order that minimizes rework

The work should converge in this order:

### A. Finish the app/context shell

Because every later feature needs a real event/render loop.

Acceptance:

- native GL context opens;
- if native creation fails, ANGLE is attempted;
- one context can render to both UI and output surfaces;
- fullscreen output can be selected independently of UI window;
- resize events update the renderer;
- no GL calls originate on worker threads.

### B. Finish diagnostics

Do this before script/audio/import because those systems will otherwise produce opaque failures.

Acceptance:

- shader errors point to source;
- context creation errors identify native vs ANGLE path;
- audio disconnect is non-fatal;
- import failure identifies network/asset/conversion phase.

### C. Finish JS

Do this before playlist precompile and presets because script-created state is part of scene identity.

Acceptance:

- setup/update ordering;
- event handlers;
- controls API;
- setUniform arities;
- legacy `inputs`/`uniforms`;
- interrupt deadline;
- memory limit;
- stack limit;
- deterministic fault behaviour.

### D. Finish audio

Do this before final playlist beat-locking and audio-reactive acceptance tests.

Acceptance:

- live capture;
- offline WAV calibration harness;
- no analysis allocations on callback;
- device recovery;
- stable `SCALARS` snapshot;
- spectrum/level texture upload.

### E. Finish importer

Use recorded fixtures rather than depending on Shadertoy availability during CI.

Acceptance:

- valid exported JSON imports without network;
- multipass survives round trip;
- texture/sampler metadata survives;
- inaccessible API result produces actionable needs-work output;
- license/source metadata is retained.

### F. Finish playlist

Only after scenes, scripts and audio have stable runtime contracts.

### G. Wire adaptive performance

Only after real hardware results exist.

## 15. Tests future agents should add

### Script conformance

One fixture scene per behaviour:

- `setup` only runs once;
- `update(dt)` receives the frame delta;
- `setUniform("x", 1)` creates float;
- `setUniform("x", [1,2])` creates vec2;
- a later type mismatch is diagnosed;
- 10k control writes remain below the intended budget;
- infinite loop is interrupted;
- memory limit is enforced;
- stack recursion is bounded;
- handler exceptions fault the script but keep the scene rendering.

### Audio

- synthetic sine waves produce predictable band energy;
- 120 BPM click track produces stable BPM;
- silence yields `syn_BPM=120` and zero confidence/energy;
- device failure produces silent fallback, not render failure;
- analyzer latency is bounded;
- FFT buffers do not allocate on the callback path.

### Renderer

- resize resets feedback only when deliberately requested;
- timer queries do not block on the current frame;
- adaptive scale respects min/max and cooldown;
- output remains renderable while a query sample is unavailable.

### Import

Fixtures should include:

- one-pass image shader;
- one buffer;
- multiple buffers;
- common code;
- texture input;
- cubemap input;
- audio input;
- non-default sampler options;
- an inaccessible/private shader response;
- malformed JSON.

No CI test should require the live Shadertoy website.

### App

Under Xvfb/software GL:

- both windows create;
- one frame renders;
- egui input arrives;
- output surface can be resized;
- closing one window shuts down cleanly;
- context provider selection appears in diagnostics.

## 16. Work separation for the current parallel agents

The current roadmap lists these parallel efforts:

- `agent/strobe-script`
- `agent/strobe-audio`
- `agent/strobe-app`
- `agent/strobe-diag`
- `agent/repo-ci`
- a real T480 hardware test session

Keep this guide as a coordination document, not as a reason to edit their source branches.

In particular:

- app agent owns window/context/event-loop integration;
- diagnostics agent owns logging/crash reporting;
- audio agent owns capture and analysis;
- script agent owns QuickJS and Synesthesia JS compatibility;
- repo/CI agent owns workflow/release/automation alignment;
- hardware session owns real-device measurements.

The first integration pass should combine interfaces, not duplicate implementations.

## 17. Remaining questions that actually need real evidence

These are the few areas where documentation alone is insufficient:

1. Exact `syn_Spectrum` and `syn_LevelTrail` texture semantics in real Synesthesia scenes: the sampler type and spectrum width are settled (R10, D269); the frequency axes and the trail's direction are not (Q10).
2. Real-world ANGLE compatibility rate for community SSF scenes containing desktop-only GLSL idioms.
3. Whether Synesthesia's event/control write ordering has any observable frame-level edge cases.
4. The visual effect of adaptive target reallocation on long-running feedback scenes.
5. Actual T480/native-WGL and ANGLE performance characteristics.
6. Which Shadertoy acquisition path is most robust in the owner's real browser environment.

Everything else in this document should be treated as an implementation constraint unless later evidence overturns it.

## 18. External research sources

Primary/technical sources consulted:

- Synesthesia scripting documentation: https://app.synesthesia.live/docs/ssf/script.html
- Synesthesia standard uniforms: https://app.synesthesia.live/docs/ssf/standard_uniforms.html
- Synesthesia audio uniforms: https://app.synesthesia.live/docs/ssf/audio_uniforms.html
- rquickjs Runtime: https://docs.rs/rquickjs/latest/rquickjs/struct.Runtime.html
- CPAL: https://docs.rs/cpal/latest/cpal/
- CPAL DeviceTrait: https://docs.rs/cpal/latest/cpal/traits/trait.DeviceTrait.html
- WASAPI crate: https://docs.rs/wasapi/latest/wasapi/
- realfft: https://docs.rs/realfft/latest/realfft/
- Winit 0.30 lifecycle: https://docs.rs/winit/latest/winit/changelog/v0_30/index.html
- glutin WGL context implementation: https://docs.rs/crate/glutin/latest/source/src/api/wgl/context.rs
- Khronos timer query specification: https://registry.khronos.org/OpenGL/extensions/ARB/ARB_timer_query.txt
- Electron 44 release notes: https://www.electronjs.org/blog/electron-44-0
- Electron breaking changes: https://www.electronjs.org/docs/latest/breaking-changes/
- Shadertoy API example client: https://github.com/dfranx/PluginShadertoy/blob/master/Shadertoy.cpp
- Shadertoy API client model: https://github.com/ivoleitao/shadertoy_api
- Shadertoy acquisition fallback tooling: https://github.com/dwgx/ShadertoyTool/blob/master/README.md
- Shadertoy export-format renderer: https://github.com/reindernijhoff/shadertoy-package

## 19. Source-of-truth rule

When future agents encounter a conflict:

1. current code and tests describe what exists;
2. `docs/DECISIONS.md` describes deliberate architectural choices;
3. `docs/OPEN_QUESTIONS.md` identifies behaviour not yet verified;
4. this document supplies implementation research and external facts;
5. `docs/PLAN.md` is historical and yields to the other documents.

When new research changes an implementation constraint, update this document and add a dated decision/open-question entry rather than letting the knowledge live only in an agent conversation.


## 20. UI research and design direction

The UI is intentionally a redesign of the Synesthesia presentation rather than a clone. Synesthesia remains the behavioural reference for Scene Controls, Meta Controls, Media, UI_GROUP, presets, FavSlots, MIDI/OSC and the integrated editor. Its current direction already includes improved filtering, panes, scene cards and scene building. The new Strobe plan is recorded in [UI_DESIGN.md](UI_DESIGN.md).

The strongest transferable patterns found in current creative tools were:

- Resolume: a Dashboard surfaces the small set of parameters a performer uses constantly, while layout presets keep advanced panes out of the way. This becomes Strobe Performance Macros. https://resolume.com/support/en/dashboard
- Ableton Live 12: searchable/tagged content browsers and independently show/hideable views keep large libraries from permanently consuming workspace. https://help.ableton.com/hc/en-us/articles/12927340213660-The-Live-12-Browser
- TouchDesigner: one consistent parameter-editor model works across different pane placements. https://derivative.ca/UserGuide/Parameter_Dialog
- Rustaman: derived theme tokens, WCAG checks, 4/8/12/20 spacing, custom chrome, geometry icons and a four-duration motion vocabulary provide the implementation baseline for Strobe's visual system.

The important product conclusion is that Strobe should optimize the Perform workspace first: live output, immediate controls, next-cue awareness, persistent Performance Macros, and health status. Library, playlist, mapping, editor, diagnostics and settings become dedicated workspaces instead of permanent stacks of panels.

The UI should share one application action boundary with the renderer/audio/script/playlist subsystems. Immediate-mode drawing emits actions; it does not perform file I/O, network requests, audio work, QuickJS calls, shader compilation or GPU readback.
