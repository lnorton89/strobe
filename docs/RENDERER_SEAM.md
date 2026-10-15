# The renderer seam

Backlog G-02. This page names the contract between the app and its renderer:
what a second backend would have to provide, and where the GL path leaks
past that line today. It defines the seam; it does not build one. No trait
or abstraction layer exists in the code. The GL 3.3 path and ANGLE are the
only backends (D1). The G-03 spike found no GLSL front end that could feed
another one yet (D255, `docs/research/g03-frontends/README.md`).

An interface written before a second implementation exists is guesswork,
and a guess on the frame path costs every frame. So the rule is: when a
backend is started (G-04), it is built against this list, and the list
becomes a trait then, shaped by both implementations.

## The contract, by stage

### 1. Device and context

| Today | What a backend must provide |
|---|---|
| `WindowedContext` (`gl::window`) for each window, choosing native WGL or ANGLE (`GlChoice`, `Backend`), with vsync and a swap | A surface per window, presentation with and without vsync, and the chosen backend's name for diagnostics |
| `Headless` (`gl::context`) for tools and tests, with `offscreen::run` choosing headless or a hidden window (D70, D140) | An offscreen device for `strobe-bench`, `strobe-autocontrol`, the corpus harness and the renderer tests, with no display server |
| `Caps` from the context's `Info`: dialect, float formats, timer queries, robustness, limits | The same facts, decided once at creation and never queried per frame |

### 2. Scene preparation (load time, never in a frame)

| Today | What a backend must provide |
|---|---|
| `ScenePlayer::new` / `with_extras`: `strobe_scene::assemble::fragment` per pass, compiled and linked, with uniform locations and sampler units resolved at link time | Turn the assembled GLSL into its pipeline. Today that is the driver; tomorrow it would be a GLSL front end (D255). Resolve every uniform and texture binding before the first frame. |
| Pass targets sized per D11, D191, D194, double-buffered when a pass reads itself (D203), RGBA16F or RGBA32F (`FloatPrecision`) | The same targets and formats, cleared to black on creation and on resize |
| `assets`: pictures, cubemaps, media, decoded off-thread, uploaded once, each problem reported (`take_asset_problems`) | Upload and bind textures with the scene's wrap, filter and mipmap settings (D238) |
| A compile error is a `PlayerError::Program` with the user's file and line (`describe_log`) | Errors mapped back to the user's file and line, whatever compiled them |

### 3. A frame

| Today | What a backend must provide |
|---|---|
| `render(&FrameInputs)`: time, frame, mouse, audio scalars, the control bank, transitions; one draw per pass, then the implicit final pass | The same inputs; no allocation and no driver queries on this path (CLAUDE.md) |
| `upload_audio`: spectrum, level trail, the Shadertoy audio texture | Small texture updates every frame, with no stall |
| `set_script_uniforms`, `set_date`; uniforms a script declares at load come in through `with_extras` (D161) | Uniforms set per frame from the CPU, and script-declared ones known before the first frame |
| `Post`: the meta stage (colour, mirror, limit colours), skipped when neutral (D180, D207) | A post pass that costs nothing at neutral settings |
| `Compositor`: the output, the preview and the crossfade between scenes | Blending two scenes' outputs and presenting to two windows |
| `output_texture`, `present`, `read_output` | The output as something the UI can draw (egui's painter takes a `glow::Texture` today) and read back for screenshots |

### 4. Readback

| Today | What a backend must provide |
|---|---|
| `enable_final_pass_readback`, `final_pass_pixels`, `final_pass_dropped`: a ring of PBOs and fences, two frames late, never a stall (D162, D245) | Asynchronous readback with a fixed latency, dropping a read rather than waiting for one |

### 5. Size

| Today | What a backend must provide |
|---|---|
| `resize` (history cleared) and `resize_keeping` (history scaled, for the adaptive resolution controller) | Both, without recompiling |

### 6. Diagnostics and timing

| Today | What a backend must provide |
|---|---|
| `GpuTimer`: a ring of timer queries, never waited on | GPU time per frame, read late |
| `gl::debug`: KHR_debug messages into the log (D44) | The API's validation or debug messages, in the same log |
| `Info` into every crash report (`record_gl_facts`) | Vendor, renderer, version and backend name for reports |

### 7. Losing the device

| Today | What a backend must provide |
|---|---|
| `ResetStatus`, `Gl::mark_lost`, and the app's `recovery` module: detect a reset, drop every object without a GL call, rebuild the context and reload the scene, on ANGLE if native keeps failing (D66, T480 Task 8) | Detect device loss, free nothing through the dead device, and rebuild from the scene folder, since nothing GPU-side survives |

## Where GL leaks past the seam today

A second backend would have to change these places in the app. None of
them is worth abstracting before then.

- **`&Gl` is passed into the app** in `session.rs`, `show/run.rs`,
  `corpus/run.rs`, `app.rs` and `gldebug.rs`, mostly to hand it back to
  `strobe-gfx` constructors. A backend handle would replace it one to one.
- **`ResetStatus`** is read by `app.rs`, `recovery.rs` and `failures.rs`.
  It would become a backend-neutral "device lost" signal.
- **egui is drawn with `egui_glow`** (`ui/host.rs`), and previews are
  `glow::Texture` ids. A second backend needs its own egui painter, or to
  hand the UI a texture it can draw.
- **The dialect is GLSL.** `Caps::dialect` picks between desktop 3.30 and
  ES 3.00. A non-GL backend would add a step from GLSL to its own
  language. D255 says glslang and SPIRV-Cross would be the likely
  candidates, and naga is ruled out.

## Acceptance (G-02)

- The GL path is unchanged by this page. No code moved, so behaviour and
  `BENCHMARKS.md` baselines are untouched.
- Nothing on the frame path allocates or reads back synchronously because
  of it.
- When G-04 starts, this list is the checklist for the first vertical
  slice, and the slice's two implementations decide the trait.
