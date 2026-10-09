# strobe-diag

Diagnostics for a show that cannot be retried. Strobe runs on a projector,
often on a laptop nobody is watching, so when it hangs or dies the evidence
has to already be on disk. This crate provides a `log` backend (rotating
per-session file plus an in-memory ring for the UI console), a table of
session facts, structured failure records, a panic hook that writes
`crash-*.txt`, out-of-process native crash capture that writes a minidump,
and the `--diagnose` system report.

## Role in the workspace

- **Depends on:** no other Strobe crate. Third-party: `log`, `dirs`,
  `crash-handler` and `minidumper` (and `anyhow` for tests).
- **Depended on by:** `strobe-app`, which installs it. Nothing else uses it
  today. `strobe-gfx` and the other libraries use only the `log` facade, and
  the app installs the backend, so the dependency direction stays one-way
  ([`ARCHITECTURE.md`](../../docs/ARCHITECTURE.md), "Diagnostics wiring").
- **Does not:** show UI. The message box that tells a person about a crash is
  a callback the app registers (`notify::set_notifier`); the crash monitor
  process calls it. It also does not upload anything.
- **Holds the other `unsafe`.** `src/native.rs` contains the single unsafe
  call that builds the crash handler's callback, under the same leaf rules as
  `strobe-gfx/src/gl/` (D41).

## Main types / entry points

| Item | What |
|---|---|
| `init()` -> `Session` | Starts the clock, installs the logger and panic hook, logs the build, and starts a background system probe. `Session::log_file` is the session log path, if it could be opened. |
| `logger` | The `log` backend. `recent_lines(n)`, `records_written()` (one atomic load, so a UI can tell when to refresh), `flush(timeout)`. Keeps the newest 10 session logs and a 2000-line ring. |
| `filter::Filter` | `STROBE_LOG` parsing: `debug`, `strobe_gfx=trace,warn`, most specific directive wins. |
| `context` | Session facts every report includes: `set`, `get`, `note_frame`, `note_pass`, and keys such as `GL_RENDERER`, `GL_BACKEND` and `SCENE`. |
| `failure::{Failure, Subsystem, report}` | Structured records: `Subsystem` is `Shader`, `Audio`, `Import`, `Context`, `Script`, `Playlist` or `Io`; `Failure::new(..).scene(..).message(..).action(..)`. `failure::recent(n)` returns the last 100. |
| `panic_hook` | `install_panic_hook()` (called by `init`); writes a report with backtrace, context, failures and log tail. |
| `crash` | `run_monitor_if_requested(args)` and `start_monitor()` for out-of-process native crash capture; `MonitorHandle::wait_connected`. |
| `notify` | `set_notifier(..)` and `CrashNotice`: how a person is told. |
| `diag::system_report` | The text behind `strobe --diagnose`. |
| `paths` | `base_dir()`, `logs_dir()`, `crashes_dir()`, `sockets_dir()`. |
| `buildinfo` | Version, git hash (from `build.rs`), dirty flag, target and profile. |

## Usage

The integration order matters; it is written out in the crate docs
(`src/lib.rs`). A minimal `main`, which compiles and runs (the host crate
needs `log` as a dependency for the macro):

```rust
use strobe_diag::context;
use strobe_diag::failure::{self, Failure, Subsystem};

fn main() {
    // If this process was launched as the crash monitor, run that and exit.
    if strobe_diag::crash::run_monitor_if_requested(std::env::args()) {
        return;
    }
    let session = strobe_diag::init(); // logger, panic hook, system probe
    let _capture = strobe_diag::crash::start_monitor(); // hold it for the run

    context::set(context::SCENE, "09-sdf-cathedral");
    log::info!("log file: {:?}", session.log_file);
    failure::report(
        Failure::new(Subsystem::Shader, "link")
            .scene("09-sdf-cathedral")
            .message("unexpected token")
            .action("scene skipped; previous scene keeps playing"),
    );
}
```

`strobe-app` registers its notifier first, then follows that order
(`crates/strobe-app/src/main.rs`).

Environment variables this crate reads (the full table is in
[`DEBUGGING.md`](../../docs/DEBUGGING.md)):

| Variable | Effect |
|---|---|
| `STROBE_LOG` | Log filter; default `info` in release, `debug` in debug builds. Noisy third-party targets are capped at `warn` unless named. |
| `STROBE_LOG_STDERR=1` | Mirror log lines to stderr (debug builds do anyway). |
| `STROBE_DIAG_DIR` | Replace the base directory for logs, crash reports and the monitor socket. |
| `STROBE_NO_CRASH_MONITOR=1` | Do not start the monitor or attach the native handler (for debuggers). |
| `STROBE_GIT_HASH` | Build time only: overrides the git hash `build.rs` records. |

Default base directory: `%LOCALAPPDATA%\strobe` on Windows,
`~/.local/share/strobe` on Linux, `~/Library/Application Support/strobe` on
macOS.

## Testing

```bash
cargo test -p strobe-diag
```

`tests/crash_pipeline.rs` runs the `crash_demo` example as a real process and
checks what a panic, an abort, a stack overflow and a clean run leave in a
scratch `STROBE_DIAG_DIR`. `cargo test -p strobe-diag` builds the example
first; running `--test crash_pipeline` alone does not. The native-crash tests
are Linux-only, because on Windows `abort()` is a fast-fail that bypasses
exception filters.

The crash drill used on the T480 is the same demo:

```bash
STROBE_DIAG_DIR=target/diag cargo run --release -p strobe-diag --example crash_demo -- <mode>
```

with `<mode>` one of `panic`, `abort`, `overflow`, `panic-abort-profile`
or `ok`. The expected files per mode are in
[`DEBUGGING.md`](../../docs/DEBUGGING.md), "The crash drill".

## Design notes

- **Our own `log` backend** (D40): a bounded channel to a writer thread (a
  full channel drops and counts lines rather than blocking a frame), a ring
  for the UI, and a flush crash paths can demand. Timestamps are seconds
  since start plus a UTC session header.
- **`unsafe` in exactly one file** (D41), `src/native.rs`.
- **Release builds keep line tables and are not stripped** (D42), and PDBs are
  published separately (D53), so a minidump can be symbolicated.
- **Crashes are captured out of process** (D43): a second process (the same
  executable re-launched with `--strobe-crash-monitor`) writes the dump and
  shows the message, so it works even if the app cannot open a window. A crash
  before the link is up is not captured, and a bare `abort()` on Windows
  leaves no dump.
- **Failures are structured records** (D45), greppable blocks that the log,
  the console and a crash report share.
- **The app's console is a view of the ring** (D60), not a second log.

## Status and known gaps

- Verified end to end on Linux (the pipeline tests above). On the T480,
  `overflow` produced a `.txt` and `.dmp`, and `panic` a `.txt`; `abort`
  leaves only the session log by design
  ([`T480.md`](../../docs/hardware/T480.md)).
- Open question Q11: no `.dmp` has been symbolicated with a PDB yet, and no
  debugger is installed on the T480
  ([`OPEN_QUESTIONS.md`](../../docs/OPEN_QUESTIONS.md)).
- Under an MSIX-packaged agent (the Claude desktop app) the monitor socket
  cannot connect on Windows; set `STROBE_DIAG_DIR` inside the repo for drills.
  A normal terminal is unaffected.
- No crash upload, and no local-time timestamps (D40, D43).
