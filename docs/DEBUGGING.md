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
| `logs/` | One session log per run, newest first, the last 10 kept. The first lines say the build, the filter, and any filter directive that did not parse. Times are seconds since process start. |
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
| `STROBE_FAULT` | Fault injection, to exercise context-loss recovery without a broken driver. See below. |
| `STROBE_NO_CRASH_MONITOR=1` | Do not start the crash monitor process or attach the native crash handler. Use it under a debugger. |
| `STROBE_DIAG_DIR` | Replace the base directory above. |
| `STROBE_REQUIRE_HEADLESS_GL=1` | Make the headless-GL tests fail instead of skip when no EGL device exists. CI's Linux job sets it. |

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

## Crashes

A panic writes `crash-*.txt` with a backtrace. A native crash writes
`crash-*.txt` and `crash-*.dmp`; the **monitor** process shows the message
box, so it works even when the app cannot open a window. A crash in the
first moments, before the monitor link is up, is not captured. A bare
`abort()` on Windows is a fast-fail that bypasses exception filters and
leaves no minidump (the `.txt` of a panic that led to it still exists).

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
