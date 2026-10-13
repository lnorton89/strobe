# strobe-app

The desktop app, built as the `strobe` binary (plus a library so its logic is
testable). It opens two windows that share one GL context: a control window
(egui, Contextual Perform: rail, live preview, transport and macros, and
panes for the scene browser, controls, diagnostics, import and settings) and a
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
- Also depends on `strobe-io` (show control, `remote.rs`: MIDI, OSC, mappings,
  macros, fav slots; D170-D179), `strobe-playlist` and `strobe-shadertoy`: the playlist tick,
  transitions between two scenes and the Playlist pane are in the frame loop
  ([`ARCHITECTURE.md`](../../docs/ARCHITECTURE.md), "A frame" and
  "Playlists").
- **The meta controls work (D180-D184).** The live session's meta bank drives
  the output post stage (master, invert, brightness, gamma, contrast, hue,
  saturation, mirrors, Limit Colors) through `strobe_gfx::Post`, run in `App::apply_post`
  after the compositor and **skipped entirely at neutral**; its reactivity and
  audio speed multiply the Audio panel's gains (`AudioInput::set_meta_gains`);
  and the inspector's "Output (meta)" section edits them. They carry across
  scene changes and cues, under whatever the new scene's preset, cue or
  script sets (D207). Blackout is separate and applied after the post stage
  (D181).
- **Show control (D170-D179).** `Remote` owns the MIDI and OSC workers and
  the mapper; `Running::drain_remote` runs once per scene frame before the
  scene advances and writes the live session's banks through `ControlBank`;
  commands, cue jumps, fav slots and presets come back as `Action`s. The MAP
  pane (`ui::map`), the macro strip (`ui::macros`), the preset row and fav
  slots (`ui::presets`) and `commands::Bindings` (hotkeys) are the UI side.
  `show-control.json` is written by the debounced saver, never in screenshot
  mode.
- **Does not (yet):** audio-event advance, MIDI or OSC output, or video and
  webcam media.
- The app has no `#[cfg(windows)]` code of its own, only `cfg!` values; that is
  why the Windows cross-check from Linux can exclude it (D63).

## Main types / entry points

The binary entry is `src/main.rs`; everything else is in the library. A
second binary, `strobe-corpus` (`src/corpus_main.rs`, run with `cargo xtask
corpus -- --library <dir>`), is the compatibility harness (D195): it loads
every scene in a folder through `Session::load`, script included, renders it
in both GLSL dialects in a child process per scene, and writes
`target/corpus/report.{json,md}` (module `corpus`).

| Module | What |
|---|---|
| `main.rs` | Start-up order: register the crash notifier, run as the crash monitor if asked, `strobe_diag::init()`, `--diagnose`, arm crash capture, open windows, run the event loop. |
| `cli` | The clap `Cli` (flags below). |
| `app` | The winit application and the frame loop (`redraw_output` is the frame). |
| `session` | One scene: load, advance (clock, audio, script, controls), render, prime, restart its clock. |
| `show` | A playlist being played (D150-D158): `run::Show` (precompile, clock events, warm-up, transitions), `cache`, `resolve`, `start`, `resume`, `watch`, `view`, `generate`, `beat`. |
| `audio` | `AudioInput`: live capture started on a thread, with synthetic fallback and settings (device, reactivity, speed, tap tempo). |
| `script`, `mouse` | A scene's `script.js` at run time, and the pointer uniforms. |
| `perf` | GPU timer ring and adaptive resolution. |
| `recovery`, `fault` | The context-loss policy (a pure state machine) and `STROBE_FAULT` injection. |
| `ui` | The control window: a shell, panes and widgets as a function of a `Model` that returns `Action`s (D120, D130-D138). |
| `workspace`, `prefs`, `commands` | The pane model and how it fits a window, the saved UI preferences (`ui.toml`), and the keyboard command table with its focus rule. Pure. |
| `editor` | The Editor's documents: a scene's source files as tabs, unsaved state, atomic saves, backups in the data directory, error locations and the GLSL highlighter (D204). No egui. |
| `commands`, `command_palette` | The command table and focus rule, and what the Ctrl+K palette lists, how it ranks a query, and why an entry is unavailable. No egui. |
| `color`, `palette`, `motion`, `icon` | The UI system ported from rustaman (D5): colour maths, themes with WCAG tests, durations and reduced motion, geometry icons. No egui. |
| `library`, `console`, `stats` | The data the UI shows: scenes with their tags (D205), health and search over names and tags, the console, frame timings. |
| `import` | Shadertoy import: the worker thread that runs the converter, the panel's state, and the listener's handler (D113, D114, D116). |
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
| `--osc` | off | Listen for OSC on `127.0.0.1` for this run (the MAP pane's switch saves it). |
| `--osc-port <n>` | `9000` | The UDP port, for this run. |
| `--osc-all-interfaces` | off | Listen on every network interface, for this run only. OSC has no password (D172); never saved. |
| `--no-midi` | off | Do not open MIDI input devices, for this run. |
| `--show-control <file>` | data dir's `show-control.json` | Read mappings, macros, fav slots and hotkeys from this file. |
| `--screenshot <path>` | none | Render `--frames` frames, write `<path>-control.png` and `<path>-output.png`, exit. |
| `--frames <n>` | `60` | Frames before the screenshot; requires `--screenshot`. |
| `--listen`, `--listen-port <n>` | off, `47811` | Accept shaders from the "Send to Strobe" userscript on 127.0.0.1 (the Import panel's switch does the same; the panel shows the token). |
| `--no-texture-download` | off | Do not download textures on import; they read black and are listed in `images/pending-assets.json`. |
| `--diagnose` | off | Print a machine/build/GL/audio report, save it beside the logs, exit. |
| `--open-pane <list>` | saved layout | Open panes at launch: `browser`, `import`, `inspector`, `diagnostics`, `settings` (comma separated). |
| `--no-panes` | off | Close every pane first (the bare Perform surface). |
| `--window-size WxH` | maximized | Open the control window at exactly this size, not maximized (reproducible screenshots; bare Xvfb has no window manager). |
| `--browser-query <text>` | saved | Put this in the scene browser's search field. |
| `--preview <scene>` | none | Prepare this scene in the browser's Preview at launch, without making it live. |
| `--blackout` | off | Start with the output blacked out. |
| `--media <path>` | none | Show this `.png`/`.jpg` file, or a folder of them played as a sequence in name order, behind `syn_Media` in every scene loaded by hand (instead of the scene's `MEDIA`). Decoded off the frame thread (D186). A missing path is reported and the scene runs with black media. |
| `--request-load <scene>` | none | With `--screenshot`: ask for this load through the UI path on the frame before the control capture, so the capture shows "Preparing". |
| `--theme <id>` | saved | Use a theme for this run: `strobe`, `graphite`, `midnight`, `void`, `high-contrast`, `daylight`, or a file in `<data dir>/themes/`. |
| `--system-title-bar` | off | Use the system's title bar for this run (D132); Settings has the saved choice. |
| `--generate-controls <scene>` | | Hidden: the app starts itself this way to generate controls in a worker (D202). Not for people. |
| `--palette <text>` | closed | Open the command palette at launch with this text typed (screenshots). |
| `--help`, `--version` | | Standard clap flags. |

Keys (D134; the table is `commands.rs`, shown in Settings): Left and Right
step the scene, `Space` holds (not wired yet: it shows a notice), `B` blacks the
output out, `F11` toggles fullscreen on the output window, `Ctrl+F` opens the
scene browser's search, `Esc` closes the topmost pane (and leaves output
fullscreen), `T` taps tempo, and `Ctrl+K` opens the command palette (D200):
every command and pane by name, with its shortcut, or why it cannot run now.
The Inspector shows the playing scene's tags; Edit writes them into its
`scene.json` (D205). In the Editor (EDIT), `Ctrl+S` saves and `Ctrl+Enter` saves and reloads the
scene (D204); typing alone never changes the output.
None fires while a text field has focus or the palette is open, and the
arrows and Space stay with a focused slider. The control window always opens
maximized (D65) unless `--window-size` is given.

UI preferences (open panes and their widths, the browser's query and filter,
the theme, reduced motion, the title-bar choice, and the last scene loaded,
which the next launch reopens unless `--scene` or `--playlist` says
otherwise) are saved to `ui.toml` in the
data directory (D133), debounced and off the UI thread. Screenshot mode never
reads or writes it.

The app also reads these environment variables (details in
[`DEBUGGING.md`](../../docs/DEBUGGING.md)): `STROBE_LOG`, `STROBE_LOG_STDERR=1`,
`STROBE_GL_DEBUG`, `STROBE_FAULT` (e.g. `lose-context@60`),
`STROBE_NO_CRASH_MONITOR=1`, `STROBE_DIAG_DIR` and `STROBE_SHADERTOY_KEY`. Logs and crash reports go
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
`tests/show.rs` plays a whole playlist on a headless context in both
dialects: a failed scene skipped, warm-up, a crossfade seen in the mixed
frame, hold, next, a reload, and zero shader compiles after the load. The
pure parts of the show (beat mapping, cache, overrides, start order, resume
writer, watcher, Generate form) are unit tests in `src/show/`.

To check what the windows actually draw with no display or person present,
use screenshot mode under Xvfb (what CI does). A playlist is a flag away, and
`--frames` then counts frames of *playback* (after the load), so a capture
lands mid-transition on purpose:

```bash
strobe --playlist playlists/transition-demo.toml --screenshot target/demo --frames 660 \
  --window-size 1280x720 --audio synthetic --no-adaptive --open-pane playlist
```

The log says `transition 1 started`, `warming up cue 4` and, at close,
`closed with 0 runtime shader compile(s)`. Resume is never read or written in
screenshot mode; for a real run use `STROBE_DIAG_DIR` to keep it out of your
profile. The general form:

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
  windows and the context are recreated and the scene reloads at its clock;
  a playlist is compiled again and resumes at its cue (D150).
- **A playlist compiles everything at load, one scene per frame, parks scenes
  at 1x1 and proves it compiled nothing later** (D151, D152); two scenes
  render during a transition and the compositor mixes them (D154).

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
- Playlists are built (stage 2, D150-D158) and verified on llvmpipe only. Not
  built: audio-event advance, the Playlist workspace's inspector, Shadertoy
  thumbnails and the Needs-work bin, NDI.
- Show control (D170-D179) is built and verified under Xvfb with real OSC
  datagrams, a real click on Learn, and headless GL for cue presets; **no MIDI
  controller has been used**, and nothing here has run on the T480 or Windows.
  Not built: OSC or MIDI output, a macro range and colour editor, an explicit
  blackout on/off command, an in-app hotkey editor.
  Cube maps and `syn_Media` stills and sequences work (D185, D186), video
  and webcam do not. Script `meta/` writes land in the session's meta bank,
  which the post stage renders from (D180), checked on llvmpipe under Xvfb
  only (its cost on an HD 4000 is unmeasured); meta values reset on a scene
  change (D182). `textureFinalPass` is a two-frame-late asynchronous readback (D162).
- Linux runs under Xvfb and Mesa only; macOS is not built (no CGL provider,
  D55).
