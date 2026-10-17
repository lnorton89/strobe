# Debugging Strobe

Strobe runs on a projector, often on a laptop nobody is sitting at, so
when it hangs, crashes or misbehaves the evidence has to already be on
disk. This page says where it is and how to ask for more. The mechanism
lives in the `strobe-diag` crate (decisions D40 to D45); the app wiring is
D60. Context-loss recovery is D66.

## Where things go

Everything is under one base directory:

| Platform | Base directory |
|---|---|
| Windows | `%LOCALAPPDATA%\strobe` |
| Linux | `~/.local/share/strobe` |
| macOS | `~/Library/Application Support/strobe` |

`STROBE_DIAG_DIR` replaces the base (tests and the crash demo use it so
they never touch a real profile). Inside it:

| Path | What |
|---|---|
| `logs/` | One session log per run, newest first, the last 10 kept. A file stops at 64 MiB with a line saying so (D380); the console and crash reports keep the newest lines. The first lines say the build, the filter, and any filter directive that did not parse. Times are seconds since process start. |
| `crashes/crash-*.txt` | A readable report: the panic or crash summary, the session context (GL renderer, backend, scene, render size, frame count), recent structured failures, and the tail of the log. |
| `crashes/crash-*.dmp` | A minidump of a *native* crash (a driver access violation, a stack overflow, an abort), written by a second process. A panic has a `.txt` only, and a panic followed by `panic = "abort"` shares one stem. |
| `run/` | The crash monitor's socket file (Windows and macOS). Linux uses an abstract socket with no file. |

`strobe --diagnose` prints a report of the machine, build, GL and audio
setup, saves it beside the logs, and exits. Attach it to bug reports.

## Environment variables

| Variable | Effect |
|---|---|
| `STROBE_LOG` | Log filter. A comma-separated list: a bare level (`debug`) sets the default, `target=level` sets one target and everything under it (`strobe_gfx=trace,warn`). Levels `off`, `error`, `warn`, `info`, `debug`, `trace`. The most specific directive wins. Default is `info` in release builds and `debug` in debug builds; noisy third-party targets (`winit`, `egui`, `glutin`, `x11rb` and others) are capped at `warn` unless named, as in `STROBE_LOG=winit=debug`. A directive that does not parse is skipped and reported in the log's first lines. |
| `STROBE_LOG_STDERR=1` | Mirror log lines to stderr. Debug builds do it anyway. The Windows release build has no console, so this is for a console build on a test laptop, and for runs under Xvfb. |
| `STROBE_GL_DEBUG` | Driver debug output (`KHR_debug`), routed to the log under the target `gl_driver`. Unset: warnings and errors in debug builds, errors only in release. `1` or `all`: everything. `errors`, `warn`, `info` pick a level. `0` or `off` turns it off. Old Intel drivers without `KHR_debug` report that it is unavailable. `STROBE_LOG=gl_driver=debug` shows the notification-level messages too. |
| `STROBE_GL_TRACE` | Records where the renderer binds, draws, clears, blits and reads: the framebuffer (0 is the window's) and viewport of each, once per distinct event, under the log target `gl_trace`, with a count summary (`strobe-bench` prints it at the end). `1` turns it on; a number caps the distinct events kept. The first time each viewport is set it is read back from the driver and a mismatch warns, which shows a driver clamping it. Off by default and free when off; for a machine that renders black (D296). `strobe-bench --probe` is the companion. |
| `STROBE_GL_NO_PRIME=1` | Skip the once-per-context prime: the 16x16 draw every GL context makes when it is created, because AMD's WGL driver drops draws into targets over 16384 pixels until a context has drawn into a small one (D311). For checking whether a driver still needs it; expect black offscreen frames on the mini PC's Vega 3 with it set. `STROBE_LOG=strobe_gfx=debug` shows the prime's `GL prime: ... landed in N us` line. |
| `STROBE_FAULT` | Fault injection, to exercise context-loss recovery without a broken driver. See below. |
| `STROBE_FINISH_AFTER_SWAP` | Whether the output waits (`glFinish`) for each swap to complete, so switching the context to the control window does not (D221). Unset: only after the control window has been seen waiting (NVIDIA, windowed output), and not while a fullscreen output covers it. `1`: always. `0`: never. |
| `STROBE_NO_CRASH_MONITOR=1` | Do not start the crash monitor process or attach the native crash handler. Use it under a debugger. |
| `STROBE_DIAG_DIR` | Replace the base directory above. |
| `STROBE_REQUIRE_HEADLESS_GL=1` | Make the headless-GL tests fail instead of skip when no EGL device exists. CI's Linux job sets it. |
| `STROBE_DUMP_GLSL=<dir>` | Write each pass's assembled fragment shader to `<dir>` when a scene loads, as `<title>.<dialect>.pass<N>.frag` (dialect `glsl330`, `glsl410` or `es300`). Use it to compare what the desktop and ANGLE drivers are given. A failed write is a log warning. |

## Context loss and `STROBE_FAULT`

A failed `make_current` or swap, or a driver reset report, is not fatal.
The app counts failed frames, asks the driver (once a second, and after a
failed swap) for `glGetGraphicsResetStatus`, and rebuilds the context,
failing over from native GL to ANGLE if the policy says so. See
ARCHITECTURE.md ("Context loss") and DECISIONS.md D66 for the rules.

Each loss and each rebuild is a structured `CONTEXT` record in the log and
the console:

```text
CONTEXT
scene=09-sdf-cathedral
phase=lost
window=output
reset_status=not queried (the context was not current)
backend=native
message=3 failed output frames in a row: make current: [6] The handle is invalid. (os error 6)
action=failing over to angle; the output goes dark until it is up
```

followed by a `phase=rebuild` record (`backend_before`, `backend_after`,
`elapsed_ms`, `outcome=recovered|failed`) and an `INFO` line,
`Output recovered on angle in 30 ms; 09-sdf-cathedral reloaded`. At start
the log also says which context it got: `GL context: robust (lose context
on reset)`, or that robustness was refused and a driver reset will not be
reported.

`STROBE_FAULT` is read once, at startup, and is a comma-separated list:

| Item | Effect |
|---|---|
| `lose-context@60` | After 60 scene frames (shown or hidden output) the context dies: every `make_current`, on both windows, fails with an injected error until the context is rebuilt, and the `Gl` is marked lost, as if the driver had reset. |
| `lose-context@60,120` | Two losses, at frames 60 and 120. |
| `fail-rebuild:native` / `fail-rebuild:angle` | Rebuilding on that backend fails. |
| `crash@300` | After 300 output frames the app overflows its stack on purpose: the crash drill for the **real app**, packaged or not. It writes `crash-*.dmp` and `.txt` like `crash_demo overflow`. |

A malformed value is logged and ignored. Under Xvfb with Mesa (where
"ANGLE" is the EGL/GLES backend):

```bash
apt-get install -y xvfb libegl1 libegl-mesa0 mesa-utils libgl1-mesa-dri
cargo build --release -p strobe-app
run() { STROBE_LOG_STDERR=1 STROBE_FAULT="$1" xvfb-run -a -s "-screen 0 1920x1080x24" \
  target/release/strobe --scene 03-kaleido-tunnel --screenshot target/recover/shot --frames 200 "${@:2}"; }
run lose-context@60                                  # rebuild in place, native
run lose-context@60,120                              # second loss within 60 s: fail over to ANGLE
run lose-context@60,fail-rebuild:native              # the native rebuild fails: ANGLE
run lose-context@60,fail-rebuild:native,fail-rebuild:angle   # gives up: the failure dialog
run lose-context@60,120 --gl native                  # a forced backend is retried, never abandoned
```

The output window is hidden unless `--show-output` is added; both modes recover
and keep their visibility (`xdotool search --name Strobe` and `xwininfo -id`
show the output's `Map State`). Each run except the give-up one writes `target/recover/shot-output.png`
and `-control.png`; open them to check the output is not black.

## Frame rate and the `perf:` line

Once a minute the app logs one line (D206):

```
perf: 59.9 fps over 60 s, 0 late of 3594 frames, worst gap 17.9 ms, gpu 2.10 ms, control 1800 redraws 2.40 ms (worst 6.1 ms, 0 unpainted): make-current 0.10, layout 1.20, paint 0.80, swap 0.30 ms
```

- **late**: frames whose gap was over 1.5 frame budgets, so each is a
  missed vertical blank.
- **gpu**: the scene's mean GPU time. Late frames with a low GPU time mean
  the time went somewhere else.
- **control**: the control window's redraws and their whole cost, then the
  mean of each part: making the context current, egui's layout, egui's GL
  paint, and the buffer swap. A part near one refresh period (16.7 ms at
  60 Hz) is a wait for a vertical blank, and each such wait makes an output
  frame late (TORNADO T11, T12).
  The count drops to about 120 a minute while the fullscreen output covers
  the control window ("control window covered" in the log).
  **unpainted** counts the redraws the event loop ran because Windows sent
  the control window no paint (D290): an output above it in z-order takes
  every `WM_PAINT`. After `control window: no paint from Windows for ...`
  in the log, expect about 120 a minute, all of them unpainted (300 while
  a desktop remote is connected). **`control 0 redraws` with the output
  shown is a bug**: nothing in the control window's frame is running (its
  actions, the desktop stream, the playlist compile).

If make-current sits near a refresh period, the control window is waiting
for the output's vsynced swap. The app then slows the control window to
10 Hz and logs `control window: making it current waits ...` (D216), and
logs again when it recovers.

A soak should show `0 late` minute after minute. The fps in the control
window's status bar is only as fresh as that window's last paint; while the
fullscreen output covers it, Windows hardly repaints it (the event loop
redraws it at 2 Hz then, D290), so trust the log. To compare with an older
run, grep the log for `perf:`.

## GL problems

1. Run with `STROBE_GL_DEBUG=1 STROBE_LOG=gl_driver=debug` and read the
   `gl_driver` lines before the failure.
2. `--gl native` and `--gl angle` force a backend. The first log lines
   name the backend, renderer and driver version, and every crash report
   carries them.
3. Check the context line: robust or not, and what
   `reset_status` said after a failure.
4. A GPU timing line says whether adaptive resolution can work on this
   context.
5. Frames black on one machine and fine on another: `STROBE_GL_TRACE=1`, and
   for the bench `--probe` (D296, `hardware/MINIPC.md`). `STROBE_GL_DEBUG` is
   also read by `strobe-bench` and `strobe-autocontrol`, off unless set. A
   `GL prime` warning in the log means the once-per-context prime (D311)
   did not land, and large targets may be black on an AMD WGL driver.

## Crashes

A panic writes `crash-*.txt` with a backtrace. A native crash writes
`crash-*.txt` and `crash-*.dmp`; the **monitor** process shows the message
box, so it works even when the app cannot open a window. The box goes up
after the crashed app has been let go, so the app exits at once and does not
wait for the click. An unattended run (`--screenshot`) raises no box from
either process; the files are written as usual. A crash in the
first moments, before the monitor link is up, is not captured. A bare
`abort()` on Windows is a fast-fail that bypasses exception filters and
leaves no minidump (the `.txt` of a panic that led to it still exists).

**Before sharing crash files.** A `.txt` report names only the `STROBE_*`
variables and `RUST_BACKTRACE`, but its paths (the log file, the data
directory, `STROBE_DIAG_DIR`) contain the user name, and its log tail has
the scene names and the LAN addresses of paired phones. A `.dmp` is the
crashed process's memory. On Linux it also carries the process's whole
environment (`/proc/<pid>/environ`, and the top of the main thread's
stack), so `STROBE_SHADERTOY_KEY` and any other secret set in the
environment is in it: this was checked by crashing `crash_demo` with a
dummy key set. A Windows dump is written with `MiniDumpNormal` (stacks and
modules, no environment block); that is still to be confirmed on the T480.
Keep the Shadertoy key in `shadertoy.key` rather than the environment, and
send a `.dmp` only to whoever is debugging it.

### Symbolicating a `.dmp`

You need the debug information of the *same build*. For a local release
build, that is the `.pdb` files under `target/release` (on Linux or macOS,
the unstripped binaries). For a published release, it is the
`strobe-<tag>-<target>-symbols.zip` archive (D53); keep every supported
release's archive. Release builds have line tables (D42), so frames show
function names and `file:line`.

No debugger is needed:

```bash
cargo install dump_syms minidump-stackwalk --locked     # once
cargo xtask symbolicate path/to/crash-….dmp             # uses target/release's PDBs
cargo xtask symbolicate crash.dmp --symbols strobe.pdb --out stack.txt
```

The command runs `dump_syms` on each PDB and files the `.sym` where
`minidump-stackwalk` looks for it (`target/symbols/<debug-file>/<id>/<name>.sym`,
read from the `MODULE` line). Then it prints the stack: the crash reason and
address, and every thread's frames, the crashed thread first.

Verified on the T480 (R5): a stack overflow came back as

```
Crash reason:  EXCEPTION_STACK_OVERFLOW
 1  crash_demo.exe!crash_demo::recurse() [crash_demo.rs : 32 + 0xc]
246  crash_demo.exe!crash_demo::main() [crash_demo.rs : 90 + 0x19]
```

WinDbg or Visual Studio work too: put the PDB folder on the symbol path
(`.sympath+ <folder>`, `.reload`, `!analyze -v`).

### The crash drill (T480 checklist)

`strobe-diag` ships a demo that installs the whole pipeline and then fails
on purpose:

```bash
STROBE_DIAG_DIR=target/diag cargo run --release -p strobe-diag --example crash_demo -- <mode>
```

| Mode | Expected files |
|---|---|
| `panic` | `.txt` only |
| `abort` | session log only on Windows: Rust's `abort` is `__fastfail`, which no in-process handler sees (confirmed on the T480, 2026-10-09). `.txt` and `.dmp` on Linux |
| `overflow` | `.txt` and `.dmp` |
| `panic-abort-profile` | one stem shared by the panic report and the dump |
| `ok` | a session log only |

The same drill on the app itself, for a packaged build (R-04): run
`strobe.exe` with `STROBE_FAULT=crash@300` and it overflows its stack
after 300 output frames, writing the same `.dmp` and `.txt`. A packaged
build's PDB is under `target/x86_64-pc-windows-msvc/release`, which
`cargo xtask symbolicate` searches first.

On the laptop, `overflow` should produce its files within a few seconds.
An agent driving the laptop runs inside the Claude desktop app's MSIX
package context, where `%LOCALAPPDATA%` is virtualised and the monitor
socket cannot be connected (`os error 10022`); it must set
`STROBE_DIAG_DIR` to a folder inside the repo. A person in an ordinary
terminal needs no override (T480 bug 2). Then, per run: confirm the notifier message appeared,
the `.txt` names the GL renderer and the scene, and the session log's last
lines are present. Report the paths of the files, not their contents, and
attach the `--diagnose` report.

## The UI's saved state and screenshots

The control window's layout is `ui.toml` in the data directory (beside the
logs; `STROBE_DIAG_DIR` moves it). A malformed file is ignored with a warning
in the log, so deleting it is always safe. Custom themes go in
`<data dir>/themes/*.toml` (the format is in `crates/strobe-app/assets/themes.toml`).
`STROBE_REDUCED_MOTION=1` forces reduced motion on.

To see what the UI looks like without a screen, use
`strobe --window-size 1280x720 --open-pane browser --screenshot out/shot`
under Xvfb (`Xvfb :99 -screen 0 1440x900x24`); the flags are in the
strobe-app README. To drive keys, `xdotool windowfocus` the window first (there
is no window manager to focus it) and capture with `import -window root`.

## Playlists

A playlist logs every step it takes, all at `info`, all prefixed `playlist`:

```
playlist "Transition demo": 5 cue(s), 6 step(s) to compile before it plays
playlist: compiled 03-kaleido-tunnel (5/5) in 142 ms
playlist "Transition demo": 4 scene(s) compiled, 1 failed; shader compiles from here on are counted (12 so far)
playlist: cue 2 started: 06-feedback-trails (pass 0)
playlist: warming up cue 4 (02-plasma-pulse) at 480x270
playlist: transition 1 started: cue 2 to cue 4 (Custom(0))
playlist: cue 3 (ghost-scene) skipped: its scene failed to compile
playlist "Transition demo": closed with 0 runtime shader compile(s) (...)
```

- **A scene that will not play** is a `PLAYLIST` / `phase=precompile` failure
  block naming the scene (a reference that is not in the library, or the
  compile error), and its cues are `skipped` in the LIST pane.
- **`N shader compile(s) happened during playback`** (a warning) means the
  zero-compile promise broke (D152). A late script uniform used to be the
  cause; D161 now refuses it instead of recompiling, so this is a bug: report
  it with the log lines before it.
- **A playlist that will not load** is `PLAYLIST` / `phase=load` with the file
  and the TOML error (`line 3, column 13: "3mn" is not a duration`); on a hot
  reload it is `phase=reload` and the old playlist keeps playing.
- **The resume point** is `resume.toml` in the data directory (five lines;
  `STROBE_DIAG_DIR` moves it). `--no-resume` neither reads nor writes it, and
  screenshot mode never does. `playlist: not resuming: ...` says why a saved
  point was refused. Delete the file to start from the top.
- **Screenshots of a playlist**: `--frames N` counts frames after the load
  finishes, at a fixed 1/60 s step, so `--frames 360` on
  `playlists/transition-demo.toml` is 1 s into the first transition, every time.

## MIDI, OSC and show control

Show control (D170-D179) logs at `info`, prefixed `show control:`, `OSC:`,
`MIDI:` and `learned`:

```
OSC: listening on 127.0.0.1:9000
show control: /strobe/control/hue_shift 0.800: applied
show control: /strobe/control/nope 0.500: unavailable: "nope" is not a control of the scene that is playing
show control: /strobe/cmd/toggle_blackout 1.000: applied
blackout on
learned /9/fader -> macro #1
MIDI: nanoKONTROL2 connected
MIDI: nanoKONTROL2 disconnected          (a warning)
```

One line per *new* thing heard: a fader sweep folds into one line (the MAP
pane's Activity shows the count). The MAP pane is the first place to look: its
Connections block says whether OSC is listening and each MIDI device's state,
and Activity shows the last messages and what each did.

- **OSC "Cannot listen"** (`OSC: cannot listen on 127.0.0.1:9000: ... is
  another program using port 9000?`): the port is taken. Pick another in the
  pane, or `--osc-port`. OSC is **off** until `--osc` or the pane's switch.
- **OSC messages do not arrive:** check the address and the port with the
  Activity monitor. From the same machine use `127.0.0.1`. From a phone or
  another computer the listener must be on every interface (`--osc-all-interfaces`
  or the pane's checkbox, which logs a warning because OSC has no password,
  D172) **and** the machine's firewall must allow UDP on that port (Windows asks
  the first time). `--osc` alone never accepts other machines.
- **`unmapped`** in Activity: nothing is mapped to it and, for an OSC address,
  it is not one of the built-in `/strobe/...` addresses (the README of
  `strobe-io` lists them) or built-ins are off. **`unavailable: ...`**: it is
  mapped, but the control is not in the scene that is playing. Not an error.
- **`waiting for pickup`:** the mapping uses Pickup and the fader has not yet
  reached the control's value; move it through the value, or set the mapping's
  Fader to Jump.
- **A mapping shows Conflict:** two enabled mappings answer to the same
  source. Replace or delete one.
- **`MIDI is not available here: no MIDI service`:** on Linux there is no ALSA
  sequencer (`/dev/snd/seq`; `sudo modprobe snd-seq`; a container usually has
  none). On Windows a driver problem. The app carries on with OSC and the
  keyboard. It tries again every few seconds.
- **A MIDI device is listed but "Could not open":** on Windows another program
  (a DAW) may hold the device; close it and press Refresh. The error text is
  the driver's.
- **A device was unplugged:** `MIDI: X disconnected`, and it reconnects by
  itself when it returns, with its mappings, because they name the device by
  its name without ALSA's `20:0` suffix or WinMM's `2- ` prefix. A mapping that
  names a device spelled differently ("Any device" in the row's options matches
  every device).
- **`N input message(s) dropped (queue full)`:** the app did not drain for a
  while (a long load). Harmless for faders, which resend; a missed button press
  is the cost.
- **`show-control.json` had parts that could not be read:** the log names each
  skipped mapping (`mapping 3: unknown variant ...`); the rest loaded and the
  original is kept beside it as `show-control.json.bak`. `--show-control
  <file>` reads another file; screenshot mode reads it and never writes it.
- **A hotkey does not change:** `hotkeys: ...` warnings at start name unknown
  commands, unreadable chords and clashes. Restart to apply.
- **Testing MIDI without a controller:** there is no virtual port in a
  container. The parser is tested from recorded byte sequences
  (`strobe-io/src/message.rs`); on a desktop, `aconnect`/`amidi` (Linux) or
  loopMIDI (Windows) make a virtual port Strobe will list.
- **Driving OSC by hand:** `strobe-io`'s README has a ten-line `python3` sender
  that needs no library.
