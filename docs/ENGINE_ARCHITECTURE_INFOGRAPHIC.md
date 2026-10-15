# Strobe engine architecture — technical companion

**Source:** [`lnorton89/strobe`](https://github.com/lnorton89/strobe), `master` at commit [`c7547c760579f4b4ee6557b71df44d41f53f2d7c`](https://github.com/lnorton89/strobe/tree/c7547c760579f4b4ee6557b71df44d41f53f2d7c), reviewed 2026-10-14 (America/Los_Angeles).

This document accompanies the vector infographic. It distinguishes **what happens during scene/show loading** from **what happens each output frame**. It is an architectural reading of the implementation and its documentation, not a benchmark or a claim that all features have been production-validated.

## 1. Major ownership boundaries

| Component | Responsibility | Frame-loop relationship |
|---|---|---|
| `strobe-app` | `winit` event loop; `Running`, `Session`, two-window lifecycle; controls; playlist orchestration | Coordinates the output frame and owns the GL context thread |
| `strobe-scene` + `strobe-ssf-lib` | Parse/validate `.synScene`, controls/presets/meta, build GLSL with SSF helpers and dialect rewrites | Loads upfront; `ControlBank::tick(dt)` runs on active sessions |
| `strobe-audio` | Capture and pure signal analysis (FFT, bands, onsets, tempo, texture data) | DSP runs outside the graphics thread; snapshots are read without waiting |
| `strobe-script` | QuickJS implementation of scene `script.js` and its uniform/control API | `ScriptHost` is driven by `Session` on the render thread with a per-frame interrupt budget |
| `strobe-playlist` | Pure TOML model, cue inheritance, timeline, clock, beat-aware show scheduling | Clock is ticked by app `show::run`; scene compilation is performed outside the output frame |
| `strobe-io` | MIDI/OSC workers, mapping, learn, soft takeover, feedback | Bounded events drained by app; no GL |
| `strobe-remote` | Optional HTTP/WebSocket LAN listener, pairing, permission-scoped remote commands and state snapshots | Own worker threads and a bounded bridge; no GL or UI renderer dependencies |
| `strobe-gfx` | Context providers, GLSL programs, FBO passes, compositor, post, GPU queries and adaptive scaling | Sole graphics API boundary; one context owned by the event-loop thread |
| `strobe-shadertoy` | Converts Shadertoy JSON + buffers to an ordinary scene folder with source/licence metadata | Import-time workflow, **not a distinct rendering engine** |
| `strobe-autocontrol` | Analyzes shader literals, adds uniforms and scores visible impact with offscreen rendering | Offline/worker process; **not in the per-frame render path** |
| `strobe-diag` | Structured failure records, logging, crash capture and symbolication | Observability across components; native crash-handler boundary |

## 2. The scene loading path (preparation, not every frame)

1. Read a `.synScene` folder: `scene.json`, shader sources, optional `script.js`, images and media. Shadertoy sources are converted to this same scene representation.
2. Parse the scene into `SceneDef` and create its `ControlBank` and meta bank. Assemble per-pass shader source with SSF functions and dialect-specific transformations.
3. If a script is present, run its first frame **before program compilation** to discover script-created uniforms. Include their declarations in the compiled GLSL, then seal the set of names.
4. Compile the graphics programs and allocate pass render targets, resolving uniform/sampler locations up front. By default, each pass has a specialized `PASSINDEX` constant. Ping-pong targets support double-buffered passes and previous-frame access.
5. When loading a playlist, precompile its scenes and transition shaders before the clock plays, cache parked scenes, and warm the next scene in advance. This preserves a no-mid-show-compilation invariant.

## 3. The output-frame sequence

This is the central sequence illustrated in the bottom seven-step strip.

1. **Input.** Drain up to 256 pending MIDI/OSC events and up to 8 LAN remote commands; resolve mappings, action commands and control-bank updates without blocking the frame.
2. **Clock.** Advance the playlist; process cue starts, warm-ups, transition changes, holds and blackout changes. With no playlist, the live session advances directly.
3. **Session.** For each live/outgoing/warming session: advance its clock, read audio snapshot (or synthetic stand-in), execute `script.js`, apply script writes, tick scene/meta controls.
4. **Draw.** Upload audio textures and script uniforms, pass `FrameInputs` into `ScenePlayer`, run shader passes into framebuffers, and optionally schedule asynchronous low-resolution final-pass readback for scripts that request `textureFinalPass`.
5. **Mix.** If a transition is active, composite the outgoing and incoming scene textures using the chosen cut/crossfade/through-black/custom transition. The warmer is advanced at half size, not presented as the current scene.
6. **Post/present.** Apply master/colour/mirror effects after compositing; bypass this pass entirely if every meta setting is neutral. Present letterboxed pixels (or blackout) to the output window when visible. Only the output window's swap uses vsync.
7. **Feedback.** Poll **older** GPU timer-query results without waiting. Feed `AdaptiveScale` so changes to rendering resolution apply to subsequent frames. Monitor GL reset state at most once per second on robust contexts.

The UI redraw happens separately at approximately **30 Hz**, or about 2 Hz while the fullscreen output covers the control window. Windows can stop painting a covered control window altogether, so since D290 the event loop runs those frames itself when no paint arrives. The output scene frame continues to be evaluated while the output window is hidden, so the control window preview remains usable.

## 4. Inside a shader render

- **API backend:** native OpenGL 3.3 core (GLSL `#version 330 core`) or ANGLE OpenGL ES 3.0 (`#version 300 es`, using Direct3D 11 on Windows). The scene player logic above the context is shared.
- **Pass sequence:** a scene has zero or more listed buffer passes plus an implicit final pass. A pass reading an earlier pass sees its current-frame output; reading its own pass or a later pass sees the previous-frame value.
- **Feedback:** the final pass is double-buffered so `syn_FinalPass` is the prior final frame. Listed passes can also have `DOUBLE_BUFFER` enabled.
- **Data:** scalar `syn_*` uniforms, control values, custom script uniforms, `syn_Spectrum`, `syn_LevelTrail`, Shadertoy audio texture, images, cube maps and `syn_Media` are bound based on what each program uses.
- **Post:** colour correction, master adjustment, mirrors and Limit Colors are applied as a last output-wide stage. Neutral settings require no extra pass.

## 5. Why the engine can remain responsive

- `Gl` is **not `Send`**: the event-loop thread alone owns GL use; audio/network/MIDI and file work are isolated.
- Queues are **bounded** and workers do not wait for the rendering thread; overload is handled by drop/accounting policies.
- Audio capture uses a lock-free ring and atomic frame publications; the renderer does a wait-free snapshot read.
- GPU timing uses a **four-query nonblocking ring**; rendering resolution adapts to measured GPU cost.
- On GL failures, a recovery state machine can rebuild the context and scene state, and in automatic mode fall back from native GL to ANGLE on Windows. It does not guarantee every device loss can be recovered.
- QuickJS has an interrupt budget; scenes that fail to load or compile leave the previous valid scene playing, where that fallback path applies.
- Structured logs, failure reports and minidumps support postmortem debugging.

## 6. Evidence and caveats

The repository states the project is in its **v0.1.0 test release**, not production-ready. Open validation areas include real MIDI controller hardware, Shadertoy access on the live service, additional GPU models, phones and parts of Synesthesia compatibility. The LAN controller is presently an **opt-in HTTP/WebSocket page**, not an installable PWA (per its crate README). Since this snapshot, a second Strobe can also take full desktop control of a headless show machine with a shared trust file and automatic reconnect (venue mode, D288 and D294); the diagram predates it. The diagram describes the paths implemented/documented at the cited commit and must not be interpreted as a performance guarantee or a fully validated compatibility matrix.

### Source code and documentation

- [Architecture walkthrough, including exact frame ordering](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/docs/ARCHITECTURE.md)
- [Top-level README and workspace ownership](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/README.md)
- [`Session::advance` and first-script-frame loading behavior](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-app/src/session.rs)
- [`strobe-gfx` renderer design](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-gfx/README.md)
- [`strobe-scene` data model and shader assembly](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-scene/README.md)
- [`strobe-audio` capture and DSP](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-audio/README.md)
- [`strobe-script` QuickJS integration](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-script/README.md)
- [`strobe-playlist` clock/playlist model](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-playlist/README.md)
- [`strobe-io` show-control queues](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-io/README.md)
- [`strobe-remote` LAN protocol and security](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-remote/README.md)
- [`strobe-shadertoy` conversion](https://github.com/lnorton89/strobe/blob/c7547c760579f4b4ee6557b71df44d41f53f2d7c/crates/strobe-shadertoy/README.md)

## Regenerate the PNG export

The SVG is the editable source of truth. Render it at its native **3600 × 2400** dimensions with CairoSVG:

```bash
python -m pip install cairosvg
cairosvg docs/images/strobe-engine-architecture.svg -o docs/images/strobe-engine-architecture.png
```

The raster and vector exports should be reviewed at 100% scale for label legibility and overlap.
