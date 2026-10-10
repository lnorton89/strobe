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

### D20: the analyser has two clocks (2026-10-08)

Spectral analysis runs on audio time (one step per 512-sample hop), and
presentation runs on the caller's `dt`. A hop only writes *targets*
(normalised levels, hit peaks, beat events); `analyze(dt)` then glides,
decays and integrates toward them with `1 - exp(-dt/tau)`. Hits keep the
peak of every hop since the last call so a 20 fps render loop cannot miss
a kick. The result is the same frame-rate independence as D9, and the
analyser stays a pure function of (samples, dts), which is what makes
the WAV harness and the tests exact.

### D21: tempo by autocorrelation comb, with an octave prior (2026-10-08)

Peak-picking the autocorrelation lands on any harmonic of the beat. Each
candidate tempo (50 to 220 BPM, every 0.25) is instead scored by the
mean autocorrelation at its first four multiples, times a soft prior
centred on 120 BPM (one octave sigma). The biased autocorrelation also
favours the shorter period. A clean track resolves to the tapped tempo;
a track near the range edges can still report double or half time, which
tap tempo and nudge exist to override. Confidence is peak sharpness times
a stability term.

### D22: capture is one thread that owns the stream (2026-10-08)

cpal streams need not be `Send` on every backend, so the analysis thread
builds and keeps the stream, and handles device loss itself:
Running, Recovering, SilentFallback, retrying every 2 s. A missing device
is an `Err` from `Capture::start` and a silent, retrying handle from
`Capture::start_resilient`; the render thread only does an atomic pointer
load (`Capture::latest`). Windows loopback is cpal's behaviour of
building an input stream on a render device, so output devices are listed
(on Windows only) as `DeviceKind::Loopback`.

### D23: texture widths are a `Layout`, not constants (2026-10-08)

The real Synesthesia texture layout is unverified (OPEN_QUESTIONS Q10),
so `AudioFrame` carries `Vec`s sized by `Layout` (default 256 spectrum,
256 trail, 512x2 Shadertoy) and every texel is built from a bin span
computed from the layout. Changing a width changes data, not code.

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

### D50: repository tooling is an `xtask` crate, not examples or scripts (2026-10-08)

rustaman and rustdirstat keep their changelog generator as
`cargo run --example changelog`. In a single-package repo that is fine;
in a workspace an example belongs to one package, so workspace tooling
would be parked in whichever crate was convenient. `crates/xtask`
(`cargo xtask ci | changelog | bench`, via an alias in
`.cargo/config.toml`) is built and linted by the same `--workspace`
commands as everything else, can have unit tests (the changelog parser
does), and runs the same on Windows, where a shell script would not. The
alias is the whole install step. `cargo xtask ci` exists so "does CI
pass" is one local command in CI's order; a step that cannot run
locally (Windows target, cargo-deny) is reported as skipped with the
fix, never silently dropped.

**Would reopen:** the tooling needing a dependency heavy enough to slow
`cargo build --workspace`; it would then move behind a separate
workspace.

### D51: `CHANGELOG.md` is generated from conventional commits (2026-10-08)

Ported faithfully from rustaman: released sections only are checked
(`Unreleased` moves with every merge), a changelog-only commit is
excluded so the release commit need not list itself, `--release vX.Y.Z`
writes the section before the tag exists, and a version tag no branch
can reach fails `--check`. Differences: the repository URL is read from
`[workspace.package]` rather than hard-coded, the version check reads
`[workspace.package]` rather than the first `version =` line, and
merge commits are excluded (`--no-merges`) because their subjects are not
conventional while the commits they bring in are listed individually.
This repo's history is merge-based (the coordinating session merges
agent branches), so this matters: a squash merge would instead list one
commit, whose title should then be conventional.

### D52: release flow is draft, build, attest, undraft (2026-10-08)

`.github/workflows/release.yml` follows rustaman's: a tag push creates a
draft release, the Windows build attaches archives, SBOM and SHA-256
files, and only the final job (the one with `id-token` and
`attestations: write`) attests provenance and the SBOM and undrafts, so
a failed build leaves a draft. Additions over rustaman: the draft job
fails fast if the tag disagrees with the workspace version; the release
notes are the tag's own `CHANGELOG.md` section (generated notes are the
fallback); the tag name reaches shell steps through `env`, never by
interpolation, because a manual run takes it as free text in a job
holding a write token; the program archive and the symbols archive are
both attested, but only the program archive is the subject of the SBOM
attestation. Tags are never moved (the changelog and the release assets
both depend on it).

### D53: PDBs are published separately, and the release build must override `strip` (2026-10-08)

Crash minidumps (written to `%LOCALAPPDATA%\strobe\crashes\`) are
unreadable without the matching `strobe.pdb`. The workspace release
profile sets `strip = "debuginfo"`, and for the MSVC linker rustc turns
that into `/DEBUG:NONE`, so **no PDB is produced at all**. The release
workflow therefore builds with `CARGO_PROFILE_RELEASE_STRIP=none` and
`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` (function names and
lines, not types) and ships the PDB as `strobe-<tag>-<target>-symbols.zip`
beside, never inside, the user's zip. The code and `strobe.exe`'s size
are unchanged. This is done in the workflow, not `Cargo.toml`, so local
and CI builds keep the profile the diagnostics work was tuned against;
the cost is that the shipped binary differs from CI's `--release` build
in symbols only. Keep every release's symbols archive for as long as
that release is supported: a PDB only matches its own build.

**Would reopen:** the diagnostics crate choosing a symbol server or a
different crash-report format.

### D54: actions pinned by SHA, permissions declared, both enforced by a test (2026-10-08)

rustaman pins by SHA and asserts the toolchain by test, but nothing
asserts the pinning, so a new action added with `@v4` would pass.
`strobe_conventions::actions` fails the build for a `uses:` that is not a
40-hex SHA followed by a version comment, and for a workflow with no
top-level `permissions:` (least privilege is the starting point; `draft`
and `publish` elevate explicitly). The toolchain test already read every
file in `.github/workflows/`, so `release.yml` is covered by it; a new
test pins that `ci.yml` and `release.yml` are both seen and both name a
toolchain.

### D55: macOS CI is two-tier until strobe-gfx has a CGL context (2026-10-08)

glutin compiles its EGL backend out on Apple targets (`not(apple)`), and
`strobe-gfx`'s headless context is written against it, so the renderer
crates cannot build on macOS yet. The macOS job therefore blocks on the
crates with no GL context (clippy and tests, excluding `strobe-gfx`,
`strobe-bench`, `strobe-app`) and runs a whole-workspace clippy with
`continue-on-error` as a running list of what the port still needs.
Renderer tests would self-skip on a Mac runner anyway (no EGL), so the
Linux job stays the place shaders are compiled.

**Would reopen:** a CGL (or ANGLE-on-Metal) provider landing; delete the
excludes and the `continue-on-error` then.

### D56: CI keeps the rendered frames (2026-10-08)

The Linux job runs `strobe-bench` at 640x360 with 10 timed frames and a
60-frame warm-up, once per GLSL dialect (desktop 3.3 and GLES 3.0), and
uploads both output folders (`bench-desktop`, `bench-gles`) as artifacts
for 14 days. Frames are a pure function of time, so the contact sheet is
a visual diff any reviewer can open from a pull request; llvmpipe
timings are CPU numbers, but the ranking of scenes is meaningful. A scene
that fails to compile in either dialect fails the job.

### D57: issue forms, and CODEOWNERS on one line (2026-10-08)

Issue templates are forms (`.yml`), as in rustaman, tailored to a
graphics app: GPU and driver version, context backend (native or ANGLE),
scene, render resolution, and attachments for the crash report, the log
and `strobe --diagnose` output. rustaman's CODEOWNERS puts `*` and
`@lnorton89` on separate lines, which GitHub reads as a pattern with no
owner followed by a pattern named `@lnorton89`; Strobe's is the working
single-line form.

### D58: Dependabot ignores nothing, and cannot see ANGLE (2026-10-08)

The ANGLE version is a constant in `scripts/fetch-angle.ps1`, not a
manifest, so no Dependabot ecosystem can bump it; the pin stays at
Electron 38.2.0 because Electron 44+ no longer ships the DLLs
(`AGENT_RESEARCH.md` section 8). The egui, glow, glutin and winit crates
are released in lock-step, so their individual PRs will not build alone;
the config says so rather than ignoring them, and they are taken as one
deliberate upgrade.

### D59: the SBOM describes `strobe-app` for the Windows target (2026-10-08)

`cargo-cyclonedx` 0.5.9 (pinned) has no `--package`, and writes a file
per workspace member whatever manifest it is given. The release step
points it at `crates/strobe-app/Cargo.toml` with `--target
x86_64-pc-windows-msvc` and `--override-filename strobe.cdx`, and
copies only that file out, so the SBOM lists the crates in `strobe.exe`
and not the Linux-only ones a host-target resolve would add.

### D70: the bench gets a window context rather than a different GL path (2026-10-09)

Intel's Windows driver exposes no EGL devices, so `Headless` cannot
open there, and real-GPU numbers are the point of the bench.
`strobe-bench --context window` opens one hidden winit window through
`WindowedContext`, the app's own provider, so the bench measures the same
native (WGL) and ANGLE contexts the show runs on, not a third path.
winit 0.30 only creates windows inside the event loop, so the whole
benchmark runs inside `resumed` and the loop exits when it returns; the
per-scene loop is one function taking `&Gl` and `&Caps` that both modes
call. `--context auto` is the default: headless when a device exists
(CI and cloud containers behave exactly as before), the window
otherwise. The window never presents; scenes render into the player's
offscreen targets, so size, visibility and compositor do not affect the
frames or the timings, and the output is identical to headless (checked
on llvmpipe, desktop and GLES).

### D71: hidden window first, tiny off-screen window as the fallback (2026-10-09)

Some platforms will not give an invisible window a GL surface (a Wayland
surface with no commit), and a visible window can be throttled or
re-composited by others. The bench tries a hidden window first and, if
the context cannot open on it, a 16x16 undecorated window parked at
(-32000, -32000); `--show-window` starts at the second. Both were
exercised under Xvfb and produce identical frames. Only the first attempt
that opens is used, and `results.md` says which. The surface's swap
interval is set to zero even though nothing is swapped, so a driver that
ties other synchronisation to the interval has no reason to throttle the
timed frames.

### D72: `--api` is optional, and in window mode the backend decides the dialect (2026-10-09)

Headless chooses its dialect with `--api desktop|gles`. A window context
gets its dialect from the backend (native is GLSL 330, ANGLE is GLES
3.0), so `--api` became an `Option`: unset means desktop for headless and
whatever `--gl` implies for a window. `--api gles` with `--gl auto` means
ANGLE; `--api gles` with `--gl native`, or `--api desktop` with
`--gl angle`, is an error, not a silent override.

### D13: float passes default to RGBA16F (2026-10-09, measured)

SSF specifies RGBA32F for `FLOAT` passes. On the T480's Intel UHD 620,
RGBA16F made the bandwidth-bound reaction-diffusion scene 2.8× faster
(4.47 → 1.59 ms at 720p), and no scene in the library was meaningfully
slower (BENCHMARKS.md, 2026-10-09). Integrated GPUs share system memory
bandwidth, and that is the wall on Strobe's target machines.
`FloatPrecision::Full` and the bench's `--full` keep 32-bit available.

**Would reopen:** a real scene whose simulation visibly degrades in 16
bits (accumulators, positions in world units). The fix then is a
per-scene override, probably in `X_STROBE`, not a global flip back.

### D14: per-pass specialisation stays the default, despite being neutral on Intel (2026-10-09, measured)

On the UHD 620, one program per pass vs one shared program is within
noise on frame time. On llvmpipe, specialisation made Bloom Orbs 35%
faster. It never measured slower, so it stays the default (D7). Its cost
is compile time on multi-pass scenes (07: 154 ms vs 57 ms), which
playlist precompilation hides.

**Would reopen:** compile time becoming user-visible, e.g. scene switches
without precompile. Then shared mode is the right default for
interactive browsing, and specialisation becomes a background upgrade.

### D60: the app's console is a view of strobe-diag's ring, not a second log (2026-10-09)

`strobe-app` once kept its own `strobe.log` and its own console buffer.
Both are gone. The app installs `strobe_diag::init()` and writes by
logging (`log::info!`) or by reporting a structured failure
(`strobe_diag::failure::report`); the bottom panel is a view of
`strobe_diag::logger::recent_lines`, so the console, the session file and a
crash report's log tail are the same text. The view refreshes only when
`logger::records_written()` (a new atomic counter, one relaxed load) moves.
Shader compile failures, scene load errors, resize failures and startup
failures are `Failure`s (`failures::scene_load` is the one conversion, and a
unit-tested one). The startup dialog and the crash notifier share
`dialog::show_error`.

Two supporting choices. `Filter::from_env` now caps a short list of noisy
third-party targets (`winit`'s `tracing` span bridge, egui, glutin, ...) at
`warn` unless `STROBE_LOG` names them, because the debug-build default of
`debug` otherwise produced about a hundred lines per frame of window calls.
And GL driver debug output is on at *warnings* in debug builds (synchronous)
and *errors only* in release, `STROBE_GL_DEBUG` overriding either way
(`gldebug::level`): errors are the evidence a crash report needs, and a
conformant driver with nothing to report pays nothing.

`--diagnose` probes GL with a headless EGL context, because the windowed
context needs the event loop. That cannot see a WGL-only Windows driver, so
the report says so and points to the session log, where the windowed
context's facts are recorded. The report is also saved as `diagnose.txt`
beside the logs because the release build has no console on Windows.

**Would reopen:** a need for the console to show only some subsystems (a
filter UI), which wants the structured failure list rather than ring text.

### D61: audio is the app's, started on a thread, falls back to synthetic by capture state (2026-10-09)

`AudioInput` (`strobe-app/src/audio.rs`) lives for the whole run, not per
scene, so switching scenes neither restarts capture nor loses a tapped
tempo. Opening a device can take seconds on WASAPI, so
`Capture::start_resilient` and `list_devices` each run on a short-lived
thread and the render thread only `try_recv`s the result; synthetic audio
plays until a device is open. After that the active source follows the
capture's atomic state: `Running` is live, `SilentFallback` is synthetic,
`Recovering` keeps whichever was active (so a one-second device hiccup does
not flash the synthetic track). `start_resilient` is used rather than
`start` because it keeps retrying, so a device plugged in later takes over
without a restart. Each change is logged once and shown in the status bar;
losing a device that had worked is an `AUDIO` failure, never having had one
is a single warning (a laptop with no microphone is not a fault).

The synthetic source also fills `syn_Spectrum` and `syn_LevelTrail`
(`strobe_scene::synthetic::fill_textures`), shaped like the analyser's
channels, so a scene that draws the spectrum is not dead without a device.
`ScenePlayer::upload_audio(&AudioTextures)` takes plain `[f32; 4]` slices so
`strobe-gfx` still knows nothing of the audio crate; it quantises into
staging buffers kept between frames, uploads only a texture some pass
samples, and reallocates a texture once if the slice width differs. `glow`
has no `glTexSubImage1D`, so the desktop path re-specifies the 1 KB 1D
texture with `glTexImage1D` each frame, which is not a measurable cost.

`--audio synthetic` forces the fixed pattern. Screenshot mode does not:
under `--audio auto` it attempts capture too, so the fallback is exercised
and its status line is honest; pass `--audio synthetic` for reproducible
frames on a machine that has a device.

**Would reopen:** a measured upload cost on a real Intel HD 4000, or a
Windows device-change notification (the `wasapi` crate) replacing the
2-second retry.

### D62: script uniforms are compiled in at load, from the script's first frame (2026-10-09)

A shader cannot use a uniform nobody declared, and the roadmap promises no
shader compilation during playback. So `Session::load` builds the
`ScriptHost` and runs its **first frame before it builds the player**: with
the real audio at the scene's start and a 60 Hz step, not as a throwaway
probe, so the scene's second frame is the script's second frame. Whatever the
script published in `setup()` or that first `update` is declared in every pass
program (`ScenePlayer::with_extras`, which resolves their locations at link
time like all other uniforms) and sent each frame with
`set_script_uniforms`, a copy into a reused buffer. `FrameInputs` was left
alone on purpose: the benchmark builds it by struct literal, and a new
required field would have broken a crate this work does not own.

A script that publishes a *new* uniform later (only once a beat is heard, say)
costs one recompile at that moment, logged as a warning and never repeated if
it fails; the alternative is a shader that cannot compile at all. Names the
scene already declares (pass targets, images, cubemaps, transition uniforms;
`strobe-script` itself already refuses control names) are dropped with a
warning, because redeclaring one is a compile error that would take the scene
down. A `script.js` that fails to load is a structured `SCRIPT` failure and
the scene runs without it. A faulting script shows its state beside the scene's
controls ("script.js fault (2 in a row): ...", or "disabled ... reload the
scene"), and the first fault of a streak and the disabling are `SCRIPT`
failures; the script's own console goes to the log under the target `script`.

The shipped script scene, `11-script-orbits`, drives its picture through
`setControl` on four `xy` controls rather than `setUniform`, because
`strobe-bench` loads every scene in `scenes/` without running scripts, and a
shader that needs a script-created uniform does not compile there. The
`setUniform` path is covered by `crates/strobe-app/tests/script_uniforms.rs`
on a headless context in both dialects instead. Without the script the
scene's controls sit at their defaults, a valid still picture.

Not provided yet: the previous frame's 64x36 downsample behind
`textureFinalPass`. Producing it means a GPU readback every frame, which
stalls the pipeline on the hardware this targets; Q14 records what it wants
(an asynchronous PBO readback a frame or two late).

**Would reopen:** the bench growing a script host (then scenes could use
`setUniform` freely), or a measured readback cost that makes
`textureFinalPass` affordable.

### D63: the Windows cross-check from Linux now excludes `strobe-app` too (2026-10-09)

The check was `--exclude strobe-script` because rquickjs compiles QuickJS's C
sources, which needs MSVC and the Windows SDK. Once `strobe-app` depends on
`strobe-script`, excluding the script crate no longer keeps it out of the
build, so the command fails in `rquickjs-sys`'s build script. Options weighed:
making scripting a cargo feature of the app (a stub `ScriptRun`, `cfg` at
every call site, and `--all-features` would turn it back on), faking an MSVC C
toolchain with clang (the SDK headers are still missing), or excluding the app.
The last costs almost nothing: the app has no `#[cfg(windows)]` code, only
`cfg!(target_os = "windows")` values, which the Linux build type-checks
anyway, and `strobe-gfx` (WGL) and `strobe-audio` (WASAPI), which hold the
Windows-specific code, stay in the cross-check. The native `windows` CI job
(and the T480) remain the real proof for the app and the script crate.
`xtask ci`, `ci.yml`, `CLAUDE.md` and `CONTRIBUTING.md` all carry the new
command.

**Would reopen:** adding `#[cfg(windows)]` code to the app, which would then
want a feature gate or a Windows runner for the check.

### D64: GPU timing is a four-slot query ring that never waits; adaptation waits a second after a load (2026-10-09)

`strobe_gfx::GpuTimer` keeps four `GL_TIME_ELAPSED` queries in flight. `begin`
skips the frame when all four are still pending (a missing sample costs the
controller nothing; queueing more would grow without bound), and `poll` reads
only results the driver reports available, oldest first, stopping at the first
that is not, so no read ever blocks (AGENT_RESEARCH section 7). It is the one
per-frame path that makes driver queries (`glGetQueryObject`, and on ES the
`GPU_DISJOINT_EXT` flag, checked only when a result was read and discarding the
batch if set); capability facts stay decided once (`Info::gpu_timer`: desktop
3.3+ or `ARB_timer_query`, ES with `EXT_disjoint_timer_query`). Results above a
second are discarded as driver glitches. 32-bit `glGetQueryObjectuiv` is used
for the result on both APIs (saturates at 4.29 s), so there is one code path.
On a context without timers the toggle is disabled and the scale stays manual.

`strobe-app::perf` wires it to `AdaptiveScale`. Budget is one refresh period of
the output window's monitor (winit `refresh_rate_millihertz`, re-read with the
monitor list every two seconds), 16.67 ms when unknown. The render-scale slider
becomes the **maximum** when Adaptive is on (default on; `--no-adaptive`), and
moving it rebuilds the controller at the new ceiling. A scene load forgets the
measurements (`AdaptiveScale::reset_measurements`, new) and holds the scale for
one second of wall-clock time, because a new scene's first frames pay for
compiling and allocating and the old average describes a different scene.
Every change is logged with the GPU time that caused it. The timer brackets the
scene's work and the blit to the output, not egui. Verified under llvmpipe
(which exposes timer queries in both dialects): the heavy cloud scene dropped
1.00 to 0.25 at 188 ms against 16.7; real-GPU behaviour is unmeasured
(AGENT_RESEARCH 14.G says to wait for hardware data before trusting the
constants).

Each resize still reallocates targets and restarts feedback from black; that
cost is accepted, and the controller's cooldown and quantisation limit how
often it happens.

**Would reopen:** T480 data showing the 0.70/1.05 thresholds or the 12-frame
drop delay are wrong, or a visible hitch from restarting feedback on a change.

### D65: the control window always opens maximized (2026-10-09)

Owner requirement: `WindowAttributes::with_maximized(true)` on the control
window on every launch (nothing is remembered between runs, so "not just the
first" is automatic). The output window is unchanged. The 1360x820 inner size
stays as the fallback for a window system that cannot maximize; bare Xvfb has
no window manager, so under `--screenshot` the control window stays 1360x820
and captures are still deterministic, which is why screenshot mode needs no
exception.

### D66: a lost or unusable GL context is rebuilt, on ANGLE if native keeps failing (2026-10-09)

T480 bug 1: native `make_current` fails mid-show (`The handle is invalid`,
os error 6 or 2004) on the heaviest scene, ANGLE does not, cause unknown (a
silent driver reset is the leading guess). The app used to end on a failed
`make_current` and log a failed swap. It now recovers:

- **Robust context first.** `RobustLoseContextOnReset` is requested on both
  backends, with a fallback to default attributes (logged). Without it a
  driver reset is not reported and the context carries on with undefined
  results.
- **Reset status through the glow loader.** glow 0.17 has no
  `GetGraphicsResetStatus`; `Gl` loads it (plain, `ARB`, `KHR`, `EXT` name,
  gated by version and extension because Mesa's loaders return stubs for any
  name) as an address and calls it in one leaf. It is queried after a failed
  swap and once a second, never per frame, and never after a failed
  `make_current` (the context may not be current).
- **Lost flag, no GL in `Drop`.** `Gl::mark_lost()` makes every owning type
  skip its delete, and egui's painter is forgotten rather than destroyed. The
  driver frees a destroyed context's objects; a delete against a dead
  context can fault in the ICD.
- **The policy is a pure state machine** (`recovery.rs`), unit-tested without a
  GPU. Three consecutive failed frames on one window, or any reset report, mean
  rebuild (a single failure is often a window being moved or minimised; a
  reset report is unambiguous). A second native rebuild inside 60 s, or a
  failed native rebuild, fails over to ANGLE, but only under `--gl auto`: a
  forced backend is retried, never abandoned. ANGLE failing twice in 60 s or
  failing to rebuild means give up through the existing failure dialog; any
  backend stops after four rebuilds in 60 s rather than thrash the GPU.
- **Windows are rebuilt with the context.** WGL sets a window's pixel format
  once, so a window that has had a context is not reused. The old context is
  also made not current before it is dropped: under glvnd an EGL context
  cannot become current while a GLX one is (found by the fault run: the
  native to ANGLE fail-over failed with `EGL_BAD_ACCESS` until fixed).
- **State carried over**: the output's fullscreen state and window geometry, the
  control window's maximized state, the adaptive render scale (restarting from
  the ceiling would reload the load that may have caused the loss), and the
  scene at its clock with its control targets. Audio, library and console are
  not GL state.
- **`STROBE_FAULT`** (`lose-context@N[,N...]`, `fail-rebuild:native|angle`)
  injects a loss so the whole path runs under Xvfb and in CI.

**Unverified:** real WGL behaviour. Whether `wglMakeCurrent(NULL)` on a lost
Intel context is safe, whether the T480's failure is a driver reset
(`reset_status` will say), and whether three failed frames is the right
threshold there. The T480 check is in `docs/hardware/T480.md`.

**Would reopen:** T480 data showing recovery thrashing (a loss every few
seconds on ANGLE), a swap failure on the control window alone causing needless
rebuilds (it counts like the output's), or the robust request itself costing
frame time on Intel (robust buffer access can).

### D73: the playlist model is authored data, resolved once into a flat view (2026-10-09)

`strobe-playlist` keeps two shapes. `Playlist`/`Section`/`Cue`/`CueSettings`
is what a person wrote, every inheritable field an `Option`. `resolve()`
walks cue, section, playlist, built-ins (180 s, timer, 2 s crossfade) and
yields `ResolvedCue`s with nothing optional and a record of which level
supplied each field, so the UI can show "inherited" and nothing else repeats
the rules. `meta` is the one field merged key by key rather than replaced.
Choices worth knowing:

- A cue's `transition` is how the show *arrives at* that cue. The first cue
  cuts in; a looping wrap uses cue 0's transition from the last cue. A
  transition is capped at its cue's length, so cues stay contiguous and
  total runtime is the plain sum of durations.
- Control overrides use a crate-local `ControlValue` (float, int, bool, vec2,
  colour, text), not `strobe-scene`'s runtime types, so the file format does
  not move when the scene crate does. The app converts at the boundary.
- `[shuffle]` supplies the seed for `rand(a, b)` durations and the generator's
  defaults. It does not reorder authored cues at run time. A runtime shuffle
  of authored cues is possible later and would not change the file format.
- Random durations are drawn once per cue from `derive(seed, cue position)`,
  identical on every pass of a loop. That makes the editor's preview exact and
  the wall-clock lookup a modulo. Cost: a looping show repeats its lengths.
- The resume identity hashes name, section names, and each cue's scene and
  enabled flag, not timing, so hand-editing a duration keeps the saved
  position and adding or reordering cues drops it.
- The TOML reader goes through serde with `Spanned` leaves and converts by
  hand, so every error carries line, column and the offending text, and
  unknown keys are errors. The writer is hand-rolled for a fixed, readable
  layout (one cue per line).

**Would reopen:** a need for per-pass random durations, or per-field transition
inheritance (today a cue's `transition` replaces the inherited one whole).

### D74: playlist randomness is our own SplitMix64 (2026-10-09)

The generator and random durations use `strobe_playlist::rng::SplitMix64`
(eight lines of arithmetic, reference outputs for seed 0 pinned in a test),
not `rand`. A seed written in a playlist or shown by the generator preview is
a promise that the same section comes back, and `rand` reserves the right to
change its streams between versions. The generator sorts its pool by name
first, so a library snapshot's order cannot change the result, and a golden
test pins a whole generated section. Changing the algorithm, the draw order
or the pick rule breaks saved seeds, so it must be a deliberate act that
updates the golden test and says so here.

**Would reopen:** nothing short of a better reason than a dependency's
convenience.

### D75: beat-locked ends: fix the length at cue start, end on the next downbeat (2026-10-09)

A `Bars`/`Beats` cue's length is computed when the cue starts, from the live
BPM if `syn_BPMConfidence` is above 0.5 and the tempo is plausible (50-220),
else from the timeline's fallback BPM (120). When that time is up the cue
waits for the beat grid to enter a new bar and ends there; a downbeat
exactly at the computed end counts. The wait is abandoned at once if
confidence is lost, and after six beats if no downbeat arrives, so a stalled
grid cannot strand a show. Seconds cues end exactly on time. Bars are four
beats (no time signature in the analysis). The clock takes a small `BeatInfo`
(bpm, confidence, grid position in beats) rather than depending on
`strobe-audio`. Open: the app must supply a grid position whose bars line up
with the music; strobe-audio's beat grid is phase-locked to detected beats
but has no downbeat detector, so "bar" is counted from the grid's origin
until one exists.

**Would reopen:** real tracks showing the six-beat timeout or the 0.5
threshold are wrong, or a downbeat detector landing in `strobe-audio`.

### D76: the scene clock and the wall clock are separate, and time is always an argument (2026-10-09)

`PlaylistClock` advances by `dt`, stops while held, and answers from its own
state. Wall-clock scheduling (`Timeline::place`, `elapsed_since_anchor`) only
reads a `Timeline`, takes "now" as seconds since local midnight, and never
edits the playlist. The two meet in one place, `sync_to_wall_clock`, which
the app calls on a late start. Wall times come from the static timeline, so
beat-locked ends and manual advances drift the real show away from them;
the schedule is a plan. Nothing in the crate reads a system clock, which is
why a 4-hour soak at 60 Hz runs in under a second. Hold freezes the cue's
elapsed time but lets an in-flight transition finish. Events are indices and
ids in a queue reserved at construction (1024), so `tick` never allocates;
if the app stops draining, events are counted as dropped instead of queued.

**Would reopen:** a need to resync the live show to the wall clock while it
runs (today only at start).

### D77: failed scenes are marked on the clock, not the playlist (2026-10-09)

Precompile results arrive after the playlist is loaded, and a scene can also
be lost at run time, so `PlaylistClock::mark_failed` / `mark_scene_failed`
flag timeline cues. A flagged cue is skipped with a `CueSkipped` event
wherever the clock would have gone (start, next, previous, skip-section,
auto-advance, wall-clock catch-up), cannot be jumped to, and if it is the
active cue the clock leaves at once without a transition from the missing
scene. If nothing is playable the playlist ends rather than spinning.
`distinct_scenes()` is the precompile list: enabled cues only, first-use
order.

### D90: one GLSL file per pass (`PASS_SOURCES`), top-level `SOURCE`, and per-pass programs forced (2026-10-09)

Shadertoy buffers each define their own helpers (`hash`, `palette`,
`rotate`), usually with different bodies, so merging the passes into one
`main.glsl` with a `PASSINDEX` switch (the SSF way) cannot work. `scene.json`
gains an optional **top-level** `PASS_SOURCES`: an array with exactly one
relative path per pass, the implicit final pass last. The assembler is
unchanged: it is called once per pass with that pass's text as `user_code`,
which is still source string 1, so a driver error maps to a line of *that
file* and the player reports it as `passes/buffer_b.glsl:14`
(`SceneFolder::file_for`). `main.glsl` stays mandatory (the scanner keys on
it) and is the final pass's file. A `SOURCE` block (`kind`, `id`, `url`,
`author`, `name`, `license_note`, and free extras such as `tags` and a
per-pass line map) records provenance.

PLAN 4.5 put these inside an `X_STROBE` object. They are top-level keys
instead, as the Phase 4 brief specified; Q6 (Synesthesia ignores unknown
keys) applies either way. Where the two disagree, this entry wins.

**Shared mode.** `--shared` (`Options::specialise_passes == false`) cannot
share a program across different sources. The player builds per-pass
programs anyway and logs a warning, rather than failing a whole bench run
over one imported scene; the bench's `--shared` label is then wrong for
that scene, which is the lesser evil.

**Other scene additions** made for the importer, all optional and
defaulting to SSF's behaviour: per-image `WRAP`, `FILTER`, `VFLIP`
(Shadertoy sampler hints; defaults repeat, mipmap, flipped as before), and a
per-pass `FULL_PRECISION` (RGBA32F even where the player defaults to
RGBA16F, D13). The new validation errors are reported as
`SceneError::Json` with a message (`SceneError::invalid`) rather than as new
variants, because `strobe-app` matches `SceneError` exhaustively and is not
this change's to edit. Promote them to variants when the app next changes.

**Would reopen:** a Synesthesia that rejects the extra keys (Q6), or a need
to share one program for memory reasons on the oldest GPUs.

### D91: the uniform shim is generated GLSL around untouched code; time comes from a clock pass (2026-10-09)

Each pass file is: a 3-line header, the shim, the `Common` tab, the pass's
tab, and a generated `renderMain` that calls `mainImage`. The author's code
is dialect-normalised (D94) but no statement of it is edited. The shim:

- sets globals (`strobe_iResolution`, `strobe_iTime`, ...) in
  `strobe_begin()` and `#define`s the author's names to them, **only for the
  names that pass mentions**. Defining to globals rather than to expressions
  over Strobe's uniforms keeps the shim working when the author's code
  redefines `TIME` or `RENDERSIZE`;
- aliases any assembler-declared global the author also declares
  (`const float PI` is the common case) with
  `#define PI strobe_user_PI`, unless the author `#define`s it;
- binds `iChannelN` by `#define iChannel0 st_buf_a` (a pass target, an image,
  `strobe_ShadertoyAudio`, or `st_black`), so a sampler passed as a
  function parameter still works;
- forces alpha to 1 on the Image pass.

**Time.** `iTime`, `iTimeDelta`, `iFrame`, `iFrameRate` need host-side
state (a clock that advances by `dt * speed`). Instead of new host state the
converter adds a one-texel, double-buffered, **FULL_PRECISION** float pass,
`st_clock`, first in `PASSES`, only when some pass mentions a time name. It
stores `(clock, last TIME, real dt, start frame + 1)` and advances the clock
by `dt * rate`, with `rate` from the Strobe controls (D92). `dt` comes from
`TIME` minus last frame's `TIME`, so the app needs no change. A cleared
target (first frame, or a resize, which wipes feedback) restarts the clock
at `TIME` and `iFrame` at 0, which is also what a Shadertoy feedback shader
that initialises on `iFrame == 0` needs after its buffers were wiped.
Full precision is required: in RGBA16F the clock stops advancing after about
16 seconds. The cost is one 1-pixel draw per frame.

**Known loss:** a resize with a non-default speed makes the clock jump to
`TIME`. The adaptive-resolution controller resizes rarely, and at default
controls the clock equals `TIME`, so nothing visible happens.

**Would reopen:** host-side clock state in the frame loop (it would remove
the pass), or a driver that cannot do `texelFetch` on RGBA32F (the ES path
needs `EXT_color_buffer_float`, which `Caps` already requires for float
passes).

### D92: the "Strobe" control group is `st_`-prefixed, and acts on the Image pass only (2026-10-09)

PLAN 6.3 names the controls `speed`, `audio_time`, `bass_flash`, `zoom`,
`hue`. Controls are global-scope uniforms, and `zoom`, `speed` and `hue`
are among the most common identifiers in Shadertoy code (`const float zoom
= 2.0;`, `vec3 hue(float)`), so plain names would fail to compile some
fraction of imports. They are `st_speed`, `st_audio_time`, `st_bass_flash`,
`st_zoom`, `st_hue`, in UI group "Strobe", always present, and the
converter errors (`NameClash`) if a shader uses an `st_` name it generates
or any `strobe_` name. `strobe_` is not usable for controls: the loader
reserves it.

- `st_speed` (0 to 3, default 1) and `st_audio_time` (0 to 1, default 0)
  drive the clock pass: `rate = speed * mix(1, 0.3 + 2 * syn_BassLevel,
  audio_time)`.
- `st_zoom` (0.5 to 2), `st_hue` (-180 to 180) and `st_bass_flash` (0 to 1,
  `* (1 + flash * syn_BassHits)`) apply **only to the Image pass**. A buffer
  holds per-pixel state; zooming `fragCoord` in a buffer would write a
  different pixel's value to every texel and corrupt a simulation.
- All defaults are exact identities (`x * 1.0`, `hue != 0` branch), so an
  untouched import renders what Shadertoy renders; a test checks it.

**Would reopen:** a `LABEL` mechanism for controls (then the uniform and
display names can differ).

### D93: Shadertoy audio and `iDate` ride the existing player paths; the rest of the contract (2026-10-09)

- **Audio inputs** (`music`, `musicstream`, `mic`) bind a new reserved
  sampler, `strobe_ShadertoyAudio`, declared by the assembler. It is a
  512x2 R8 texture (row 0 spectrum, row 1 waveform, `v = 0.25` and `0.75`),
  created at its full size **only when some pass samples it** (otherwise
  1x2), and filled inside `upload_audio` by resampling the `AudioTextures`
  spectrum texels: raw FFT (`r`, -100 to -20 dB) converted to the browser's
  scale (-100 to -30 dB, 6 dB lower) for row 0, and the waveform (`a`) for
  row 1. `strobe-audio` already builds a browser-exact, smoothed texture
  (`AudioFrame::shadertoy`), but `AudioTextures` has no field for it and the
  app builds that struct literally, so adding one would break a crate that
  is not this change's. **Gap:** a follow-up should add
  `AudioTextures::shadertoy: Option<&[u8]>` and prefer it.
- **`iDate`** is a new reserved uniform `strobe_Date`. The player reads the
  wall clock in UTC (`strobe_scene::date`) each frame unless the host calls
  `ScenePlayer::set_date`. UTC because `std` has no time-zone database
  (Q15). The bench and tests are deterministic by setting it or by not using
  it.
- `iSampleRate` is the constant 44100. `iChannelTime` is `iTime` for audio
  channels and 0 for the rest (Q15). `keyboard`, `video` and `webcam` inputs
  read black with a warning; `cubemap` and `volume` inputs and `sound` and
  `cubemap` passes are errors naming the pass and channel.
- `iMouse` is `_mouse`, which the player already documents as Shadertoy's
  `iMouse`; the app feeds it (Q13, Q15).

### D94: dialect normalisation is token-level, dialect-neutral, and stops where types begin (2026-10-09)

Shadertoy source is already inside the intersection of GLSL 3.30 core and
ES 3.00 (it compiles as ES 3.00 in a browser), so the converter writes **one**
rewritten file for both dialects and the assembler supplies `#version`. The
rewriter (`strobe_shadertoy::dialect`) works on a lossless tokenizer
(`glsl`), so it cannot touch comments, longer names or numbers, and every
removal keeps its newlines. It renames `texture2D`-family calls, removes
`precision` statements, `#version`, `#extension`, redeclarations of
Shadertoy uniforms, replaces non-ASCII comment text, and normalises line
endings. Two semantic rewrites live in the converter instead: `gl_FragColor`
becomes a shim global, and a **built-in the shader defines itself**
(`float tanh(float)`, common in WebGL1-era code) is aliased to a private
name under `#ifdef GL_ES`, because GLSL ES 3.00 forbids redefining a
built-in (Mesa rejects it; found by the legacy fixture) while desktop 3.30
lets the definition hide it.

Not attempted: implicit `int` to `float` (needs types; Shadertoy source
never needs it, SSF scenes do; Q8) and the packing built-ins
(`packHalf2x16`) which are ES 3.00 core but GLSL 4.00 on desktop (Q8).

**Would reopen:** a real shader that needs a rewrite the token level cannot
express, which is the point at which a GLSL parser (`glsl-lang`) earns its
place.

### D95: converter output rules (2026-10-09)

- **Every buffer is `FLOAT` and `DOUBLE_BUFFER`**, as PLAN 6.2 says, even
  one nothing reads back from itself. That costs memory a pipeline-only
  buffer does not need; the alternative (single-buffer unless it reads
  itself) leaves a trap for anyone who later edits a buffer to feed back.
  Revisit if memory on the HD 4000 matters.
- **Buffers run A, B, C, D, then Image**, ordered by the letter in the pass
  name, else the output ids 257 to 260, else listed order, whatever order the
  JSON lists them in. Strobe's existing read rule (pass `j >= i` reads last
  frame) is Shadertoy's, and a test checks it numerically.
- **Buffer sampling** (filter, wrap) comes from the first pass that reads the
  buffer; a second reader with different settings gets a warning.
- **No network, no clock, no randomness** in the converter. A texture that is
  not supplied becomes a warning, a black channel and an
  `images/pending-assets.json` entry with its URL (an error with `--strict`).
  Re-running the import with the file present replaces the scene.
- **Output is deterministic**: `scene.json` is built from `Serialize`
  structs (no `serde_json::Value`, whose key order depends on a cargo
  feature), files are in a `BTreeMap`, and golden files lock it.
- **Writing is atomic**: files go to a hidden `.<name>.importing` folder
  (`main.glsl` last), then one rename; `--force` swaps an existing scene
  aside first and restores it if the rename fails.
- **`SOURCE.license_note`** says Shadertoy shaders are CC BY-NC-SA 3.0
  unless the author says otherwise and that commercial use needs
  permission. That is the importer's best knowledge, not a legal
  determination (Q15).
- The converter keeps the original JSON as `shadertoy.json` when the caller
  passes it, so an import can be redone offline.

### D96: the output window starts hidden, and the scene frame no longer depends on it (2026-10-09)

Owner requirement: at launch only the control window opens; the windowed
preview ("Strobe Output") is shown on request.

- **Created invisible, never absent.** The output window is built with
  `with_visible(false)` unless `--show-output`. It must exist, because the
  shared GL context is made on it (and WGL fixes a window's pixel format at
  creation). The bench already gets its context from a hidden window
  (D70/D71), verified on the T480, so an invisible first window is a known
  quantity for both WGL and EGL; under Xvfb the GLX/EGL path is fine too.
- **The scene frame has its own schedule.** It used to live in the output
  window's `RedrawRequested`, which a hidden window may never receive
  (Windows sends no `WM_PAINT` to a hidden window), and with no swap there is
  no vsync to pace it. `scene_frame` now runs on `next_output`: shown, from a
  requested redraw as before; hidden, directly from `about_to_wait`, on the
  control surface, one output-monitor refresh period apart (60 Hz if
  unknown; never faster than the 144 Hz cap). Present and swap happen only
  when shown. The control preview paints the scene's output texture, so it is
  unchanged.
- **Hidden has no vsync, so it has backpressure.** A frame is skipped while
  every GPU timer slot is still waiting, which bounds the queue to four frames
  where the context has timer queries. Where it has none, only the control
  window's swaps limit the queue (unverified on a real driver).
- **Adaptive resolution is unchanged.** The same budget (one refresh period of
  the output's monitor); the timer brackets the scene without the blit when
  hidden.
- **Showing and hiding.** `Action::SetOutputVisible`, a "Show output" /
  "Hide output" button beside the fullscreen button. `ToggleFullscreen` (F11)
  and `MoveToMonitor` show a hidden output, since asking for fullscreen on a
  monitor means "put it on the projector". Esc leaves fullscreen only on a
  shown output. The output's close button **hides** it (it used to quit the
  app); closing the control window quits. After showing, the control window
  takes focus back, so the output never steals it from the operator.
  Visibility is tracked in the app: winit's `is_visible` is unimplemented on
  X11 and unsupported on Wayland.
- **Screenshots.** A shown output is read from its own back buffer (it checks
  the present path the projector uses). A hidden one is read from the scene's
  output texture: the default framebuffer of a window nobody can see has
  undefined contents (pixel ownership), the texture does not depend on the
  window, and it needs no show. The PNG is then the render size, not the
  letterboxed window size. `--show-output` gives the old behaviour.
- **Context loss (D66)** keeps the state: the layout creates the new output
  window with the old visibility, and fullscreen is re-applied (winit defers
  it on X11 until the window is mapped).

**Unverified (needs real Windows):** that winit's `set_fullscreen` on a hidden
window, followed by `set_visible(true)`, ends fullscreen on the right monitor
without a visible 960x540 flash; the output's close button hiding it; the
GPU queue staying short with the output hidden on Intel. **Not available:**
`set_visible` is unsupported on Wayland, where the output window would simply
show.

**Would reopen:** hidden-mode frames drifting from 60 Hz under Windows timer
granularity, or a driver that lets the queue grow with no swap.
