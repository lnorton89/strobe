# Decisions

This is an append-only log of choices that aren't obvious from the code.
Each entry has a date, the decision, why it was made, and what would
reopen it. Newest entries are at the bottom. To reverse a decision, add a
new entry that says so; don't edit the old one.

---

### D1: native OpenGL (glow + glutin), not wgpu (2026-10-08)

Scenes and Shadertoy shaders are GLSL. Native GL hands GLSL to the
driver's compiler, which is the model Synesthesia and Shadertoy (WebGL)
both use. wgpu ingests GLSL through naga, which rejects samplers passed
as function parameters, combined image/samplers, and some `const int`
switch patterns that real Shadertoy code uses (see PLAN.md §2 D1).

**Would reopen:** naga's GLSL frontend reaching parity, or macOS removing
OpenGL.

### D2: ANGLE as the fallback context, not native-only (2026-10-08, owner's choice)

The owner's `rustaman` app found that WGL fails on hybrid-graphics and
stale-driver laptops, which are exactly Strobe's target machines. ANGLE
translates GLES 3.0 to Direct3D 11, which is what Chrome uses to run
Shadertoy on Windows. glutin's EGL backend loads `libEGL.dll` on
Windows, so the ANGLE path is the same code as the Linux EGL path
pointed at ANGLE's DLLs. Native GL stays the first choice because it's
faster, and desktop GLSL is what SSF scenes are written in.

**Cost:** SSF scenes that rely on desktop-only GLSL (implicit
conversions) need source fixups on the ANGLE path. That's not built
yet; see OPEN_QUESTIONS.md.

### D3: GL 3.3 hardware floor (2026-10-08, owner's choice)

This means Intel HD 4000 / Ivy Bridge (2012) on Windows 10. Shadertoy
`#version 300 es` code is rewritten to `#version 330` when
`ARB_ES3_compatibility` is absent.

### D4: cargo workspace with rustaman's rules (2026-10-08, owner's choice)

Crates give fast incremental builds and hard boundaries (audio can't
touch GL). The lints, header banners, unsafe-leaf discipline, pinned
toolchain, cargo-deny, and LF line endings all come from
`lnorton89/rustaman`.

### D5: port rustaman's UI system (2026-10-08, owner's choice)

That means the themes.toml catalog with WCAG tests, motion durations,
the spacing scale, geometry icons, and the custom title bar. The default
theme is dark with a single amber accent. Not built yet.

### D6: `#version 330 core` on every desktop context (2026-10-08)

Real SSF scenes compare `PASSINDEX == 0.0` (int vs float) and write
`float x = 2;`. Desktop GLSL converts implicitly; GLSL ES doesn't.
Every 4.x core context, including macOS 4.1, accepts 330. One dialect
means one set of behaviours.

### D7: one program per pass, `PASSINDEX` a constant (2026-10-08)

A fragment program's register allocation, and therefore its occupancy,
is set by its heaviest path. If a cheap blur pass shares a program with
a raymarcher, the blur runs at the raymarcher's occupancy. Specialising
lets the driver delete untaken branches. The trade-off is N compiles
instead of one at load time, which is fine because playlists precompile.
`Options::specialise_passes = false` keeps the faithful mode, and the
bench measures both (`--shared`).

### D8: the final pass is implicit (2026-10-08, verified)

Verified against real community scenes: a scene listing two `PASSES`
renders `PASSINDEX` 0 and 1 into them, then `PASSINDEX == 2` produces
the output. `SceneDef::passes` always ends with that final, untargeted
pass.

### D9: frame-rate-independent smoothing (2026-10-08)

SSF documents smoothing as "about `1/PARAMS` frames", which is a
per-frame exponential step. Per-frame steps mean a scene tuned at 60 fps
feels twice as sluggish at 30 fps, which is exactly what happens on an
old laptop. We use `alpha = 1 - (1 - PARAMS)^(dt*60)`, which is
identical at 60 fps.

### D10: original benchmark scenes (2026-10-08)

The owner asked for 10 scenes spanning GPU load. Shadertoy is
unreachable from the cloud environment (Cloudflare 403 on every
request), and the community Synesthesia scenes found on GitHub carry no
license. So the base library in `scenes/` is original and MIT-licensed,
written to span five load tiers on purpose (see BENCHMARKS.md).
Unlicensed community scenes are used only as local, uncommitted
compatibility fixtures.

**Would reopen:** a source of permissively licensed SSF or Shadertoy
scenes, or the owner supplying their own scenes.

### D11: pass sizes as fractions of the design resolution (2026-10-08)

`"WIDTH": 960` in a 1920-wide scene is stored as `0.5` and multiplied by
the actual render width. That way render scale (the main lever on old
GPUs) shrinks every buffer in proportion instead of leaving 1080p
buffers behind a 720p output.

### D12: ANGLE comes from a pinned Electron release (2026-10-08)

ANGLE doesn't publish binaries, and building it needs Chromium's
`depot_tools` toolchain. Every official Electron `win32-x64` release ships
Google's ANGLE build (`libEGL.dll`, `libGLESv2.dll`) plus
`d3dcompiler_47.dll`, under BSD/MIT licenses with
`LICENSES.chromium.html`. `scripts/fetch-angle.ps1` downloads Electron
v38.2.0, checks the archive's SHA-256 against the release's
`SHASUMS256.txt`, checks each DLL's SHA-256, and copies them beside the
executables. To upgrade, bump the version and all four hashes in one
commit.

**Would reopen:** a need for ANGLE features newer than Electron ships, or
bundle size (about 13.5 MB for the three DLLs) becoming a problem.

### D40: our own `log` backend in `strobe-diag`, not tracing or env_logger (2026-10-08)

The logger has to satisfy four things at once: no file or console I/O on
the render thread, a ring buffer of recent lines readable by the UI
console and by crash reports, a rotating per-session file, and a flush
that crash paths can demand. `env_logger` gives none of the ring or the
flush, and `tracing` brings a subscriber stack for spans Strobe does not
use. The backend is about 400 lines over the `log` facade the other
crates already use: a bounded channel to a writer thread (a full channel
drops and counts lines instead of blocking a frame), a mutex-guarded ring
held for one `push_back`, and `STROBE_LOG` filtering (`debug`,
`strobe_gfx=trace,warn`). Timestamps are seconds since process start plus
a UTC wall-clock session header and UTC file names, because `std` cannot
read the local UTC offset and a date/time dependency for one file name is
not worth it. Ten session logs are kept.

**Would reopen:** a need for spans or structured fields in logs, or a
local-time requirement.

### D41: unsafe is allowed in exactly one `strobe-diag` file (2026-10-08)

Native crash capture needs `crash_handler::make_crash_event`, which is
`unsafe` because the callback runs in a signal handler or exception
filter. (`CrashHandler::attach` is a safe function in crash-handler
0.8.1, so there is one unsafe call, not two.) Rather than widen the rule
to a crate or a directory, `strobe_conventions::unsafe_boundary::ALLOWED`
is now a list holding `crates/strobe-gfx/src/gl/` and the single file
`crates/strobe-diag/src/native.rs`; an entry ending in `/` is a
directory, anything else is one file. The same leaf rules apply (one
operation per private safe function, a real SAFETY comment, no `pub
unsafe fn`, none in tests). The callback itself is safe code in
`crash.rs`, written to take no lock it could wait on (`try_lock` only)
and to do no blocking work besides local socket writes and a
timeout-bounded log flush.

**Would reopen:** a second unsafe need in diagnostics, which should be
another single file in this list rather than a broader exemption.

### D42: release builds keep line tables and are not stripped (2026-10-08)

A minidump from a release build is only as readable as its symbols.
`[profile.release]` was `strip = "debuginfo"`; rustc turns that into
`/DEBUG:NONE` for the MSVC linker, which writes no PDB at all (this is
rustc's behavior as I understand it; check by looking for
`target\release\strobe_app.pdb` after a release build and reopen if it
exists). It is now `debug = "line-tables-only"` and `strip = "none"`.
Line tables give function names and `file:line` in backtraces, panic
reports and WinDbg stacks for a small size cost, with no variable info
and no effect on the optimizer. On Windows the symbols are a separate
`.pdb` that the loader never reads, so the shipped `.exe` does not grow;
the PDB for each release build must be archived with it (see DEBUGGING.md).
On Linux the line tables are inside the binary, so release binaries are
larger there.

**Would reopen:** binary size on Linux or macOS becoming a problem (use
`split-debuginfo`), or a need for local-variable inspection (use a
separate profile, not release).

### D43: native crashes are captured out of process, and the monitor tells the person (2026-10-08)

An access violation inside a GPU driver, a stack overflow or a C++
`abort()` leaves the crashed process unfit to write a report. So
`crash::start_monitor` re-launches the current executable as
`--strobe-crash-monitor <socket>`; the app attaches a `CrashHandler`
whose callback sends the session context, recent failures and log tail
over the minidumper channel and requests the dump; the monitor writes
`crash-*.dmp` and a companion `crash-*.txt`. Consequences worth knowing:

- The notifier (the app's message box) runs in the **monitor**, not the
  crashed process, so it works even when the app cannot open a window.
  `notify::set_notifier` therefore has to be called before
  `run_monitor_if_requested`.
- The monitor connects on a background thread and `start_monitor` returns
  at once, so startup never waits on a slow antivirus scan of a new
  process; a crash in the first moments before the link is up is not
  captured. The handler must still be attached on the main thread (it
  installs the main thread's signal stack for stack overflows).
- A panic followed by `panic = "abort"` produces both a panic report and a
  native abort. The panic hook records its report path; the monitor reuses
  its stem (`crash-X.dmp` beside `crash-X.txt`), appends a pointer to the
  report and does not announce a second time.
- On Windows `std::process::abort()` is a fast-fail that bypasses
  exception filters, so a bare abort leaves no minidump there. Panics
  still leave their text report, and access violations and stack
  overflows are captured. Windows Error Reporting remains the fallback
  for a bare abort.
- The IPC socket is a Unix-domain socket (abstract on Linux), which on
  Windows needs version 1803 or later. The T480 is Windows 10.
- `STROBE_NO_CRASH_MONITOR=1` disables all of it, for debugger sessions.
- The monitor installs no logger, so it cannot rotate the app's session
  logs away; its own trouble goes to stderr (null when spawned).

**Would reopen:** needing capture before the connection is up, or
crash-upload support, which wants a proper crash-reporting service.

### D44: GL debug output is installed through `&mut Gl` right after context creation (2026-10-08)

`glow` 0.17 installs `glDebugMessageCallback` through
`&mut Context` and panics if a callback is installed twice. `Gl` holds
its context in an `Arc` so it can be cloned, and the only mutable access
is `Arc::get_mut`, which works only while unshared. So
`gl::debug::enable_debug_output(&mut Gl, DebugLevel)` must be called
immediately after the context is created, before cloning the `Gl` or
giving `Gl::glow()` to egui; otherwise it returns `false` and logs why.
`Headless::gl_mut` was added for that (a four-line addition to
`context.rs`); a windowed provider owns its `Gl` by value and needs no
such accessor. Polling `get_debug_message_log` instead is not possible:
glow's `DebugMessageLogEntry` has private fields and no accessors. Calling
through a pointer derived from `&Gl` was rejected as undefined behavior.
Messages go to the `log` target `gl_driver` (high is error, medium warn,
low info, notification debug), each distinct (source, type, id) is limited
to 5 per second with a suppressed-count note, and the last 64 are kept for
the UI. Debug builds also enable `GL_DEBUG_OUTPUT_SYNCHRONOUS` so a
breakpoint in the callback has the offending call's stack.

**Would reopen:** a glow release that exposes the callback through
`&self`, or accessors on its log entries.

### D45: failures are structured records, not log strings (2026-10-08)

Every user-visible failure should say what failed, in which scene or
asset, at which path, in which phase, why, and what the program did about
it (AGENT_RESEARCH.md section 12). `failure::Failure` carries exactly
those fields, renders them as a greppable block (`SHADER` / `scene=` /
`phase=` / `message=` / `action=`), logs it at error level, and keeps the
last 100 in a bounded list for the UI (`failure::recent`). Crash reports
include the recent ones. Reporting a failure formats a string and takes
two short locks; it is for things that already went wrong, never called
per frame, and nothing in the renderer's hot path depends on it.

**Would reopen:** a need to localise failure text or to attach machine-
readable error codes.
