# strobe-gfx

The GL renderer, and the only crate in the workspace (with one file in
`strobe-diag`) that contains `unsafe`. It creates GL contexts (native OpenGL
3.3 core, ANGLE/GLES 3.0 through EGL, or a headless EGL device), wraps every
GL call in a safe function, and runs a loaded scene: it compiles one program
per pass, owns the render targets, renders every pass each frame, and blits
the result to a window. It also measures GPU time and turns that into an
adaptive render scale.

## Role in the workspace

- **Depends on:** `strobe-scene` (what to render, and the shader text),
  `glow` (bindings), `glutin` / `glutin-winit` / `winit` /
  `raw-window-handle` (context and window creation), `image` (scene image
  files), `log`, `thiserror`. It uses only the `log` facade, so it does not
  depend on `strobe-diag`.
- **Depended on by:** `strobe-app`, `strobe-bench` and `strobe-autocontrol`.
- **Does not:** know about audio capture, scripts or playlists. It renders a
  scene given a `FrameInputs` (time, audio scalars, a `ControlBank`, mouse)
  and, optionally, `AudioTextures` (plain `[f32; 4]` slices) and script
  uniform values. Producing those values is the app's job.

## The unsafe boundary

This rule comes from [`CLAUDE.md`](../../CLAUDE.md) and is checked by
`strobe-conventions`:

- `unsafe` lives only in `crates/strobe-gfx/src/gl/` (the one other
  permitted file is `crates/strobe-diag/src/native.rs`).
- One named, private, **safe** function per unsafe operation. The `unsafe`
  block holds that one call and nothing else; argument checks and
  conversions happen outside it.
- Every block carries a `// SAFETY:` comment stating the real reasoning. For
  GL calls that is the current-context invariant (`Gl` is built only by
  `gl::context` / `gl::window`, right after making a context current, and
  `Gl` is `!Send`) plus, for pointer-taking calls, the checked slice length.
  Clippy's `undocumented_unsafe_blocks` enforces the comment.
- Every GL object has an owning type with a `Drop` (`Program`, `Texture`,
  `Framebuffer`, `VertexArray`, `Shader`, `TimerQuery`). A lost context sets
  a shared flag so these skip their deletes (D66).
- No `pub unsafe fn`, and no `unsafe` in tests.
- Need a new GL call? Add a leaf wrapper in `src/gl/mod.rs` next to its
  siblings. Do not call `glow` methods from anywhere else.

Per-frame paths do not allocate or ask the driver: uniform locations and
sampler units are resolved when a program links, and capabilities are decided
once in `Caps`. The one deliberate exception is `GpuTimer`, whose whole job
is a non-blocking timer-query read (D64).

## Main types / entry points

| Item | What |
|---|---|
| `gl::Gl` | The safe GL handle: one wrapper per call, `!Send`, cloneable. Also `reset_status()` and `mark_lost()` for context-loss handling. |
| `Headless`, `Api` | A surfaceless EGL device context (`Api::DesktopCore33` or `Api::Gles30`). Used by tests, CI and the bench on Linux with Mesa. |
| `WindowedContext`, `SwapWindow` | One GL context drawing to several winit windows (the app's control and output windows). `make_current`, `swap_buffers`, `set_vsync`, `resize`. |
| `Backend`, `GlChoice` | `Native` or `Angle`; `GlChoice::Auto` tries native then ANGLE, `Only(backend)` forces one. `WindowedContext::robust()` says whether a robust context was granted. |
| `offscreen::run`, `Request`, `ContextMode` | A context for work that draws nothing: an EGL device if the machine has one, else one hidden window (hidden, then tiny off-screen). Calls a job with `&Gl`, `&Caps` and a description and returns its value. Main thread, once per process. Used by `strobe-bench` and `strobe-autocontrol` (D140). |
| `Caps` | Dialect, 1D-texture support and float-render support, decided once per context. `FloatPrecision` (`Half` default, `Full`). |
| `ScenePlayer` | `new` / `with_extras` compile a `SceneFolder`; `render(&FrameInputs)`, `upload_audio`, `set_script_uniforms`, `resize`, `present`, `read_output`, `enable_final_pass_readback` / `final_pass_pixels`. |
| `Post`, `PostError` (`post`) | The output post stage (D180): master, invert, brightness, gamma, contrast, hue, saturation and the two mirrors in **one program compiled once**, applied to the finished frame (after the compositor). `apply(&mut self, src, &MetaValues)` does **nothing** and returns `false` when every value is neutral; otherwise one full-screen pass into its own target (`output_texture`, `present`, `read_output`). `render` always runs the program (tests). `color_matrix(hue, saturation)` is the CPU-built 3x3, exactly the identity at neutral. |
| `assets` (`AssetProblem`, `MediaRequest`) | CPU-side decoding with no GL: cube map faces from six files or a strip/cross picture (`load_cubemap`, D185), and the `syn_Media` decoder thread (`spawn_decoder`, D186). `ScenePlayer::take_asset_problems()` hands back what failed to load so the host can report it; the scene runs on with a black cube or black media. |
| `ScenePlayer::set_media`, `finish_media_loading` | Show a user-chosen picture or folder behind `syn_Media` (decoded off the frame thread, one upload per frame at most, swapped in when its first picture lands); `finish_media_loading` blocks until done, for tests and the bench only. |
| `readback::FinalPassReader` | The asynchronous 64x36 final-pass readback behind `textureFinalPass` (D162): a chain of 2:1 blits, a ring of three pixel-pack buffers with fences, the oldest collected only when already signalled. Two frames late, never a stall, nothing queried with `glGet*`. Off unless asked for. |
| `gl::Buffer`, `gl::Fence` | Owning wrappers for pixel-pack buffers and sync objects; `Drop` skips the delete on a lost context. `Gl::shader_compiles()` counts compiles, so a test can prove none happen mid-show (D161). |
| `Options` | `specialise_passes` (default true) and `float_precision`. |
| `FrameInputs`, `Mouse`, `AudioTextures` | The per-frame values the player is handed. |
| `GpuTimer` | A four-slot ring of `GL_TIME_ELAPSED` queries read without ever waiting. |
| `adaptive::AdaptiveScale` | Pure controller: GPU milliseconds in, render scale out. |
| `gl::debug` | `enable_debug_output(&mut Gl, DebugLevel)` routes `KHR_debug` messages to the `log` target `gl_driver`. |

## Usage

Render one frame of a library scene on a headless context and read it back
(this compiles against the current API and runs wherever `Headless::new`
finds an EGL device):

```rust
use std::path::Path;
use strobe_gfx::{Api, Caps, FrameInputs, Headless, Mouse, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let headless = Headless::new(Api::DesktopCore33)?;
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let scene = SceneFolder::load(Path::new("scenes/01-aurora-gradient.synScene"))?;
    let controls = ControlBank::new(scene.def.controls.clone())?;
    let audio = vec![0.0; SCALARS.len()];
    let mut player = ScenePlayer::new(gl, &caps, &scene, [320, 180], Options::default())?;
    player.render(&FrameInputs {
        time: 0.0,
        frame: 0,
        mouse: Mouse::default(),
        audio: &audio,
        controls: &controls,
        transitions: &[],
    });
    let rgba = player.read_output(); // 320 * 180 * 4 bytes
    println!("{} bytes", rgba.len());
    Ok(())
}
```

For a window, the app's frame loop is the reference
(`crates/strobe-app/src/app.rs`, `redraw_output`) and the frame order is
written out in [`ARCHITECTURE.md`](../../docs/ARCHITECTURE.md). The bench
(`crates/strobe-bench/src/main.rs`) shows both headless and windowed use
through `offscreen::run`.

## Testing

```bash
cargo test -p strobe-gfx                                       # unit tests + render tests
apt-get install -y libegl1 libegl-mesa0 mesa-utils             # headless EGL on Linux (Mesa llvmpipe)
STROBE_REQUIRE_HEADLESS_GL=1 cargo test -p strobe-gfx --test render
```

- `tests/render.rs` has eleven tests that open a headless context in both
  dialects, compile and render real scenes, and assert on pixels. They cover
  control values reaching the shader, the SSF library compiling in both
  dialects, buffer and double-buffered passes, `syn_FinalPass`, compile
  errors mapping to `main.glsl` lines, audio texture upload, the GPU timer
  never waiting, and reset status on a healthy context.
- `tests/post.rs` (eight tests, both dialects): a neutral pass through the
  post program equals its input bit for bit and `apply` draws nothing at
  neutral; invert, master, hue, saturation, brightness, contrast, gamma and
  both mirrors do what they say on known colours.
- `tests/assets.rs` (nine tests, both dialects): a cube-map fixture where each
  of +X..-Z returns its face colour (six files and a 4:3 cross), a missing and
  a mismatched cube map reporting a problem and reading black, `syn_Media`
  reading a still upright with `syn_MediaType` 1, an image sequence playing
  and looping, a user-chosen file replacing the scene's, and the failure
  cases.
- Without an EGL device they print a skip and pass. Set
  `STROBE_REQUIRE_HEADLESS_GL=1` to make that a failure (CI's Linux job does).
- Intel's Windows driver exposes no EGL devices, so on Windows these tests
  skip (T480, [`T480.md`](../../docs/hardware/T480.md)); use
  `strobe-bench --context window` there instead.
- The windowed providers are not exercised by these tests; they are covered by
  running `strobe-app`, the bench's window mode and
  `crates/strobe-autocontrol/tests/window.rs` (which spawns the CLI on a hidden
  window under Xvfb). `offscreen`'s unit tests cover the `--api`/`--gl` folding.
- Unit tests cover the adaptive controller, capability decisions,
  reset-status name selection, the debug rate limiter and backend selection.

## Design notes

- **Native GL via glow, not wgpu** (D1), with **ANGLE as the fallback
  context** (D2) through the same glutin EGL code. GL 3.3 is the hardware
  floor (D3).
- **`#version 330 core` on every desktop context** (D6); GLES 3.0 on ANGLE.
- **One program per pass, `PASSINDEX` a constant** (D7). On the T480 this is
  neutral, on llvmpipe it was faster, so it stays the default (D14).
  `Options::specialise_passes = false` selects the shared-program mode.
- **Float passes default to RGBA16F** (D13), from T480 measurements.
- **Debug output through `&mut Gl`** (D44): call `enable_debug_output` right
  after creating the context, before cloning the `Gl`.
- **GPU timing never waits** (D64), and **a lost context is never touched
  again** (D66): robust contexts are requested first, `Gl::reset_status` is a
  driver round trip that the app calls after a failed swap and at most once
  a second, and `mark_lost` makes every `Drop` skip its delete.

## Status and known gaps

- Renderer, multipass, ping-pong, `syn_FinalPass`, images, audio textures and
  script uniforms work, and are tested on llvmpipe (headless Mesa) in both
  dialects ([`ROADMAP.md`](../../docs/ROADMAP.md)).
- Real-hardware evidence is the ThinkPad T480 (Intel UHD 620) only: native
  and ANGLE render the scene library, but `10-volume-clouds` ran at 3.8 fps
  on native at full scale ([`T480.md`](../../docs/hardware/T480.md),
  [`BENCHMARKS.md`](../../docs/BENCHMARKS.md)).
- T480 bug 1: native `make_current` failed after 13 to 16 seconds on
  `09-sdf-cathedral` at full render scale. Adaptive resolution avoids it and
  context recovery (D66) is built, but real WGL recovery is unverified; the
  cause was never confirmed.
- Cube maps load (D185) and `syn_Media` shows a still or an image sequence
  (D186), verified on llvmpipe in both dialects only. Video and webcam are
  not built (later). The post stage's GPU cost on an HD 4000 or the T480 is
  unmeasured. `MEDIA`, `CUBEMAPS` and `syn_MediaType` follow Strobe's reading
  of SSF, not its documentation (Q22). `textureFinalPass` data (Q14) comes from the
  asynchronous readback (D162), verified on llvmpipe in both dialects but not
  yet on the T480 or ANGLE/D3D11.
- macOS is not built: glutin compiles its EGL backend out on Apple targets
  and there is no CGL headless provider (D55).
- The audio textures' layout is unverified against real Synesthesia (Q1,
  Q10), and the ANGLE path rewrites only `PASSINDEX == 0.0` of the desktop-only SSF idioms (Q8, D163); see
  [`OPEN_QUESTIONS.md`](../../docs/OPEN_QUESTIONS.md).
