# strobe-app

The desktop app, built as the `strobe` binary (plus a library so its logic is
testable). It opens two windows that share one GL context: a control window
(egui: scene browser, controls, audio settings, console, live preview) and a
fullscreen-capable output window that shows the scene letterboxed and waits
for vertical blank. It wires the other crates together: it loads scenes,
drives the frame loop, runs each scene's `script.js`, feeds live or synthetic
audio to the shaders, measures GPU time for adaptive resolution, and recovers
from a lost GL context.

## Role in the workspace

- **Depends on:** `strobe-scene`, `strobe-gfx`, `strobe-diag`, `strobe-audio`
  and `strobe-script`, plus `winit`, `egui`, `egui_glow`, `egui-winit`, `clap`,
  `image`, `log`, `thiserror` and `anyhow`.
- **Depended on by:** nothing. It is the top of the graph.
- **Does not (yet):** use `strobe-playlist`. The playlist crate is not a
  dependency of the app; the playlist tick, transitions between two scenes,
  MIDI, OSC and the meta post stage (brightness, hue, mirror) are not in the
  frame loop ([`ARCHITECTURE.md`](../../docs/ARCHITECTURE.md), "A frame").
  `strobe-shadertoy` and `strobe-io` do not exist yet.
- The app has no `#[cfg(windows)]` code of its own, only `cfg!` values; that is
  why the Windows cross-check from Linux can exclude it (D63).

## Main types / entry points

The binary entry is `src/main.rs`; everything else is in the library.

| Module | What |
|---|---|
| `main.rs` | Start-up order: register the crash notifier, run as the crash monitor if asked, `strobe_diag::init()`, `--diagnose`, arm crash capture, open windows, run the event loop. |
| `cli` | The clap `Cli` (flags below). |
| `app` | The winit application and the frame loop (`redraw_output` is the frame). |
| `session` | The one active scene: load, advance (clock, audio, script, controls), render. |
| `audio` | `AudioInput`: live capture started on a thread, with synthetic fallback and settings (device, reactivity, speed, tap tempo). |
| `script`, `mouse` | A scene's `script.js` at run time, and the pointer uniforms. |
| `perf` | GPU timer ring and adaptive resolution. |
| `recovery`, `fault` | The context-loss policy (a pure state machine) and `STROBE_FAULT` injection. |
| `ui` | The control window's panels as a function of a `Model` that returns `Action`s. |
| `library`, `console`, `stats`, `theme` | The data the UI shows and the look. |
| `failures`, `dialog`, `gldebug`, `diagnose`, `screenshot` | Errors, crash dialogs, driver messages, `--diagnose` and `--screenshot`. |

## Usage

```bash
cargo run --release -p strobe-app -- --scene 05-domain-warp
cargo run --release -p strobe-app -- --gl angle --render-scale 0.5
cargo run --release -p strobe-app -- --scene 05 --screenshot target/shot --frames 60
cargo run --release -p strobe-app -- --diagnose
```

Flags (from the clap `Cli`):

| Flag | Default | Meaning |
|---|---|---|
| `--library <dir>` | `scenes` | Folder of scene folders. |
| `--scene <name>` | none | Scene to load at start: a folder name or a unique prefix. |
| `--gl auto\|native\|angle` | `auto` | Native GL 3.3 first, then ANGLE; or force one. |
| `--render-scale <f>` | `1.0` | Render resolution as a fraction of the output window, 0.25 to 1.0. |
| `--no-adaptive` | off | Turn adaptive resolution off; the scale stays where you put it. |
| `--audio auto\|synthetic` | `auto` | Capture an input device (synthetic fallback), or always the fixed 120 BPM pattern. |
| `--screenshot <path>` | none | Render `--frames` frames, write `<path>-control.png` and `<path>-output.png`, exit. |
| `--frames <n>` | `60` | Frames before the screenshot; requires `--screenshot`. |
| `--diagnose` | off | Print a machine/build/GL/audio report, save it beside the logs, exit. |
| `--help`, `--version` | | Standard clap flags. |

Keys, handled in `app.rs`: `F11` toggles fullscreen on the output window,
`Esc` leaves fullscreen, and `T` taps tempo (ignored while a text field in the
control window has focus). The control window always opens maximized (D65).

The app also reads these environment variables (details in
[`DEBUGGING.md`](../../docs/DEBUGGING.md)): `STROBE_LOG`, `STROBE_LOG_STDERR=1`,
`STROBE_GL_DEBUG`, `STROBE_FAULT` (e.g. `lose-context@60`),
`STROBE_NO_CRASH_MONITOR=1` and `STROBE_DIAG_DIR`. Logs and crash reports go
under `%LOCALAPPDATA%\strobe` (Windows) or `~/.local/share/strobe` (Linux).
The Windows release build has no console (`windows_subsystem = "windows"`), so
startup failures appear in a dialog and in the session log. ANGLE needs
`libEGL.dll` and `libGLESv2.dll` beside the executable (fetched by
`scripts/fetch-angle.ps1`, D12).

## Testing

```bash
cargo test -p strobe-app
```

The unit tests cover the CLI, the recovery policy, fault parsing and parts of
the UI (layout and widgets) as pure logic. `tests/script_uniforms.rs` runs script-created
uniforms end to end on a headless context in both GL dialects; it needs
headless EGL (`libegl1 libegl-mesa0`) and skips without it unless
`STROBE_REQUIRE_HEADLESS_GL=1`, which CI's Linux job sets.

To check what the windows actually draw with no display or person present,
use screenshot mode under Xvfb (what CI does):

```bash
apt-get install -y xvfb libegl1 libegl-mesa0 mesa-utils libgl1-mesa-dri
xvfb-run -a -s "-screen 0 1920x1080x24" cargo run -p strobe-app -- --scene 01 --screenshot target/ci-shot --frames 10
```

Context-loss recovery can be exercised the same way with `STROBE_FAULT`; the
runs are listed in [`DEBUGGING.md`](../../docs/DEBUGGING.md).

## Design notes

- **Native GL, ANGLE as the fallback** (D1, D2): `--gl auto` tries native then
  ANGLE; a forced backend is retried, never abandoned (D66).
- **The console is a view of `strobe-diag`'s ring** (D60), not a second log;
  failures are structured records.
- **Audio belongs to the app and falls back to synthetic by capture state**
  (D61), so switching scenes never restarts capture or loses a tapped tempo.
- **Script uniforms are compiled in at load** from the script's first frame
  (D62), so no shader compiles during playback.
- **GPU timing never waits, and adaptation holds for a second after a scene
  load** (D64); the render-scale slider becomes the ceiling while adaptive is on.
- **A lost context is rebuilt, on ANGLE if native keeps failing** (D66): both
  windows and the context are recreated and the scene reloads at its clock.

## Status and known gaps

- Phase 1 minimal app is done ([`ROADMAP.md`](../../docs/ROADMAP.md)): two
  windows, scene browser, controls per type, console, preview. The
  Perform-first UI in [`UI_DESIGN.md`](../../docs/UI_DESIGN.md) is not built.
- Real-hardware evidence is the ThinkPad T480 (Intel UHD 620): native and
  ANGLE smoke tests, live capture, a script scene, adaptive resolution and
  `--diagnose` all ran ([`T480.md`](../../docs/hardware/T480.md)).
- T480 bug 1 (native `make_current` failing on `09-sdf-cathedral` at full
  scale) no longer reproduces under adaptive resolution. Context recovery
  (D66) is verified under Xvfb only; real WGL recovery and a forced scale of
  1.0 are still unchecked. F11 on the 1440p panel, scene switching and DPI at
  150% scaling are still waiting for an interactive check.
- Not built: playlists in the app (stage 2), transitions, presets, MIDI, OSC,
  hotkey mapping, Shadertoy import, NDI. Cubemaps and media input are
  placeholders. `textureFinalPass` returns black and script `meta/` writes are
  dropped.
- Linux runs under Xvfb and Mesa only; macOS is not built (no CGL provider,
  D55).
