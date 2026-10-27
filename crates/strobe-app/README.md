# strobe-app

The desktop app, built as the `strobe` binary (plus a library so its logic is
testable). It opens two windows that share one GL context: a control window
(egui, laid out as Synesthesia's: a rail of views, Library, Controls, Import,
Playlist, Editor, Mappings, Diagnostics and Settings, beside a persistent
side panel with the live preview, and the transport, BLACKOUT and macros
along the bottom; docs/UI_REVIEW.md) and a
fullscreen-capable output window that shows the scene letterboxed and waits
for vertical blank. It wires the other crates together: it loads scenes,
drives the frame loop, runs each scene's `script.js`, feeds live or synthetic
audio to the shaders, measures GPU time for adaptive resolution, and recovers
from a lost GL context.

## Role in the workspace

- **Depends on:** `strobe-scene`, `strobe-gfx`, `strobe-diag`, `strobe-audio`,
  `strobe-script`, `strobe-autocontrol` and `strobe-remote`, plus `winit`,
  `egui`, `egui_glow`, `egui-winit` (0.37), `rfd` (native file dialogs, D470),
  `clap`, `image`, `log`, `thiserror` and `anyhow`.
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
  and the "Output controls" cards under the scene's controls edit them (D477). They carry across
  scene changes and cues, under whatever the new scene's preset, cue or
  script sets (D207). Blackout is separate and applied after the post stage
  (D181).
- **Show control (D170-D179).** `Remote` owns the MIDI and OSC workers and
  the mapper; `Running::drain_remote` runs once per scene frame before the
  scene advances and writes the live session's banks through `ControlBank`;
  commands, cue jumps, fav slots and presets come back as `Action`s. The
  Mappings view (`ui::map`), the macro strip (`ui::macros`), the Presets tab
  and fav slots (`ui::presets`) and `commands::Bindings` (hotkeys) are the UI
  side.
  `show-control.json` is written by the debounced saver, never in screenshot
  mode.
- **LAN remote (D250-D254, D468, D469).** `lan::Lan` owns the `strobe-remote`
  listener and its discovery responder while it runs; `Running::drain_lan`
  takes its commands once per scene frame (never waiting) and publishes the
  state at 10 Hz. Phones and other Strobes ask, and the host approves or
  denies them; there is no code. Settings' Remote page (`ui::lan`), its
  request banners and the title bar's "Remote on" with its Stop button are
  the UI. Controlling another Strobe is `desktop` (the viewer and its
  worker), `trusted` (venue mode's trust file, D294) and `venue` (D288).
- **Does not:** video and webcam media, NDI output.
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
| `lan` | The LAN remote as the app holds it: the `strobe_remote::server` listener and the discovery responder and scanner, the app end of its bridge, which commands may run now (`refusal`), the 10 Hz state publish, and what Settings and the title bar show (D254, D468). |
| `desktop`, `trusted`, `venue` | Controlling another Strobe: the viewer, input translation and the bounded JPEG capture and decode workers (D275); the controller's proof of a venue trust file, its retry schedule and what a refusal says (D294); venue mode's launch-time pieces, `--make-remote-trust` and loading `--remote-trust` so a failure is reported without stopping the show (D288). |
| `picker` | Native file and folder dialogs (`rfd`, the Browse buttons) on a thread of their own, polled for their answer, so the frame loop never waits on one (D470). |
| `output_monitor` | Which monitor the output goes to, followed by name, position and size across display changes (D400, D451). |
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
| `--gl auto\|native\|angle\|angle-d3d11\|angle-vulkan\|angle-gl\|angle-metal` | `auto` | Native GL 3.3 first, then ANGLE; or force one. `angle-*` also names ANGLE's backend, and fails with a message if ANGLE cannot start on it (D485). |
| `--no-synesthesia` | off | Leave Synesthesia's installed scenes out of the library for this run (D271). |
| `--render-scale <f>` | `1.0` | Render resolution as a fraction of the output window, 0.25 to 1.0. |
| `--no-adaptive` | off | Turn adaptive resolution off; the scale stays where you put it. |
| `--audio auto\|synthetic\|ambient` | `auto` | Capture an input device (synthetic fallback), always the fixed 120 BPM pattern, or that pattern without its hits and beats. |
| `--show-output`, `--fullscreen`, `--output-monitor <n>` | off | Show the output window at launch, fullscreen, on monitor N as Settings numbers them (D451 waits for one not connected yet). |
| `--playlist <file>`, `--no-resume` | none | Load a playlist at launch (everything compiles before the first cue); start it from the top rather than where it was. |
| `--watch-folder <dir>` | saved | Add scene folders that appear in this folder to the library (D240). |
| `--osc` | off | Listen for OSC on `127.0.0.1` for this run (the Mappings view's switch saves it). |
| `--osc-port <n>` | `9000` | The UDP port, for this run. |
| `--osc-all-interfaces` | off | Listen on every network interface, for this run only. OSC has no password (D172); never saved. |
| `--no-midi` | off | Do not open MIDI input devices, for this run. |
| `--show-control <file>` | data dir's `show-control.json` | Read mappings, macros, fav slots and hotkeys from this file. |
| `--screenshot <path>` | none | Render `--frames` frames, write `<path>-control.png` and `<path>-output.png`, exit. |
| `--frames <n>` | `60` | Frames before the screenshot; requires `--screenshot`. |
| `--remote`, `--remote-bind <ip>`, `--remote-port <n>` | off, LAN address, `47812` | Start the LAN remote for this run (Settings' Remote page does the same; never saved). A public address is refused (D250-D254). |
| `--remote-proxy <name[:port]>` | none | The name of your own HTTPS reverse proxy on this computer, let in as the page's origin (D258). |
| `--allow-desktop-control`, `--remote-trust <file>` | off | Allow full desktop control for this run; venue mode, where a controller that proves this trust file is paired with no click (D288). |
| `--control <host:port>` | none | Open straight into the viewer of another Strobe (D275, D294). |
| `--make-remote-trust <file>` | | Write a new trust file, print how to use it, and exit. |
| `--listen`, `--listen-port <n>` | off, `47811` | Accept shaders from the "Send to Strobe" userscript on 127.0.0.1 (the Import panel's switch does the same; the panel shows the token). |
| `--no-texture-download` | off | Do not download textures on import; they read black and are listed in `images/pending-assets.json`. |
| `--diagnose` | off | Print a machine/build/GL/audio report, save it beside the logs, exit. |
| `--open-pane <list>` | saved layout | Open panes at launch, comma separated: `browser`, `controls`, `import`, `playlist`, `editor`, `map`, `inspector` (the side panel), `diagnostics`, `settings`, or `settings:<page>`. |
| `--no-panes` | off | Close every pane first: the main view is then the large preview. |
| `--window-size WxH` | maximized | Open the control window at exactly this size, not maximized (reproducible screenshots; bare Xvfb has no window manager). |
| `--browser-query <text>` | saved | Put this in the scene browser's search field. |
| `--preview <scene>` | none | Prepare this scene in the browser's Preview at launch, without making it live. |
| `--blackout` | off | Start with the output blacked out. |
| `--media <path>` | none | Show this `.png`/`.jpg` file, or a folder of them played as a sequence in name order, behind `syn_Media` in every scene loaded by hand (instead of the scene's `MEDIA`). Decoded off the frame thread (D186). A missing path is reported and the scene runs with black media. |
| `--request-load <scene>` | none | With `--screenshot`: ask for this load through the UI path on the frame before the control capture, so the capture shows "Preparing". |
| `--theme <id>` | saved | Use a theme for this run: `strobe`, `graphite`, `midnight`, `void`, `high-contrast`, `daylight`, or a file in `<data dir>/themes/`. |
| `--density comfortable\|compact` | saved | Use this density for this run (D220). |
| `--system-title-bar` | off | Use the system's title bar for this run (D132); Settings has the saved choice. |
| `--generate-controls <scene>` | | Hidden: the app starts itself this way to generate controls in a worker (D202). Not for people. |
| `--palette <text>` | closed | Open the command palette at launch with this text typed (screenshots). |
| `--help`, `--version` | | Standard clap flags. |

Keys (D134; the table is `commands.rs`, rebound in Settings > Keyboard):
Left and Right step the scene, `Space` holds the cue (with a playlist), `B`
blacks the output out, `F11` toggles fullscreen on the output window,
`Ctrl+F` opens the library's search, `Ctrl+L` flips between the library and
the Controls view (D479), `Esc` closes the topmost pane but never the last
main view (and leaves output fullscreen), `T` taps tempo, and `Ctrl+K` opens
the command palette (D200): every command, view, scene and preset by name,
with its shortcut, or why it cannot run now. The side panel's Scene tab
shows the playing scene's tags and writes them into its `scene.json` (D205).
In the Editor, `Ctrl+S` saves and `Ctrl+Enter` saves and reloads the scene
(D204); typing alone never changes the output.
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
`libEGL.dll` and `libGLESv2.dll` beside the executable, and
`--gl angle-vulkan` also `vulkan-1.dll` (all fetched by
`scripts/fetch-angle.ps1`, D12, D485).

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

Open work for this crate is tracked in
[`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md) under the
IDs given here; a gap with no ID is by design, with its decision named.

- The control window is Synesthesia's layout (D281-D285) with every pane
  rebuilt on one form grammar after a screenshot review (D464-D484,
  [`UI_REVIEW.md`](../../docs/UI_REVIEW.md)): the Playlist workspace with its
  cue editor (D157, D213, D475), Mappings (D177, D467), the Editor (D204,
  D482), the command palette (D200), named layouts (D215) and density
  (D220). Left: the rebuilt layout on the T480, with the title bar's drag,
  Aero Snap, edge resizing and DPI changes at 150% (backlog U-09).
- Real-hardware evidence is the ThinkPad T480 (Intel UHD 620), TORNADO
  (GTX 1080 Ti), the AMD desktop (RX 580) and the venue mini-PC (Vega 3):
  native and ANGLE, live capture, scripts, adaptive resolution and
  `--diagnose`; on the T480 also F11, the hidden output (Task 9) and the
  first Perform UI at 150% scaling (Task 12)
  ([`hardware/`](../../docs/hardware/)).
- T480 bug 1 (native `make_current` failing on `09-sdf-cathedral` at full
  scale) no longer reproduces under adaptive resolution, nor at a forced
  scale of 1.0 (Task 8). Context recovery (D66) is verified under Xvfb and
  on real WGL with injected losses: native rebuild, ANGLE fail-over and
  give-up (T480 Task 8). A real driver reset has not been seen, and native
  WGL on the RX 580 gives no robust context to report one (backlog R-07).
- Playlists are built (stage 2, D150-D158), verified on llvmpipe and on the
  T480 (the transition demo, Task 14; media per cue, Task 26). Audio-event
  advance is built (drops and breakdowns, D226; song changes, D237) and so
  are the scene transitions a song change drives (D489); all are tested on
  synthetic audio only, not yet on real music (backlog P-01, M-05). NDI is
  not built (backlog M-06, deferred).
- Show control (D170-D179) is built and verified under Xvfb with real OSC
  datagrams, a real click on Learn, and headless GL for cue presets. On the
  T480: OSC in and out (Task 25), a macro's key and glide (Task 29, D227,
  D228) and Synesthesia's OSC addresses (Task 31, D234). OSC feedback
  (D209), MIDI output (D257), the macro range and colour editor (D210),
  shortcuts in Settings > Keyboard (D208) and explicit blackout commands
  (D201) are built. **No MIDI controller has been used yet** (backlog P-04,
  P-05).
- Cube maps and `syn_Media` stills and sequences work (D185, D186); video
  and webcam are not built (backlog M-04, deferred). Script `meta/` writes
  land in the session's meta bank, which the post stage renders from
  (D180); its cost on an HD 4000 is unmeasured (backlog P-03). Meta values
  carry over a scene change (D207). `textureFinalPass` is a two-frame-late
  asynchronous readback (D162).
- Linux has run under Xvfb, on Wayland (Weston) and under WSL2 with Mesa,
  including Mesa's D3D12 driver on an RX 580, but not yet on a real Linux
  GPU (backlog R-08); macOS is not built yet (no CGL provider, D55; backlog
  G-07).
