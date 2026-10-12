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
it fails; the alternative is a shader that cannot compile at all.
**Superseded by D161: a late uniform is now refused, and nothing recompiles.** Names the
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
(an asynchronous PBO readback a frame or two late). **Provided by D162.**

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

**Verified on the T480 (2026-10-10, Task 9):**
- Only the control window is visible at launch.
- The preview runs at 59.6 fps with the output hidden.
- F11 on the hidden output brings it up fullscreen. It was sampled every
  15 ms, and the first sample was already the borderless window filling
  the screen.
- The output's close button hides it, and the app keeps running.

**Still unverified:** that the GPU queue stays short with the output hidden
on Intel. GPU time read 0.9 ms against a 16.7 ms budget, so it is not in
question for light scenes. **Not available:**
`set_visible` is unsupported on Wayland, where the output window would simply
show.

**Would reopen:** hidden-mode frames drifting from 60 Hz under Windows timer
granularity, or a driver that lets the queue grow with no swap.

### D120: the Perform UI is "Contextual Perform" (direction B) (2026-10-10)

The owner chose direction **B · Contextual** from
`docs/mockups/strobe-perform-directions.html`, which is the topology
`UI_IMPLEMENTATION_PLAN.md` recommends.

Always visible:
- a navigation rail;
- the live preview;
- a compact health strip;
- transport and cue state;
- the Performance Macros.

Opened as panes when needed, without changing the primary task:
- the scene browser;
- the control inspector;
- diagnostics;
- mapping detail;
- playlist detail.

The others were rejected:
- **A · Instrument:** everything visible at once.
- **C · Minimal:** too much behind menus.
- **D · Studio:** an authoring layout with dockable panes, which is the
  busiest.

Implementation follows `UI_IMPLEMENTATION_PLAN.md`, porting rustaman's UI
system (D5).

**Would reopen:** if the 1280x720 acceptance screenshots show the
persistent strip and macros leave the preview too small to judge the
output.

### D121: work reaches `master` by direct fast-forward once CI passes (2026-10-10)

The owner chose this over pull requests. Agent sessions develop on their
`claude/*` branch, and when that branch passes `cargo xtask ci`
(`STROBE_REQUIRE_HEADLESS_GL=1` in the cloud) they fast-forward `master`
to it. Nothing is ever force-pushed, and no release tag is ever moved.
Pull requests stay the route for outside contributors and dependabot.

**Would reopen:** if the owner wants to review before merging, or if a
second person starts contributing.
### D110: the HTTP client is ureq with rustls and compiled-in roots, behind a `net` feature (2026-10-10)

Shadertoy stage 2 needs an HTTPS client (the REST API, texture downloads).
`strobe-shadertoy` uses `ureq` (blocking, no async runtime) with `rustls`
and Mozilla's root bundle compiled in (`webpki-roots`), so a machine with
a broken or empty system certificate store still works. Proxies come from
`HTTPS_PROXY`, which a school or office network and the cloud containers
need. `deny.toml` gained `CDLA-Permissive-2.0` for the roots' data.

rustls' crypto provider (`ring`) compiles C, and compiling C for MSVC
needs the Windows SDK, which the Linux cross-check does not have. The
client therefore sits behind the crate's default `net` feature and the
Windows cross-clippy builds the crate with `--no-default-features`; the
`strobe-import` binary requires `net`. A real Windows build (CI's
`windows` job, the T480) checks the client. Everything else in the crate,
including the listener, is plain `std`.

**Alternatives:** `reqwest` (drags in an async runtime), the OS's own TLS
(three code paths, and the roots are the thing that breaks).

**Unverified:** that `ring` builds under MSVC in CI's `windows` job, and
any request to the live site (Cloudflare blocks the cloud containers).

**Would reopen:** a Windows build failure from `ring`, or a proxy that
needs authentication the environment variables cannot carry.

### D111: acquisition is a seam, kept apart from conversion (2026-10-10)

`acquire` gets bytes and `convert` turns them into a scene; neither knows
the other. Getting bytes goes through a `Fetch` trait with a `ureq`
implementation, so every test runs against fakes and a loopback mock of
Shadertoy; **no test needs the live site**. Path B is
`GET /api/v1/shaders/<id>?key=<key>`; path A is the listener (D114); path
C (a dropped or typed file) needs no acquisition.

Textures are downloaded after the converter says which it lacks
(`pending_assets`), then the same conversion runs again with the files
(`import_json` converts twice; nothing is written between). Each texture
has a 10 s limit and 16 MiB cap, only Shadertoy's own hosts are fetched,
an unreachable host stops the remaining downloads, and an HTML body served
with status 200 (a challenge page) is refused. The cache keeps one file per
URL named by the SHA-256 of the URL, so a texture shared by many shaders is
fetched once and a URL cannot name a path outside the cache. A texture that
cannot be fetched is a warning and reads black; it never fails the import.
`--no-texture-download` skips the step.

**Would reopen:** Shadertoy's media URLs or `/api/v1` shape differing from
what AGENT_RESEARCH records (unverified live).

### D112: the player takes the browser-matched audio slice, and `iDate` is local time (2026-10-10)

Closes the gap D93 named. `AudioTextures` has an optional `shadertoy`
slice (the audio crate's `AudioFrame::shadertoy`, already on the browser
analyser's decibel window and smoothed); the player uploads it as given,
and falls back to resampling the spectrum texture when the host has none.
The app supplies it. `strobe_Date` is the viewer's **local** time, as on
Shadertoy: the app caches the OS's UTC offset (`chrono`, used for that one
question, refreshed with the app's other housekeeping, never on the frame
path) and the player does the calendar arithmetic. Shader failures now
name the pass file and line (`passes/buffer_a.glsl:14`).

Supersedes D93's "iDate is UTC" for the app; the bench and tests still use
the player's UTC default so frames stay a pure function of time (Q15).

### D113: re-importing asks first, and a failed re-import changes nothing (2026-10-10)

A scene folder is named from the shader (`<slug>-<id>`). Importing the
same shader again is detected **before** any download and returns
`Exists`; nothing is replaced unless `replace` is set. The app shows
"Replace / Cancel", the userscript a `confirm()`, `strobe-import` has
`--force`. The new folder is written beside the old one and swapped in by
renames, so a failure leaves the old scene intact.

Imports run on one worker thread, one at a time, through a bounded queue
(16); a full queue refuses with a message. The worker never touches GL.
When it finishes, the event loop rescans the library, selects the new
scene and loads it, because compilation needs the context that thread
owns. The app's import state (`ui/import.rs`) is a view of that, with no
runtime state kept in egui memory.

**Would reopen:** a person importing dozens at once (a batch route would
want progress and a queue longer than 16).

### D114: the listener trusts a secret, not "localhost" (2026-10-10)

Any web page open in the same browser can send requests to `127.0.0.1`,
so a loopback socket is an attack surface. In the order a request meets
them: bound to `127.0.0.1` only (the port, default 47811, is configurable;
the address is not); `Host` must be `127.0.0.1:<port>` or
`localhost:<port>` (refuses DNS rebinding); only `POST /import`, and an
`OPTIONS` preflight is a 405 with **no CORS headers**, so a page's script
cannot send the real request; `X-Strobe-Token` must equal the per-install
secret (401 before the body is read), and the custom header already makes
a cross-origin request need that refused preflight; `Content-Length`
required and capped at 2 MiB; the body must be a JSON object with a
`shader` that has a `renderpass`. At most four connections at once, each
with timeouts. The listener never imports anything itself; it hands the
job to the import worker and returns `imported`, `exists` (409) or the
converter's error.

The token is 32 random bytes (hex) from the OS, written to `import-token`
in the data directory on first use, owner-only on Unix and the per-user
`%LOCALAPPDATA%` ACL on Windows, compared without early exit. The
listener is **off by default**: `--listen` or the Import panel's switch,
which also shows and copies the token.

The userscript's `GM_xmlhttpRequest` is made by the userscript manager,
not the page, so CORS does not apply to it and none is offered.

**Would reopen:** a browser or manager that stamps a page `Origin` on
`GM_xmlhttpRequest`, or a request to import over the network.

### D115: the userscript sends what the page already has, and its route is a guess (2026-10-10)

`tools/userscript/send-to-strobe.user.js` adds a "Send to Strobe" button to
`shadertoy.com/view/*` and POSTs `{shader, pageUrl, replace}` to the
listener. It does not call the Shadertoy API (no key needed in the
browser) and keeps the token in the manager's storage, asked for once.

**How the page exposes the shader JSON is not verified** (Q19): the site
was unreachable (Cloudflare) when this was written. The script tries the
same-origin `POST /shadertoy` form request the site itself makes to load a
shader, then the page's `gShaderToy` object, and says what it tried when
both fail; the fallback is the export JSON dropped on Strobe. Everything
after "have the JSON" is tested: `tools/userscript/test/run.mjs` drives the
script against stubbed `GM_*` and a fake listener (token, 409 replace flow,
401 re-prompt, unreachable listener), and against the real listener with
`STROBE_LISTENER` and `STROBE_TOKEN` set.

**Would reopen:** the first look at a real `/view/` page's network tab.

### D116: the Import panel and dropped files are one door (2026-10-10)

A file dropped on either window (`WindowEvent::DroppedFile`) and text typed
in the Import panel (a path, a shader id, or a URL) both become an
`Input::Text` and are classified by the worker: an existing file is read
(8 MiB cap) as path C, anything else must parse as an id or URL and goes
to path B, which needs a key (D117). Only a `.json` drop is accepted; any
other file says so in the panel and the console. Several drops queue
several jobs. The panel shows progress, the converter's warnings,
the failure with its next step (through `strobe_diag::failure::report`),
and the Replace prompt.

**Unverified:** a real file drop (X11's XDND and Windows' drag-and-drop
could not be driven under Xvfb; the typed path runs the same job).

### D117: the API key is an environment variable or a file, not the keychain (2026-10-10)

`STROBE_SHADERTOY_KEY`, else the first line of `shadertoy.key` in the data
directory. The key is a read-only, per-app string the owner can regenerate
at `shadertoy.com/myapps`; a keychain crate would bring a platform service
stack for it, and the data directory is already per-user. It goes into a
URL query, so it is validated to letters, digits, `_` and `-` (no `&`,
`#` or space can be injected), and `ApiKey` prints as `ApiKey(***)`. The
error for a missing key names both places and the page that makes one.

**Would reopen:** keys becoming secrets that matter (a write scope).

### D122: winit without its Wayland title-bar decorations, not an advisory ignore (2026-10-10)

`cargo deny` failed on RUSTSEC-2026-0192: `ttf-parser` is unmaintained and
has no fixed release. Its only path into the build was winit's default
`wayland-csd-adwaita` feature (winit, then sctk-adwaita, ab_glyph,
owned_ttf_parser, ttf-parser), which draws client-side title bars on
Wayland compositors that do not draw their own. The workspace now asks
for winit's other defaults explicitly (`rwh_06`, `x11`, `wayland`,
`wayland-dlopen`), and the crate is gone from the tree. The other option
was an `ignore` entry in `deny.toml`, which is deliberately empty.

The cost: on such a compositor (GNOME's, notably), Strobe's windows have
no title bar. Windows, the target, and X11 are unaffected. The custom
title bar from the rustaman UI port (D5) removes the cost.

**Would reopen:** a Linux user on GNOME Wayland before that port lands,
who needs to move or resize the control window.

### D123: auto-generated controls are their own crate, `strobe-autocontrol` (2026-10-10)

The roadmap allowed either a module in `strobe-shadertoy` or a small crate.
It is a crate, because the engine also runs on plain SSF scenes whose
`scene.json` has no `CONTROLS` (a community scene, a library scene), and
those must not need the Shadertoy importer. Its dependencies are
`strobe-scene` (the control model, name rules, `SceneFolder`),
`strobe-shadertoy` (only the lossless `glsl` tokenizer and the tab-marker
constants, now `pub` in `convert`), `strobe-gfx` (the headless context and
`ScenePlayer`, for scoring) and `strobe-ssf-lib` (its identifiers are names a
control must avoid). **`strobe-gfx` must never depend on it**, and nothing
in the frame loop calls it: scoring runs at import time.

The app will call `generate(&Gl, &Caps, &SceneFolder, &Options)` with the
context it already has, so a Windows machine whose driver exposes no EGL
device (the T480) can run it; the `strobe-autocontrol` binary opens a
headless context itself and has `--analyze-only` for machines without one.

**Would reopen:** if a second consumer needs the tokenizer without the
importer (then the tokenizer moves to its own crate).

### D124: only patterns that are almost certainly knobs, and only where a uniform is legal (2026-10-10)

False positives cost more than misses: a slider that does nothing, or breaks
the picture, is worse than one never offered. So each detector is narrow:

- **Named constants:** a top-level `#define N <number>` or `const float|int N
  = <number>;`. Skipped when the name or value is mathematical (`PI`, `TAU`,
  `EPS`, 3.14159..., under 1e-3), when it is used in an `#if`, as an array
  size, as a `case` label, in another `const`, at global scope, or when it is
  defined twice or undefined. An integer constant is a candidate only as a
  `for` loop bound (otherwise it is a mode switch, a count or an array size).
- **Time:** `TIME * k`, `k * TIME`, and `iTime` likewise, also with a named
  constant as `k`. Not after a `/` or `%`.
- **Loops:** `for (.. i < N ..)` or `<=`, N a literal or constant, at least 4,
  and the body does not index with `i` (N may be an array size).
- **Colour:** `vec3(r, g, b)` and `vec4(r, g, b, a)` of literals in 0..1.
  Not axis masks (all components 0 or 1), greys, luminance weights or
  directions (arguments of `dot`, `normalize`, `cross`...), geometry
  variables (`ro`, `rd`, `n`), or a `const` statement.
- **Scale and frequency:** `coord * N`, `N * coord`, `coord *= N` where coord
  is `_uv`, `_uvc`, `_xy`, `uv`, `p`, `fragCoord`, `st`, `pos`, `coord`, `q`
  (swizzles allowed), and `x * N` inside `sin`/`cos` when the statement
  mentions a coordinate. Not 0, 1, 0.5, under 0.05, `2.0` followed by a
  minus (the `* 2.0 - 1.0` and `2.0 * fragCoord - iResolution.xy` remaps),
  or anything inside a function named like `hash`, `noise` or `fbm`.
- **Nothing at global scope** (a uniform is not a constant expression), and
  nothing in a Shadertoy import's generated shim or entry point: only the
  author's tabs between the converter's markers are scanned.

A `const` cannot read a uniform, so a rewritten `const float N = 0.3;` becomes
`#define N (name)` on its own line (a statement sharing its line is skipped).
If a rewrite still does not compile (a macro used in a constant context the
scan cannot see), the scoring stage compiles each candidate alone, drops the
ones that fail, and reports them `does not compile`.

**Would reopen:** a real shader where a missed pattern hides an obvious knob.
The traps each pattern avoids are pinned in `analyze/expr.rs`'s tests.

### D125: a speed control replaces `TIME * k` with an integrator (2026-10-10)

`TIME * 0.3` made a plain slider would jump the phase whenever the slider
moved (the phase is `TIME * k`, and `TIME` is large). SSF's own answer is the
`slider speed` type, which integrates its value per second; so the whole
`TIME * k` expression becomes the uniform and the control is `slider speed`
with default `k`. Steady state is identical, and moving it is smooth. Several
`TIME * 0.5` with the same factor are one control. The range is `[0, 2k]`.

**Cost:** in a Shadertoy import `iTime` is the clock pass's time, scaled by
`st_speed` and `st_audio_time`. A term rewritten this way no longer follows
those two; the other terms of the shader still do. The alternatives (keep
`iTime`, make `k` a plain uniform; or multiply by `st_speed`) both make the
phase jump when something is dragged, which looks worse on a projector.
Closing the gap needs the clock pass to publish its rate where other passes
can read it.

**Would reopen:** if operators drag `st_speed` and expect every term to follow.

### D126: controls are chosen by measured effect, with a small fixed protocol (2026-10-10)

The scene is rewritten once with every candidate as a uniform and compiled
once; a probe is only a different uniform value. For the baseline and for two
nudges per candidate (half and one and a half times the default; integers
rounded; a colour at half brightness and halfway to white) the rig:

1. resets history (resize to another size and back, which clears every pass
   target to black), builds a fresh `ControlBank`, and
2. renders 10 frames of 160x90 at times `0, 0.2, ... 1.8` s with
   `strobe_scene::synthetic` audio, and reads the last 2.

The score is the mean per-channel absolute difference (0..1) to the
baseline, the larger of the two nudges. Below 0.003 the candidate is dead; a
nudge that turns a lit picture black rejects it (what a NaN looks like in an
8-bit output); the rest are ranked, the top `keep` (default 8) are kept and
grouped Motion, Shape, Colour, Quality. Time starts at 0 and the speed
integrators start at 0 too, so the rewritten scene's default picture equals
the original's, and the pipeline measures that (`Report::fidelity`, 0 for all
25 scenes tested) and compiles the final scene once more before returning.

Ranges: `[0, 2x]`, or `[-2|x|, 2|x|]` for negative; colours 0..1; a loop
count `[max(1, n/4), min(2n, 256 or n)]`, so the maximum is never over twice
the default and never over 256 unless the scene already is. Names are
snake_case from the identifier or context (`fog_col`, `zoom`, `steps`,
`shadow_steps`), numbered `_1`, `_2` in source order when shared, and avoid
every control, reserved name, GLSL word, library function and identifier in
the scene's code (a local variable of that name would shadow the uniform at
the very site that uses it) and the `st_` prefix.

Scores are rounded to five decimals. Everything is a pure function of the
scene and the options, so two runs agree on one machine. The rasteriser is
part of that function: the tests pin the pure analysis and rewrite byte for
byte, and pin the measured half by invariants (kept controls above the
threshold, both dialects agree, a second run is identical), not by score.

**Audio suggestions** are data on the report: shape controls to
`syn_BassLevel`, colour to `syn_HighLevel`, speed to `syn_Level`, never
quality, at most three and never two on one band. With `bind_audio` a float
or colour site is written as `(name * (1.0 + name_audio * syn_BassLevel))`
and `name_audio` is a new slider in the "Audio" group that defaults to 0, so
nothing changes until it is raised. A speed control is an integrator and
cannot be modulated in GLSL, so its suggestion is data only.

**Authored controls:** a scene with controls other than the five "Strobe"
ones is refused unless the caller asks to merge; a merge keeps them and adds
only new names. The original folder is never written: the result is
`<name>-auto.synScene`, a copy whose `scene.json` has the new entries spliced
in as text (key order and the `SOURCE` block survive), staged and renamed in
one step.

**Would reopen:** a scene where the 10-frame window misses an effect that
only appears later (a slow fade-in), or a driver where the 160x90 render is
not representative.

### D130: what was ported from rustaman, and what was changed on the way (2026-10-10)

D5 said to port rustaman's UI system. Stage 1 did, under these rules:

- **Ported:** `color` (Rgb, WCAG contrast, OKLCh ramp), the theme catalog as
  `palette` (thirteen stated colours, derived rest) with `assets/themes.toml`,
  `motion` (four named durations, curves, `Tween`), the geometry icons, the
  spacing scale and egui `Visuals` binding (`ui::theme`), `widgets`, the
  custom title bar and resize handles (`ui::chrome`), and the single modal
  (`ui::modal`). Every test came with them: contrast, layer separation, a
  scrollbar handle never being its surface's colour, icon grid and centring,
  motion monotonicity and convergence, the eight resize regions, and the
  source scans (no colour literal, no hand-picked gap, no icon in a font, no
  hand-rolled animation, no fixed panel with a dead resize handle).
- **Kept portable.** `color`, `palette`, `motion`, `icon` and `workspace`
  know nothing of egui; `ui::theme`, `ui::motion` and `ui::icon` are the
  bindings, as in rustaman.
- **Changed, for Strobe's frame rules.** `theme::palette()` returns an `Rc`
  (a count bump) instead of cloning a palette's strings every call. Icons are
  `&'static` geometry (`Seg`), not a `Vec<Path>` built per paint, and circles
  flatten into a stack buffer. Readout text is formatted at 4 Hz, not 30.
  The theme is applied when it changes, not every frame.
- **Added.** A Strobe default theme (blue-black, amber), a high-contrast
  theme (UI_DESIGN section 27), a light one, and reduced motion
  (`motion::set_reduced`: every duration is zero and the spring has no
  overshoot; the preferences file, the Settings pane and
  `STROBE_REDUCED_MOTION=1` feed it). rustaman has no reduced-motion setting.
- **Not ported.** `egui_extras` tables, the treemap and the Windows task
  manager views, `dnd`, and `font.rs`. rustaman bundles no font (it loads a
  configured system family), so there was nothing to license-check or
  commit; Strobe keeps egui's bundled faces. A system-font setting is left
  until someone asks for Segoe UI.

**Would reopen:** a theme that passes the contrast tests and still reads
badly on a projector, or the T480 showing the geometry icons costing frame
time (they are a few allocations per icon; a cached mesh would replace them).

### D131: Contextual Perform stage 1: the shell, the docks, and the fit rules (2026-10-10)

Implements D120 and UI_IMPLEMENTATION_PLAN sections 1 to 7.

- **Persistent:** the rail (PLAY / LIB / LIST / EDIT / MAP / DIAG / SET), the
  preview, the title bar's health readouts (fps, GPU time against budget,
  render scale, backend, audio, output, and a script readout only when the
  script is faulted or disabled), the transport strip and the macros.
- **Contextual:** the scene browser, the inspector, diagnostics, import and
  settings, as *panes*. A pane is data (`workspace::Pane`: dock, width range,
  key). The left dock holds the browser and import; the right dock holds the
  inspector, diagnostics and settings. A dock shows its most recently opened
  open pane, so Esc on Diagnostics puts the Inspector back.
- **Fit.** `workspace::arrange` shows both docks only if the preview keeps
  55% of the width beside the rail (`PREVIEW_SHARE`). So 1920 and 1440 show
  the browser and the inspector together (1440 with the browser at 280 and
  the inspector at 300), and 1280 shows one: the newer pane wins and the
  other stays "open" but hidden, and its toolbar button says so. A lone pane
  shrinks to its minimum, and below that becomes an overlay drawer. Rail
  labels and the secondary readouts drop below 1000 and 1180 points. The
  share began at 0.58 and became 0.55 when the real rail width (60) pushed
  1440 just under the line.
- **Rail semantics.** PLAY closes every pane (a larger preview), LIB, DIAG
  and SET toggle theirs, and LIST, EDIT and MAP are drawn disabled with
  "not built yet" tooltips rather than hidden, so the rail already has the
  shape of the app. Inspector and browser also have buttons in the preview's
  toolbar, where the output controls (show or hide, fullscreen, monitor)
  live too.
- **Pane widths are ours.** Docked panes are fixed-size panels; a splitter on
  each inner edge writes the width into `Panes`, which the preferences file
  keeps. egui's own panel resizing would have kept widths in its memory.
  A drag stops at `drag_limit`, so widening one pane never evicts the other.
- **The 1280x720 check (D120's reopen condition).** At 1280x720 the preview
  is about 880 x 495 whether no pane or one is open (it is height-limited),
  with the transport strip and the macros both visible. That is dominant, so
  D120 stands.

**Would reopen:** the T480's 1366x768 panel (both panes fit there, with a
preview about 725 wide) feeling cramped, which would raise `PREVIEW_SHARE`.

### D132: the custom title bar is a setting, on by default on Windows and Linux (2026-10-10)

D122 removed winit's Wayland title bars, so GNOME's Wayland sessions had no
caption at all, and D5 had promised the custom bar for exactly that. The
control window now opens undecorated (`with_decorations(false)`) and
`ui::chrome` draws the bar.

- **Behaviour.** Drag hands the gesture to the window manager
  (`ViewportCommand::StartDrag`, which is what keeps Aero Snap, snap layouts
  and drag-to-another-monitor), double-click maximises, the three buttons
  minimise, maximise or restore, and quit, and eight invisible handles
  (`BeginResize` on the press frame) resize. Maximize-on-launch (D65) is
  unchanged. The bar also carries the readouts, so a second status row is not
  needed.
- **The setting.** `custom_title_bar` in `ui.toml`, a checkbox in Settings,
  and `--system-title-bar` for one run. Turning it off calls
  `set_decorations(true)` live and the bar keeps its readouts. The default
  is `cfg!(any(windows, target_os = "linux"))`: Windows because the system
  caption cannot be themed, Linux because GNOME has none; off on macOS,
  whose traffic lights Strobe does not imitate.
- **Why a setting.** Window-manager behaviour is the one thing the cloud
  cannot test: bare Xvfb has no window manager, so drag, snap and resize were
  verified only as far as the events they send (the unit tests assert a
  corner press asks for a south-east resize). The fallback must be one click
  away on a machine where it misbehaves.
- **Not verifiable here:** drag, Aero Snap and snap layouts, resize at
  125% and 150% scaling, a maximised undecorated window against the taskbar,
  and GNOME Wayland's behaviour with `drag_resize_window`. They are listed in
  STATUS_AUDIT as open for the T480.

**Would reopen:** any of those misbehaving on Windows, which would flip the
Windows default to off.

### D133: presentation state lives in `UiState` and is saved by a debounced writer (2026-10-10)

UI_IMPLEMENTATION_PLAN section 17 asks for pane visibility, widths and the
browser's query to survive a restart, and section 3 for runtime state to stay
out of egui memory.

- **`UiState`** (`ui::state`) owns the `Panes`, the browser's query, filter
  and selection, the collapsed inspector groups, the theme, reduced motion
  and the title-bar choice. Nothing is in egui `Memory`.
- **`ui.toml`** in the data directory (`strobe_diag::paths::base_dir()`, so
  `STROBE_DIAG_DIR` moves it): every field has a default, unknown keys and
  panes are ignored, widths are clamped on the way in, and a malformed file
  loads the defaults and logs why.
- **When it is written.** Never from the paint path. Each frame hashes the
  state (no allocation) and compares it with the last frame's; a change
  starts a `Debounce`, which fires when changes have been quiet for 500 ms or
  have gone on for 5 s (a drag). The text goes to a writer thread that
  writes a temporary file and renames it, so a crash cannot leave half a
  file and a slow disk cannot stall a frame. A pending change is written at
  shutdown.
- **Screenshot mode never reads or writes it**, so a capture depends on, and
  changes, nothing.
- **Verified under Xvfb:** opening the browser, typing a query and dragging
  the splitter wrote one file (width 280 to 360, query `tun`, the order the
  panes were opened in), and a second launch restored all of it.

**Would reopen:** a need to save per-scene state (macros, presets, favourites),
which wants its own file keyed by scene, not this one.

### D134: commands, and the focus rule that decides whether a key may fire one (2026-10-10)

UI_IMPLEMENTATION_PLAN sections 15 and 16. `commands` is a table (id, name,
category, default chord) and one pure function, `dispatch(chord, focus)`.

- Keys: Left and Right (previous, next scene), Space (hold, which is not wired
  and shows a notice instead of doing nothing silently), B (blackout), F11
  (fullscreen), Ctrl+F (open the browser and focus its search), Esc (close the
  topmost pane), T (tap tempo, from before).
- **The rule.** While a *text field* has focus nothing fires. While a
  *modal* is open nothing fires. While another widget has keyboard focus (a
  slider, a button) the keys it uses itself, arrows and Space, do not fire
  transport commands, so nudging a slider with the arrow keys never changes
  the scene; B, F11, Ctrl+F and Esc still do. The search field handles its
  own Esc (clear, then leave), its own Up and Down (move the selection) and
  Enter (load it).
- **A bug this fixed.** The old `key()` ignored keys whenever
  `egui_wants_keyboard_input()` was true, which is true whenever *any* widget
  has focus (a slider keeps it after a click), so T and F11 went dead after
  touching a control. The new rule uses
  `text_edit_focused()` for typing and the widget flag only for the keys a
  widget uses. Clicking the preview takes focus off whatever had it.
- **The output window** has no UI, so its keys are mapped in `app.rs` with the
  same table and no focus (`output_key`). Modifiers are not tracked there, so
  Ctrl+F is a control-window command.
- **Verified under Xvfb with xdotool:** Right, Left, B, F11; Ctrl+F then typing
  "kal" then Right changes nothing, Down and Enter load the match, and Esc
  twice closes both panes.

There is no command palette (Ctrl+K) yet; the table is what it will list.

**Would reopen:** a stage that adds MIDI or OSC, which should map onto
`CommandId` rather than onto a second list.

### D135: transport semantics without a playlist: what is wired and what is shown as not (2026-10-10)

The app has no cue list (`strobe-playlist` is not a dependency yet), and the
plan says to show that rather than fake it.

- **Previous and Next** step through the library in sorted order, wrap, and
  skip scenes the app has seen fail. The strip names what a Next press would
  load, labelled "library order".
- **Hold** is drawn unavailable (dashed look, faint), with a tooltip that
  says why. Space shows a notice banner and logs the request.
- **Time** is the playing scene's own clock, labelled as such, not a cue's
  remaining time.
- **Blackout** is real and safe: the shown output is cleared to black
  instead of presented, the preview in the control window keeps showing the
  scene (the operator has to see what they are about to bring back), the
  scene clock keeps running, and the button is danger-outlined, fills when
  on, and says `BLACKOUT ON`. It survives a context rebuild. It is a
  present-time clear, not a multiply in the final pass, so a future NDI or
  capture output would need the same gate (OPEN_QUESTIONS Q20).
- **Loading is two-phase.** A load asked for by the UI sets `pending_load`;
  the next control frame paints "Preparing X..." over the live picture, and
  the blocking compile runs after that frame is presented. The old scene
  stays live until the new one is ready, and a failure leaves it playing.

**Would reopen:** the T480 showing a compile long enough that a 30 Hz
"preparing" frame is not enough (it would want the compile on a worker, which
the shared GL context does not allow today).

### D136: Preview is separate from Load, and each scene carries a health (2026-10-10)

UI_IMPLEMENTATION_PLAN sections 8 and 9.

- **Preview** loads the scene into a *second* `Session`, rendered at 384
  pixels wide and advanced once per control-window frame (30 Hz), and shown
  in a card at the top of the browser. It never touches the live scene, the
  controls, the macros or the output; **Load** is a separate, visibly
  different button. The preview shares the audio input and runs the scene's
  script, so a script fault in a preview is visible too. A context rebuild
  drops it.
- **Cost.** Preparing a preview compiles shaders on the UI thread, like a
  load, so the output stalls for the same compile time; once prepared it adds
  one small render per control frame. It is off until asked for, and closing
  the card frees it. Neither is measured on the T480 yet.
- **Health.** `library::Health` is what the app has *seen*: unknown until a
  scene is loaded or previewed, ready after it is, failed (with the first two
  lines of why) after it fails. Rows show an icon and a word, never a colour
  alone; "Needs work" filters to the failed ones; the Diagnostics pane lists
  them. Thumbnails are not rendered (UI_IMPLEMENTATION_PLAN section 18 forbids
  rendering them in the frame loop).
- **Search** is every word, in any order, in the scene's name. `tag:` and
  `author:` wait until the library reads scene metadata.

**Would reopen:** previews being too costly on the T480, which would make them
a single still frame rather than a running scene.

### D137: Performance Macros, stage 1, are the first controls of the playing scene (2026-10-10)

The strip shows the playing scene's first N controls (N follows the width,
at most six) with a `SCENE` badge, using a widget fitted to each kind: a rail
for sliders and knobs, a switch, a trigger button, a dropdown, a swatch, and
a button that opens the inspector for an XY pad. They are the same
`ControlBank` entries the inspector edits, so there is no second control
model.

**Left to stage 2, and shown as such rather than faked:** a curated,
persistent set; pin, unpin and reorder; target resolution (exact id, group
plus name, alias, meta); the Resolved, Unavailable, Broken, Disabled,
Learning and Conflict states (a stage-1 macro is always Resolved because it
*is* the control); MIDI and OSC sources; shortcuts; identity colours.

Slider interaction follows Synesthesia's conventions (UI_DESIGN section 10):
click or drag to set, Shift for a tenth of the speed, double-click or
middle-click to reset, a reset button that appears only on a modified
control, click the number to type an exact value, and arrow keys (Shift finer,
Ctrl coarser) when the rail has focus. Group headers carry Default and
Random, which act on the group.

**Would reopen:** the macro model landing in stage 2 with a different shape
for targets than the strip assumed.

### D138: how the UI is verified without a window manager (2026-10-10)

- **Headless egui tests.** `ui::shell_tests` draws the whole shell at 1920x1080,
  1440x900, 1366x768, 1280x720, 900x600 and 640x480 in twenty states (empty
  library, loading, failure, missing and synthetic audio, script fault and
  disabled, blackout, output hidden, notice, preview, each pane and all of
  them, the replace question), and asserts that nothing overflows the window,
  that no shape is NaN, and that an untouched frame emits no action.
- **Screenshots** use the app's own `--screenshot`, which reads the control
  window's back buffer, so what is checked is what egui painted. Because bare
  Xvfb has no window manager (D65), flags were added to make captures
  reproducible: `--window-size WxH` (not maximized), `--open-pane`,
  `--no-panes`, `--browser-query`, `--preview`, `--blackout`, `--theme`,
  `--system-title-bar`, and `--request-load`, which asks for a load through
  the UI's own path on the frame before the control capture so the capture
  really shows "Preparing". Screenshot mode never reads or writes `ui.toml`.
- **Interaction** was driven with xdotool against a live window on Xvfb
  (focus needs `xdotool windowfocus`, since there is no window manager to
  give it) and captured with `import -window root`.
- **Scans.** The source scans from rustaman run over every drawing module,
  plus one of Strobe's own: no drawing module touches the filesystem, the
  driver, a socket or a sleep, which is UI_IMPLEMENTATION_PLAN section 18 as
  a test.

**Would reopen:** a real window manager in CI (a Wayland or X11 session with
a compositor), which would let drag, snap and resize be driven.
### D140: the offscreen context runner lives in strobe-gfx, and auto-controls use it (2026-10-10)

On the T480, `strobe-autocontrol` failed with `no headless DesktopCore33
context (EGL devices: Enumerating devices is not supported by the EGL
instance)`: Intel on Windows has no EGL devices, and the bench's answer
(D70-D72, a hidden winit window owning a `WindowedContext`) lived in
`strobe-bench/src/window.rs`, where nothing else could reach it.

It moved to `strobe_gfx::offscreen`, beside the context providers it
wraps. `strobe-gfx` already depends on winit and glutin, so
`strobe-autocontrol` gains no dependency and `strobe-bench` loses one.
The shape:

- `run(&Request, notice, job)`. `Request { mode: Auto|Headless|Window,
  api: Option<Api>, gl: GlChoice, tiny }` is the whole decision. `job` is
  `FnOnce(&Gl, &Caps, &str) -> T` (the str says where the context came
  from) and `T` comes back through the event loop, so a caller returns a
  `Result` of its own. `run_headless` and `run_window` are public for a
  caller that wants only one. `notice` is called once, before the job,
  when `Auto` falls back, so the bench prints the same line it always did.
- `--context auto` is headless when an EGL device gives a context and the
  window otherwise. `--gl` and `--api` fold exactly as D72 says, and a
  contradiction is an error in **every** mode, so a mistake is not hidden
  by the machine it was tried on.
- Hidden window first, tiny off-screen window second (D71), vsync off. A
  vsync refusal used to be a `println!` in the bench; it is now a log
  warning and a suffix on the context line, which reaches `results.md`.
- Window mode must run on the main thread, once per process (winit
  0.30). That is documented on the module and is why the test for it
  spawns the binary. The app, which owns an event loop and a context
  already, must not call this; it hands its own `&Gl` and `&Caps` to the
  library.

**Autocontrol.** The library API is unchanged: `generate(&Gl, &Caps, ...)`
takes a context the caller already has, so the app can score on its own
later. Only the CLI picks a context. It gained `--context
auto|headless|window`, `--gl native|angle|auto` and `--show-window`, with
the bench's meaning; `--api` became optional (unset is desktop, or
whatever `--gl` implies). `--analyze-only` opens no context, and still
works with no GL and no display. The CLI prints one `context:` line before
its report and nothing in the report depends on it.

**Determinism, checked.** On llvmpipe under Xvfb, `scenes/05-domain-warp`
with its CONTROLS stripped (in a temp copy, defaults baked in as
`#define`s; 8 candidates, 7 kept, 1 dead) gave a byte-identical
`report.json`, an identical written scene folder and identical `--report`
text from `--context headless`, `--context window` (hidden), `--context
window --show-window` (tiny) and `--context auto`; and from headless
`--api gles` against window `--gl angle` (Mesa's EGL GLES 3.0). The bench
on `05` wrote byte-identical PNGs in all four modes, and identical to the
build before the move. `crates/strobe-autocontrol/tests/window.rs` pins
the CLI comparison and CI runs it under Xvfb.

That is evidence that the context path does not change the picture, not
that two GPUs agree: both modes were the same rasteriser. On Intel the
scores will differ from llvmpipe's (as every timing does), and a score
rounded to five decimals can still straddle a kept/cut boundary between
drivers. What must hold on any one machine is that headless and window
agree, which is checkable only where both exist.

**Would reopen:** if the T480's `--context window` run keeps a different
set than a Linux run in a way a driver cannot explain, or if the app needs
to score before it has an event loop.

### D150: a playlist is a `Show` in the GL stack, and the live scene's slot stays the clock's current cue (2026-10-10)

Playlist playback is `strobe_app::show::Show`: the clock, the compiled scenes,
the warming and outgoing scenes, the resume writer and the file watcher. It
lives in `Stack` (beside the compositor), so it is dropped, in order, before
the context it owns objects of, and a lost context takes it down with the
rest. It does **not** own the live scene: the app lends the show an `Env` (the
context, the `Option<Session>` slot, the compositor, the audio, the sizes) for
the length of a call, and `Stack::session` stays "the scene the clock is on".
So the preview, the resize logic, the controls pane, screenshots, the crash
report's scene name and the browser's "playing" mark keep working unchanged,
and a show is a few calls in `scene_frame`, not a second code path.

- A **context rebuild** carries the playlist, its source file and the clock's
  resume point (`Show::reload_spec`), then compiles every scene again on the
  new context and resumes at the cue that was playing (a transition in flight
  is lost). The single-scene reload is skipped when a show is carried.
- **Loading a scene by hand** (a browser double-click, an import) unloads the
  playlist, with a notice. Keeping both would let a cue's start swap a scene
  the clock does not know it is looking at.
- **Previous, Next and Hold** are cue commands while a playlist is loaded and
  library-order stepping otherwise (D135 stands for the no-playlist case).

**Would reopen:** a need to keep a playlist across a manual load (a "pin the
playlist, audition a scene" mode), which wants a second live slot.

### D151: every distinct scene is compiled at load, one per frame, and parked at 1x1 (2026-10-10)

`distinct_scenes()` is compiled when the playlist loads, with progress in the
LIST pane, **one scene per control-window frame after that frame is painted**
(the D135 rule: the frame that shows "compiling 3 of 11" is never the frame
that blocks), so no frame waits for more than one scene. A scene that fails
(or whose reference does not resolve against the library) is marked failed on
the clock (D77), its cues are skipped with a `PLAYLIST` console entry, and its
library row gets the same `Failed` health the browser uses.

**The cache holds sessions, not just programs, and parks them.** A compiled
`Session` is programs, images, the script and render targets. Targets are
what cost video memory (a 3-pass scene with two double-buffered float passes
at 1080p is tens of MB, so 80 scenes would be gigabytes a 2012 laptop does
not have), and programs are what cost *time*. So a cached scene's targets are
cut to 1x1 (`ScenePlayer::resize`, a few textures) and grown only while the
scene is active, outgoing or warming: **at most three scenes hold full-size
targets at once, whatever the playlist's length.** What stays resident per
scene is the linked programs, its images, its script's QuickJS context and
1x1 targets. The price is that activating a scene allocates its targets (a
handful of textures, no compile) and a feedback scene starts from black unless
it was warmed (D153). Compiling programs without targets is not possible
today, so the alternative (a cache of programs only) would mean splitting
`ScenePlayer`; that is the place to look if the resident cost of 80 scripted
scenes turns out to matter.

Each scene is also **primed**: one draw at 1x1 right after it links
(`Session::prime`), because Mesa's llvmpipe and several Windows drivers defer
finishing a program to its first draw, which would put the cost on a cue's
first frame. The scene's clock is put back afterwards.

**Would reopen:** the T480 showing a load time that matters for an 80-scene
show (the compile could move behind a progress screen with the output blacked
out), or resident memory for many scripted scenes.

### D152: "zero compiles during playback" is measured, not hoped (2026-10-10)

`Gl::shader_compiles()` counts every shader stage the context is asked to
compile (a shared counter, like the lost flag). `Show` records it when the load
finishes and checks it each frame; a change is added to
`Show::runtime_compiles()`, logged as a warning, and reported at close
(`closed with N runtime shader compile(s)`). Tests assert it: `tests/show.rs`
plays a whole show on headless GL in both dialects and compares the counter
before and after, `crates/strobe-gfx/tests/compositor.rs` shows a hundred
transition frames compile nothing, and the Xvfb runs log `closed with 0
runtime shader compile(s)`.

`Gl::shader_compiles()` is the counter D161 relies on too; there is one counter
and this decision uses it. D161 removed the one known way a scene recompiled
after load (a script creating a uniform late), so the count is a tripwire for
a regression rather than a known leak.

What the counter **cannot** see: a driver that finishes a program late (hence
the priming draw in D151), and state-dependent recompiles inside the driver.

### D153: warm-up renders the next scene at half each edge, and the resize keeps what it drew (2026-10-10)

On `WarmUp` (two seconds before a cue ends) the incoming scene is taken from
the cache, given the cue's control values, and rendered with its own clock at
**half each edge, a quarter of the pixels** ("quarter resolution"). At its
cue it is grown to full size with `ScenePlayer::resize_keeping`, which scales
every pass target and the finished frame into the new targets with a GPU blit
(linear, except nearest for 32-bit float passes, which GL only copies that
way). The ordinary `resize` restarts from black, which would make the warm-up
pointless for exactly the scenes it exists for, so it was not reused.
`tests/compositor.rs` proves a feedback accumulator continues across it (0.4
becomes 0.5 where a plain resize gives 0.1).

- A cue reached by a command (Next, jump, restart) has no warm-up and starts
  cold, at full size.
- The same scene twice in a row, or a restart, keeps the live session and only
  re-applies the cue's settings (an A-to-A "transition" would mix a frame with
  itself).
- Control overrides are written as targets when the scene is taken for
  warm-up, so smoothing has had two seconds to settle; a cold start eases from
  the defaults. `preset` is read and ignored with a warning until presets
  exist (Phase 6); `meta` overrides are written to the session's meta bank (D160), which no
  post stage reads yet.

### D154: the compositor is one program compiled when the context opens (2026-10-10)

`strobe_gfx::Compositor` mixes the outgoing and incoming frames into a target
of its own at the render size, with the same orientation as a player's output,
so the preview, the letterboxed present and screenshots read it exactly like a
scene's. Crossfade and fade-through-black share one program: the CPU turns
`progress` into two weights (`weights()`, unit-tested) and the shader is
`from * wa + to * wb`, so progress 0 is exactly the outgoing frame and 1
exactly the incoming one. Fade-through-black is `1 - 2p` then `2p - 1`.

It is built in `Stack::open`, so a context rebuild gets a new one and nothing
compiles per transition. A **custom GLSL transition** is a file defining
`vec4 transition(vec2 uv)` with `getFromColor`, `getToColor`, `progress` and
`ratio` (the GL Transitions shape, so that collection's files work), compiled
by `Compositor::add_custom` as a step of the playlist's load; a file that does
not compile is a console entry and its cues crossfade instead. Parameters
with default-value comments, as that collection uses them, are not read.
`playlists/transitions/wipe.glsl` is the sample. A cut has no compositor step.

During a transition both scenes render, which doubles the scene cost for its
length; adaptive resolution (D64) is what absorbs that on an old laptop.

### D155: the resume point is written by a thread, and the wall clock outranks it (2026-10-10)

`SaveTimer` says "now" every five seconds of frame time, and only if the point
changed (a held cue is not rewritten for an hour). `ResumeWriter` is a thread
that keeps the newest point it was sent and writes it with the playlist crate's
atomic save, so the frame loop does file I/O never. One file,
`resume.toml` in the data directory (`STROBE_DIAG_DIR` moves it), holding the
playlist identity; a point for another playlist, or for a cue that is gone, is
refused with the reason and the show starts from the top. A finished
playlist clears the file. `--no-resume` neither reads nor writes it, and
**screenshot mode never does**, like the saved UI layout, so a capture does not
depend on the last run.

At start the order is: the **wall clock** if the playlist is anchored (a late
start jumps to the cue the schedule names, an early start waits and says so, a
finished one-shot says so), then the resume point, then the first cue
(`show::start::plan`, tested). A resume point is ignored for an anchored
playlist: it would put the show where the schedule says it is not. Verified
under Xvfb: stop mid-show, restart, `resumed the saved position`; an anchor a
minute in the past lands on the scheduled cue.

### D156: the transport with a playlist (2026-10-10)

The strip keeps its layout and changes its meaning, and says which it is in:
**Hold** latches (accent fill, `HELD` beside the time, Space), the time is a
countdown (`-1:23`) or, for a cue only a person ends, time played (`+0:41`),
`FADE 58%` appears during a transition, NEXT says `cue order`, and with no
playlist everything is as D135 left it. Hold freezes the cue's *clock*, not
the picture (D76). Added commands, in the one table that keys, buttons and
later MIDI/OSC share: **E** extend 30 s, **R** restart cue, **Shift+Right**
skip section; the LIST pane has buttons for the same. Blackout is one flag
with two writers (the button and the clock's command); `SetBlackout` keeps the
clock's in step and the clock's event sets the app's. An anchored playlist
that is waiting shows the wait in place of the time.

### D157: LIST is a pane, the generator draws on name words, and Save rewrites the file (2026-10-10)

UI_DESIGN section 13 asks for a dedicated workspace with a selected-cue
inspector. What is built is a **right-dock pane** (`Pane::Playlist`, 340 to 720
points) because the pane system (D120) gives docking, saved width, Esc, the
narrow-window drawer and the rail's active state for free, and a true
workspace means a second shell. The pane has the load field, the summary, the
cue commands, the Generate card and a virtualised timeline (a screenful of
rows, however long the show), each cue with its computed start, length,
transition, wall time and state (`NOW`, `NEXT`, `manual`, `skipped` with the
reason on hover), double-click to jump. Left for the workspace: the selected
cue's inheritance inspector, multi-select edits, drag from the library.

**Generate** is a card in the pane over `strobe_playlist::generate`: target,
pool, duration rule, transition rule, no-repeat window and seed, a preview
(count, estimated length, pool size, effective window, first cues), then
"Add to playlist". The scene format has no tag metadata, so the library
snapshot gives every scene its **name words as tags** (`05-domain-warp` is
`domain` and `warp`); a pool word also matches a whole scene name. With no
playlist loaded, Add starts one. **Save** writes the model with
`file::save`, so comments in a hand-written file are lost; the button says so,
and the file watcher is told the new version is the app's own.

### D158: the playlist file is watched by polling, and a reload is lenient (2026-10-10)

`show::watch` reads the file's modification time and size once a second
(counted from frame `dt`: one `stat` per second, no thread, no crate) and
reports a change only after the stamp is the same on two polls in a row, so an
editor's truncate-write-rename is not loaded half done. A reload that parses
swaps the playlist in place (`Show::replace_playlist`): compiled scenes are
kept, only new ones compile (the live scene keeps rendering meanwhile), and
the clock goes back where it was by `restore_lenient`, because adding a cue
changes the identity and a hot edit is meant to be the same show. A reload that
does not parse keeps the playlist that is playing, shows the precise error
(line, column, the text) in the LIST pane and a banner, and the next save is a
new change. Loading from the field or `--playlist` follows the same rule: an
error is shown and the previous playlist stays.

**Would reopen:** an editor that saves in a way two polls apart cannot settle,
or a playlist directory with many files (a recursive watch).

### D160: meta controls are a second `ControlBank`, and a script's `meta/` writes take the normal path (2026-10-10)

Scripts reach Synesthesia's meta controls with a `meta/` prefix or
`bank: "meta"` on `setControl`, `setControlNormalized`, `randomizeControl`,
`defaultControl`, `randomizeGroup` and `defaultGroup` (PLAN §1.5). Strobe
used to record those writes in `FrameOutput::meta_writes` and the app dropped
them with one warning.

They now go the way every control write goes. `strobe_scene::meta` defines
the output-wide set (master, invert, brightness, gamma, contrast, hue,
saturation, vertical and horizontal mirror, reactivity, audio speed) as
`scene.json`-style control objects validated by `ControlDef::from_raw`, and
`meta::bank()` builds a `ControlBank` from them. `ScriptHost::new` takes that
bank beside the scene's own, resolves `meta/` names against it, and
`ScriptHost::frame` takes both banks mutably. A native call queues a
`Command` tagged with its target bank, and the host applies the queue in
order at the end of the frame, so clamping, toggle snapping, normalised
writes, `randomize` (the host's seeded generator) and `default` are the one
implementation. An unknown meta name is one console error, as for scene
controls ("no meta control named `x`"), not a fault. `MetaWrite`,
`MetaAction` and `FrameOutput::meta_writes` are gone, because the host now
applies what it used to report.

`Session` owns the meta bank (`pub meta`), ticks it after the scene's
controls, and passes it to the script.

**What this is not.** There is no output post stage in the renderer, so
nothing reads the meta values: brightness, hue and mirror do not change the
picture. That is stated in the code, the READMEs and the audit rather than
faked. A scene change builds a fresh `Session`, so meta values also reset
there; carrying them across scenes belongs with presets and the playlist's
`MetaOverrides`, which are still placeholders. Scripts cannot *read* meta
values as globals: nothing documents that they can (Q21).

The ranges and the list are Strobe's, from the Synesthesia UI as PLAN §1.6
records it; Q21 lists what is unverified.

**Would reopen:** a real script that reads a meta value, or the output post
stage landing (then the bank is what it reads).

### D161: a script uniform created after load is refused, not recompiled for (2026-10-10)

D62 let a script publish a uniform after load at the price of one logged
recompile. That broke the rule AGENT_RESEARCH §3 states, "zero shader
compilation in the show": a recompile on an old laptop is a visible hitch,
and the precompile guarantee for playlists (§11) cannot hold if any script
can ask for another. Two options were weighed: refuse, or pre-declare through
a new mechanism (a manifest in `scene.json`, a `declareUniform` call).
Refusing needs nothing new from scene authors: `setup()` and the first
`update()` already run before the shaders are compiled, so a uniform created
there is declared, and that is the documented way to create one.

- `ScriptHost::seal_uniforms()` (replacing `announce_extras`) lists the
  committed uniforms and seals the store. After it, `setUniform` of a name
  not in that set stores nothing and logs **one** console error naming the
  uniform and the rule ("created after the scene loaded... create every
  uniform in setup() or in the first update()"). Declared uniforms keep
  updating, a different arity keeps being padded or truncated, and the
  deprecated `uniforms` object obeys the same rule. A refusal is not a fault.
- `FrameOutput::new_uniforms` is removed: nothing can become new.
- `Session::load` seals right after building the player;
  `Session::adopt_new_uniforms`, its `rebuild_failed` flag and the stored
  `SceneFolder`, `Gl` and `Caps` it needed are gone.
- `Gl::shader_compiles()` counts compile requests (a client-side counter,
  no driver query). `script_uniforms.rs` loads a scene, runs twelve frames
  in which the script creates `late` on frame four, and asserts the counter
  did not move and the uniform was refused, in both dialects.

A scene whose *shader* uses a late uniform fails to compile at load, as it
always did: it was never declared. The error says which name; the author
moves the `setUniform` into `setup()`.

**Would reopen:** a real scene that creates a uniform conditionally and
reads it in the shader. The answer would then be a declared-uniform manifest,
not mid-show compiles.

### D162: `textureFinalPass` is a 64x36 asynchronous readback, two frames late (2026-10-10)

`textureFinalPass(x, y)` used to return black (D62). The picture reaches the
CPU like this, and `crates/strobe-gfx/src/readback.rs` is the whole of it:

1. **Downsample on the GPU.** A chain of `glBlitFramebuffer` linear blits,
   each at most 2:1 per axis (1280x720 goes 640x360, 320x180, 160x90, 80x45,
   64x36), so each step is an exact 2x2 box average and a busy picture does
   not alias the way one four-tap blit from 1080p would. The chain's textures
   are allocated at load and on resize.
2. **Start the read.** `glReadPixels` of the 64x36 RGBA8 image into the next
   of **three** pixel-pack buffers, then a fence behind it. The call returns
   at once. PBOs, `glMapBufferRange`, sync objects and `glBlitFramebuffer`
   are all core in desktop 3.3 and in GLES 3.0, so one code path serves
   native and ANGLE.
3. **Collect without waiting.** The oldest slot (issued two frames ago) is
   polled with `glClientWaitSync` at timeout 0 and, only if signalled, mapped
   with `MAP_READ_BIT`, copied to an RGB8 buffer kept between frames and
   unmapped. If it is not signalled it is skipped; when the ring comes round
   to it the stale read is dropped (counted) and the slot reused. A slow GPU
   costs freshness, never frame time. No `glGet*`, no per-frame allocation.

**Latency, the answer to Q14's "how stale".** After frame `k` renders,
`pixels()` is frame `k - 2`; a script running before frame `k + 1` sees the
picture of three frames earlier (about 50 ms at 60 fps), more if the GPU lags.
Until two frames have been rendered the script reads black. This is
documented on `textureFinalPass`, `ScriptInputs::final_pass` and the READMEs.

**Size.** 64x36 is `strobe-script`'s existing constant (`FINAL_PASS_WIDTH`,
`HEIGHT`), kept. AGENT_RESEARCH does not mention `textureFinalPass` at all
and PLAN §1.5 gives only "RGB of the final pass at normalized coords", so the
resolution is a Strobe choice, not a verified Synesthesia fact (Q14). An
integration test pins that the gfx and script constants agree.

**Cost control.** The readback runs only for a scene whose `script.js`
contains the text `textureFinalPass` (`Session::load`). Other scenes pay
nothing. A script that builds the name dynamically would get black; nobody
writes that.

**Lost contexts (D66).** `Buffer` and `Fence` are owning types whose `Drop`
skips the delete once `Gl::is_lost()`, and `capture` does nothing on a lost
context. `resize` rebuilds the reader and the picture restarts from black.

**Rows run bottom-up**, the orientation of `_uv`, as `strobe-script`
documented; a test pins it with a gradient.

**Evidence**: llvmpipe, both dialects (`crates/strobe-gfx/tests/render.rs`,
`crates/strobe-app/tests/script_uniforms.rs`). Nothing here has run on the
T480 or on ANGLE/D3D11, where fences and PBO maps behave differently; that
is the open check.

**Would reopen:** a measured readback cost on the T480, or a real script
that needs the previous frame exactly (then a synchronous path for that
scene alone, chosen by the author).

### D163: `PASSINDEX` compared with a float literal is cast on GLSL ES (2026-10-10)

R4: real scenes compare `PASSINDEX` with `0` and with `0.0`, and `PASSINDEX`
is an `int`. Desktop GLSL converts; GLSL ES rejects `int == float`, so such a
scene could not use the ANGLE fallback. `strobe_scene::fixups` rewrites the
user's code for `Dialect::Es300` only: a `PASSINDEX` that is an operand of
`== != < > <= >=` whose other operand is a float literal (`0.0`, `.5`, `2.`,
`1e2`, either order) becomes `float(PASSINDEX)`.

Chosen over declaring `PASSINDEX` as a float on ES because the int form is
just as common and the two cannot both compile against one declaration.
The rewrite is a small tokeniser, not a parser: it skips comments, never
touches a longer identifier, treats `<<` and `>>` as shifts, leaves hex and
`u`-suffixed literals alone, and never adds or removes a line, so
driver error lines still map to `main.glsl`. In specialised mode the cast is
of a `const int`, so the branch still folds away.

Not done, and why: `float p = PASSINDEX;`, `float s = 2;` and every other
implicit int to float conversion need types, so a GLSL parser (Q8 stays
open for them). The packing built-ins on a 3.30 context are unrelated.

**Would reopen:** adopting a GLSL parser, which would replace this module.

### D170: show control is one crate, `strobe-io`, that depends on `strobe-scene` and nothing that draws (2026-10-11)

Phase 6 (PLAN.md section 11, ROADMAP item 7). MIDI input, OSC input, the
mapping model, learn, soft takeover, the Performance Macro set and the saved
file are all in `crates/strobe-io`, which depends on `strobe-scene` (for
`ControlBank`, the script API's name matching and presets) and on `rosc` and
`midir`. It does **not** depend on `strobe-gfx`, a window or egui, so every
rule in it runs in a unit test with no GPU, no device and no display: over
120 tests there and a dozen for presets in `strobe-scene`.

- **Libraries.** `midir` 0.11 for MIDI (ALSA, WinMM, CoreMIDI) and `rosc` 0.11
  for OSC 1.0. Context7 has neither, so their sources were read as fetched.
  Both pass `cargo deny` with no new licence. **`midir` needs no C compiler on
  Windows** (it uses the `windows` crate's WinMM bindings), so the Windows
  cross-check builds `strobe-io` whole and needed no exclusion, unlike
  `strobe-shadertoy`'s `net` feature (D110). The `midi` feature (on by
  default) is still there so a build for a target without a MIDI backend can
  drop it; the crate then reports "built without MIDI support" and carries on.
- **Why `strobe-scene` is a dependency.** A mapping has to know that a toggle
  flips, a bang counts presses and a dropdown rounds, and it must not know
  those rules twice (AGENT_RESEARCH: "UI, JavaScript host, MIDI and OSC should
  all target the same control bank"). The mapper writes `ControlBank::
  set_normalized`, `set` and `press` and nothing else, so smoothing, `speed`
  integration, bang envelopes and clamping are the one implementation. A test
  pins that a mapped smoothed slider's target moves at once and its output
  eases.
- **Presets are in `strobe-scene`**, not here (PLAN section 3.1 puts them
  there): they are a function of a `ControlBank` and a folder, and the playlist
  needs them without MIDI.

**Would reopen:** a target that needs a second crate to resolve (a media slot,
a script uniform), which would want the `Scope` to grow rather than the crate
to depend on more.

### D171: two worker threads and one bounded queue, drained once per scene frame (2026-10-11)

ARCHITECTURE's frame step 1 ("drain I/O events") is built. `Hub` runs a MIDI
worker and an OSC worker; both hand typed `Event`s to the app through **one
bounded `std::sync::mpsc::sync_channel` of 2048**, with `try_send`: a full queue
drops the newest message and counts it (`Status::Dropped`, shown in the MAP
pane and logged), because a stalled frame loop must never push back into a
driver callback and a flood of fader messages is exactly what should be
thinned. No new dependency (`crossbeam-channel` was in PLAN; std was enough).

- **Drain.** `Remote::drain` runs in `scene_frame` after the resize housekeeping
  and **before the scene advances**, takes at most 256 events, never blocks,
  and reuses its buffers. A message that arrives mid-frame waits for the next
  one (at most one frame, 7 ms at 144 Hz, 16 ms at 60). MIDI and OSC do not
  go through the 30 Hz control-window redraw, which would have made a fader
  feel laggy and stopped when the window was covered.
- **Hot-plug.** `midir` reports nothing when a device appears or goes, so the
  MIDI worker polls the port list once a second, opens new devices, drops the
  ones that vanished (`DeviceLost`, logged as a warning) and opens them again
  when they return. A device is identified by `device_key`, the port name
  without ALSA's `20:0` suffix and WinMM's `2- ` prefix, so a mapping survives
  a replug. A device that fails to open is recorded in its list entry and not
  retried every second; Refresh retries. No MIDI service at all (a container,
  a missing driver) is one `MidiUnavailable` status, retried every five
  seconds, and nothing else.
- **OSC thread.** One long-lived thread with a 50 ms read timeout, so a
  rebind, a stop or a shutdown is noticed without the app ever joining a
  blocked read. `Hub::set_osc` returns at once.
- **Activity.** The monitor folds consecutive messages from one source with the
  same result into one line with a count, so a sweep is one line and one log
  line, not hundreds.

**Not verifiable here:** MIDI hardware. There is no `/dev/snd/seq` in the
cloud container and `snd-virmidi` cannot be loaded, so MIDI is tested through
`message::parse` with recorded byte sequences (a fader sweep, a pad, a wheel, a
patch change, clock ticks, sysex, malformed and truncated messages) and the
worker's own code paths only compile and start (reporting "no MIDI service" in
the container, which the test pins). What only a controller can show: the
real ALSA/WinMM port names, hot-plug timing, and a Windows device another
program has open. See STATUS_AUDIT.

**Would reopen:** a controller whose message rate overruns 256 per frame at
60 fps (about 15,000 a second), which would raise the cap, or a driver that
does report hot-plug, which would replace the poll.

### D172: OSC is off until asked for, binds to this machine, and a command-line flag is never saved (2026-10-11)

OSC has no authentication. Anyone who can send a UDP packet to the port can
move controls, change scene, recall a preset or **black out the projector**.

- **Off by default.** `--osc`, or the MAP pane's switch (saved), turns it on.
- **Loopback by default.** It binds `127.0.0.1:9000` (`--osc-port`, or the
  pane). Projection rigs often *do* need LAN control (a phone running
  TouchOSC), so every interface is available, but only by an **explicit
  choice**: `--osc-all-interfaces`, or the pane's "All network interfaces"
  checkbox, whose tooltip says what it means. Choosing it logs a warning and
  the pane shows "Anyone on this network can send commands to the show."
- **A flag is not a setting.** `--osc`, `--osc-port`, `--osc-all-interfaces`
  and `--no-midi` change what runs *this time*. The saved settings stay as
  the operator left them (a test pins this for OSC and for `--no-midi`, which
  was found persisting in the first live run), so a run with
  `--osc-all-interfaces` cannot leave the next launch open to the network. The
  pane says "Running with command-line settings; they are not saved" while the
  two differ. Choosing in the pane replaces both.
- **The datagram is hostile input.** `rosc` decodes bundles recursively; a
  64 KB packet can nest thousands deep and a release build aborts on a stack
  overflow. `osc::nesting_ok` walks the bundle structure first, iteratively
  and bounded (depth 4), before `rosc` sees the bytes; lying sizes, negative
  sizes and truncation are rejected there. Also bounded: 256 messages per
  datagram, 8 arguments, 256-byte addresses; strings and blobs are dropped, and
  a message that had arguments but kept none is dropped whole (so
  `/strobe/cmd/blackout "hi"` is not a bare "go"). Garbage is dropped silently:
  logging each packet would be a way to fill a disk. Tested with a 3000-level
  bundle, lying sizes, garbage, NaN and over-long fields.
- **Not done:** a shared secret or an allow-list of senders. The planned LAN
  web remote (ROADMAP item 7b) has pairing and per-client tokens and is the
  route for an untrusted network; OSC stays a trusted-network protocol.

**Would reopen:** a request for OSC across networks the operator does not
control, which should be refused in favour of the paired remote.

### D173: mappings name their targets, resolve them against the scene that is playing, and show six states (2026-10-11)

UI_DESIGN section 8 and UI_IMPLEMENTATION_PLAN sections 11 to 12, as code.

- **Source -> target.** A `Source` is a MIDI CC, note, pitch bend or program
  change (device or any device, channel 1 to 16) or an exact OSC address. A
  `Target` is a scene control, a meta control (each by name, with a component
  for a pad or colour), a command (by the stable id in the command table), a
  cue number, a fav slot, a preset, or a Performance Macro. Nothing is
  stored by index, so a mapping outlives the scene it was made in.
- **Resolution** is exact and never fuzzy: the script API's normalised name
  (case, `_`, `-` and spaces ignored), then `group/name`, then the macro's
  aliases in order. `spee`, `speeds` and `zoom` for `zoom_level` all fail.
- **Six states** (a word and an icon, never colour alone): *Resolved*;
  *Unavailable* (the control is not in the playing scene: kept, muted, says
  which; not an error and no log line per message); *Broken* (can never work
  as written: unknown command, fav slot 9, an XY pad with no component, a macro
  that points at a macro); *Disabled*; *Learning*; *Conflict*.
- **What a signal does** (engine module docs have the table): analog goes
  through `invert`, `curve` (linear, log, exp: one-parameter exponential
  families that are each other's inverse), then `range` as fractions of the
  target's own range, then `set_normalized`. A button on a non-button control
  is `toggle` (flip, or flip a slider between its range ends) or `momentary`.
  An analog source driving a button-like target presses at `threshold` and
  releases 0.05 below, so a shaky fader at the line does not machine-gun a
  command (tested). Dropdown: analog picks the option, a button steps and wraps.
  XY and colour need a component.
- **Built-in OSC addresses** work with no mapping: `/strobe/control/<name>`,
  `/strobe/control/<name>/<x|y|z|r|g|b>`, `/strobe/meta/<name>`,
  `/strobe/cmd/<id>`, `/strobe/cue/<n>`, `/strobe/fav/<n>`,
  `/strobe/preset/<name>` (`_` reads as a space), `/strobe/macro/<id or name>`.
  This is what makes "every playlist action and control reachable from OSC"
  true on a fresh install. Values are **normalised 0 to 1** (what TouchOSC
  sends) and clamped; toggles follow their argument (`1`/`T` on, `0`/`F` off);
  commands fire on `1`, `T`, an impulse or no argument and ignore `0`, and
  keep no state, so a script sending only `1` still fires every time. A mapping
  for an address always wins over its built-in meaning. Strobe's own
  namespace is a choice, not Synesthesia's (Q23).
- **Commands.** D134 asked that MIDI and OSC "map onto `CommandId` rather than
  a second list". `CommandId` gained stable ids (`next_scene`, `toggle_hold`,
  `toggle_blackout`...) and `mappable()` (the two presentation commands, focus
  search and close pane, are not reachable from a controller). The mapper
  returns the id and the app looks it up in the same table the keys use and
  runs the same `Action`. Blackout is a toggle only; there is no explicit
  on/off command yet.

**Would reopen:** Synesthesia's real OSC addresses (Q23), or a request for
explicit blackout on/off.

### D174: soft takeover, learn, and what a conflict is (2026-10-11)

- **Pickup** (the default for MIDI faders) ignores the fader until it
  *reaches* the control's value, by landing within 0.025 of it or by crossing
  it between two readings; from then on it drives the control. If something
  else moves the control (the inspector, a preset, a fav slot, another
  mapping) the fader is dropped again and must be picked up anew; "something
  else moved it" is noticed by comparing the control's target with what the
  mapping last wrote, so it needs no hook in the UI. The comparison is in the
  control's own normalised terms after range, curve and invert. It applies to
  continuous kinds only: a dropdown quantises, which would read as a foreign
  write. **Jump** follows at once. OSC mappings learn as Jump (a sender cannot
  be picked up).
- **Learn.** Arm a target (Add..., or Learn on a macro card or a row); the
  next *usable* message binds: a note off or a stray release is ignored, so a
  pad binds on its press. Learn on an existing row replaces its **source** and
  keeps its range, curve and the rest. A learned MIDI source remembers its
  device; "Any device" is one click in the row's options.
- **Conflict.** If the heard source is already mapped to something else (any
  enabled mapping whose source would also fire: same message, same device or
  either with no device), nothing is bound and the banner says "CC 21 ch 1 is
  already mapped to speed" with **Replace** (remove the holder, bind the new
  one) and **Cancel**. Binding the same source to the same target again is a
  quiet success. Two existing rows that share a source, from a hand edit,
  both show *Conflict* until one is changed.

**Would reopen:** a fader that never engages in practice (a tolerance too
tight for a 7-bit controller), which would widen it or add a per-mapping
setting.

### D175: Performance Macros, stage 2: a curated, pinned, ordered, saved set (2026-10-11)

D137's stage 2. A `Macro` is a stable operator binding: id (never reused), name,
a target (never another macro), aliases, `pinned`, `enabled`, an optional
`range` that narrows what a mapping may reach, an identity colour slot. The
set is **global to the show** and saved with the rest of show control; a scene
change never deletes or reorders a macro, and one whose control the new scene
lacks goes **unavailable** ("unavailable in current scene", muted) until a scene
that has it plays. A mapping to `Macro { id }` is how a fader drives "macro
slot 2" without caring what it is today.

- **The strip** shows the pinned macros in order, scrolling sideways when they
  do not fit, each as a card: target badge (SCENE, META, PLAYLIST, SYSTEM),
  name, a MIDI or OSC tag with the controller on hover, the state when it is not
  working, and a widget fitted to the target (rail, switch, trigger, dropdown,
  swatch; a button for a command, a cue, a fav slot or a preset). The widget
  writes the same `ControlBank` entry the inspector does. Right-click: Learn,
  Move earlier, Move later, Unpin.
- **With no macros at all** the strip keeps its stage 1 behaviour (the scene's
  first controls, marked SCENE), so a new install is not an empty bar; one
  macro and only macros show.
- **The MAP pane** lists every macro (pinned or not) with its state, sources
  and Learn, and edits name, aliases and on/off; *Add...* builds one from a
  scene control, a meta control, a command, a preset (with "Pin it as a macro
  too"). Pinning a control from the Controls pane is not built (Add... takes
  the name, with a picker over the playing scene's controls).
- **Not built:** the range override and the identity colour have data and
  engine support (the range narrows a mapped fader, tested) but no editor;
  smoothing and a shortcut per macro (UI_IMPLEMENTATION_PLAN section 11) are
  left, though a macro can be a mapping target and so has MIDI/OSC.

**Would reopen:** per-scene macro sets (a VJ who wants a different strip per
scene), which would key the set by scene name; the global set was chosen
because the brief says never to reorder because a scene changed.

### D176: presets live in the scene folder, are read at load, and layer under a cue's overrides (2026-10-11)

`presets/<name>.json` in the scene folder, in PLAN section 10's shape
(`name`, `channels: { scene, meta }`, `scene: { control: value }`,
`meta: {...}`), values a number or one per component, a dropdown's index.

- **Capture and apply go through `ControlBank`** (`target`, `set`): a recalled
  preset is a write of targets exactly like the inspector's. Bangs are not
  saved; the meta `master` is not saved (PLAN: the operator's fader, not a
  look). Apply is **by name** (the script API's match), so a preset survives a
  scene gaining controls; a name the scene lacks, a wrong component count or a
  NaN is *reported and skipped* and the rest applies; a control the preset does
  not mention is left alone, so a hand-written preset can hold two values.
- **Names are file names, so they are checked.** A preset name arrives from a
  text field, a playlist and an OSC address: letters, digits, space, `-`, `_`,
  `.` (not first), at most 48; and a preset is found by *listing the folder*
  and comparing stems, never by joining the name onto a path. Tests cover
  `..`, separators, drive prefixes, NUL and length.
- **Read at scene load** (`Session::load`), kept in the session: recalling a
  preset, a cue starting and the MAP pane touch no disk. A save writes one
  small file aside and renames it, on the action's thread (a button press, not
  a frame); the session's copy is updated in memory.
- **`default.json`** is the scene's starting look: applied after load, and to a
  cue that names no preset.
- **Cue `preset`** (ignored with a warning since D153) now applies. The cue's
  settings layer, later over earlier: scene defaults (reset), the cue's preset
  (or `default`), the cue's explicit `overrides`. A preset the scene lacks is
  warned about **at compile time** (not mid-show), and the cue falls back to
  `default`. Tested on headless GL in both dialects
  (`tests/show.rs::a_cues_preset_is_applied_...`), with zero compiles.
- **Fav slots**: eight, global, each a scene and optionally one of its presets,
  stored in `show-control.json`. A click recalls (loading the scene first if it
  is not the one playing, which unloads a playlist as any manual load does,
  D150); an empty slot stores the playing scene and its last recalled or saved
  preset. Reachable from MIDI, OSC (`/strobe/fav/n`), the macro strip and the Controls
  pane.

**Would reopen:** a scene that wants presets that cover media (PLAN section 1.6
lists a Media channel); there is none until media is controllable.

### D177: MAP is a pane, built the way LIST is (2026-10-11)

UI_DESIGN section 15 describes a Mappings *workspace*; like LIST (D157), this
is a right-dock **pane** (`Pane::Map`, the rail's MAP entry, `--open-pane map`),
because the shell has panes and no workspaces yet. Top to bottom: a learn or
conflict banner; Connections (OSC switch, status chip, port, "All network
interfaces", built-in addresses, MIDI switch, one row per device with its
state); Activity (the last eight lines: time, what arrived, what it did,
a count, dropped total); Macros; Mappings with search, *Add...* and per-row
options (device, range, invert, curve, fader mode, button mode, press
threshold). The UI never touches a socket, a device or the disk (the source
scan still runs over it and `presets`): worker, file and scene changes go back
as `Action`s (`SetOsc`, `SetMidiDevice`, `SavePreset`, `RecallFav`...);
mappings and macros are edited in place, they being the mapper's own data.

- **Found and fixed by the live run:** a row that cannot shrink (a monitor line
  with a long result) makes an egui panel wider than its dock and shoves the
  preview; headless layout tests did not see it because they had no activity.
  Rows now truncate or wrap, and the test scenario carries the long lines.
- **Verified** under Xvfb with real UDP datagrams (hand-built, from
  `python3`): hue_shift 0.8 and master 0.5 on the macro strip, a control that
  is not in the scene reported "unavailable", a command that does not exist
  warned, `next_scene` loaded the next scene, `toggle_blackout` logged
  `blackout on`; and a real click on a macro's Learn button followed by an OSC
  message bound `/9/fader` to macro #1, wrote it to `show-control.json`, and
  the next message moved the control. Screenshot:
  `docs/images/perform/map-learning-1440x900.png`.

**Would reopen:** the Mappings workspace proper (a full-width table with the
inspector beside it) once the shell has workspaces.

### D178: keyboard shortcuts are configurable by a file, and cannot be configured into a clash (2026-10-11)

`hotkeys` in `show-control.json` maps a command id to a chord
(`"toggle_blackout": "Ctrl+B"`). D134's focus rule is untouched: the same
`dispatch` decides, now over `Bindings` (the defaults plus the overrides), so a
text field, a modal and a focused slider still win. `Key` gained letters,
digits and F1 to F12 (B, E, R, F, T and F11 keep their own variants, so a key
has one form). A line that names an unknown command, cannot be read, or would
put two commands on one chord is **left out and logged**; the command keeps its
default, so no command is ever left unreachable. A swap is two moves through a
free key. Read once at start; the Settings pane lists the effective
shortcuts. **No in-app editor** (the table of commands is not built into a
palette yet); the output window maps the same keys, with no modifiers.

**Would reopen:** the command palette (UI_DESIGN section 25), which is where a
rebinding editor belongs.

### D179: `show-control.json` is tolerant, atomic, and keeps a backup of what it could not read (2026-10-11)

One JSON file in the data directory (`--show-control <file>` for another):
OSC and MIDI settings, mappings, macros, fav slots and hotkeys. Every field
has a default; unknown keys are ignored; **one mapping that does not parse is
skipped and named, the others load** (an unknown source kind, a typo in a
target); a file that is not JSON loads as the defaults with the reason; and
before the app first writes over a file that had unreadable parts it copies
the original to `show-control.json.bak`, so a hand edit is never lost to a
typo. It is written by the same debounced, atomic (temp file and rename)
writer thread as the UI preferences (`prefs::Saver`), never from the paint
path, 500 ms after the last edit and at shutdown, and **never in screenshot
mode** (which reads a `--show-control` file but cannot write it). JSON rather
than TOML because the preset files are JSON (PLAN section 10), and an
externally readable tagged enum is the natural shape for a target.

**Would reopen:** a sync of the file between machines, which would want a
version and a merge story the `version` field only anticipates.

### D180: the output post stage is one program, after the compositor, and it costs nothing at neutral (2026-10-11)

D160 built the meta bank and nothing read it. `strobe_gfx::post::Post` is the
reader: master, invert, brightness, gamma, contrast, hue, saturation and the
two mirrors, applied to the finished frame.

- **Where.** After the last pass of the live scene, or after the compositor's
  mix while a playlist transition runs (ARCHITECTURE, "A frame"). Applying it
  after the mix means a crossfade fades one picture, and the output has one
  master however many scenes are rendering.
- **One program, compiled once.** `Post::new` runs in `Stack::open` beside the
  compositor (D154, D152), so a context rebuild gets a new one and nothing
  compiles during a show. A change of value is a handful of uniforms.
- **Zero cost at rest.** `MetaValues::post_is_neutral` is true when every
  pixel-affecting value is within 1e-4 of its default (a hue of exactly one
  turn counts: it is no rotation). Then `App::apply_post` returns before
  touching the stage: no draw, no target, and the present, preview and
  screenshot keep reading the scene's or the compositor's texture. Reactivity
  and audio speed do not count, they are not pixels. When it does run it is
  one full-screen pass into an RGBA8 target of the source's size; the target
  follows the source's size on the first frame after a resize (no steady-state
  allocation, no `glGet*`).
- **Order, per pixel.** Mirrors (exact `texelFetch`, no resample), brightness
  `c + b`, contrast `(c - 0.5) k + 0.5`, gamma `c ^ (1/g)`, hue then
  saturation as one 3x3 matrix, clamp, invert, master. Master is last so `0` is
  black whatever else is set. Alpha passes through.
- **Hue and saturation** are a CPU-built matrix (`post::color_matrix`): hue is
  a rotation about the grey axis `(1,1,1)`, saturation mixes towards Rec. 709
  luma. At hue 0 and saturation 1 it is the identity *exactly*, so a neutral
  pass through the shader is bit-for-bit the input (a test, in both dialects).
  A rotation about the grey axis is not luma-preserving, so a large hue shift
  changes perceived brightness a little; a YIQ rotation has the same fault and
  does not give an exact identity.
- **Mirrors.** *Horizontal* reflects the left half onto the right; *vertical*
  reflects the bottom half onto the top; an odd size's centre line is its own
  mirror. Which half is kept and which axis the names mean are guesses (Q21).
- **The preview shows what the output shows.** When the stage ran this frame
  the control window's preview paints its target.

**Would reopen:** a measured cost on the T480 that justifies folding the
stage into the present blit when the output is shown (saving the extra target)
at the price of the preview needing its own pass; a real Synesthesia scene
showing a different order of operations.

### D181: blackout and master are separate, and they compose (2026-10-11)

Q20 asked whether blackout should gate every output. They are two things.
**Master** (a meta control, D160) is a level in the *picture*: the post stage
multiplies it in, so the preview, a screenshot and any future capture or NDI
output see it, a script can drive it, and a playlist cue can set it. **Blackout**
(D135) stays what it was: a panic key that clears only what the shown output
window presents, leaving the scene running, its clock going and the preview
live, so bringing it back is instant. Blackout is applied after the post stage
(it replaces the present), so blackout over any master is dark and releasing it
shows whatever master says. Neither writes the other.

Not decided: whether blackout should gate a future NDI or capture output. That
output does not exist; when it does it should take blackout from the same flag
(the shown window is not its only reader), which is a decision for that stage.

### D182: meta values live and die with the session, and a transition uses the live scene's (2026-10-11)

`Session::meta` is per session (D160 said so). The post stage and the audio
gains read the **live** session's, which in a playlist is `Stack::session`,
the incoming scene during a transition. The outgoing scene's meta is ignored
(its frame is already inside the mix), and a warming scene's too. Consequences,
all deliberate for now: loading a scene resets the meta controls, a cue's
`meta` overrides (D153) land in the new live session, and an operator who
fades master to 0 and then switches scene sees master 1 again. Carrying meta
across scenes belongs with presets and the Meta preset channel (Phase 6), where
"master is not saved in presets" (PLAN section 9) can be decided with the rest.

### D183: the Inspector shows the meta bank as "Output (meta)" (2026-10-11)

`Model::meta` carries the live session's meta `ControlBank` and the inspector
draws it under the scene's own groups, with the same rows, collapsed state
saved with the layout, and one Reset that returns every output control to its
default. It reuses `controls::rows`, so a slider, a toggle and a group behave
exactly like a scene's. It is not a pane of its own: it is the same task,
adjusting the picture, and the inspector is where the hands already are.

### D184: meta reactivity and audio speed multiply the audio panel's gains (2026-10-11)

The app already applied reactivity and audio speed, from the Audio panel, to the
live analyser (`AudioInput::set_reactivity`, `set_speed`). The meta bank's two
controls are the same gains reached another way, so they **multiply** the
panel's rather than replace them: effective = panel x meta, clamped to the
analyser's 0 to 4. `AudioInput::set_meta_gains` is called once per scene frame
with the live session's values (not from `Session::advance`, which a playlist
runs for two or three scenes a frame and which would fight over one analyser),
and the capture is only told when the product changes. The meta range is 0 to 2
(Strobe's, Q21); with the panel at 4 the product reaches the analyser's ceiling.
Synthetic audio ignores both, as it ignored the panel: it is a fixed pattern.

### D185: cube maps are loaded at scene load, and a bad one is black, not fatal (2026-10-11)

`CUBEMAPS` entries are loaded by `strobe_gfx::assets::load_cubemap` from six
face paths (+X, -X, +Y, -Y, +Z, -Z) or one picture: a 6:1 strip, a 1:6 strip or
a 4:3 horizontal cross. Faces must be square, equal and at most 2048. They go
into a `gl::Texture` of target `TEXTURE_CUBE_MAP` (`Gl::texture_cube`, one leaf
wrapper per GL call, the owning type is the existing `Texture` with its `Drop`
that respects the lost flag, D66), linear with a mip chain, clamped on all
axes, with seamless filtering enabled on desktop (GLES always has it, and
rejects the enum). Faces are used as decoded, not flipped.

Unlike a missing `IMAGES` file, which fails the scene load, a missing or
invalid cube map **keeps the scene running**: its sampler reads a 1x1 black cube
and `ScenePlayer::take_asset_problems` returns what went wrong. The app reports
each through `strobe_diag::failure` (`failures::asset_problem`: subsystem IO,
phase `asset`, the sampler, the files, the action "runs with a black cube map"),
once, from `Session::load`. A cube map is decoration in most scenes and a
projector should not go dark for a typo; images were left as they were.

Unverified (Q22): that SSF's own `CUBEMAPS` uses these key names and a face
order, and what orientation real cross layouts have. The six-path form follows
the data model the repository already had (`CubemapDef`, from PLAN).

### D186: `syn_Media` shows a still or an image sequence, decoded off the frame thread (2026-10-11)

A scene names its picture with a `MEDIA` key (a Strobe extension: SSF documents
the key but its shape was not available to check, Q22): a path, or
`{ PATH, FPS (24), LOOP (true), WRAP (clamp), FILTER (linear), VFLIP (true) }`.
A **file** is a still; a **folder** is a sequence of its PNG and JPEG files in
name order. `--media PATH` shows one in place of the scene's own for scenes
loaded by hand (a playlist's cues keep theirs).

- **Off the frame thread, uploaded once.** `assets::spawn_decoder` reads and
  decodes on a thread of its own and sends pictures through a 3-deep channel;
  `ScenePlayer::render` uploads at most **one** per frame (`Media::poll`, a
  non-blocking `try_recv`), as a texture of its own. After that the frame only
  *binds* the texture for the scene clock (`floor(TIME * FPS)`, looping or
  holding the last). No upload, allocation or `glGet*` in steady state.
- **Limits.** Pictures longer than 2048 on a side are shrunk; a sequence stops
  at 192 MiB of decoded frames (reported, and the frames already loaded play).
  Video memory on the target is shared with the system.
- **`syn_MediaType`** is 0 with no picture showing and 1 while a still or a
  sequence is. The SSF library only tests `> 0.5` (`_isMediaActive`), which is
  all that is relied on; values for video and webcam are left unassigned.
- **Changing media** (`set_media`) keeps the old picture until the new one's
  first frame has landed, so there is no black flash; a failure leaves the old
  picture and is reported.
- **Failure** is an `AssetProblem` named `syn_Media`, reported like a cube
  map's (D185), and the scene runs with black media.
- `finish_media_loading` blocks until decoding is done, for tests and the
  benchmark's reproducible frames. The app never calls it.

**Video and webcam are not in this stage.** They need a decoder (or a capture
dependency) the engine does not carry and the target machine may not decode in
time; they are in the roadmap as later work. Nothing here should grow a heavy
video dependency.

### D190: the Parametric Flow brand is the default theme, and the icons are rendered from SVG (2026-10-11)

lnorton89/strobe#6 added the brand (`docs/branding.md`, `assets/brand/`).
Three things follow from it in the app and the repository.

**The default `strobe` theme takes the brand palette.** Void, Panel,
Border, Text and Muted are the brand's tokens verbatim, and the accent is
the brand's Cyan, because the brand's product-UI rules (section 6) say
cyan marks active selection and focus. That replaces UI_DESIGN section
18's amber. The id stays `strobe`, so a saved preference still resolves,
and the amber theme is kept as `amber` for anyone who preferred it.
`raised` and `hover` are the two steps between Panel and Border that the
brand does not name. `info` is a lighter brand Blue (`#5b9cf8`), not cyan,
so an informational chip never reads as a selection. Violet and magenta
stay out of the chrome: the brand reserves them for the output. The
existing readability tests (AAA text, AA secondary text, layer
separation) pass on the brand values unchanged.

**The icons are derivatives, generated by `cargo xtask brand`.** The guide
makes vector the source of truth and asks for a simplified small-size
variant, so `assets/brand/icon.svg` (the full ribbon with two contours, on
the dark tile) draws 48 px and up, and `icon-small.svg` (no contours,
innermost turn dropped, heavier stroke) draws 16 to 32 px. The command
writes `icon-{16..512}.png` and an `icon.ico` with PNG entries. resvg is
an xtask dependency only, with default features off: no text and no
system fonts, so a render depends on nothing but `Cargo.lock`, which is
what lets `cargo xtask brand --check` (in `cargo xtask ci` and CI's
changelog job) compare bytes and fail on a source edited without
regenerating.

**The app uses them.** Each window gets two icons (`strobe_app::brand`,
compiled in with `include_bytes!`): the 48 px render of `icon.svg` as
Windows' big icon, which is what the taskbar and Alt-Tab show at the
T480's 150%, and the 24 px render of `icon-small.svg` as the small one.
winit's `with_window_icon` sets only the small one on Windows, which T480
Task 15 caught (`WM_GETICON` returned no big icon). The big one needs the
Windows-only `with_taskbar_icon`, so `strobe_gfx::with_icons` holds that
`cfg` in a crate the Linux cross-check covers. The app stays free of
`#[cfg(windows)]`. The title bar's mark is
the simplified ribbon, flattened from the same six cubic segments as
`icon-small.svg` and painted in the theme's accent: the guide's monochrome
variant, so it follows the theme like every other UI colour.

Not done, and why: the icon is not yet embedded as an `.exe` resource, so
Explorer still shows the generic executable icon. That needs a resource
compiler at build time (`embed-resource` and `rc.exe` or `llvm-rc`),
which the Linux cross-check cannot run. It belongs with the release
packaging work (Q12). The wordmark and lockups are still to be drawn
(branding.md section 11).

**Would reopen:** a theme the owner prefers as the default, or a final
production mark replacing the concept ribbon (then edit the two SVGs and
run `cargo xtask brand`).

### D191: a pass can opt out of scaling with `FIXED_SIZE` (2026-10-11)

D11 stores pass sizes as fractions of the design resolution, so a buffer
follows the output and the render scale. That is right for an image, and
it is what makes render scale the main lever on old GPUs. It is wrong for
a **simulation**: the look of a reaction-diffusion, fluid or cellular
automaton is set by its grid, in cells. TORNADO (Task T1) showed it.
`07-reaction-diffusion` at 2160p ran its Gray-Scott grid at 1920x1080
instead of 960x540, and the spots came out half the size on screen. Under
adaptive resolution the grid also changed size mid-show.

`"FIXED_SIZE": true` on a pass keeps its `WIDTH` x `HEIGHT` as absolute
pixels at the design resolution, whatever the output size or render
scale. It is a Strobe extension, beside `FULL_PRECISION`, and defaults to
off, so every SSF scene keeps D11's behaviour. The final pass is never
fixed. `resize_keeping` copies a fixed pass into a target of the same
size, so its state survives a resize exactly.

The trade-off is cost: a fixed pass no longer shrinks when the render
scale drops, so on an old GPU it costs the same at every scale. That is
the price of a stable look. A scene author who needs both can declare a
smaller fixed grid. `07` now declares its four simulation passes fixed at
960x540. On llvmpipe its pattern is identical at 720p and 1440p, and
1440p went from 65 ms to 42 ms a frame. Evidence: `scene.rs`
`a_fixed_size_pass_keeps_its_design_pixels`, and `render.rs`
`a_fixed_size_pass_keeps_its_grid_when_the_output_resizes` (both
dialects: a 16x8 grid stays 16x8 after a resize, a normal pass follows to
12x6).

**Would reopen:** finding that Synesthesia treats `PASSES` sizes as
absolute (Q24), in which case the default should flip for SSF scenes.

### D192: the output is opaque, and ES global initializers move into `main` (2026-10-11)

TORNADO's run over 65 real Synesthesia scenes (T2) found two bugs that no
original scene had shown.

**Alpha.** `cosine_palette_generator`, `hello_world_start` and
`hello_world_end` return alpha 0 from the final pass (`hello_world_start`
returns `vec4(1.0, 0.0, 0.0, 0.0)`, meant to be red) and came out white or
transparent. Synesthesia's output and Shadertoy's canvas are opaque, so the
assembler now writes alpha 1 after the final pass's `renderMain()`. With
`PASSINDEX` a constant that is one line in the final program. In shared mode
it is `if (PASSINDEX == last)`. Buffer passes keep their alpha, which scenes
use as a fourth data channel. When Synesthesia's `Alpha Channel` meta control
is built (Q21), it is the one thing allowed to undo this.

**Global initializers on ES.** GLSL ES 3.00 requires a global initializer to
be a constant expression. Desktop GLSL drivers (Mesa, NVIDIA) accept any
expression, and real scenes rely on it: 31 of the 65 open with `vec4 iMouse =
vec4(MouseXY*RENDERSIZE, MouseClick, MouseClick);`, so ANGLE rejected them.
`fixups::hoist_global_initializers` runs for `Dialect::Es300` only, like
D163. It turns each plain global `T name = expression;` into `T name;`,
blanking the initializer with spaces and keeping newlines so no line moves.
The assembler then runs the assignments at the top of `main`, in source
order, after `_xy`/`_uv`/`_uvc` are set, each under a `#line` pointing back
at its line of `main.glsl`. That is equivalent, because a global can only be
read from a function and every function runs from `main`, and the order lets
one global use another. Only value types (`float` through `mat4`, with an
optional precision qualifier) are touched. `const`, `uniform`, arrays, structs,
declarations of several names, function bodies and preprocessor lines are
left alone.

Evidence: `fixups::hoist_tests` (4 tests) and `render.rs`
`a_non_constant_global_initializer_compiles_in_both_dialects` and
`the_output_is_opaque_and_buffer_passes_keep_their_alpha`, on both dialects
and in both `PASSINDEX` modes.

**Would reopen:** a scene that reads a global from a global initializer
*before* `main` would run on desktop (impossible in GLSL), or Synesthesia's
output turning out to honour alpha.

### D193: real Synesthesia scene shapes load (2026-10-11)

TORNADO's run over 65 real scenes (T2) found shapes that the SSF documentation
and Strobe's original scenes never showed. Each now loads:

- **CRLF.** 50 of 64 real `main.glsl` files have Windows line endings. On
  ANGLE a `\` continuation ending in `\r\n` is not honoured, and error line
  numbers came out as about 2n-1 in five scenes. `SceneFolder::load`
  normalises `\r\n` to `\n` for every file it reads. A file without `\r` is
  not copied.
- **A control with no `TYPE`.** 7 controls have only `NAME`, `MIN`, `MAX`
  and `DEFAULT`, and Strobe rejected them as type "". Now two default
  components make an `xy` pad and anything else a `slider`.
- **A pass sized `0`.** It means the output size (scale 1.0), instead of an
  error.
- **`MEDIA` as a list.** Synesthesia's own form is `"MEDIA": [ { "FILTER",
  "WRAP" } ]`: input slots with sampler settings and no file, filled at run
  time from the user's media. D186's guessed `MEDIA` (a path or an object)
  still works. A slot with no path means no picture until `--media` gives
  one.
- **Several names in one global declaration.** `float a = x, b = y;` is
  hoisted per initializer on ES (extends D192).
- **A script naming a control the scene lacks** is now a console warning,
  not an error. A real template script references eight controls its scene
  does not declare, which is the scene's business. A missing callback stays
  an error.

Evidence: `scene.rs` `real_synesthesia_shapes_load`, `load.rs`
`windows_line_endings_are_normalised_at_load`, `fixups::hoist_tests`
`a_declaration_of_several_names_hoists_each_initializer`, and the two updated
conformance tests in `strobe-script`.

**Still failing on the corpus, and why:**
- **The user-media library** (`syn_UserImage`, `_loadUserImage`,
  `_loadUserImageAsMask`, `_exists`, `_correctImageCoords`,
  `_invertYAxisVideo`; 8 scenes). It waits for Synesthesia's own engine files
  (the private corpus, Task 18), so it is copied rather than guessed.
- **On ANGLE only:** implicit int to float (10 scenes, e.g. `clamp(x, 0, 1)`
  or `mix(2, 20, a)`), and `#if` on a name that is not a macro (2). Both
  need types (Q8).

### D194: literal pass sizes are absolute pixels, as in Synesthesia (2026-10-11)

Reverses D11 for one case. Q24 asked whether SSF pass sizes are absolute. The
65 real scenes say yes:
- a 10x10 data pass beside a 640x360 one;
- a halving pyramid down to 14x8 and 7x4;
- a 3000x1500 pass in a 1920x1080 scene.

Under D11 that 10x10 pass became 7x7 at 720p, so a shader reading fixed texels
from it read the wrong ones. Now:

| A pass's `WIDTH`/`HEIGHT` | Size |
|---|---|
| Both literal numbers | Absolute pixels |
| `$WIDTH` / `$HEIGHT` expressions | Follow the output and the render scale (D11) |
| Absent or `0` | Full output size |
| Mixed (one literal, one expression) | Follow the output, conservatively |

`FIXED_SIZE` (D191) overrides either way: `true` fixes, `false` follows.
Shadertoy imports write no sizes, so they are unaffected. `08-bloom-orbs`
now writes `$WIDTH/4` so its blur keeps shrinking with render scale, and
`07` keeps `FIXED_SIZE`.

The cost on old GPUs: a literal pass no longer shrinks when adaptive
resolution lowers the scale. That is the faithful behaviour, and a scene
that wants the lever uses an expression. Evidence: `scene.rs`
`literal_pass_sizes_are_absolute_and_expressions_follow_the_output`.

**Would reopen:** a Synesthesia scene that clearly expects a literal size to
scale with the output.

### D196: more real-scene shapes, from Synesthesia 1.25.4's own scenes (2026-10-11)

The owner's private corpus, `lnorton89/strobe-syn-corpus` (90 scenes from
Synesthesia 1.25.4.189, never copied into this repo), surfaced more shapes.
These now load:

- **`null`** where a list or a type belongs (`"HARD_TRANSITIONS": null`)
  reads as empty or default (`json::null_as_default`).
- **Names equal only once normalised** (`in_or_out` and `inOrOut`) are
  legal GLSL, and a real scene has them, so they are no longer a load error.
  `ControlBank::find` prefers the exact name and the normalised key reaches
  the first. An exact duplicate is still an error.
- **An image is decoded by its content.** A real scene ships a PNG named
  `skytex.jpg` (`assets::decode_file`).
- **ISF's image macros** `IMG_NORM_PIXEL`, `IMG_PIXEL` and `IMG_SIZE` are
  defined in the preamble.

Measured with `strobe-bench` on llvmpipe, which runs no scripts: desktop
failures went from 55 of 90 to 50. Most of the rest are uniforms that a
`script.js` creates, a benchmark artefact that the corpus harness (D195)
removes. The rest:
- the `traveler` control modifier (9 scenes);
- `#include` of the engine's `lib/` (2);
- standard-library names the docs have to settle (`syn_BassExpoCurve`,
  `syn_InputLevel`, `_loadImage`, `_applyInvertMedia`, `_flipMediaCoords`,
  and the user-media family);
- on ES, the implicit-conversion family (Q8).

Evidence: `scene.rs` `real_synesthesia_shapes_load`, `runtime.rs`
`names_equal_once_normalised_load_and_the_exact_name_wins`, `assets.rs`
`an_image_is_decoded_by_its_content_not_its_extension`.

### D197: `traveler` controls, the image family, and Synesthesia's older names (2026-10-12)

From Synesthesia 1.25.4's own scenes (the private corpus) and its bundled docs
and editor, which the T480 read (Task 19).

- **`traveler`** (`slider traveler`, `knob traveler`, `xy traveler`) is a
  real, current control type: the editor lists it, and 9 of 90 shipped scenes
  use it, with `PARAMS`. The docs omit it. `Mode::Traveler` is a speed control
  (the output integrates the control, per second, like `speed`, which the docs
  do describe) whose rate eases toward the control with `PARAMS`, so a change
  of speed accelerates instead of jumping. That fits the scenes, which give
  traveler controls a `PARAMS` ("fly in/out", "spin", "automatically pan").
  It is an inference, recorded in Q23.
- **The image family**, which the editor lists and the docs do not describe,
  mirrors the documented media functions for any sampler: `_textureImage`,
  `_loadImage(img[, offset])`, `_loadImageAsMask`, `_textureImageAsMask`,
  `_imageToMask`. `_correctImageCoords(size, uv)` is the cover fit that
  `_correctMediaCoords` now uses, with an `ivec2` overload because scenes pass
  `textureSize()` straight in. `syn_pass_edgeDetectSobel(pass)` wraps
  `_edgeDetectSobel`.
- **Older names** in real scenes: `syn_UserImage`, `_loadUserImage` and
  `_loadUserImageAsMask` are `#define`d to `syn_Media`, `_loadMedia` and
  `_loadMediaAsMask` (the user image was the media input before it was called
  that). `_invertYAxisVideo(uv)` flips y. `_flipMediaCoords`,
  `_applyMediaScale`, `_applyInvertMedia` and `_applyMediaContrast` are the
  identity, because Strobe has no per-media flip, scale, invert or contrast
  option yet. `_exists(tex)` is true when a sampler holds more than Strobe's
  1x1 placeholder.
- **Undocumented audio names**, stand-ins until their meaning is known:
  `syn_InputLevel` is `syn_Level`, and `syn_BassExpoCurve` is
  `syn_BassLevel * syn_BassLevel`.

Evidence: `runtime.rs` `a_traveler_accelerates_toward_its_speed_and_integrates_it`,
and the library compiling in both dialects (`render.rs`
`the_ssf_library_compiles_in_both_dialects`). With these and D196, the
corpus's desktop failures on llvmpipe fell from 55 to 45 of 90. All 45 are
either uniforms a `script.js` creates (the bench runs no scripts; D195's
harness does) or `#include` (D198).

### D198: `#include` resolves against the scene and the GLSL library folders (2026-10-12)

Synesthesia scenes `#include` the libraries the engine ships (`lygia` 1.1.4,
`hg_sdf`) and the user's own `.glsl` files, per its docs (`libraries.html`).
`strobe_scene::include` expands them when a scene loads. It looks relative to
the including file (lygia includes with `../`), then in the scene folder,
then in `STROBE_GLSL_PATH`, then in Synesthesia's `lib` folder and user
`glsl` data folder when they exist. Strobe ships neither library, because
lygia's licence forbids it, so a machine with Synesthesia installed simply
works and any other machine points `STROBE_GLSL_PATH` at a copy.

Each file is included once, which is what lygia's include guards do anyway,
and that also stops cycles. Nesting is capped at 16 levels. A missing file
stays as the `#include` line, so the driver's error names it. Included text
compiles under source string 3 (`source::INCLUDED`) between `#line`
directives, so `main.glsl` keeps its line numbers.

Evidence: `include.rs` tests (relative then search path, once each, line
numbers kept; missing left alone; only real include lines). On the corpus,
`SkaiWalkerOG` now finds `hg_sdf.glsl`. It then fails inside it on Mesa,
because hg_sdf's `float sgn(float x) { return (x<0)?-1:1; }` returns an int.
NVIDIA accepts that and Mesa does not; the int-to-float family is Q8.

