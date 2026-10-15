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

*Addendum (2026-10-13):* the fixups are built. A typed rewrite writes
implicit int-to-float out on every dialect, ANGLE included, and the
owner's 90-scene corpus went from 17 to 87 of 90 on GLES (D199).

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

*Addendum (2026-10-13):* the downbeat estimate landed (D233). `BeatInfo`
gained `downbeat`, and bars start at `4n + downbeat`; it is 0 (the grid's
origin, as above) whenever the estimate is unsure.

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

**Known loss (narrowed by D287):** a resize with a non-default speed makes
the clock jump to `TIME`. The adaptive-resolution controller resizes rarely,
and at default controls the clock equals `TIME`, so nothing visible happens.
Resizes stopped being rare (D280 probes, undoes and probes again), and D287
now keeps a pass whose size a resize does not change. The clock is kept
when only the Image pass reads it. When a buffer reads it, `resize` wipes
that buffer and so still restarts the clock with it, which is what the
`iFrame == 0` initialisation above needs; `resize_keeping` keeps both.

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

*Addendum (2026-10-13):* both happened. Explicit blackout is D201, and
Synesthesia's addresses are accepted as aliases where they mean the same
(D234).

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

*The reset half is superseded by D207: meta values now carry across scenes.*

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

### D195: compatibility is measured by `strobe-corpus`, with the app's own load (2026-10-13)

Stage 0 of `docs/GFX_API_RESEARCH.md`. `cargo xtask corpus -- --library <dir>`
loads every scene in a folder through `Session::load`, so the script runs and
its `setUniform` uniforms are declared before the shader compiles (D62, D161).
It renders a few frames headless in both dialects and writes `report.json` and
`report.md` under `target/`.

Why this shape:
- **The app's load, not a copy.** `strobe-bench` skips scripts, so it reported
  most script-created uniforms as "undeclared". That was a bench artefact, not
  a gap. A report that runs a different pipeline from the one that plays is
  worse than none.
- **A binary of strobe-app.** The xtask shells out to it, as it does for the
  bench. `xtask` must build without the MSVC C toolchain that QuickJS needs.
- **A child process per scene and dialect.** A driver hang is killed at
  `--timeout`, and a driver crash becomes a `gl_error` record. A GL context
  cannot be abandoned from a thread.
- **Third-party corpora stay private.** The report names scenes and quotes
  one error line each. It never copies a scene.

First run on the owner's private corpus (90 real scenes, llvmpipe):

| Dialect | OK | Script error | Compile error |
|---|---:|---:|---:|
| Desktop GL 3.3 | 85 | 2 | 3 |
| GLES 3.0 (ANGLE dialect) | 17 | 2 | 71 |

Desktop failures:
- `syn_PrimeBang` is called from script (2);
- `_hueSaturationContrastLEGACY` is missing from the library (1);
- an hg_sdf-style `sgn` returns an `int` from a `float` function (1). Mesa
  rejects it; NVIDIA accepts it;
- a `sampler2D` is an operand of `?:` (1).

GLES failures:
- 65 are implicit int→float conversion (Q8);
- the rest are a few library overloads that take an `int`, an initialised
  `uniform`, and a write to a `const`.

The ES column is the case for the type-aware int→float rewrite, and the first
thing the ANGLE path needs.

**Would reopen:** a failure the harness reports that the app does not have, or
the reverse.

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

### D199: implicit int→float is written out, by a typed pass, on every dialect (2026-10-13)

Q8 said the remaining GLSL ES failures needed a parser. On the owner's
90-scene corpus they were 65 of 73 ES failures (D195). `strobe_scene::promote`
is that pass. It does not use a GLSL parser crate, because none handles
SSF's macros. It has its own lexer and a small expression parser that types
every subexpression against everything the program declares:
- the generated preamble;
- the SSF library;
- the user's globals, structs, functions and overloads;
- object-like macros, typed from their bodies;
- built-in variables.

Where desktop GLSL would convert an `int` to a `float`, it writes the
conversion out:
- **Where:** operands of `+ - * /` and comparisons, assignments,
  initializers, `return`, ternary branches, float built-in arguments, and
  user-function arguments when exactly one overload is reachable.
- **Literals:** `2` → `2.0`. A literal-only expression built with `+ - *`
  is converted literal by literal.
- **Everything else is wrapped:** `float(i)`, `vec2(p)`. An integer division
  is wrapped, never rewritten, so `1/2` stays 0 as it is on desktop.
- **Never** against a type it does not know, such as a function-like macro's
  result or an unresolvable name. A body it cannot parse keeps its text.
- **Lines never move,** so driver errors still point at `main.glsl`.

It runs on **every** dialect. Mesa and Apple's compilers reject some of the
same forms NVIDIA accepts, such as an `int` returned from a `float` function
(`hg_sdf`'s `sgn`). The explicit form is valid everywhere. Evidence that it
keeps desktop's meaning:
- every desktop scene in the corpus drew a bit-identical frame with and
  without it, except 8 scenes whose frames differ run to run anyway;
- `render.rs` `implicit_int_to_float_and_non_macro_if_compile_in_both_dialects`
  checks the values on both dialects.

Two smaller rules came out of the same corpus work (TORNADO T4/T5):
- **`#if NAME` on a name that is not a macro** reads `NAME` as `0` on every
  dialect, as NVIDIA's compiler and the C preprocessor do
  (`strobe_scene::directives`).
- **`uniform T x = v;`** becomes the global `T x = v;` on ES. Nothing sets
  such a uniform, so it always holds `v`.

The rewrites follow `#if` groups (`directives::Branches`), so a
`#ifdef KODELIFE` block that is never compiled is neither parsed nor
hoisted. Before this, D192's hoist assigned to a control's uniform from
inside such a block.

Corpus after this and the library additions below, on llvmpipe:

| Dialect | Before (D195) | After |
|---|---:|---:|
| Desktop GL 3.3 | 85 / 90 | 87 / 90 |
| GLES 3.0 | 17 / 90 | 87 / 90 |

Library additions, from TORNADO T5's call sites:
- `syn_pass_edgeDetectSobel` returns `float`, as Synesthesia documents;
- the one-argument `_correctImageCoords(size)`;
- `_ratioFix` (Q27);
- `_invertImage`, an identity like the other media options;
- `syn_PrimeBang` as a script global (Q25).

Left:
- `_hueSaturationContrastLEGACY` (Q26);
- a `sampler2D` as an operand of `?:`, which Mesa rejects on both dialects;
- a function-like macro whose body mixes int and float, which needs macro
  expansion.

**Would reopen:** a scene that compiled before this and does not after, or
whose desktop frame changes.

### D200: the command palette lists everything, runs through the shortcut's code, and owns the keyboard (2026-10-13)

Ctrl+K opens a palette over the command table (UI_DESIGN section 25,
backlog U-01). The choices:
- **One path.** Choosing an entry calls `ui::keys::run`, the function its
  shortcut calls. The palette cannot mean something different from the
  key, and a command added to the table appears in it with no further work.
- **Panes are entries too** ("Show Mappings"), so everything on the rail is
  reachable by typing.
- **Unavailable entries stay listed, with the reason** ("needs a playlist",
  "only one scene"), and Enter on one does nothing. A palette that hides
  commands teaches the operator they do not exist.
- **It owns the keyboard while open,** passed to the shortcut dispatcher as
  a modal, so `b` typed into it never blacks out the show. Escape, a click
  outside it, or Ctrl+K again closes it. The replace-scene question
  outranks it and closes it.
- **Ranking** is a subsequence match: a title start beats a word start,
  which beats anywhere, and fewer gaps win. Ties keep table order, so the
  list does not reshuffle while typing. A match on the category ranks
  below every title match.
- `OpenPalette` is a control-window command. It is not mappable (MIDI and
  OSC cannot open it) and does nothing in the output window.

Evidence: `command_palette` unit tests (ranking, reasons, bindings), and
`ui::shell_tests` driving the whole shell with real key events (run by
name, typing fires nothing, Escape, toggle, unavailable, panes, empty).
The screenshot is `docs/images/perform/palette-1440x900.png`.

**Would reopen:** an operator who expects scenes and presets in it too;
they would be more entry kinds, not a different design.

### D201: explicit blackout commands, a missing picture plays black, and every line ending is one (2026-10-13)

Three small rules, two of them from TORNADO T6.

- **`blackout_on` and `blackout_off`** sit beside `toggle_blackout`
  (backlog P-06):
  - they are in the command table, the palette, OSC
    (`/strobe/cmd/blackout_on`), MIDI mappings and the remote protocol;
  - repeating either changes nothing, so a controller or phone that has
    lost track of the state cannot bring the output back by accident;
  - they have no default key, so a command's chord is now optional; the
    operator can still bind one in `hotkeys`.
- **A picture that will not load is black,** and the scene plays. The
  problem is reported the way a missing cube map is (D185). A real scene
  names `x.png` where its folder holds `x-01.png`, and Synesthesia plays it.
- **Every line-ending form is one `\n`** when a scene or an include is read:
  `\r\n`, a run of `\r` before `\n`, and a lone `\r`. 38 of the owner's 65
  scenes end lines `\r\r\n`, which broke `\` continuations and made error
  line numbers disagree with editors.

**Would reopen:** a scene that depends on a missing picture failing the
load, or that is laid out so that `\r\r\n` counts as two lines.

### D202: the app generates controls in a worker process, and the operator reviews a copy (2026-10-13)

The Inspector's "Generate controls" button, and the palette command of the
same name, start the app's own executable again as
`strobe --generate-controls <scene> ...`. The worker:
- opens an offscreen context (D140: an EGL device, or one hidden window on
  Windows);
- runs `strobe_autocontrol::generate`;
- writes the result as `<name>-auto.synScene` with `write_scene` (a copy,
  licence and attribution included);
- writes a JSON summary and exits.

The app polls it once a frame, never blocking, and shows the result in the
Inspector:
- the controls made, with group, default and range, and a bar for how much
  each moves the picture;
- the fidelity at their defaults;
- the audio pairings the engine suggests;
- two buttons: **Open** loads the copy, **Discard** deletes it.

Why a process, not the app's own context:
- **The show must not wait.** Scoring renders the scene dozens of times:
  seconds on an HD 4000 with the output on a projector.
- **A thread cannot do it either.** The app's context is current on the
  main thread, and winit allows one event loop per process, so a second
  context cannot be made there on Windows.
- **A driver crash during scoring** ends the worker, not the show.
- **A worker stuck for ten minutes** is stopped.

Safety rules:
- The source is never written: the engine refuses to, and the worker
  writes only the copy.
- Discard deletes only a `*-auto.synScene` folder directly inside the
  library.
- A scene with authored controls gets "Add generated controls" (merge):
  its own controls are kept, and new ones are only added beside them.
- The command is not mappable. A MIDI pad that starts minutes of
  measuring mid-show is a mistake.

Evidence:
- `tests/generate.rs` runs the real executable as the app does:
  - the source is byte for byte unchanged;
  - controls are made;
  - fidelity is under 1%;
  - the licence is copied;
  - Discard removes only the copy;
  - authored controls are refused, then merged.
- An Xvfb run drove it with Ctrl+K, "generate", Enter.

Left (U-03): per-control keep, drop and re-range before the copy is
written, and the same button on the Import pane.

**Would reopen:** a machine where starting a second process is not allowed,
or a need to apply generated controls to the playing scene in place.

### D203: a pass that samples its own target is double-buffered (2026-10-13)

GL leaves reading a texture while rendering into it undefined, and drivers
answer differently:
- **NVIDIA** returns the previous contents.
- **llvmpipe** also returns the previous contents.
- **D3D11 under ANGLE** returns zeros: it cannot bind one texture as both
  shader input and render target, so it unbinds the input.

Real scenes rely on NVIDIA's answer. `magnetismic` keeps an accumulator in a
single-buffered 10x10 pass, and on ANGLE it drifted to a mean of 19 levels
from native by frame 100 (TORNADO T7). With `DOUBLE_BUFFER` set, native and
ANGLE matched, and double-buffered native was bit-identical to
single-buffered native. So the faithful answer is "the previous frame", on
every driver.

The player now decides this when it allocates targets: a buffer pass whose
**linked program** has its own target as an active sampler gets two targets
and swaps them, exactly as `DOUBLE_BUFFER` does, and the log says so once. It
reads the linked program's active samplers, so a pass that names its target
only on a branch the compiler removed (`PASSINDEX` folds in each pass's own
program) keeps one target.
- **Cost:** one more texture the size of that pass.
- **Effect on other passes:** they still read the frame just written, as
  before.
- **Evidence:** `render.rs`
  `a_pass_that_reads_itself_sees_its_previous_frame_in_both_dialects`.
  It checks the values, and checks the buffering through
  `ScenePlayer::is_double_buffered`, because llvmpipe's own answer happens
  to be NVIDIA's and the values alone cannot catch a regression there.

This resolves Q5 (now R9).

**Would reopen:** a scene that wants the D3D11 behaviour (reading zeros from
itself), which no driver promises.

### D204: the Editor edits files; only "Save and reload" reaches the show (2026-10-13)

The EDIT workspace (backlog U-02, UI_DESIGN section 14) opens one scene's
sources as tabs: `main.glsl`, `passes/*.glsl`, `scene.json` and `script.js`.
GLSL is highlighted, and lines do not wrap, so the line numbers in an error
match the lines on screen.

- **Editing is not playing.** Typing compiles nothing and changes nothing on
  the output. **Save** (Ctrl+S) writes the files, and the show keeps the
  scene it has. **Save and reload** (Ctrl+Enter) writes them and loads the
  scene again through the Reload button's path, so a scene that fails to
  compile leaves the last good one playing (D66's rule). A status line says
  which of these is true: unsaved, error, or ready. The reason is the old
  laptop mid-show: a compile per keystroke is a stall per keystroke.
- **Saves are atomic.** The file is written beside itself as
  `<file>.strobe-tmp` and renamed into place.
- **Backups go to the data directory, never the scene folder.** Before the
  first save of a file in a session, its on-disk text is copied to
  `backups/<scene>/<file>.<timestamp>`, keeping the newest 10. **Restore
  backup** puts the newest one in the editor, unsaved, so the operator
  decides. The scene folder is the operator's, and a library synced or
  shipped elsewhere should not grow our files.
- **The error is one click from its line.** The failure banner on PLAY gets
  "Open in Editor", and the Editor's own banner "Go to line N". Both read
  `<file>.glsl:<line>` from the compiler message (`editor::error_location`).
- **The playing scene opens itself** when the Editor pane is open and empty,
  once per scene, so opening EDIT during a show lands on what is on screen.
  A scene that is open but not on screen is labelled so.
- **An unsaved document is never replaced.** Opening another scene while
  one has unsaved changes is refused, with a notice, rather than dropping
  the work or prompting mid-show.

**Evidence:** `editor.rs` unit tests (load, dirty state, atomic save, one
backup per session, the cap of 10, revert, error locations, the
highlighter), the shell scenario "editor open with an unsaved edit" at six
window sizes, and `docs/images/perform/editor-1440x900.png` (Xvfb, llvmpipe).
Not checked: typing through a real window manager, and the T480.

**Would reopen:** wanting live preview while typing. That would need a
compile off the frame loop (a second context or a worker), not a compile on
the UI thread.

### D205: tags are Synesthesia's `TAGS`, read leniently, normalised, and written in place (2026-10-13)

D157 had the generator draw on name words because no scene carried tags.
Real Synesthesia scenes do: 83 of the 91 `scene.json` files in the private
corpus have `"TAGS": [...]`, 79 distinct tags in all (`media`, `dark`,
`feedback`, `raymarching`, `2d`, `3d`...). So Strobe uses that key rather
than inventing one, and a library tagged in Synesthesia arrives tagged.

- **Normalised.** Authors write `3D` and `3d`, `audio reactive` and
  `audio-reactive`. A tag is compared and stored lowercase, with every run
  of spaces, `-` and `_` turned into one `-`, and cut at 48 characters
  (`strobe_scene::tags::normalize`).
- **Read leniently.** `TAGS` is metadata, so it never fails a scene. An
  array keeps its strings, a string is a comma list, and `null`, a number
  or an object is no tags. The library scan reads only `TAGS` from each
  `scene.json`, and a missing or broken file is a scene with no tags.
- **Written in place.** The Inspector's tag field rewrites only the
  `TAGS` value in the text, or appends the key with the file's own indent,
  and keeps every other byte: key order, formatting, a byte-order mark. A
  `serde_json` round trip would sort the keys of the operator's file. A
  file that is not a JSON object is refused, not repaired. The write is
  atomic. It is refused while the Editor holds unsaved edits to the same
  scene, so neither write loses the other's work. Checked against the
  corpus: every file round-trips with all other keys unchanged.
- **Search** matches a word in the name or in any tag.
- **Generate** gives each scene its tags, then its name words, so a
  library nobody has tagged still generates as before. Pool words are
  normalised like tags. The preview says why each scene is in the pool
  (`strobe_playlist::Why`: tag, name, folder, every scene), and an empty
  pool lists the library's most-used tags instead of a bare "nothing
  matched".
- **The base library is tagged** with the corpus vocabulary, plus `heavy`
  on the two raymarched scenes and `light` on the cheapest two, so a show
  for an old laptop can exclude by cost.

**Not done:** tags in the Import pane, editing tags of a scene that is not
playing, and exclude lists in the Generate form (the generator supports
them; the form has one pool field).

**Would reopen:** Synesthesia normalising differently (for example,
treating `audio reactive` and `audio-reactive` as two tags), which would
matter only if Strobe wrote scenes back for Synesthesia to read.

### D206: a covered control window redraws at 2 Hz, and a perf line is logged every minute (2026-10-13)

TORNADO T10 ran a fullscreen show at 48 to 56 fps with the GPU busy for
under 3 ms a frame. The output window is the only vsynced one (the control
window has vsync off, so two vsynced swaps cannot halve the rate), but the
control window was fully covered by the fullscreen output on the same
monitor. On Windows, an unsynced swap of a covered window can still wait
for the compositor, and a 30 Hz control window doing that would cost about
every other output frame its vertical blank. That fits the numbers. It is
not proved, and the log could not say either way.

- **A perf line every minute** (`stats::PerfWindow`): frame rate, frames
  later than 1.5 frame budgets, the worst gap, mean GPU time, and the
  control window's redraw count and mean and worst cost, swap included.
  Recording is a few additions a frame; the line is formatted once a
  minute. A soak's log now says where frames went.
- **A covered control window redraws at 2 Hz.** Covered means: the output
  is shown, fullscreen, on the control window's monitor, and the control
  window has not come above it since. Focus cannot decide this. The app
  hands focus back to the control window when the output is shown, and F11
  in the control window then puts the output over it with the focus left
  where it was. winit reports no occlusion on Windows. So the control window
  counts as on top again when it gains focus or the pointer reaches it, and
  then redraws at 30 Hz at once.
- **Nothing a show needs waits on the control window.** MIDI, OSC and the
  remote are drained in the scene frame, not the control redraw, and so are
  cues and the playlist clock.

**Would reopen:** the perf line showing the late frames without the control
window's redraw cost explaining them. Then the cause is elsewhere: the audio
thread, the timer ring's backpressure, or the driver's flip queue.

**Addendum (T11, the same day).** The perf line settled it the other way:
fullscreen, the show ran at 59.9 fps with almost no late frames, and T10's
48-56 was the status bar, which a covered control window rarely repaints
(Windows sent it 0 to 2 paints a minute). The 2 Hz throttle stays, since it
costs nothing and matters where a covered window is still painted, but it
fixed nothing on Windows. The real cost showed up windowed: a *visible*
control window's redraw took a full refresh for minutes and made about a
third of the output's frames late. The perf line now times that window's
swap on its own, and after T12 found the swap returning in 0.03 ms, splits
the whole redraw into make-current, layout, paint and swap.

### D207: meta settings carry across scenes, Limit Colors is built, Alpha Channel waits for an output that carries alpha (2026-10-13)

Backlog C-01 asked for the last two meta controls and for a decision on
whether meta values persist across scene changes. D182 had them reset with
each session.

**They carry.** D182 meant an operator who faded master to 0 and changed
scene got the full picture back at once, and a hue or mirror set "for the
room" vanished at every playlist cue. Synesthesia calls these controls
output-wide, and its presets never save master, "the operator's fader".
So when a scene goes live, by a load or a cue, each meta control the
incoming scene has **not** set itself takes the outgoing scene's state,
target and output (so nothing re-smooths): `ControlBank::carry_unchanged_from`.
"Set itself" means moved from its default by the scene's default preset, the
cue's preset, the cue's `meta` overrides or the script's first frame, and
those still win. A cue that restarts the same scene keeps its settings the
same way. Cost: one bank copy per cue change, never per frame.
- **The cost of the rule:** a preset or cue that sets a meta control
  explicitly *to its default* (hue 0, say) looks unset and does not reset
  the operator's value. Resetting is the Inspector's Reset or the meta
  reset command. Distinguishing "set to the default" from "unset" would
  need a flag per control, and no real playlist needs that yet.
- **A context rebuild (D66) still resets meta.** The old session is gone
  by then.

**Limit Colors** is three meta controls: `limit_colors` (a toggle),
`limit_low` and `limit_high` (colours, black and white by default). When
on, the post stage maps each pixel's Rec. 709 luma onto `mix(low, high,
luma)`, after hue and saturation and before invert and master. Off is
neutral, so it costs no pass. Evidence: `post.rs`
`limit_colors_maps_luma_onto_the_two_colours` (both dialects) and the
`meta.rs` tests. The order is Strobe's guess (Q21).

**Alpha Channel is not built.** D192 makes the final pass opaque, because
an output window shows whatever alpha it is given and real scenes return
alpha 0. A control that let alpha through would change nothing anyone can
see until an output carries alpha (NDI or Spout, Phase 7), so it would be
a control that lies. It is built with that output, and it is the one thing
D192 lets undo the forced alpha.

This supersedes D182's reset; D182's other half (a transition reads the
live scene's meta) stands.

### D208: shortcuts are edited in Settings, a clash is named and moved only on purpose (2026-10-13)

D178 put shortcut overrides in `show-control.json`'s `hotkeys` table, read
once at start. Backlog U-05 asked for an editor that captures a key, shows
conflicts and defaults, resets, and saves.

- **Capture.** Settings > Keyboard lists every command with its shortcut
  (custom ones in another colour). Change waits for the next key; Esc
  cancels. While it waits, every shortcut is off (the same rule as the
  palette and a modal), so the key being captured never also runs its
  command.
- **A clash is a question.** If another command holds the key, nothing
  changes; the row says "Ctrl+B is Toggle blackout" and offers *Move it
  here* (the other command is left with no shortcut, visibly) or *Keep it
  there*. A key is never moved silently.
- **"none" is a value.** A command can be cleared, and the table writes
  `"none"`. `Bindings::from_config` applies the `none` lines first, so a
  moved key reads the same whatever order the file lists the lines in.
  Before this, a table could not express a swap.
- **Defaults stay out of the file.** Choosing a command's default chord, or
  Reset, removes its line. Reset is refused, with the holder named, when
  another command has taken that key meanwhile.
- **Live.** The bindings are rebuilt from the table at once, and the remote
  config's saver writes the file. No restart.
- Keys the table cannot hold (anything but letters, digits, F1 to F12,
  Space, the arrows and Esc) are ignored while capturing, and Esc cannot
  be captured, since it cancels. A file can still bind Esc.

**Evidence:** `commands.rs` tests (assign, steal, reset refused and then
allowed, the table back to empty, labels round-trip) and `shell_tests`
(Ctrl+B while capturing becomes tap tempo's key and fires nothing; Esc
cancels and the next B is blackout again; the clash row at six sizes).

### D209: OSC feedback sends what changed, on the listener's own addresses, from a thread of its own (2026-10-13)

Backlog P-05: motorised faders and a TouchOSC layout should follow the show.

- **The same addresses.** Feedback goes to `/strobe/control/<name>` (with
  `/x`, `/y`, `/r`, `/g`, `/b` for pads and colours), `/strobe/meta/<name>`
  (floats 0..1 across the range), `/strobe/state/blackout` and
  `/strobe/state/cue` (ints). One TouchOSC layout both drives and follows.
- **Only what changed, at most 30 times a second.** A value is sent when it
  moves more than 0.001 from what was last sent, at most 64 messages per
  send (the rest go next time). It sends a control's target, not its
  smoothed output, so a fader is not sent a tail of tiny steps after each
  move. Bangs are not sent. A new scene, and a new feedback target, send
  everything once.
- **The frame never waits.** Addresses are built when the scene's controls
  change. The frame `try_send`s a shared address and a number to a
  thread with a 512-slot queue, which encodes and sends. A full queue drops
  the value and counts it; the MAP pane shows the count.
- **Off by default**, with host and port in the MAP pane's OSC block
  (default `127.0.0.1:9001`, TouchOSC's usual pairing with a listener on
  9000), saved in `show-control.json`.
- **No loop.** Feedback to this machine on the listener's own port is
  refused with a reason. A controller that echoes what it is sent settles:
  the echo equals the target, so nothing moves and nothing is resent.

MIDI feedback is not built. It needs a device to test against (P-04).

**Evidence:** `strobe-io` `feedback` tests (the first send is everything,
then only changes, the rate, the bound, a new scene, the loop refusal, and
delivery over loopback UDP). In the app under Xvfb, with OSC on port 9100
and feedback to 9101, a Python listener received the state at start, then
`/strobe/control/warp` 0.5 → 0.25 after one OSC message set it.

### D210: a macro's reach and colour are edited in MAP, and a control is pinned from its name (2026-10-13)

Backlog U-04. D175 gave a Performance Macro a `range` (the part of its
control's range a mapped fader reaches) and a `color` (an identity), and the
engine already applied the range. Neither had an editor, and a macro could
only be made from the MAP pane's add form.

- **Reach** is two percentages in the macro's options in MAP, with *Full*
  to clear it. `MacroSet::set_range` puts the ends in order, clamps them to
  0..100%, keeps them at least 1% apart (a point would make every fader a
  constant), and stores the full range as no range, so the file holds only
  real changes. The card's tooltip says the reach.
- **Colour** is one of eight swatches, or none. The index is stored, not an
  RGB value, and drawn from the theme's series ramp, so a colour follows a
  theme change and stays distinct from its neighbours. The card shows it as
  a band along its top.
- **Pin from the control.** In the Inspector, right-clicking a control's
  name (or a bang's button), scene or meta, offers *Pin as a macro*, or
  *Unpin the macro* when it has one. Pinning an existing macro's target
  re-pins it instead of making a second card (`MacroSet::pin`).

**Not done:** a shortcut per macro and a smoothing per macro. A control's
own smoothing already applies to what a macro writes, and a shortcut needs
somewhere to say what a key press means for a continuous value (jump to
the top? toggle?), which no request has defined yet.

**Evidence:** `strobe-io` `macros` tests (order, clamp, the minimum reach,
the full range as none, colour bounds, lookup by target); the shell
scenarios with a coloured, ranged macro at six sizes; and a screenshot of
the strip with four macros, three of them coloured, under Xvfb.

### D211: favourite scenes are a star on the browser row and a filter (2026-10-13)

Backlog U-06 asks for favourites. A star sits at the right of every scene
row in the browser, shown when the scene is a favourite, or when the row is
hovered or selected. Clicking it stars or unstars the scene without
selecting the row. A *Favourites* chip beside All, Ready and Needs work
shows only the starred scenes.

- **Saved with the layout**, in `ui.toml` under `[browser] favourites`, by
  library name and sorted. A hand-edited list is sorted and de-duplicated
  on load. A starred scene that is renamed or removed simply stops
  matching; nothing is rewritten behind the operator's back.
- **The star is the warning colour**, which no other element of a row
  uses, so a favourite reads at a glance in any theme. It is an icon, so
  it is not colour alone.
- The filtered list is still cached; a star or unstar refilters it.

**Evidence:** `library` and `ui::state` tests (the filter, toggling,
refiltering, the save being noticed and written).

### D212: the browser shows a scene's own picture, decoded off the UI thread (2026-10-13)

Backlog U-06 asks for thumbnails. Synesthesia scenes already carry one:
`IMAGE_PATH` in `scene.json` (90 of the 91 corpus scenes set it). So the
browser shows that picture. It does not render scenes to make one, which
would need a GL context and seconds per scene, and which can come later for
scenes without a picture.

- **Read at the scan.** The library reads `IMAGE_PATH` with the tags, from
  the same parse. A path that leaves the scene folder is ignored, by the
  same rule the loader uses (`is_folder_relative`).
- **Decoded on a worker.** `ui::thumbs::Thumbs` queues a picture the first
  time its row is drawn. A thread decodes it and shrinks it to fit 160x90,
  and the UI uploads at most 8 a frame. The browser is virtualised, so only
  rows on screen are asked for. A picture that will not decode is
  remembered as missing, and tried once a run.
- **Bounded:** at most 256 textures. Past that, the least recently drawn
  goes and is asked for again if it scrolls back.
- **One layout.** When any scene in the library has a picture, every row
  gets a 16:9 slot. A scene without one shows its colour swatch in the
  slot. A library with no pictures keeps the compact swatch rows.
- **Our scenes have pictures now:** a 320x180 JPEG of each, rendered by
  `strobe-bench` (under 30 KB each).

**Evidence:** `ui::thumbs` tests (a picture arrives shrunk with its aspect
kept; a broken file is remembered; the bound evicts the oldest); the
library scan test (`IMAGE_PATH` read, a path out of the folder ignored);
[images/perform/browser-thumbnails-1440x900.png](images/perform/browser-thumbnails-1440x900.png)
under Xvfb.

### D213: the LIST pane edits the selected cue, by authored position, and never loses edits to the file (2026-10-13)

Backlog U-07. The LIST pane could play, generate and save a playlist, but
not change a cue. Selecting a cue now opens a card above the timeline:

- **Length, transition and preset**, each shown with where its value
  comes from ("set on this cue", "from the section", "from the playlist",
  "the default"). Setting a field writes it on the cue; *Inherit*, or an
  empty length, removes it so the level above applies again. That is the
  inheritance inspector UI_DESIGN section 13 asks for, kept to one cue.
- **Up, Down, Duplicate, Delete**, and **Add after** for the scene
  selected in the browser.
- **Edits address the cue as authored** (section, place in section;
  `show::edit::At`), not its place in the timeline, which shuffle and
  disabled cues make different. `show::edit::apply` is pure: it returns a
  new playlist and the edited cue's new position, or a one-line reason,
  and never half changes anything. The app hands the result to
  `Show::replace_playlist`, the reload path: only scenes new to the show
  compile, and the show keeps its place. The selection follows the cue.
- **Unsaved edits are never lost to the file.** The watcher used to reload
  a changed file over whatever was in memory. Now, while there are unsaved
  edits, a change on disk is held. The pane says the file changed, Save
  writes the edits over it, and *Use the file* takes the file's version.

**Not done:** multi-select edits, dragging from the library, pasted
Shadertoy URLs, enabling or disabling a cue from the pane (a disabled cue
is not in the timeline, so it could not be re-enabled there), and keeping
the file's comments on Save (D157's rewrite stands).

**Evidence:** `show::edit` tests (moves within a section, duplicate,
insert, delete with the neighbour selected, settings set and inherited
again, a stale position refused, the original untouched); the shell
scenario with a cue selected and a changed file, at six sizes.

### D214: presets can be renamed, and replacing or deleting one asks first (2026-10-13)

Backlog P-07: "add preset rename", and "the user sees what will be changed
before destructive replacement".

- **Rename** renames the active preset to the name typed in the save
  field. `preset::rename` writes the new file whole before removing the old
  one, so a crash leaves at least one copy. It refuses a name another
  preset has (a change of case only is allowed), because a rename must
  never replace anything. The fav slots that recall the preset on this
  scene follow it. A playlist cue that names the old preset is **not**
  rewritten (the playlist is the operator's file); the notice says so, and
  that cue warns at compile time as any missing preset does.
- **Save says Replace** when the typed name is an existing preset, and its
  tooltip names the preset it will overwrite.
- **Delete is two clicks.** The first arms the button and a warning line
  appears; the second deletes. The arming is dropped as soon as another
  preset becomes active.

**Evidence:** `strobe-scene` `preset` tests (rename moves the file and its
name, refuses to replace, allows a change of case, reports a missing
preset; fav slots of that scene follow and others do not).

### D215: named layouts save the panes, and the palette applies them (2026-10-13)

Backlog U-06 asks for named layouts. A layout is what the pane model
already saves at quit: which panes are open, in which order, and every
pane's width (`prefs::Layout`, the same code `ui.toml` uses, now shared).

- **Settings > Layouts** saves the panes as they are under a name (*Replace*
  when the name exists), and lists the saved ones with *Apply* and
  *Delete*. They are kept in `ui.toml` under `[layouts]`.
- **The command palette** lists each one as *Layout: name*, so Ctrl+K and
  a few letters switch from a prep layout to a show layout.
- Applying a layout changes only the panes. It does not change the scene,
  the output or anything a show depends on, and the workspace fits it to
  the window as it fits any pane change.

**Not done:** density (compact and comfortable rows), which touches every
row height in the UI, and a shortcut per layout.

**Evidence:** the `ui::state` test (save, apply in order, an unknown name
changes nothing, written to the preferences, delete) and the palette test
(a layout is an entry, found by name).

### D216: the control window slows to 10 Hz while switching to it waits on the output (2026-10-13)

TORNADO T11 to T13 traced late output frames, windowed on a GTX 1080 Ti,
to the control window. Making the shared context current on the control
window's surface waited about 16 ms for the output's vsynced swap. Each
control redraw at 30 Hz therefore cost the show a vertical blank: 37-45% of
output frames were late. The wait came and went by itself (gone after
1.5 to 5 minutes in some runs, never in others), so it is the driver's or
the compositor's choice of present path, not the app's state. The T480
(Intel) never waits there, and a fullscreen output covering the control
window was always fine, because a covered window is not painted.

- **A pacer.** `stats::ControlPacer` smooths each redraw's make-current
  time. Above 8 ms the control window redraws at 10 Hz; below 3 ms it
  returns to 30 Hz. The gap keeps it from flapping. Both changes are
  logged. Ten redraws a second still cost up to ten blanks a second in
  that state, but a third as many, and the preview and sliders stay
  usable. The show's frames come before the UI's smoothness.
- **The cure is structural.** Drawing the control window from its own
  thread, with its own shared context, would end the switching. That is a
  large change to the frame loop (the UI model borrows the app's whole
  state each frame), so it waits until this pacer proves too blunt on a
  real two-screen show.
- **Why not more.** Turning vsync off on the output would tear the
  projector. Skipping the control window's swap would freeze the operator's
  screen.

**Would reopen:** a two-screen show (output fullscreen on a projector,
control window visible on the laptop) still losing frames with the pacer
engaged.

*Addendum (TORNADO T14):* engaged at 0.56 s and held late frames at
568-581 a minute for 6 minutes, 36% of the unpaced figure and about one per
control redraw, as designed.

### D217: media is chosen in the Inspector and per cue; a cue's beats the operator's beats the scene's (2026-10-13)

Until now only `--media` could put a picture behind `syn_Media`, and a
playlist cue always played its scene's own `MEDIA` (backlog M-02).

- **Inspector.** A Media row under the tags says what shows, by file name
  (the full path on hover), and where it comes from: the scene's own, named
  by this cue, or chosen. Choose opens a path field (a picture, or a folder
  of pictures as a sequence); Scene's own clears the choice. The row is
  hidden for a scene whose shaders never read `syn_Media` or
  `syn_MediaType` (`ScenePlayer::reads_media`), unless media was chosen
  anyway. A path that does not exist is refused with a notice, not sent to
  the decoder.
- **The operator's choice lasts the run.** It shows in the live scene, in
  every scene loaded by hand, and in each cue that names no media of its
  own, as `--media` did for hand loads. It is not saved: a picture chosen
  for one night should not appear at the next launch.
- **A cue's `media` key.** `media = "loops/rain"` in a cue (relative to the
  playlist file) shows that picture for the cue. It is not inherited from
  the section or playlist, because media belongs to the look, like the
  preset. The LIST cue card edits it (`CueEdit::SetMedia`); the file writer
  keeps it.
- **Precedence: cue, then operator, then scene.** A choice made while a cue
  with its own media plays is kept for later cues and said so in a notice.
- **Choosing is idempotent.** `ScenePlayer::choose_media(None | Some)`
  compares with what shows or is on its way and does nothing when they
  match, so a cue that restarts its scene, or every cue of a show with one
  chosen loop, does not decode the pictures again. The old picture stays up
  until the new one lands (D186). Going back to no media drops the textures
  at once (black, `syn_MediaType` 0).
- **Left:** the preset media channel (PLAN's third channel). It needs a
  rule for a preset recalled while a cue names media, and is backlog M-02's
  remainder.

**Would reopen:** an operator wanting their chosen media remembered across
launches, or a cue wanting media inherited from its section.

### D218: a drop goes where it belongs, decided from the path alone (2026-10-13)

A drop used to be a Shadertoy import or a refusal. Now `intake::classify`
reads the path (and, for a folder, what is in it) and the app routes it:

- **a scene folder** (one with `main.glsl`, or a dropped `scene.json` or
  `main.glsl` standing for its folder): played. One already in the library
  plays from where it is; one from elsewhere is copied in first
  (`intake::copy_scene`), as Synesthesia's import does, then the library is
  rescanned and it loads, the same path a finished Shadertoy import takes.
  A name the library already has is refused, never merged or overwritten.
  The copy is bounded (2000 files, 256 MB), refuses symbolic links, and a
  copy that fails part-way is removed. It runs on the UI thread because
  scenes are small; the bound is what keeps a mistaken drop of a big folder
  from freezing the window.
- **a `.toml` file**: loaded as a playlist, with the LIST pane opened.
- **a picture, or a folder of pictures**: shown behind `syn_Media`, exactly
  as the Inspector's Choose does (D217).
- **any other `.json`**: the Shadertoy import, as before.
- **anything else**: a notice saying what can be dropped.

While a file hovers, a banner above the preview says what dropping it will
do, so a drop is never a surprise. Several files dropped at once arrive one
by one (winit sends one event each), so the last picture wins for media.

**Would reopen:** a scene folder too big for the UI thread to copy, or a
person expecting a drop of a scene to link to it rather than copy it.

### D219: the generated-controls review edits the copy without measuring again (2026-10-13)

The review card (D202) could only open or discard the copy the worker
wrote. Backlog U-03 asked for keep, drop and re-range per control before
the copy is used.

- **No second measurement.** Scoring is the slow part: tens of seconds on
  an HD 4000, and it needs its own context. The analysis that finds the
  candidates is a pure function of the code, so
  `strobe_autocontrol::rewrite_chosen(folder, choices)` finds them again by
  `id`, rewrites only the chosen ones under the names the engine gave them,
  and writes each slider's chosen range. No GL is touched, so it runs in the
  app on the UI thread, in milliseconds.
- **A changed source is refused.** Each choice carries the candidate's
  context line from the measurement. If the code no longer reads the same
  (the scene was edited in the Editor meanwhile), the review is refused
  with "generate them again", never applied to the wrong literal.
- **Fidelity is kept by construction.** Every kept control's default is
  still the original literal, and a dropped one leaves the literal in the
  code, so the picture at defaults is the one the worker verified. The range
  fields keep the default inside the range.
- **The card.** Each control has a keep box; a kept slider has its range
  as two drag fields. Open becomes "Apply and open" once anything is
  edited. Dropping every control deletes the copy (the same guarded
  `discard`). The source scene is read, never written.

**Would reopen:** a person wanting to add back a candidate the engine cut
(the review only narrows what was kept), or to rename a control.

### D220: density is one window-wide setting, applied with the theme (2026-10-13)

UI_DESIGN section 21 asks for a compact Perform density, chosen once and not
by each widget. Backlog U-06 had it left.

- **Two densities.** Comfortable (the default, as before) and Compact:
  body text 12.5 instead of 13.5, small 11 instead of 11.5, item spacing
  6x2 instead of 8x4, controls at least 20 tall instead of 24, and list
  rows (browser, cue list) 32 instead of 44, with their thumbnails and
  text lines placed from the density. At 1280x720 compact shows all 12
  library scenes where comfortable shows 8.
- **Where it lives.** `theme::Density`, set by `theme::apply` beside the
  palette (a thread-local like the palette, read by `theme::row_height()`),
  saved in `ui.toml` as `density`, chosen in Settings > Appearance, and
  `--density` overrides it for a run (screenshot mode reads no saved
  preferences, as with `--theme`). An unknown saved value falls back to
  comfortable.
- **Not Perform versus Edit.** The design doc pictures a different density
  per workspace. One setting is simpler and covers the case that matters
  (a small laptop screen). Per-workspace density can come later.
- **Found on the way.** A macro card's rows inherited the window's minimum
  control height, so header, value and rail did not fit the 60-point card,
  and its slider was clipped off at both densities. Rows inside a card
  are now as tall as their text.

**Would reopen:** a third density, or a person wanting Edit roomier than
Perform.

### D221: the output waits for its own swap once the control window is seen waiting (2026-10-13)

D216's pacer cut TORNADO's late frames to a third by redrawing the
control window less. T15 tested the cause directly: with the output
calling `glFinish` right after its vsynced swap, switching the shared
context to the control window took 1.2-1.7 ms instead of 16, the control
window ran at 30 Hz, and late frames fell from about 568 a minute to 5-8.
The driver was waiting for the windowed output's flip at the context
switch; waiting for it on the output's side, where a wait is due anyway,
removes it.

- **Automatic, once.** `stats::SwapPolicy` starts off. The first time the
  pacer would slow the control window, the policy engages the wait instead
  and the pacer starts over at 30 Hz (logged as "the output now waits for
  its own swap instead"). If the pacer slows again with the wait on, its 10
  Hz stands: the wait did not help on that machine.
- **Not everywhere.** `glFinish` after the swap costs the overlap of one
  frame's CPU work with the GPU's. The T480 never shows the wait, so it
  never pays. While a fullscreen output covers the control window (which
  then hardly redraws) the wait is skipped.
- **Overrides.** `STROBE_FINISH_AFTER_SWAP=1` forces it on and `0` off
  (DEBUGGING.md).
- **Still deferred.** The control window on its own thread and context
  (D216's structural cure) is not needed for this; it stays the answer if
  a machine shows the wait with this engaged.

**Would reopen:** a GPU-heavy scene on an affected machine missing frames
because of the lost overlap, or T16 not engaging automatically.

*Addendum (TORNADO T16):* engaged by itself at 0.59 s, 6-14 late frames
a minute at 30 Hz. Covered, the few control redraws still waited and
tripped the pacer; the pacer now ignores redraws while covered.

*Addendum (TORNADO T17):* on volume-clouds with the GPU at 75-90% of the
frame, auto had 7-11 late frames a minute, forced on 2-6, forced off
560-573, all at 59.8-59.9 fps: the lost overlap costs nothing measurable,
so the reopen condition above did not occur.

*Also in this commit:* a player whose media has no picture yet replaces it
at once when other media is chosen, so the scene's own picture no longer
shows for a moment before `--media` (T480 Task 26), and every choice is
logged as `syn_Media: <path>` or `syn_Media: none`.

### D222: a scene's picture is taken from its live frame, by the operator (2026-10-13)

The browser shows `IMAGE_PATH` (D212), and an imported or new scene has
none. Backlog U-06 left "a picture for a scene that has none".

- **The operator chooses the moment.** "Use frame as picture" beside the
  tags in the Inspector reads the playing scene's own output back once
  (before blackout and the post stage, so the picture is the scene and not
  tonight's settings), shrinks it to fit 320x180, and writes
  `thumbnail.jpg` in the scene folder. A scene's look depends on time and
  audio; an automatic capture at a fixed time would often catch a dull or
  black frame, and rendering every scene offscreen to make pictures would
  compile them all.
- **`scene.json` is edited in place.** `IMAGE_PATH` is set with
  `strobe_scene::json_edit::set_member`, the text-preserving writer the
  tags already used, now its own module. An older picture the scene named
  stays on disk; only the name moves. Both files go through a temporary
  name and a rename.
- **The browser sees it at once.** The entry rereads `scene.json`, and the
  thumbnail cache forgets that path so the new picture is decoded again.
- **Refused** while the Editor holds unsaved edits to the same scene, as
  tagging is.

**Would reopen:** a library of hundreds of scenes without pictures, where
an offscreen batch (the generate worker's approach, D202) earns its cost.

### D223: a preset can carry media, and ranks below the operator's choice (2026-10-13)

PLAN section 10 gives presets three channels: scene, meta and media. Only
the first two existed (D170); media was not controllable until D217.

- **Saved only when it means something.** `Session::capture_preset`
  records what `syn_Media` shows when that is not the scene's own `MEDIA`
  (a cue's, the operator's, or a preset's), turning the preset's `media`
  channel on. Otherwise the channel stays off and nothing about media is
  written, so presets saved before this, and presets of scenes nobody chose
  media for, never touch it.
- **Portable.** A picture inside the scene folder is stored relative to it;
  one elsewhere is stored as chosen.
- **Order at a cue start or a load by hand:** the cue's own `media`, then
  the operator's choice (D217), then the preset's (the cue's preset, or
  the scene's `default` one), then the scene's own. The operator's choice
  is a live override for the night and a preset is part of the scene's
  authored look, so the override wins, as it already did over `MEDIA`.
- **Recalled by hand, it applies.** Recalling a preset in the MAP pane or
  from a controller is the latest instruction, so its media shows at once.

**Would reopen:** an operator wanting a preset's media to beat their own
choice at cue starts.

### D224: a browser scene drags into the cue list, and disabled cues are listed to enable (2026-10-13)

Two gaps in LIST authoring (backlog U-07):

- **Drag from the library.** A browser row is a drag source (egui's
  payload drag and drop, `ui::browser::LibraryDrag` carrying the folder
  name). While it is held, its name follows the pointer, and a cue row it
  passes over shows an accent line along its bottom edge; released there,
  it is `CueEdit::InsertAfter` that cue, the same edit as the card's "Add
  <scene> after", so the show keeps its place and only a new scene
  compiles (D213). Rows stay clickable and double-clickable; a drag starts
  only after the pointer moves.
- **Disabled cues.** The timeline lists only enabled cues, so a disabled
  cue could not be selected to enable it. The card gains Disable, and a
  folded "Disabled cues (N)" list under it names each by section and scene
  with Enable. It is absent when nothing is disabled.

**Would reopen:** dropping between sections or onto an empty playlist
(today a drop lands after an existing cue), or several scenes dragged at
once.

### D225: a long image sequence is shrunk to fit its memory, not cut (2026-10-13)

D186 bounded a sequence at 192 MiB of decoded RGBA, which on an HD 4000
with shared memory is the right order of limit, but it enforced the bound
by cutting: a 600-frame 1080p loop (about 5 GB decoded) played its first
24 frames and stopped. A VJ loop that ends a second in is worse than one at
a lower resolution.

- **Shrink the whole loop alike.** The decoder reads the first frame, and
  if that many frames of its size would pass the budget it decodes every
  frame (the first again included) with its long side at
  `sqrt(budget / total) x` the original (`assets::sequence_edge`), so all
  frames are one size and the loop fits. The 600-frame 1080p loop plays
  complete at about 386x217. Logged once, as `syn_Media: ... shrunk to N
  px`.
- **A floor.** Not below 256 px on the long side
  (`MIN_SEQUENCE_EDGE`): past that a postage stamp is worse than a shorter
  loop, so the old cut applies, with its message. Frames of mixed sizes
  are still guarded by the same byte count.
- **Unchanged:** decoding on its own thread, one upload a frame, the
  2048 px edge limit on any picture.

**Would reopen:** the T480 measurement showing 192 MiB itself is too much
(or too little) for shared memory, or a streaming cache becoming worth its
complexity for loops that would shrink too far.

### D226: breakdowns and drops come from raw energy, with hysteresis and gaps (2026-10-13)

The playlist format has had `advance = "audio-drop"` and
`"audio-breakdown"` since stage 1, and the clock acts on the events, but
nothing produced them (backlog P-01).

- **Raw energy, not the scalars.** The `syn_*Level` scalars are normalised
  to a peak that decays over 10 s, so a long breakdown is lifted back up to
  a full-looking level; a detector on them would call false drops in every
  long quiet passage. `AudioFrame::energy` now carries the latest hop's RMS
  of the whole signal and of the bass, un-normalised (zero when no audio
  arrives and for the synthetic track). Every test is a ratio between a
  short (0.4 s) and a long (20 s) average, so input gain does not matter.
- **Breakdown:** the short bass under 40% of the long bass for 2 s, while
  there is sound and the track has had bass. **Drop:** after a breakdown,
  whole and bass back to 80% of what they were before it; the long
  averages are frozen during the breakdown so it cannot become the new
  normal.
- **What it refuses.** A short dip (one bar) is not a breakdown. Silence
  (RMS under -80 dBFS for 0.25 s: a device unplugged, a paused player)
  reports nothing and the averages restart when sound returns, with 8 s of
  quiet before anything is reported, so a reconnect is not a drop. Two
  events of one kind are 8 s apart; a drop may follow its own breakdown
  sooner. Frame steps over 0.5 s are skipped.
- **The show** feeds the detector every frame from the frame it already
  reads for the beat, and calls `Command::AudioEvent`; the clock moves on
  only if the playing cue listens for it. Each event is logged, with
  whether the cue moved. A cue that can end only on audio, played with
  synthetic audio, raises a notice saying to press Next.
- **Thresholds are first estimates.** They pass synthetic structure and the
  real analyser on a kick/hiss/kick signal; real sets will tune them.

**Would reopen:** real music calling breakdowns in verses or missing
obvious drops, which would also argue for P-02's bar-aware timing.

### D227: a macro can glide, and the glide belongs to the write, not the control (2026-10-13)

Backlog U-04 and P-07 asked for smoothing per macro. A cheap MIDI fader
steps in 1/127ths and a key or a button jumps from one end to the other;
a scene's own `slider smooth` eases that, but a plain `slider` or a meta
control snaps, and the picture jumps.

- **`Macro::glide`**, in seconds (a time constant, capped at 10 s, edited
  as "Glide" in the MAP pane's macro options). The engine passes it with
  the write (`target::Flat::glide`), and `ControlBank::set_normalized_gliding`
  sets the target as usual and marks the control as gliding.
- **The glide is the write's.** While it lasts, the output eases with
  `1 - exp(-dt / glide)` whatever the control's own smoothing, frame-rate
  independent. It ends when the output arrives (within a ten-thousandth of
  the range) or when anything else writes the control (the Inspector, a
  script, a preset, another mapping), which then behaves exactly as it
  always did. So a macro's feel never leaks into the rest of the app.
- **Where it does not apply:** a bang, a toggle, a dropdown, and speed or
  traveler controls take a macro's write as before. Their meaning is the
  jump, or they already integrate.

**Would reopen:** a person wanting the glide to shape a toggle as a fade,
or per-direction (attack and release) times.

### D228: a macro can have a key, which it holds like a button (2026-10-13)

The last of U-04 and P-07: a keyboard shortcut per macro, for an operator
without a controller (or with one key short).

- **Held, not toggled.** Down drives the macro as a momentary button
  mapping would: the top of its reach while held, the bottom when let go,
  through its glide (D227) if it has one. With a reach of 50-100% and a
  glide of 0.3 s, a key is a "bump"; a toggle follows the key and a bang
  fires on the press. `Engine::macro_key` does it through the same `drive`
  as every mapping, with a button state per macro.
- **Commands keep their keys.** The keyboard handler gives a key to a
  command first; only a key no command claims reaches the macros, and not
  while a text field or a modal has the keyboard. The MAP pane's "Set"
  captures the next key (Esc cancels) and refuses one a command holds,
  naming it. One key drives one macro: giving it to another takes it away.
- **A release reaches what the press drove.** The handler remembers which
  macro each held key started, by key, so letting go after a modifier
  changed still lets the macro down.
- **Saved** with the macro (`key`, the chord's text) in
  `show-control.json`.

**Would reopen:** a person wanting a key to toggle a macro rather than
hold it, or a key on a command and a macro at once.

### D229: a Shadertoy URL on the cue card imports and becomes the next cue (2026-10-13)

U-07 asked for pasted Shadertoy URLs in the playlist. The Import pane
already imports a URL, id or exported file, but a finished import loads
the scene by hand, which unloads a running show.

- **"Import after"** on the selected cue's card takes a URL, an id or a
  file path and queues it on the same import worker
  (`Action::ImportCueAfter`). The app remembers the cue and the exact text.
- **When that import lands** the library is rescanned and the new scene is
  inserted after the cue (`CueEdit::InsertAfter`, the same edit as the
  card's other add buttons), without loading it, so the show keeps
  playing and only the new scene compiles (D213).
- **Already in the library** is not a question here: the import outcome
  now names the existing folder (`import::Outcome::Exists`), and for a cue
  that copy is used as it is, without the replace prompt. From the Import
  pane the prompt is unchanged.
- **Only the import the card asked for** becomes a cue: another import
  finishing meanwhile is handled as before.

**Would reopen:** wanting several URLs pasted at once into a section.

### D230: Ctrl-click selects several cues, and settings go to all of them (2026-10-13)

U-07's acceptance asks for an 80-cue show authored in minutes "using tags
and multi-edit". Tags drive Generate (D205); this is the multi-edit.

- **Selecting.** A click selects one cue as before. Ctrl-click (Cmd on
  macOS) adds a cue to the selection or takes it out; the card's title
  says "and N more (settings go to all)", and every selected row is
  highlighted.
- **What goes to all:** the settings that leave every cue where it is,
  so positions stay valid while the edit runs: length, transition,
  preset, media and disable (`edit::apply_many`). Moving, duplicating,
  deleting and inserting stay one cue at a time and act on the first.
- **All or nothing.** Each cue is edited in turn on a copy; a position
  that has gone stale refuses the whole edit with one line, and the show
  is untouched. Otherwise it is one playlist swap, as a single edit is
  (D213).
- **The selection** survives a setting edit, and is cleared after a
  structural edit or a disable, which move rows in the timeline.

**Would reopen:** wanting a Shift-click range, or multi-selection across a
reload from disk.

### D231: a save keeps the file's comments where the content still matches (2026-10-13)

The LIST pane edits the playlist as a model and the writer lays it out
canonically (stage 1's choice: deterministic bytes, clean diffs). Over a
hand-written file that dropped every comment the author wrote, the last
of U-07's list.

- **Merge, then verify.** `merge::keep_layout(original, fresh)` parses
  both with `toml_edit` (already in the lock file through `toml`) and
  keeps the original document, changing only what differs: a value that
  is the same keeps its exact text (`90` stays `90`); a changed value
  takes the new text inside the old comments; new keys and sections are
  added canonically; removed ones go with their comments.
- **Cues are matched by content, not index.** In an array, an element
  equal to one further on is kept whole; one for the same scene with
  other settings is the same cue edited (its comment stays); an element
  that is new is inserted bare. So inserting, deleting or moving a cue
  never leaves a comment on the wrong cue.
- **The show is never at risk.** `file::save` parses the merged text and
  uses it only if it reads back to exactly the model being saved;
  otherwise it writes the canonical text, as before. A new file is
  written canonically.

**Would reopen:** sections reordered by hand (sections match by position),
or a comment that should follow a moved key.

### D232: the remote's bridge to the app thread is a bounded queue with a reply per request (2026-10-13)

The LAN remote (epic L) needs one place where a network thread meets the
app's event-loop thread. L-05 asks that it be bounded, never block the
frame, answer each command, and give a reconnecting phone the whole state.

- **`strobe_remote::bridge`, std only.** `bridge(capacity)` returns a
  `Remote` (clonable, for the adapter) and an `AppEnd` (kept on the app
  thread). Commands go over a `sync_channel` with `try_send`: a full queue
  is `Busy` at once, so a flood costs the adapter an error, never the
  frame memory or time. Each command carries a one-slot reply channel the
  app answers with `try_send`, so answering never waits on the adapter;
  `Pending::wait(timeout)` gives the adapter `Busy` if the app did not
  answer in time and `Unavailable` if the app end is gone.
- **The snapshot is revisioned by content.** `AppEnd::publish` bumps the
  revision only when the state differs, so the app can publish every frame
  and the adapter sends nothing when nothing changed; `Remote::snapshot`
  is always the whole current state, which is what a reconnect needs.
- **Remote commands are app commands.** `RemoteCommand::id` is the wire
  name and the app's `CommandId` id; an app test asserts every remote
  command maps to a mappable app command, so the remote runs exactly what
  a key, pad or OSC message runs and the two lists cannot drift.
- **Not wired into the app yet.** Draining `AppEnd` once a frame belongs
  with the listener (L-02): until something can submit, a drained queue is
  dead code. No socket is opened by this decision.

**Would reopen:** an adapter on an async runtime that needs an awaitable
reply (wrap `Pending` in the runtime's blocking task, or swap the reply
channel for a oneshot), or a command that must be answered before the
next frame.

### D233: the downbeat is the beat with the most bass and the most change, and unsure means none (2026-10-13)

D75 counted bars from wherever the beat grid began, so a "32 bars" cue
ended on a bar line the music did not have. P-02 asked for a
confidence-aware estimate that falls back when unsure.

- **Two cues per beat.** `strobe_audio::downbeat::Downbeat` scores each
  beat of the grid by its *accent* (the mean raw bass energy over the
  beat's span) and its *change* (how far the four band levels rose from
  the previous beat, in log terms, each band past a 22 % dead zone). Each
  is divided by its running mean (the change's floored at 0.3) and the sum
  goes to slot `beat mod 4`. A harder kick or a sub drop on "one", or a
  chord or bass change at the bar line, both point at the same slot.
- **What was tried and why it changed.** Each of these, in turn, gave
  even kicks a confident wrong answer on the synthetic fixture:
  - a peak, or a narrow window around the beat, because the hop
    alignment and the grid's wobble vary it from beat to beat;
  - sampling the latest hop once a frame, because the hop-to-kick
    alignment repeats only every eight beats;
  - counting falls in the change as well as rises, which credits the beat
    after an accent equally;
  - decaying per beat or comparing mid-bar, which favours the slot scored
    last;
  - the change normalised by its own near-zero mean on steady music.

  So the estimator is fed every hop from the analyser, slots decay once a
  bar, and they are compared only after the fourth slot of a bar.
- **Confidence is the lead, past noise.** It is
  `(lead - 0.12) / (0.4 - 0.12)` clamped, where the lead is the best slot's
  margin over the runner-up as a fraction of the best. It is 0 for eight
  beats of warm-up (the running means) plus sixteen of evidence. Even kicks
  at 97, 120, 123.5, 128, 140 and 174 BPM give 0. Kicks on one and three
  tie two slots and give near 0. Both are the honest answer.
- **Hysteresis and reset.** A new leader takes over after leading two bars
  in a row, so a one-bar fill does not move every bar line; a new track's
  bars are followed within about eight bars (slot memory). The estimate
  resets when the grid loses its lock (silence), and it is fed only while
  the published tempo confidence is above 0.3.
- **In the counter's numbering.** The playlist clock counts bars on
  `syn_BeatTime`, which in the live analyser is the count of detected
  beats, not the grid's position. The estimate is made on the grid and
  converted by the counter's lead over the grid at the last detected beat,
  so `AudioFrame::downbeat = [offset, confidence]` means "a bar starts when
  `syn_BeatTime` reaches `4n + offset`". It is not a uniform: no SSF scene
  reads it, and Synesthesia documents no such value.
- **The clock uses it only when sure.** `BeatInfo::downbeat` (bars start at
  `4n + downbeat`) is set by the app at confidence 0.5 or more and is 0
  otherwise, so an unsure estimate is exactly D75's behaviour. An estimate
  that moves while a cue waits for its bar line can end that cue up to
  three beats early or late, once; the wait's six-beat timeout still
  bounds it.

**Would reopen:** recorded music where the estimate is confidently wrong
(the fixtures are synthetic: an accented kick and a chord change), music in
three (bars stay four beats), or a scene or Synesthesia feature that wants
the bar position as a uniform.

### D234: Synesthesia's OSC addresses are aliases where they mean the same (2026-10-13)

P-08 asked whether Strobe should speak Synesthesia's OSC layout. D173 chose
`/strobe/...` before the layout could be read. TORNADO read the manual
(T20), and the layout turned out to map almost one for one onto D173's
targets.

- **Accepted, with no mapping, when built-in OSC is on:**
  - `/controls/scene/<name>` and `/controls/meta/<name>`, each with an
    optional `/x`, `/y`, `/r`, `/g` or `/b`;
  - `/controls/global/<type>/<n>`: the `n`th control of that type in the
    playing scene, from 1, in declaration order;
  - `/favslots/<n>`;
  - `/playlist/next`, `/playlist/previous` and `/playlist/toggleplay`,
    which run `next_scene`, `prev_scene` and `toggle_hold`.

  Values are normalised 0 to 1 in both, so these are aliases in
  `osc::builtin_target`, not a second engine. A user mapping on the same
  address still wins.
- **Global positions are a name.** `/controls/global/slider/2` becomes the
  control target `@slider/2`, which `target::find_control` resolves
  against the playing scene's controls of that kind. It is therefore
  re-resolved on every scene change, as Synesthesia's global addresses
  are, and it can also be typed into a mapping.
- **Two or three numbers to a whole pad or colour** set its components
  (`Engine::components`), which is how Synesthesia and TouchOSC send an xy
  pad. The split happens only when the built-in meaning applies, never for
  an address a mapping answers to or while learning, so those see the
  message as sent.
- **Ignored rather than guessed:**
  - the `/raw`, `/default`, `/random`, `/preset` and `/lock` suffixes,
    which would otherwise read as a group and a name;
  - scene-specific `/controls/<scenename>/...`, since Strobe has no target
    that applies only while a given scene plays;
  - anything with a string argument (`/scenes`, `/presets`, `/media/name`,
    `/playlist/select`), which the decoder drops (D172).

  Each is a test case. Feedback stays on `/strobe/...`: Synesthesia's
  output layout (names per global slot, dropdown labels) is a separate
  feature.
- **Not imported:** Synesthesia's MIDI mappings. They are internal
  identifiers in `midi_mappings.json`, with no export of their own.

**Would reopen:** a real Synesthesia or TouchOSC layout that relies on an
ignored address (the suffixes and `/scenes` are the likely ones), a
request for Synesthesia-shaped feedback, or the underscore question in Q23
mattering (Strobe's name matching ignores `_` either way).

### D235: a dropped folder of scenes is added to the library a scene a frame, and none is played (2026-10-13)

I-02 asked for batch import. D218 handles one dropped scene folder by
copying it in and playing it; dropping a pack of a hundred scenes that way
would switch scenes a hundred times.

- **A folder whose subfolders are scenes is a pack.** `intake::classify`
  gives `Dropped::Pack` for a folder with no `main.glsl` of its own and at
  least one scene folder directly inside it (links not followed, name
  order, at most 500). It takes precedence over "a folder of pictures", so
  a pack that also holds a cover picture is still a pack.
- **One scene a frame, from `schedule`.** `intake::Pack::step` copies the
  next scene with D218's `copy_scene` (same file, size and link limits).
  The copy runs on the UI thread, and one scene at a time keeps each
  frame's share to one small folder, so the show keeps playing with no
  worker or channel to manage. A second pack dropped meanwhile joins the
  queue.
- **Nothing plays.** A scene the library already has (the same folder, or
  one of the same name) is counted as already there and not copied.
  Refusals go to the log and DIAG through `failure::report`. At the end
  the library is rescanned once, and a banner says "Added N scenes to the
  library; M already there; K could not be added (see the log)".

**Would reopen:** packs large enough that a scene a frame is too slow
(500 scenes take about 8 s at 60 fps), nested packs (only one level is
read), or a watched drop folder, which would feed the same queue.

### D236: the remote protocol's version is whole, and a stranger's hello is told why (2026-10-13)

L-01 asked for versioned, bounded messages and compatibility tests before
any listener exists.

- **Versions are whole numbers, negotiated once.** `negotiate(client)`
  accepts `MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION` (both 1 today) and
  otherwise answers `version_mismatch`. There are no feature flags inside a
  version: a change to any message's shape is a new version.
- **One decode for every adapter.** `decode_client` refuses more than
  4 KiB before parsing, then any unknown type or field (D232's strictness),
  an invalid request id, and an empty or over-128-byte pairing code or
  token. Each of these is `invalid_request`.
- **A newer client hears "version", not "malformed".** A `hello` that does
  not parse because a newer protocol added fields is read leniently for its
  `protocol_version` alone. If that version is not spoken here, the answer
  is `version_mismatch`, so the phone can say "update Strobe" instead of
  failing obscurely.
- **Version 1 is frozen by golden tests.** Every client and server message
  shape is written out as JSON in the tests, and each must read and
  re-serialise byte for byte. Changing one is breaking a shipped client,
  and the test says so.

**Would reopen:** a second protocol version, which would raise
`PROTOCOL_VERSION` and keep version 1's goldens for as long as
`MIN_PROTOCOL_VERSION` allows it.

### D237: a song change is a gap or a new tempo that holds, and only live, steady audio counts (2026-10-13)

M-05 asked for song-change transitions, safe with no audio, low confidence
and reconnects, and never a second transition on top of one in progress.

- **Two cues, both conservative.** `strobe_audio::song::SongDetector`
  reports a new song when either holds:
  - *a gap*: the raw level under -80 dBFS for 0.8 to 10 s, after at least
    30 s of music, then sound again for 1.5 s. A breath is shorter, a stop
    is longer, and a start after a stop is not a change;
  - *a new tempo*: at tempo confidence 0.6 or more, a reading over 4 % from
    the settled tempo that stays within 2 % of itself for 8 s. Double and
    half time count as the same tempo, since the estimator flips octaves,
    and the first tempo heard settles without an event.

  Changes are at least 30 s apart. A new song's tempo is learned again
  quietly.
- **Not heard:** a DJ mix that holds one tempo through the change with no
  gap. Spectral novelty could hear it, but it also hears every breakdown
  and drop, which D226 already reports as their own events. The cue's
  timer or a person covers that case; the user guide says so.
- **Only live, steady audio.** The show feeds the detector only from live
  capture, and any change of `AudioInput::source()` resets it. A device
  lost means the synthetic fallback, and the device back means live again;
  either is a reset, so a reconnect cannot pass for a gap. A cue that
  waits only on audio events gets D226's notice on synthetic audio, which
  now names a new song too.
- **The file says `audio-song`.** `AudioEventKind::SongChange` joins drop
  and breakdown, and can stand alone or sit in a `FirstOf` list with the
  timer. It works with anchors as the other events do: an event ends the
  cue, and the schedule carries on from there.
- **No event during a transition.** `PlaylistClock::audio_event` ignores
  every audio event while a transition runs. The cue coming in has only
  just started, and acting would cut the crossfade short with another.
  This is a change for drops and breakdowns too, in the same direction.
  Manual next and jumps are unchanged.

**Would reopen:** real sets where the gap or the tempo rules misfire (the
thresholds are first estimates), a DJ-mix case worth spectral novelty, or a
player whose gaps are under 0.8 s.

### D238: chosen media is sampled as the scene's MEDIA slot says (2026-10-13)

M-01 audited Strobe's media and cube-map handling against SSF's own
documentation, which TORNADO read (T21). Almost everything already matched
(Q22). One thing did not: SSF's `MEDIA` key is a slot whose only settings
are `WRAP` (default `repeat`) and `FILTER` (default `linear`), and they
apply to whatever media the operator puts in it. Strobe accepted the slot
(D193) but dropped its settings, and sampled chosen media with clamp
always.

- **`SceneDef::media_sampling`** is the slot's `WRAP` and `FILTER` (repeat
  and linear when absent or when the scene has no `MEDIA` key, as SSF
  says).
- **`Player::choose_media`** applies it to the operator's request. A scene
  with a picture of its own (Strobe's path form) samples chosen media as
  it samples its own: clamp unless it says otherwise.
- **An unknown `WRAP` or `FILTER` in a slot** is a load error, as it is
  everywhere else in `scene.json`.

**Would reopen:** a real scene whose edges look different from Synesthesia
because of the repeat default, which would mean Synesthesia does not
follow its own documentation here.

### D239: the benchmark stays script-free and says so on every report (2026-10-13)

C-05: `strobe-bench` renders scenes without running their `script.js`, and
its table could be read as the scripted scene's real cost.

- **Script-free by design.** The bench measures GPU work per scene at a
  fixed resolution and is part of the Windows cross-check. Running scripts
  would pull in QuickJS, which needs the Windows SDK to cross-compile (the
  reason `strobe-script` and `strobe-app` are excluded there).
- **Marked, not hidden.** `results.md` states above the table that scripts
  are not run, counts the scenes with a `script.js`, and marks each with
  `*`.
- **Script cost lives in the corpus harness.** `cargo xtask corpus` runs
  scripts with a fixed `dt` and synthetic audio, so it is reproducible, and
  records `frame_ms` per scene (D195). That is the number to quote for a
  scripted scene.

**Would reopen:** a need to compare a script's own CPU cost across commits,
which would belong in the corpus harness as a per-scene script time.

### D240: a watched folder feeds the pack queue, by polling, and plays nothing (2026-10-13)

I-02's last open piece is a drop folder that imports on its own.

- **`--watch-folder <dir>`, off by default.** It is a run option, not a
  setting yet: a folder that imports by itself should be chosen on
  purpose.
- **Polled, like the playlist watcher.** `intake::Inbox` polls every 2 s
  from the frame loop, with no watcher crate or thread (the reason the
  playlist watch polls, D158). Each poll is one `read_dir`, plus a bounded
  walk (10,000 files) of each entry not yet taken.
- **Settled before taken.** An entry is taken when its file count, bytes
  and newest time are the same on two polls in a row, so a folder still
  being copied waits. What was in the folder when watching began is never
  taken.
- **Into D235's queue.** A scene folder, or a folder of scenes, joins the
  pack queue: copied into the library a scene a frame, already-there
  skipped, nothing played, one banner at the end. Anything else (a
  `.json`, pictures, a playlist) is logged and left, because an import
  that started playing would change the show unasked.

**Would reopen:** wanting Shadertoy exports from the folder too (they
would need an import that does not play), or a Settings field so the
folder persists.

### D241: the executable's icon is a resource compiled by a build script (2026-10-13)

R-06: `strobe.exe` had no icon of its own. The window set one at run time,
but Explorer, a shortcut and the taskbar's pinned entry showed the generic
program icon.

- **`embed-resource` in `strobe-app`'s build script.** It compiles
  `crates/strobe-app/assets/strobe.rc` (one `ICON` resource) with the
  Windows SDK's `rc.exe` on MSVC, and does nothing for other targets. The
  icon is the brand `assets/brand/icon.ico`, which `cargo xtask brand`
  makes and checks, so there is one source. Its absolute path is passed as
  a macro with forward slashes, because a backslash in an RC string is an
  escape.
- **Missing compiler warns, broken resource fails.** A Windows build with
  no resource compiler still builds, with a cargo warning. A resource that
  fails to compile fails the build, so a release cannot ship without its
  icon unnoticed.
- **MIT-licensed, a build dependency only:** nothing is added to the
  binary but the resource.

**Would reopen:** a version resource (file version and product name in
Explorer's details), which would go in the same `.rc`.

### D242: the import pipeline needs no cache eviction and no cancel (2026-10-13)

I-04 listed reliability concerns for Shadertoy import. An audit found
almost all of them already met and tested (the backlog entry lists each).
Two are decided against:

- **No cache eviction.** The texture cache keys files by the SHA-256 of
  the URL, and the only URLs it accepts are Shadertoy's own `/media/...`
  textures (`fetch` refuses anything else). That is a finite set, tens of
  files and a few megabytes in all, so the cache cannot grow without
  bound. Deleting the folder is always safe; it refills on the next
  import.
- **No cancel.** A job is bounded:
  - one REST request with its timeout;
  - at most four texture downloads of 10 s each, and none after the host
    proves unreachable;
  - then conversion, which takes milliseconds.

  The queue is bounded too, and an import never replaces the playing scene
  without the Replace answer. A Cancel button would save at most about
  forty seconds in the worst case. It would cost a cancellation path
  through `ureq` and the converter, and it would add a half-written-folder
  case that the rename-into-place design avoids today.

**Would reopen:** the importer accepting other hosts' textures (the cache
would then need a size limit), or imports that take minutes.

### D243: keyboard focus is a 2 px accent ring (2026-10-13)

U-10 asks that keyboard focus stays visible. egui draws a focused widget
with its `active` style. Strobe's theme gave `active` the same surface
and the same 1 px border as `hovered`, so a focused button looked like a
hovered one, and tabbing through the panel showed nothing reliable.

- **`widgets.active.bg_stroke` is 2 px of the theme's accent.** Every
  focused widget gets a ring. A button being pressed shows it for the
  press, which is useful feedback in its own right.
- **Tested per theme.** `the_focus_ring_stands_out_from_every_surface_it_can_sit_on`
  checks the accent at 3:1 or more against the app, panel, raised and hover
  surfaces of every theme: WCAG 2.2's bar for a focus indicator.
- **No layout change.** A stroke does not move anything, and `expansion`
  stays 0.

**Would reopen:** a theme whose accent cannot reach 3:1 on its surfaces
(the test fails first), or egui gaining a separate focus style.

### D244: each executable carries a version block from Cargo's version (2026-10-13)

R-06: the T480 found that `strobe.exe` and `strobe-corpus.exe` had an icon
but no `VERSIONINFO`. As a result, Properties > Details showed an empty
product name and version, and so did Windows' crash and error dialogs.

- **Two scripts, one body.** `assets/strobe-common.rc` holds the icon and
  the version block. `strobe.rc` and `strobe-corpus.rc` each define
  `FILE_NAME` and `FILE_DESCRIPTION` and include it. `embed_resource`
  names its output after the script's stem, so one script compiled twice
  with different macros would collide. `compile_for` links each script
  only into its own binary.
- **The numbers come from Cargo.** The build script passes
  `CARGO_PKG_VERSION_{MAJOR,MINOR,PATCH}` and `CARGO_PKG_VERSION` as
  macros. The release workflow already refuses a tag that disagrees with
  the workspace version, so the file version always matches the release.
- **No SDK header.** The `FILEOS`/`FILETYPE` values are written as numbers
  instead of `winver.h` names. That way the scripts also compile with
  `llvm-rc`, which is how they were checked off Windows.
- **No manifest yet.** winit sets DPI awareness at run time, and nothing
  in the app uses common controls. A manifest would be a second resource
  to keep in step for no visible change.

**Would reopen:** a reason to declare DPI awareness, long paths or
supported OS versions in a manifest. That would be a `1 24 "strobe.manifest"`
line in the same body.

### D245: the readback checks also run on a window context, through the bench (2026-10-13)

C-04: `strobe-gfx`'s readback tests pin the `textureFinalPass` ring on a
headless EGL context. Windows' Intel driver has no such context, so on
the T480 they skip, and the ring's fence behaviour and cost on Intel
native and ANGLE were unmeasured.

- **`strobe-bench --readback`** runs five checks on whatever context the
  bench opens, including `--context window --gl native|angle`. They are
  latency (exactly two frames old once filled), cost, busy GPU (never
  newer than two frames, never backwards, no stall in `render()`),
  resize, and a lost context. It writes `readback.md` and exits non-zero
  when a correctness check fails.
- **Cost is a measurement, not a verdict.** Two players, readback off and
  on, render alternately frame by frame, so other load on the machine
  falls on both. On a GPU the blits and the fence should be microseconds.
  llvmpipe rasterises the downsample on the CPU and pays several
  milliseconds (+6 to +9 ms at 640x360 here). No one threshold fits both,
  so the row flags anything over 1 ms for a person to judge.
- **In the bench, not in a test.** Window mode must run on the main
  thread, once per process (D140), and a test harness gives neither.

**Would reopen:** a Windows CI runner with a GPU, where these could become
gated tests.

### D246: generated controls can be renamed, imports offer them, and speed controls follow st_speed (2026-10-13)

Three items left in U-03 and I-03.

- **Rename in the review.** `Review` carries the control's name, edited in a
  text field while the control is kept. `rewrite_chosen` checks every
  chosen name with the rules the engine applies to its own (`Taken::is_free`
  on the measured source):
  - a GLSL identifier;
  - not reserved, `st_` or `syn_`;
  - not an existing control, image, pass target or identifier in the
    scene, SSF or GLSL;
  - not given twice.

  A refused name is `ChoiceError::Name` with the reason. The app then keeps
  the review open with its edits, so the name can be fixed.
- **Import pane.** Once a scene imported from the pane is the one playing,
  the pane offers "Generate controls for it". It starts the same worker as
  the Inspector's button and opens the Inspector, where the review shows.
  Nothing new runs; the pane only gets a second door to D202.
- **Speed controls follow the import's clock (supersedes D125's cost).** A
  speed control made from an `iTime * k` term integrated at real time. The
  rest of the shader's `iTime` runs at `st_speed * mix(1, 0.3 + 2 * bass,
  st_audio_time)`, so dragging `st_speed` sped up half the picture. Now
  `ControlBank::tick_at(dt, rate)` integrates speed-mode controls at `rate`
  times their value. Smoothing, travelers' easing and glides stay on real
  time.
  - **The rate:** the app passes
    `strobe_shadertoy::convert::clock_rate`, computed on the CPU from the two
    controls and `syn_BassLevel`. It returns `None`, so the rate is 1, for a
    scene with no `st_speed`.
  - **One formula:** a test holds it equal to the clock pass's GLSL.
  - **No phase jumps:** a speed control is still an integrator, so dragging
    `st_speed` changes the pace, not the phase. That keeps D125's reason for
    integrators.

**Would reopen:** a speed control in an import that someone wants on real
time whatever `st_speed` says. It would need a per-control flag.

### D247: Synesthesia's default and random reach a control, a group or a bank (2026-10-13)

C-06 asked for each documented preset, meta and control behaviour of
Synesthesia to be classified, with a test. The bank already had `reset`
and `randomize`, and the script API and the Inspector used them. OSC,
though, ignored Synesthesia's `/default` and `/random` (D234).

- **`Target::Action(ControlAction)`:** a verb (`default`, `random`), a bank
  (`scene`, `meta`), and an optional group or control name (a name wins).
  It resolves like any target, so it can be unavailable when no scene is
  playing or when the group or control is not in this scene. A mapping or
  macro can point at it, and it serialises as `"kind": "action"`.
- **The app carries it out.** The engine fires `Output::Action` on a press,
  and the app applies it with the UI's random source, as the Inspector's
  random button does. A bang is never randomized, since a random press is
  not a value.
- **The OSC aliases:**
  - `/controls/{scene|meta}/<name>/{default|random}`, the same after
    `/controls/global/<type>/<n>`;
  - `/controls/groups/{scene|meta}/<group>/{default|random}`;
  - `/controls/banks/{scene|meta}/{default|random}`.
- **Still ignored, now named in COMPATIBILITY.md as Unsupported:** `/lock`
  on a control, group or bank, bank `/undo`, the `/preset` suffix,
  `/presets/new`, and the `/presets/...` recall addresses, which carry a
  string.
  - **Lock** needs a value that presets, cues, scripts and randomize all
    respect: a cross-cutting rule, not an alias.
  - **Undo** needs a history.

**Would reopen:** a controller layout that depends on lock or undo.

### D248: no Alpha Channel meta until an output carries alpha (2026-10-13)

C-01 lists Synesthesia's Alpha Channel meta control, which makes dark parts
of the picture transparent for a compositor downstream. Strobe's output
window is opaque. No output carries alpha yet: NDI and Spout are deferred
(M-06). A control would have no visible effect, and a control that does
nothing on a projector is worse than none.

- Not added to the meta bank. COMPATIBILITY.md lists it as Unsupported.
- It is the first thing to add with NDI or Spout output. It belongs in the
  post stage (D207), where Limit Colors already is, writing the alpha the
  stage now forces to 1.

**Would reopen:** an output that carries alpha.

### D249: a skipped GPU test is recorded, and CI counts skips apart from passes (2026-10-13)

D-04 asks that a skipped Windows or GPU test is never counted as passed.
Rust's test harness has no "skipped" result: a test that finds no GL
context returns early and passes. It printed "skipping", but the harness
swallows a passing test's output. A run where every shader test skipped
read exactly like one where they all ran.

- **`strobe_gfx::skip::note(reason)`** is the one way a test skips. It says
  so on stderr, and when `STROBE_SKIP_LOG` names a file it appends
  `test-name<TAB>reason`. The test's name comes from its thread, which the
  harness names after it. All fifteen skip sites in four crates use it.
- **`cargo xtask ci`** sets the log for its test step (`target/test-skips.log`,
  fresh per run) and adds a `note` line to its summary: how many tests
  passed by skipping, grouped by reason. On Linux
  `STROBE_REQUIRE_HEADLESS_GL` still makes a missing context fail.
- **CI's Windows job** sets the same variable. A step that runs even after
  a failure writes the count and the reasons to the job summary. That job
  has no headless EGL, so its GPU tests always skip, and now it says so on
  the run's page.
- **Not a failure.** The Windows runner cannot have a context, and the
  Linux job is where shaders compile. The point is that the count is
  visible, not that it is zero.

**Would reopen:** a Windows runner with a GPU, where the Windows job should
set `STROBE_REQUIRE_HEADLESS_GL` too.

### D250: the LAN remote speaks its own small HTTP and WebSocket on std threads, with sha1 the only new crate (2026-10-13)

L-02 asked for the smallest maintained server stack that serves an embedded
page and a WebSocket carrying protocol version 1 (D236), on threads of its
own, opt-in, bounded both ways, and stopped at once.

- **Compared.** axum or hyper on tokio: an async runtime and roughly sixty
  crates for one page and one socket, a runtime to own and shut down, and a
  reply path that would have to cross from async code to the blocking bridge
  (D232). `tiny_http` plus `tungstenite`: no runtime, but `tungstenite`
  brings `http`, `httparse`, `bytes`, `rand` (with its ChaCha stack),
  `data-encoding` and `utf-8`, and its blocking reader treats a read timeout
  as an error to recover from rather than a tick. Our own subset: the
  request-head reader the Shadertoy listener already proved (D114), plus the
  RFC 6455 framing a 4 KiB JSON protocol needs.
- **Chosen: our own, in `strobe_remote::server`.** About 600 lines across
  `http.rs`, `ws.rs` and `session.rs`, std only except **`sha1` 0.10** for
  the handshake's `Sec-WebSocket-Accept`. It is RustCrypto's, the same family
  and `digest` 0.10 as the `sha2` already in the tree, so it adds one crate
  and no transitive ones (0.11 would have pulled `digest` 0.11). Base64 for
  the accept key is twenty lines tested against RFC 4648's vectors.
  `getrandom`, `sha2` and `log` were already workspace dependencies.
  `cargo deny check` is clean. A real client interoperates: Node 22's
  built-in WebSocket (undici) paired, sent a command and was refused without
  an Origin, against the listener on loopback.
- **Strict rather than general.** One request per connection, `GET` only, no
  bodies, an 8 KiB head that must arrive within 5 s; client frames masked,
  reserved bits clear, control frames short and whole, text UTF-8, binary
  refused (1003), and a frame over the limit refused from its header (1009)
  before one payload byte is read. Fragmentation is supported (the RFC
  requires it) up to the same limit.
- **Threads.** One accept thread and one per connection, at most eight in
  all. A connection reads with a 100 ms timeout, so it wakes every tick to
  push a changed snapshot, check its timers, and notice a stop or a revoke;
  its reader keeps a partial frame between ticks. The frame loop never waits
  on any of it: the app end of the bridge only `try`s.
- **No outbound queue.** A connection sends the latest snapshot when the
  bridge's revision moved (many changes between two ticks are one message)
  and the replies to the one message it is handling, each with a 2 s write
  timeout. A client that stops reading is dropped, never buffered for.
- **Stop is immediate.** `Server::stop` (and drop) shuts every connection's
  socket at once, then wakes and joins the accept thread, which frees the
  port; connection threads end within a tick. Stopping twice is harmless.
  A close frame the server sends itself (revoked, idle, a flood) is followed
  by a short linger that reads what the client still sends, so the close
  arrives rather than a reset.
- **The two HTTP readers stay separate.** The import listener reads bodies
  and blocks; this one reads with a deadline and hands the bytes after the
  head to the WebSocket reader. Sharing 150 lines would make one crate
  depend on the other, or add a crate for them.

**Would reopen:** TLS (L-07), which would need `rustls` and probably a
maintained server rather than more hand-written protocol; or a protocol that
outgrows 4 KiB messages and a few a second.

### D251: pairing is a one-use code plus the operator's approval, and tokens are kept as digests and rotate (2026-10-13)

L-03 asked for a short-lived code, attempt limits, desktop approval,
per-client tokens persisted safely, revocation, rotation and redaction.

- **The code.** Six digits from the OS random source, uniform (rejection
  sampling), shown as `123 456` for 120 s. One successful use: the next phone
  needs a new code. Five wrong codes in all, across every connection, burn
  it; a connection with five failed attempts of any kind is dropped. Guessing
  is 5 in a million per code the operator chooses to open, with the operator
  watching.
- **The approval.** A right code pairs nothing: it puts a request (`iPhone at
  192.168.1.23`, from the User-Agent's device word and the peer address,
  never the whole User-Agent) in front of the operator with Approve and Deny.
  Undecided, it expires after 90 s; at most four wait. The protocol needs no
  new message for this: the phone waits for `paired` or `pairing_rejected`.
- **The token.** 32 random bytes as 64 hex characters, sent once in the
  `paired` message body. The desktop keeps only its SHA-256, in
  `remote-clients.json` (owner-only on Unix, written to a temporary file and
  renamed), so the file is not a credential. A damaged file is set aside as
  `.bad` and every phone pairs again. At most 32 clients are kept.
- **Rotation on every authentication.** After `authenticated` the server
  sends `paired` again with a new token, and the phone replaces its own. The
  old token keeps working only until the new one is first used, so a phone
  that lost the connection before saving the new token is not locked out,
  and a copied token dies the next time the real phone connects. At most two
  tokens per client are valid at once. This is a use of version 1's `paired`
  message, not a new shape (D236).
- **Revocation.** One or all, from Settings, with the listener on or off. An
  open connection of a revoked client is closed (1008, "revoked") on its next
  tick, and the page forgets its token.
- **Redaction.** `Token`, the pairing state and the stored records print
  `***` in `Debug`; `Token` implements neither `Serialize` nor `Display`. The
  log names client ids and addresses only. A test captures every log line of
  a full pairing, a wrong guess, a rotation and a forged token, and finds none
  of the codes or tokens in it. No secret is ever in a URL: the page reads
  nothing from the address bar and keeps its token in `localStorage`.
- **Restart.** Paired clients survive (the file); a code and waiting requests
  do not.

**Would reopen:** a phone browser that cannot keep `localStorage` across
restarts (the person would pair each time), or TLS (L-07), which would let
the token live in a secure context.

### D252: the remote binds a private address it is told or finds, and trusts only its own Host and Origin (2026-10-13)

L-04 asked for DNS-rebinding and cross-site WebSocket protection, limits,
and a refusal or warning for a public bind.

- **The bind.** The operator's address, or this computer's LAN address,
  found by a UDP "connect" to a documentation address (`192.0.2.1`), which
  sends nothing and only picks the route. A public address is refused with
  advice; `0.0.0.0` is allowed with a warning on the desktop; loopback is
  allowed with "a phone cannot reach it". A connection *from* a public
  address is closed unread whatever was bound, so a port forward on a router
  does not expose the show. Carrier-grade NAT space (100.64/10, which VPN
  meshes use too) counts as private.
- **Host is an IP literal and this listener's port.** A host name is refused
  (421), `.local` names included. That is what stops DNS rebinding: the
  attacker's page sends its own name. The cost is that a phone must open the
  address Strobe shows, which Settings shows and copies.
- **Origin is exactly `http://` plus that Host** on the upgrade; anything
  else, `null` or none is 403. A token alone is not enough: another site's
  page could otherwise drive a socket with a stolen token.
- **Limits.** Eight connections, 4 KiB messages, 10 messages a second with
  bursts of 20 (an empty bucket closes with 1008 after a `busy` error), 30 s
  with nothing heard (the page pings every 5 s from the moment it
  connects), 120 s (the code's lifetime) to pair or authenticate, 5 s for
  the request head, 2 s per write, 1 s for the app's answer. No CORS header
  of any kind.
- **Residual risk, stated.** This is plain HTTP. Anyone who can watch the
  Wi-Fi can read the pairing code as it is typed, the token as it is issued
  and every command, and can replay a token until the real phone next
  connects (D251). It is for networks the operator trusts. The user guide
  says so, and nothing claims HTTPS or a PWA (L-07).
- **Firewall.** Strobe never changes firewall rules. Windows asks the first
  time it listens; the guide says to allow private networks only.

**Would reopen:** a need for host names (mDNS), which would need an explicit
allowlist the operator types; or TLS.

### D253: the controller page is one embedded file under a hash-based CSP, with explicit blackout and no macros yet (2026-10-13)

L-06 asked for a touch-first page with no CDN or telemetry.

- **One file**, `crates/strobe-remote/src/server/controller.html`, compiled
  in with `include_str!`: markup, one `<style>`, one `<script>`, and nothing
  fetched from anywhere (a test greps the source for URLs, `src=`, `fetch(`
  and the like). The CSP allows exactly that script and style by their
  SHA-256, computed from the embedded file once, so an edit can never be
  served under a stale policy and `unsafe-inline` is not needed.
  `frame-ancestors 'none'`, `X-Frame-Options: DENY`, `no-store`,
  `no-referrer`.
- **What it shows:** now, next and the cue's time left, Hold, output shown
  or hidden, audio status, and a connection pill (Connecting, Connected, Not
  paired, Disconnected with the retry time, No answer, Remote switched off).
  Controls are disabled whenever the connection is not live.
- **Blackout is explicit.** The big button sends `blackout_on` or
  `blackout_off` according to the last state it saw, never the toggle, so a
  press on a stale screen cannot bring the output back by accident.
- **Reconnect** with backoff of 1, 2, 4, 8 and 10 s, at once when the page
  becomes visible again or the browser comes back online, and a stale state
  after 12 s without a word (the page pings every 5 s).
- **Accessibility:** a `lang`, a labelled code field, `aria-live` status,
  `aria-pressed` on Hold and Blackout so state is not colour alone, a 3 px
  focus ring, `prefers-contrast: more`, `forced-colors`,
  `prefers-reduced-motion`, safe-area insets with `viewport-fit=cover`, and
  portrait and landscape layouts. Tests check each is present; no real
  browser has rendered it yet.
- **Left out:** Performance Macros (protocol version 1 has no macro command;
  adding one is version 2 under D236, and the research note wants macros
  only once their target states can be reported), any live preview, a QR
  code (the address is short, and a QR encoder is another dependency for
  the desktop), and a manifest or service worker (L-07).

**Would reopen:** protocol version 2 (macros, scene selection), or real
devices showing the layout fails somewhere.

### D254: the app drains the remote once a frame, publishes at 10 Hz, and never starts it by itself (2026-10-13)

D232 left the bridge undrained until a listener existed.

- **Off unless asked.** `--remote` (with `--remote-bind` and
  `--remote-port`) or Settings' Start starts it for this run. The switch is
  never saved, so no launch opens a socket by itself; the address and port
  fields are not saved either. The paired-client list is read once, the
  first time the control window asks for it.
- **Step 1c of the frame**, after show control: up to eight commands are
  taken with `try`. Each is checked against what the app is doing now
  (`lan::refusal`: Hold, Restart, Extend and Skip need a ready playlist; Next
  and Previous need a playlist that has finished preparing, or another scene
  in the library), then run through the same `Action` its key or OSC address
  runs, then answered. A refusal is the phone's message ("needs a playlist;
  load one on the desktop").
- **State at 10 Hz.** Current and next names (cut to 120 characters, the
  strings replaced only when they change), time left rounded up to whole
  seconds so a running cue is one message a second, Hold, blackout, output
  and audio. The bridge's revision moves only on a change (D232).
- **Visible and killable.** While it listens the title bar shows "Remote on"
  with the connection count, warns when a phone waits for approval, and has
  a Stop button beside it; Stop drops every phone at once (D250). Settings'
  Remote section holds the address, the pairing code with its countdown,
  Approve and Deny, and the paired devices with Revoke and Revoke all. Bind
  failures go to the failure report and the section.

**Would reopen:** a command that must be answered within the frame, or the
T480 soak showing a cost in the frame (none is expected: an empty
`try_recv` and, ten times a second, a comparison).

### D255: no second backend; naga cannot take SSF, glslang and SPIRV-Cross could (2026-10-13)

G-02 and G-03. The question was whether Strobe's GLSL can reach a
Vulkan, D3D12 or Metal backend, through wgpu/naga or through
glslang -> SPIR-V. A tool in `docs/research/g03-frontends/` (outside the
workspace and CI) assembles every pass as the GL path does and runs each
route.

**The corpus:** 26 scenes and 51 passes, the library plus the 14 Shadertoy
fixtures, in the desktop dialect only, since SPIR-V needs ES 3.10.

- **naga is ruled out.**
  - Its GLSL front end accepts only `#version` 440/450 and wants every
    uniform in a block with a binding. Even with both done by hand it stops
    at the first `uniform sampler2D`: combined image samplers are not
    implemented. That gives 0 of 51.
  - Its SPIR-V front end fails on loading a combined sampled image. That
    gives 11 of 51, exactly the passes that sample nothing.
  - Every feedback, buffer, picture and media scene samples, so this is
    structural, not a list of fixes. It is D1's reason, now measured.
- **glslang and SPIRV-Cross would work as a front end:** 51 of 51 to GL
  SPIR-V, and 50 of 51 to Vulkan SPIR-V (one Shadertoy fixture overloads
  with mismatched precision). From that SPIR-V, HLSL and MSL for all 50.
  - Costs: two C++ dependencies, and about 40 ms per pass on top of the
    driver's own compile.
  - Fidelity and frame time cannot be measured until a renderer exists
    behind it.
- **So no second backend is started, and the seam is a document.**
  `docs/RENDERER_SEAM.md` lists, stage by stage, what a backend must
  provide and where GL leaks into the app today: `&Gl` in five modules,
  `ResetStatus`, `egui_glow`, and the GLSL dialect. It moves no code and
  adds no trait. An interface written before its second implementation
  is a guess, and a guess on the frame path costs every frame. G-04 stays
  deferred until a platform needs it.

**Would reopen:** macOS dropping OpenGL, a target with no GL or ANGLE, or
naga gaining combined samplers. Rerun the tool to check the last.

### D256: the wordmark is Space Grotesk set as outlines, and the lockups use the icon's ribbon (2026-10-13)

R-06 still needed the wordmark, the two lockups, a one-colour mark, the
README banner and the social preview (branding.md sections 2 and 6).

- **Type as outlines.** `cargo xtask brand` renders with no system fonts,
  on purpose, so the output never depends on the machine.
  `assets/brand/tools/typeset.py` sets the type into SVG paths with
  fontTools: STROBE in Space Grotesk Medium tracked at 0.14 em, and SHADER
  VISUALIZER at 22% of that size, tracked at 0.32 em, in Muted.
  - Space Grotesk is the guide's typeface and is SIL OFL 1.1. Outlines in a
    logo are allowed. The font file is not committed; the script's header
    says where to fetch it.
  - The script also writes the lockups, the mono mark, the banner and the
    social-preview overlay, so their geometry comes from one place: the
    wordmark's cap height is the unit, as the guide's clear-space rule asks.
- **The lockups use the production ribbon.** They take the application
  icon's heavy ribbon and two contours, without the tile. `mark.svg`'s
  fine contours are 2.3 px at 512 and vanish at the 180 px minimum width
  of a horizontal lockup.
- **The social preview is a real render.** The guide asks for real output,
  not a mockup. The background is `12-silicon-atlas` from `strobe-bench`,
  committed as `social-render.png`. `xtask brand` decodes it with
  tiny-skia and draws the SVG overlay on top, so resvg still needs no raster
  decoders and no new dependency.
- **Checked like the icons.** `banner.png`, both lockup PNGs and
  `social-preview.png` are in `xtask brand --check`, so CI fails if a
  source changes without its rasters.
- **The README header** is now the banner.

**Would reopen:** a designer's own wordmark, which would replace the
script's output and keep the same file names.

### D257: MIDI feedback undoes the mapping, and goes only to named devices (2026-10-13)

P-05 had OSC feedback (D209) but no MIDI output. Motorised faders, LED
rings and pad lights stay wrong after a preset, a cue or a randomize unless
Strobe tells the controller.

- **What is sent:** for every enabled mapping on a MIDI CC, pitch bend or
  note from a named device whose target is a control (directly or through
  a macro), the fader position that would put the control where it is.
  - `Cfg::unshape` runs the mapping backwards: range, then curve
    (`Curve::inverse`; Exp and Log invert each other), then invert, and a
    macro's reach on top. A tested round trip holds `shape` and `unshape`
    together for every combination.
  - **A CC** gets `0..=127` and **pitch bend** 14 bits, which Mackie-protocol
    motor faders use.
  - **A note** gets a light: velocity 127 from half way, else 0.
  - Bangs, commands, cues, fav slots, presets, actions and program changes
    have no position and send nothing.
- **Only to named devices.** A mapping on "any device" has no one place to
  go, and guessing would send fader positions to a synthesiser. Output
  ports are keyed exactly like input ports (`device_keys`), so a learned
  mapping's device names the controller's output as well.
- **Like OSC feedback:** changes only, at most 30 sends a second and 64
  messages a send, after a scene or mapping change everything again,
  through a thread with a bounded queue the frame only `try_send`s into.
  An echoed value moves nothing, because it equals the control's target.
- **Off by default,** behind "Send feedback" in MAP's MIDI block,
  saved with the MIDI settings.

**Would reopen:** a controller that wants its own feedback format, such as
SysEx LED colours. That would be a per-device profile.

### D258: HTTPS for the remote is the operator's own proxy, which Strobe lets in by name (2026-10-13)

L-07 was blocked on how a phone gets a secure context. REMOTE_CONTROL_PWA_RESEARCH.md
ruled out the easy wrong answers:

- a self-signed certificate means teaching people to click past a
  warning, which this project will not do;
- an HTTPS page elsewhere talking into the LAN runs into mixed content,
  Local Network Access and origin problems.

Enrolling a local certificate authority on every phone is neither simple
nor reversible enough for a VJ at a venue.

- **The supported path is the operator's own HTTPS reverse proxy.** A
  domain they own points at the LAN address. A proxy on the same computer
  (Caddy, with a DNS-01 plugin) holds a real certificate that phones
  already trust, and forwards to the listener.
- **Strobe changes only enough to let it in.** `--remote-proxy NAME[:PORT]`
  sets `ServerConfig::proxied`. A request whose `Host` is exactly that name
  is accepted only from a loopback peer, that is the proxy. The page then
  gets a `wss:` connect policy, and the socket opens only for the origin
  `https://NAME`.
  - Every other `Host` is still refused (D252), and without the option the
    name is refused too. A tested case covers all of this.
  - With no `--remote-bind` the listener then binds `127.0.0.1`, so the
    plain-HTTP port is not on the LAN at all.
  - `X-Forwarded-For` is not trusted, so the approval prompt shows the
    proxy's address, not the phone's.
- **Plain HTTP stays the default**, for trusted networks only, as
  USER_GUIDE.md already warned. There is still no manifest and no service
  worker: installability is claimed only after Android Chrome and iOS
  Safari pass on real devices through the proxy.

**Would reopen:** a certificate-enrolment flow that is secure, reversible
and tested on both phone platforms, which would let Strobe offer HTTPS
without a domain.

### D259: generated controls after 65 community scenes: script uniforms, maths traps, names from context, one Quality control, a fidelity gate (2026-10-13)

TORNADO T26 ran the engine (I-03) over the owner's 65 community scenes
(private, not in the repo). It kept 266 controls across 49 scenes and
found six problems. Each fix has an original fixture or unit test that
reproduces the pattern; no third-party code was copied.

- **Scripted scenes were refused** ("original does not compile"): the
  uniforms a script creates with `setUniform` were undeclared.
  `strobe_autocontrol::script` reads `script.js` and declares what it
  finds through `ScenePlayer::with_extras`, at zero, in the original and
  every rewrite alike. It is a **static approximation**, so the engine
  needs neither `strobe-script` nor QuickJS:
  - the type comes from the call's shape (2 to 4 numbers, or an array,
    object or `Float32Array` literal of that many, give `vec2`..`vec4`;
    anything else is `float`), and the first call for a name wins;
  - a name built at run time (`'spectrum_' + i`, a template string) is a
    prefix, and every GLSL identifier with it is declared;
  - `uniforms.name = ...` counts too;
  - a name the scene already declares is skipped, as the app does.

  The app still learns the real uniforms by running the script, so a
  scene the scan misreads plays correctly there; only its measurement is
  affected. Also, **a scene whose own GLSL does not compile is an error**
  even when it has no candidates: the original is now compiled before
  anything else, so two scenes no longer read "0 candidates".
- **Maths offered as controls.** Smoothstep's `3.0 - 2.0 * t`, the
  quintic's `6.0`, `15.0`, `10.0`, and hash arithmetic (`43758.5453`,
  `12.9898`, `78.233`, `.1031`, any non-integer of 1000 or more, anything
  inside a `fract(..)` that holds a `sin` or `cos`) are traps beside the
  existing ones (`analyze::context`). Named constants with a hash value are
  mathematics too. `grad` and `rand` as function-name words no longer make
  `gradient` opaque.
- **Names.** 116 of 266 were a stem and a number. A name now comes from
  context, in this order:
  - **Speed:** the constant, the assigned variable, the user function it
    is passed to, the helper it is in (`spokes_speed`, `rot_speed`).
  - **Scale:** the user function (`fbm(p * 3.0)` is `fbm_scale`), the
    assigned variable or field, the helper (`field_scale`).
  - **Colour:** the assigned variable or field (`mat.albedo` is
    `albedo_color`), the variable it is multiplied by (`glow_color`), the
    function, the helper.
  - **Loops:** `shadow_steps`, `ao_steps`, `march_steps` (a body calling
    `map`, `sdf`, `de`, `field` or `sd*`, also in `render_main_image`),
    `octaves` (a body calling a noise).

  Names of one or two letters and words like `col` or `res` are skipped.
  Material Maker's `p_oNNNN_` node prefix is dropped. When two kept
  controls share a name, a **role** tells them apart before a number: the
  ends of a `mix` are `low` and `high`, a constructor's arguments `x`..`w`,
  a coordinate's one-component swizzle its axis, two short variables their
  names (`color_a`, `color_b`). A plain name the scene already uses takes
  `_amount` (`rings_amount`) before a number.
- **Positions taken for colours.** A `vec3` is not a colour when it is
  assigned to a geometry name (word-based: `lk`, `lp`, `ro`, `light`,
  `light_dir`, `cam_pos`; `light_col` is still a colour), added to or
  subtracted from a position, passed to `map`, `sd*`, `op*`, `rotate`..., or
  anywhere in a distance function, or when two of its three components are
  zero. A `vec4` keeps the last rule off, since its alpha marks it RGBA.
  The engine has no signed vector control yet, so these are left alone
  rather than made shape candidates.
- **Too many Quality controls, and tiny ones.** A scene keeps **one**
  Quality control, the strongest (`Verdict::QualityLimit` for the rest).
  `FAR`, `MAX_DIST`, `tmax` and similar constants are Quality, ranged a
  quarter to twice so a slider never draws nothing. A candidate is
  **weak** (`Verdict::Weak`) under `min(0.05 x best, 0.01)`. That is
  `ScoreOptions::relative`, `--relative`, under `score::WEAK_CEILING`.
  The ceiling is T26's own figure: every tiny control it reported was
  under 0.01, while a 2% knob next to a 40% one is still worth a slider.
  A small moving dot can fall under it; that is the cost.
- **Fidelity gate.** Two scenes kept a rewrite whose default picture was
  0.020 and 0.0076 from the original. After the final rewrite the engine
  now measures it. Over `FIDELITY_LIMIT` (0.001), plus the original's own
  noise (two renders of it compared, for a scene that is not a pure
  function of time), it renders each kept control alone and drops the
  ones that change the default (`Verdict::Unfaithful`). If they are only
  unfaithful together, it drops all of them; it never writes an unfaithful
  scene. The scenes are not here, so the cause is not proven. These were
  found and fixed:
  - a `TIME * k` in a function with its own `TIME` parameter or local is
    no longer a speed (the integrator would drop that local's offset);
  - every probe now renders at a fixed `strobe_Date`, not the wall clock.

  The likeliest remaining cause on a real GPU is the compiler. A literal
  that becomes a uniform is no longer constant-folded, and an unrolled
  loop becomes a dynamic one, so rounding changes. A chaotic iteration, or
  a feedback buffer over ten frames, grows that difference. llvmpipe shows
  0 for every scene here, so only the gate can catch it, and it does.
- **Misses** (scenes with plain speed and colour literals that got
  nothing). The detectors could not see:
  - a speed in a `#define` body (`#define T (TIME * 0.3)`). It is now a
    candidate, unless it is unparenthesised and the macro is used after
    `/` or `%`, where the textual paste would read differently;
  - `TIME / 4.0`, now a speed of 0.25;
  - a colour in `#define COL vec3(..)` or a global `const vec3 COL =
    vec3(..);`, now a colour constant, unless it is multiplied by the
    clock (a set of frequencies) or named like geometry.

  Literals in helper functions and in multipass sources were already
  scanned; that was not the gap.

**Evidence:** unit tests in `analyze/context.rs`, `analyze/expr.rs`
(`fade_polynomials_and_hash_arithmetic_are_not_controls`,
`positions_directions_and_sizes_are_not_colours`,
`names_come_from_the_code_around_the_literal`,
`time_divided_by_a_number_is_a_speed_and_a_local_time_is_not`,
`loops_are_named_for_what_they_do`, `a_speed_inside_a_define_body_is_found`,
`gradient_is_not_an_opaque_function_name`), `analyze/mod.rs`, `naming.rs`
and `script.rs`, and `tests/engine.rs`
(`a_scripted_scene_compiles_with_the_uniforms_its_script_creates`,
`a_broken_scene_with_no_candidates_is_an_error_not_an_empty_report`,
`one_quality_control_at_most_and_nothing_weak_next_to_the_strongest`,
`a_control_that_changes_the_default_picture_is_dropped`,
`speeds_and_colours_in_defines_consts_and_helpers_get_controls`). Every
library scene with `CONTROLS` stripped, and every tunable fixture, still
gets controls in both dialects at fidelity 0, and the engine's own check
now holds every result to `FIDELITY_LIMIT`.

**Would reopen:** a re-run of T26 on TORNADO that still shows numbered
names, refused scripted scenes, or a scene the gate empties because its
GPU rounds every rewrite differently. That last one would need the
rewrite to keep literals as `const` where it can.

### D260: narrow windows give way in a fixed order, and two tests hold it at every size (2026-10-13)

**Context.** The T480 resized the control window at 150% (Task 35). Below
about 1024 pixels, which is 683 points, things collided:
- the toolbar's buttons overlapped;
- the title bar's readouts ran over the title;
- the Controls drawer grew past the preview, over the transport and
  BLACKOUT;
- a portrait window drew the clock and BPM over the transport buttons.

The headless shell tests only checked that nothing overflowed the window,
and only at 640x480 and up, in logical points. A drawer over BLACKOUT
passes that check.

**Decision.**
- **What gives way first.** Each strip drops its least needed part, never
  BLACKOUT, the transport buttons or the window controls:
  - The title bar drops the breadcrumb below 760 points and the wordmark
    below 420. Each readout drops its text, then itself, when it would run
    into the title. The LAN kill switch comes before every readout, so it
    keeps its room.
  - The toolbar shows icons, with the words in their tooltips, below 470
    points of preview width.
  - The transport drops BPM, then the clock, then NOW.
  - The preview's size caption makes way for the output-hidden chip, and
    that chip shortens with the preview.
- **A drawer never covers the transport or the toolbar.** It opens below
  the toolbar, is clipped to the preview, and its pane scrolls. The
  Inspector now scrolls as one below its header (tags, media and presets
  included). Before, only its control groups scrolled, so a short drawer
  ran out of room above them.
- **A docked pane cannot push into the preview.** In egui 0.36, a right
  panel whose content is wider than its `exact_size` moves its recorded
  left edge right by the overflow, and the central panel then draws over
  the pane. At 1920 the monitor picker lay over the pane's header in two
  states. Panes now draw in a child UI that cannot widen the panel. The
  rows that were too wide now wrap: the generated-controls review, and the
  cue card's transitions, which became a list.
- **The tests say what Task 35 saw:**
  - `nothing_is_painted_over_blackout_at_any_size`;
  - `no_text_is_drawn_over_other_text_at_any_size`, which compares rows
    of text and allows an overlap only under an opaque pane;
  - the overflow test, now also at the T480's logical sizes (683x427,
    533x400, 427x320 and 320x533).

  `STROBE_UI_SHOTS=<dir>` paints every state at every size to PNGs on a
  headless context, so a layout can be looked at without a window.

**Would reopen:** a person on the T480 finds a size or state where
something still collides that these tests miss. The fix is then to add
that size or state to the tests, not to special-case it.

### D261: GLSL ES sees through function-like macros, and a sampler `?:` becomes a `?:` of calls (2026-10-13)

D199 left two forms that desktop GLSL compiles and GLSL ES rejects (C-02).
Both are closed in `strobe_scene::promote`, tested with original fixtures
that failed on ES before and now pass in both dialects on llvmpipe:
- `render.rs` `function_like_macros_mixing_int_and_float_compile_in_both_dialects`;
- `render.rs` `a_sampler_chosen_with_a_ternary_compiles_in_both_dialects`;
- the rewriter's unit tests in `promote/tests.rs`.

**Function-like macros, on ES only.** `#define S(a, b, t) smoothstep(a, b, t)`
called as `S(0, 1., x)` hid the int from the D199 pass, which typed a macro
call as unknown. Now each call is typed through the expansion the
preprocessor would make (`promote::expand`), with every token remembering
whether it came from the body or from which use of which argument:
- **When every conversion lands in an argument and every use of a parameter
  agrees,** it is made at the call site: `S(0.0, 1., x)`, `ROT(1.0)`. The
  `#define` and the call's shape are unchanged.
- **Otherwise the one call is replaced by its expansion** with the
  conversions in it, on the same lines. This covers a body that mixes types
  (`#define sat(x) clamp(x, 0, 1)` called with a float) and a parameter
  used both as an index and as a factor. The body itself is never edited,
  because another call may need it as written (`sat(i)` with an int).
- **Not followed, so left as D199 left them:** a macro defined twice or
  `#undef`ed; a body with `#` or `##`; the wrong number of arguments; a body
  that is not an expression; an inlining that would let the preprocessor
  expand a self-referencing name it would have left alone. A budget of
  20,000 expansions per program keeps nested calls from costing exponential
  time.

Why not expand every macro before the pass? It would rewrite every call in
every ES scene, including the many that already compile, and error messages
would show expansions instead of the user's code. Typing through the
expansion edits only the calls that need it, and most of those keep their
text.

Desktop output does not change for a macro. Desktop GLSL converts on its
own, and the corpus passes there.

**A sampler in `?:`, on every dialect.** GLSL forbids an opaque type as an
operand of `?:`. NVIDIA accepts `texture(i == 0 ? a : b, uv)`; Mesa rejects
it on desktop as well as ES, and so does ANGLE. The call moves inside:
`(i == 0 ? texture(a, uv) : texture(b, uv))`. That is the same value on
every driver: only one branch runs, so the other arguments are still
evaluated once. Details:
- It applies to built-in and user functions alike, through parentheses
  and nested choices.
- Conversions already found in the call are kept in each copy.
- The replacement is one line, followed by the newlines the call spanned.
- It runs on desktop too, because Mesa (llvmpipe) rejects the
  original there, and the rewritten form is valid on every driver.

**Anything else is reported, not guessed.** Two choices in one call, a choice
that is not an argument, or (on desktop) one inside a function-like macro
is left as written. On ES, where it cannot compile, the assembler adds
`assemble::UNSUPPORTED_SELECT` as an `#error` at the user's line. The error
then reads `main.glsl:N: ... strobe: a sampler is chosen with ?: here ...`
instead of only the driver's wording
(`a_sampler_choice_that_cannot_be_rewritten_fails_with_strobes_message_on_es`).
Desktop gets no `#error`, since NVIDIA may still compile it.

Edits can now replace whole tokens (`promote::Edit::replace`) as well as
insert. A replacement never adds or removes a line.

**Would reopen:**
- a scene that compiled on ES before this and does not after;
- a desktop frame that changes for a scene without a sampler `?:`;
- an ANGLE run on the T480 or TORNADO, or the owner's corpus, that shows
  either rewrite rejected.
- a scene whose choice varies across pixels and whose rewritten `texture`
  calls then sample the wrong mip level. Implicit derivatives inside a
  non-uniform branch are undefined in GLSL; compilers flatten a `?:` of two
  samples, but none is required to. The fallback is to sample both and
  choose, which costs a second fetch.

### D262: the palette lists scenes and presets after exact commands, built once per change (2026-10-13)

D200 said an operator who expects scenes and presets in the palette would
need more entry kinds, not another design (U-01). This adds them.

- **What is listed.** Each library scene is two entries, `Load: <name>`
  and `Preview: <name>`, and each preset of the playing scene is one,
  `Apply preset: <name>`. They push the same actions as the browser's Load
  and Preview buttons and the preset row (`Action::Load`,
  `Action::Preview`, `Action::RecallPreset`), so the palette cannot mean
  something different from the button. Preview opens the browser too,
  because that is where the preview card is.
- **The playing scene stays listed,** with "playing now" as the reason it
  cannot be loaded or previewed, as unavailable commands are (D200).
- **Ranking.** A scene or preset is matched on its whole title and on its
  name alone, so `aurora` finds `Load: 01-aurora` as well as a title start
  would. Results come in four tiers:
  1. commands, panes and layouts whose title the query matches exactly
     (at the title's or a word's start, no gaps);
  2. scenes and presets;
  3. commands matched only loosely (letters in order with gaps, or
     mid-word);
  4. matches on a command's category.

  So `black` is still Blackout first, and `tmpo` is the scene
  `02-tempo-drift` before Tap tempo, which has those letters only with a
  gap. With nothing typed, every command comes first in table order.
- **Cheap while open.** The entries are kept in the palette's state
  (`command_palette::Listing`), built when it opens and again only when
  what they come from changes: what can run, the library's revision, the
  playing scene, its presets, the layouts. The comparison allocates
  nothing. The ranking is redone only when the query changes, and the
  match no longer builds a character vector per entry. Before this, the
  palette rebuilt and ranked every entry every frame while open; now a
  library of a thousand scenes costs nothing per frame for its entries.
- **Small windows.** The card is narrower than 520 points when the window
  is, shows fewer rows when it is short, and shortens a title rather than
  drawing it over the reason or the shortcut. The shell tests now draw the
  palette open in two states at every size.

**Evidence:** `command_palette` tests
(`scenes_and_presets_are_entries_found_by_their_names`,
`scenes_rank_after_exact_command_matches_and_before_loose_ones`,
`the_playing_scene_is_listed_but_cannot_be_loaded_again`,
`a_listing_is_built_and_ranked_only_when_something_changed`), and
`ui::shell_tests::the_palette_loads_previews_and_applies_presets_by_name`
with real key events, plus the scenarios "palette listing scenes and
presets" and "palette with nothing typed" in the overflow and overlap
tests.

**Would reopen:** a library large enough that a keystroke's ranking shows
(it is one pass over two entries a scene), or an operator wanting presets
of scenes that are not playing, which would need the presets of every
scene read in advance.

### D263: errors carry the user's column where Mesa gives one, and a reload checks scene.json first (2026-10-13)

U-02 left two things: the Editor showed lines only, and Save and reload
wrote a broken `scene.json` and then failed the scene on it.

- **A column only where the driver gives one.** Mesa writes `1:14(3)`,
  line 14 and column 3. NVIDIA's `1(14)` and the `ERROR: 1:14:` of ANGLE,
  AMD and Intel give a line only, so on the T480 (Intel or ANGLE) the
  Editor still goes to the line alone. Nothing is guessed.
- **The column is put back on the user's line, in `strobe-gfx`.** A
  driver's column does not count the user's text, for two reasons, both
  found on llvmpipe:
  - Mesa counts its preprocessor's output, in which every run of spaces,
    tabs and comments is one space, leading indentation included.
    `    return x;` has `return` at column 2.
    (`assemble::compiled_column`).
  - The assembler rewrites the user's code (`promote`, `fixups`,
    `directives`) keeping lines but not columns: `float x = 1;` compiles
    as `float x = 1.0;`. The text the two lines share at their start keeps
    its columns, the text they share at their end keeps its distance from
    the end, and a column inside the rewritten stretch goes to that
    stretch's start (`assemble::original_column`).

  Only the player has both texts (`assemble::source_line` finds the line
  through the `#line` directives in each), so `describe_log` writes
  `main.glsl:2:35: 1:2(34): error: ...`. Everything that reads `file:line`
  still does, and `editor::error_location` reads the column when it is
  there. Macro expansion moves Mesa's columns too, and is not undone.
- **Save and reload checks `scene.json` first.** The Editor's text is
  parsed by `SceneDef::parse` after the loader's newline handling
  (`editor::check_scene_json`), so it passes exactly when the loader's
  parse would. If it fails, nothing is written and nothing is reloaded:
  the scene on screen and its health are untouched. The Editor switches
  to `scene.json` with the cursor on the parser's line and column, and
  shows the message with a "Go to line, column" button. The refusal clears
  when `scene.json` is edited or reverted. A JSON syntax or type error has
  a place; a value the loader refuses (an unknown control `TYPE`) has a
  message only.
- **Plain Save still writes.** Save is how an operator keeps work in
  progress, and it does not touch the show. Only Save and reload reaches
  the show, so only it is guarded.

**Evidence:** `strobe-scene` `assemble` tests
(`only_mesas_format_gives_a_column`,
`a_source_line_is_found_through_line_directives`,
`mesas_column_counts_runs_of_blanks_as_one`,
`a_rewritten_lines_column_goes_back_to_the_users_text`), the headless
render test `a_compile_error_names_the_users_column_when_the_driver_gives_one`
(llvmpipe: a rewritten, indented line points at the user's column),
`editor.rs` tests (`a_column_is_read_when_the_message_has_one`,
`a_line_and_column_find_their_offset_within_the_line`,
`scene_json_is_checked_as_the_loader_reads_it`,
`a_doc_is_checked_by_its_scene_json_tab`), and the shell scenario
"editor refused a reload, scene.json does not parse" at every size.

**Would reopen:** a driver other than Mesa giving columns, whose
preprocessor may count differently; or wanting Save itself to refuse a
broken `scene.json`.

### D264: any library scene can be tagged by name, and Generate's exclusions say what they left out (2026-10-13)

D205 left three things: tags in the Import pane, tags for a scene that is
not playing, and exclude lists in the Generate form (U-08).

- **One row, one action, by name.** The Inspector's tag row moved to
  `ui::tags` and is drawn in three places: the Inspector (the playing
  scene), the Import pane (the scene it last imported, playing or not) and
  the browser (its selected scene, above the list). All three ask for
  `Action::SetSceneTags { scene, tags }`, the scene named by its library
  name, and the app writes that scene's `scene.json` with D205's in-place
  edit. Before, the action meant "the playing scene", so a field opened on
  one scene and saved after the show moved on would have tagged the next
  one.
- **A draft belongs to its scene.** Each place keeps its own draft
  (`TagDraft { scene, text }`), and a place showing another scene does not
  show the draft. Selecting another row or a cue change cannot carry
  half-typed tags to a different scene.
- **The same refusals.** A scene with unsaved Editor edits is still
  refused (D205), and a name no longer in the library (rescanned away) is
  refused with a notice rather than tagging whatever now has that index.
- **The browser and the Import pane name the scene** at the start of the
  row, on the chips' line. A label of its own above the chips overlapped
  their row in a short drawer (the overlap test found it at 533x400).
- **The import is found by folder once per change.** Comparing folders
  canonicalises paths, which asks the file system, so the app caches the
  imported scene's library index against the library revision and the
  folder, and the frame compares only those.
- **Exclude in the Generate form.** A new "Exclude" field takes words
  like the pool: tags (normalised like D205's) and whole scene names, read
  into the generator's `exclude_tags` and `exclude_names`, which
  `strobe_playlist` already checked first. Name words count as tags there
  too, as they do for the pool. An excluded scene is never picked, even
  when the pool names it.
- **Why picked, and why skipped.** The preview keeps its "Pool: tag dark
  5" line and adds "Skipped: excluded by tag heavy 2", with each scene and
  its reason on hover (`show::generate::excluded`). Scenes the pool simply
  did not ask for are not listed: that would be most of a large library.
  When the exclusions empty the pool, the error says so ("the exclude
  field left out 3 (excluded by tag calm 3)") instead of listing the
  library's tags.

**Evidence:** `ui::tags` tests
(`a_draft_shows_only_for_the_scene_it_was_opened_for`,
`save_tags_the_scene_the_draft_is_for`, which clicks Save headless),
`show::generate` test `excluded_tags_and_names_are_never_picked_and_say_why`,
and the shell scenarios "tagging in the browser and the inspector",
"browser with a scene selected", "tagging the scene just imported" and
"generate with an exclusion, previewed" at every size.

**Would reopen:** wanting several scenes tagged at once (a multi-select in
the browser), or exclusions by folder.

### D265: the watched folder is a setting, checked the same way at launch and in Settings (2026-10-13)

D240 made the watched drop folder a run option, `--watch-folder`, and
named a Settings field so the folder persists as the thing that would
reopen it (I-02).

- **Settings > Watched folder:** a path field, Apply, and a switch, with
  one line saying what is watched or why not. Apply (or Enter) watches the
  typed folder at once; the switch starts or stops the watch. Both go
  through `Action::SetWatchFolder { folder, enabled }`. The pane never
  touches the disk; the app checks the folder when the action arrives.
- **One check, at launch and in Settings.** `intake::check_watch_folder`
  refuses an empty field, a path that is not a folder (D240's only check
  until now), and the library folder itself, however it is spelled. The
  library's own new scenes (an import, a pack) would otherwise come back
  through the queue as "already there" after every import. `--watch-folder`
  now gets the same check, so a folder Settings refuses is refused at
  launch too.
- **Applied live.** An accepted folder replaces the running watch with a
  fresh `Inbox`. Like D240's, it leaves alone what is already in the
  folder. Switching off drops the watch and keeps the folder in the field
  and in the file, so switching on again needs no typing.
- **A refusal changes nothing but the line.** A typo in the field never
  stops a watch that works, and nothing refused is saved.
- **Saved with the other UI preferences.** The folder and the switch go
  in `ui.toml` (`[watch] folder, enabled`), through the debounced saver,
  only when the app accepts them. Typing in the field saves nothing. A file
  from before has no `[watch]` and watches nothing.
- **`--watch-folder` wins for its run and is not saved.** Settings shows it
  as "Watching ... (--watch-folder, this run only)", so the command line
  never rewrites the operator's setting. A saved folder that has gone
  missing is not watched at launch: the reason is logged and shown in
  Settings, and the setting stays as it was, so a drive plugged in later
  can be switched on again.

**Evidence:** `intake` test `a_watched_folder_must_exist_and_not_be_the_library`,
`app` tests `settings_starts_stops_and_refuses_a_watched_folder` and
`launch_watches_the_command_line_folder_first_then_the_saved_one`,
`ui::state` test `the_watched_folder_is_saved_and_reread_but_its_field_is_not`,
the `prefs` round trip and missing-field tests, `ui::settings` test
`apply_and_the_switch_ask_the_app_and_change_nothing_themselves` (headless
clicks), and the shell scenarios "settings watching a folder" and
"settings with a refused watched folder" at every size.

**Would reopen:** wanting more than one watched folder, or Shadertoy
exports taken from it (D240's own reopen condition).

### D266: an audio stream error is a failure only when the stream stops, and reopening backs off (2026-10-13)

**Context.** On the AMD desktop (Task A1) the microphone reported WASAPI
buffer glitches (`ErrorKind::Xrun`). The capture stream's error callback
marked the stream failed on any error, and the worker reopened at once:
11,200 reopens and 33,600 log lines in 158 s, and live audio was
unusable. cpal 0.18 says three error kinds leave the stream running:
- a glitch (`Xrun`);
- a reroute of the default device that it already followed
  (`DeviceChanged`);
- a refused real-time promotion (`RealtimeDenied`).

**Decision.**
- **Those three are counted,** in `Opened::glitches`. The worker logs the
  count once a minute at info, as "N buffer glitch(es) in the last
  minute, stream kept".
- **Every other kind marks the stream failed,** including kinds cpal adds
  later (`capture::device::is_fatal`).
- **Repeated quick failures back off.** A stream that fails within 5 s of
  opening counts as a quick failure. The first reopen is immediate, as
  before; after that it waits 100 ms, doubling up to the 2 s retry period
  (`worker::reopen_delay`).
  - Only the first failure of a run is a warning; the rest are debug.
  - A stream that ran 5 s before failing resets the count.
- Tests: `a_glitch_a_followed_reroute_and_a_refused_priority_are_not_failures`
  and `a_first_failure_reopens_at_once_and_repeats_back_off_to_the_retry_period`.

**Would reopen:** a backend that reports a fatal condition as one of the
three surviving kinds. The symptom would be silent levels with a "stream
kept" line. Task A2 re-runs the app on the AMD desktop to confirm the
loop is gone and the levels move.

### D267: a result thrown away outside tests says why, and the ones that could not were fixed (2026-10-13)

**Context.** A reader of `strobe-gfx/src/assets.rs` asked why its error
paths end in `let _ = sender.send(...)`. Those discards are sound: the
error is delivered as `MediaMessage::Failed`, and only a send to a
receiver that is already gone (the scene was replaced) is dropped. But the
pattern was everywhere, and an audit found 145 places outside tests that
discarded a value, with nothing to tell a deliberate discard from an
accident.

**Fixed, because the discard lost something:**
- **Shadertoy re-import.** If moving the new scene in failed, the old
  scene's restore was ignored, and could leave it under a hidden
  `.<name>.replaced` name. It now returns `WriteError::NotRestored`,
  naming where the old scene is.
- **LAN remote client list.** A damaged list was "moved to ….bad" even
  when the move failed. The message now says what really happened.
- **Token rotation.** A failed rotation (no OS random source) is now
  logged.
- **Generated controls.** A `scene.json` that exists but cannot be read
  was treated as missing and replaced, losing its other fields. It is now
  an error.
- **Editor.** Re-reading the open scene after Strobe wrote a thumbnail or
  tags blanked the Editor silently on failure. It now keeps the previous
  text and says why (`reopen_editor`).
- **Connection timeouts.** The LAN remote's and the Shadertoy listener's
  connection threads served a socket whose timeouts could not be set,
  where a silent peer could hold the thread forever. Such a connection is
  now dropped with a warning.
- **Crash-monitor threads.** A crash-monitor watchdog or a system-facts
  probe that could not start now says so.
- **GL error clearing.** `let _ = gl.error()` cleared one GL error flag
  where a driver may hold several. `Gl::clear_errors` clears them all,
  bounded.
- **Dead suppressions.** `let _ = x;` hid an unused parameter in five
  places and an unused import in two; they are removed.
  - One hid a contract that was not kept: `controls::rail` documented that
    `id_salt` identifies the control, but its id came from its position,
    so a drag could jump to a neighbour when rows above changed. It now
    uses `id_salt`.

**The rule (`strobe_conventions::discarded`).** Outside tests, a
statement that throws a result away (`let _ = …` or a trailing `….ok();`
that is not assigned) carries a `//` comment, on the line or directly
above, saying why there is nothing to do. `write!` and `writeln!` are
exempt: they cannot fail into a `String`, and to a stream they are
best-effort diagnostics. The 80 deliberate discards now each say why:
- a receiver that has already gone;
- best-effort cleanup;
- a crash handler that must not fail;
- a courtesy reply on a connection that ends anyway;
- a join on a thread whose panic already aborted the process.

**Would reopen:** a discard whose stated reason turns out false. The fix
is to handle that error, not to reword the comment.

### D268: Synesthesia's 90 default scenes all compile: two library names inferred, and a script's state is not a uniform (2026-10-13)

**Context.** The owner's private corpus of Synesthesia 1.25.4's default
scenes was re-run at 776ddf3: 88 of 90 in both dialects on llvmpipe, and
the same on the AMD RX 580 (Task A4).
- `SkaiWalkerOG` includes `hg_sdf.glsl` from Synesthesia's `lib` folder.
  D198 already resolves that where Synesthesia is installed or
  `STROBE_GLSL_PATH` names the folder. Past the include it read
  `_Media_Contrast`, which Strobe did not define.
- `hue_review` calls `_hueSaturationContrastLEGACY` (Q26), which was
  missing.

**Decision.**
- **`_Media_Contrast` is a constant 1.0.** It is the media panel's
  contrast, neutral like the `_applyMediaContrast` identity Strobe
  already has (D197).
- **`_hueSaturationContrast` and `…LEGACY` are inferred.** Hue is in
  degrees, saturation and contrast are multipliers, and `(0, 1, 1)` leaves
  a colour alone. This is an inference, not Synesthesia's code (Q26 stays
  open for the look).
- **A script's own state in the legacy `uniforms` table is skipped
  quietly.** That means a function, or an object with no `x`/`r` keys and
  no `length` (`host::is_script_state`). `thresholder` keeps a counter
  object there, and each load reported "expected at least one number". A
  string there is still reported, as a likely mistake.

**Result.** 90 of 90 in both dialects on llvmpipe, with the library path
set. The scenes' own undefined behaviour (`PLUS` takes `pow` of a negative
base) is left alone, as in COMPATIBILITY.md.

**Would reopen:** the look of `hue_review` or SkaiWalkerOG's media beside
Synesthesia's own render.

### D269: `syn_Spectrum` is 1024 texels wide, Synesthesia's width (2026-10-13)

Q10 asked how wide real scenes expect `syn_Spectrum` to be. Synesthesia
1.25.4's own `audio_indicators` (credited to Synesthesia, in the owner's
private corpus) reads the raw channel with
`texelFetch(syn_Spectrum, int(x*1024), 0)`. At Strobe's 256 the top three
quarters of that row fetched past the end of the texture, which GL leaves
undefined. Scenes that sample with normalised coordinates (`texture`,
`textureLod`) address the same frequencies at any width, so for them only
the detail changes.

- **`Layout::default().spectrum_len` is 1024.** The analyser's FFT is 2048
  points, so its 1024 bins below Nyquist map one to one onto the raw
  channel's texels: each raw texel is one bin, not the peak of four. The
  match with the FFT size suggests this is how Synesthesia gets its 1024 too.
- **The player's silent `syn_Spectrum` is 1024 too,** so a `texelFetch` by
  that count stays in range before the first upload, and the first upload
  reallocates nothing. The silent trail stays 256, the analyser's trail.
- **Autocontrol scores against a 1024-wide synthetic spectrum,** like the
  live one.
- **The trail is unchanged.** `audio_indicators` samples it with
  normalised coordinates only, which says nothing about its length.

What changes for a scene: the raw channel is spikier and a little lower on
average (one bin per texel instead of the loudest of four). The juiced
channel's 3-texel cross-smoothing now covers a quarter of the frequency
span it did, so the curve shows more detail; its time smoothing is
unchanged. The smooth channel's blur radius scales with the width
(`len / 40`), so it looks the same. The waveform shows one sample per
texel of its 1024-sample window instead of the mean of four, so it keeps
more high-frequency detail.

Cost: the upload is RGBA8, 4 KB a frame instead of 1 KB, with the same
single `glTexSubImage` and no allocation after the first frame. The
analyser does four times the per-texel work for each 512-sample hop. Its
box blur is O(width x radius), about 100k adds per hop at 1024, roughly
10 M a second at 48 kHz: well under one percent of a core. A 1D texture
of 1024 is far inside every GL 3.3 and GLES 3.0 size limit.

**Evidence:** `behaviour.rs` `texture_shapes_and_ranges` (a 1 kHz tone
peaks at the raw and juiced texels a 1024-wide axis predicts); the corpus
call quoted above.

**Would reopen:** Synesthesia drawing `audio_indicators` differently from
Strobe beside a sweep tone (Q10's remaining axis questions), or a profile
on an HD 4000 showing the analyser's blur matters, in which case it becomes
a running sum.

### D270: v0.1.0 is published as `latest`, with the unverified work named in the release body (2026-10-14)

The first tag, and the first time `release.yml` ran on one. Several
validations are still open (a MIDI controller, the live Shadertoy site, a
real phone, an HD 4000, a four-hour show of real scenes). The choice was
between holding the tag until those close and publishing now with each one
written down.

- **Publish now, as `latest`, not as a pre-release.** The release body
  carries a "Known limitations in v0.1.0" section naming every open item
  with its backlog id, so a tester meets them before running it.
  `release.yml` ends in `--draft=false --latest`; leaving that alone means
  the only release is also the recommended one, which is honest for a first
  tag and needs no workflow edit that would have to happen before the tag
  exists.
- **No version bump.** `[workspace.package] version` was already `0.1.0`
  and no tag carried it, so the release branch changed `CHANGELOG.md` and
  `README.md` only.
- **The changelog section was written before the tag** (`cargo xtask
  changelog --release v0.1.0`), as CONTRIBUTING requires, and every
  non-changelog commit landed before it. The check recomputes a section from
  the tagged range, so a commit landing after generation would show as
  drift: the README's status line was committed first, then the generated
  section on its own.
- **The workflow was proven on a throwaway public mirror first** (Q12), then
  its release and tag were deleted. It could not be a fork: GitHub refuses
  to fork your own repository. It also needed a change to the workflow file
  to register at all on the fresh repository — a push alone left
  `release.yml` unregistered while `ci.yml` appeared, which is worth knowing
  before anyone points the workflow at a new repository.

**Evidence:** run 38495718734 (draft, Windows build, publish, all green);
the dry run 38494313118 on `lnorton89/strobe-release-dryrun`, whose release
and tag were then removed. All six assets were downloaded from the release
page, every `.sha256` verified, `gh attestation verify` returned 0 for build
provenance and for the SBOM predicate, and the zip held exactly
`strobe.exe`, the three ANGLE DLLs, `LICENSES.chromium.html`, `README.md`,
`LICENSE` and the twelve scenes.

**Would reopen:** a later tag wanting the pre-release flag, which is a
`release.yml` change made before that tag and never after.

### D271: Synesthesia's installed scenes are in the library by default, and its install is never written (2026-10-14)

**Context.** A person who owns Synesthesia has its scenes on disk already,
and D268 made all 90 of them compile. Pointing `--library` at
`Program Files` was the only way to play them, which loses the person's
own library and lets Strobe try to write tags and presets where a normal
user cannot. A survey of a real 1.25.4.189 install (and the AMD desktop)
found where the scenes are: the ones it ships in `<install>\release` (87)
and `<install>\default` (the welcome scene), its GLSL libraries in
`<install>\lib`, and the person's own scene folders only where they added
them in its Settings (Custom Scene Folder), listed in
`%LOCALAPPDATA%\Synesthesia\Data\vs.json` (`sceneFolders`,
`primarySceneFolder`) and `preferences.json` (`PRIMARY_SCENE_FOLDER`,
`RECURSIVE_SCENE_SEARCH`, `RECURSIVE_SCENE_SEARCH_DEPTH`, 10). There is no
default user folder: `Documents\Synesthesia` does not exist.

**Decision.**
- **One detection, `strobe_scene::synesthesia`.** The install is
  `STROBE_SYNESTHESIA_DIR` when set (any platform; a value that is not a
  folder means none), else `Synesthesia` under `%ProgramFiles(x86)%` then
  `%ProgramFiles%` on Windows. The data folder is `STROBE_SYNESTHESIA_DATA`,
  else `%LOCALAPPDATA%\Synesthesia\Data`. Its scene folders are the
  `BUILT_IN` table (`release`, `default`; add a row if a version adds one)
  and the folders its settings name, searched recursively as deep as its
  preferences say (`load::scan_deep`; 10 when a key is missing, children
  only when recursion is off, links not followed, 20,000 folders at most).
  The settings files are read leniently: missing, unreadable or malformed
  is "no folders of the person's", logged once, never an error. D198's
  `include::search_path` asks the same detection for `lib` and the `glsl`
  data folder instead of its own hard-coded path.
- **The library has sources.** The library folder comes first and is
  scanned as before. Then, where installed, the scenes Synesthesia ships
  (`Source::Synesthesia`, read-only), then those in the person's
  Synesthesia folders (`Source::SynesthesiaFolder`, writable: they are
  theirs). A folder is read-only exactly when it is inside the install. A
  scene already listed is not listed again, and a Synesthesia folder that
  is the library folder itself is skipped. A name in two places is two
  entries, told apart by source; every lookup by name (`--scene`, the
  reopened last scene, playlists, fav slots, tags by name) finds the
  library folder's own first because it comes first.
- **Shown subtly.** A browser row ends its status line with a faint
  "Synesthesia" (dropped when the row is too narrow for it), the count under
  the filters says how many there are, the search word `synesthesia` finds
  them all, and the palette's category reads "Synesthesia". Rows are keyed
  by name and folder, so two of one name are two widgets. No new filter
  chip: the filter row's width is held by the overflow tests at every size.
- **On by default, one switch.** Settings > "Synesthesia's scenes" >
  "Show Synesthesia's scenes", saved in `ui.toml` as `[synesthesia] show`
  (a file from before is on), applied at once through
  `Action::SetShowSynesthesia`, which looks for the install again when
  switched on. `--no-synesthesia` hides them for one run and is not saved;
  Settings says so. Off, or not installed, the library is exactly what it
  was (a test holds `scan_with` off equal to `scan`). A rescan, an import
  and the watched folder rescan both parts.
- **Never a raw OS error.** Every write Strobe makes into a scene folder
  asks `Synesthesia::owns` (inside the install, case-insensitive on
  Windows, listed or not) first: tags (`SetSceneTags`), Use frame as
  picture (`CaptureThumbnail`), Editor Save and Save and reload, and
  saving, deleting and renaming presets. A refusal is a warning notice,
  "<name> is Synesthesia's own and read-only. Copy it to your library to
  <what>.", whose button is Copy to my library. The UI also avoids
  offering them: a read-only scene's tag row shows its tags and Copy to my
  library instead of Edit and Use frame as picture, and the Editor says
  "Read-only" with the same button first in its row (Save stays, and is
  refused, so edits made before noticing are kept). Generate controls
  already wrote only into the library (D202), and still does. Drops,
  packs, the watched folder and Shadertoy imports only write into the
  library.
- **Copy to my library** (`Action::CopyToLibrary(dir)`,
  `intake::copy_as_own`): the folder is copied under its own name or
  `<name>-2`, `-3`... (`x` and `x.synScene` count as one name), with
  D218's bounds and link refusal, into a hidden `.<name>.copying` folder
  renamed into place, as the importer stages. Then the library is
  rescanned and the copy selected. A scene playing by hand (no playlist)
  carries on from the copy without a reload, since its files are the
  same, so what Strobe writes for it next lands in the copy; an Editor
  open on it moves to the copy with its unsaved edits.

**Evidence.** `synesthesia.rs` tests (the override on any platform, an
override that is not a folder, Program Files order, the person's folders
from fake `vs.json` and `preferences.json` with depth, broken or missing
files, a folder inside the install, the data folder);
`load.rs` `a_deep_scan_finds_scenes_to_its_depth_and_not_inside_scenes`;
`include.rs` `the_include_path_finds_synesthesia_by_the_libraries_detection`;
`library.rs` `a_fake_install_is_listed_after_the_library_read_only`,
`switching_synesthesia_off_lists_only_the_library`,
`a_folder_of_synesthesias_that_cannot_be_read_lists_nothing_and_fails_nothing`,
`synesthesias_settings_naming_the_library_folder_list_it_once`; `app.rs`
`a_write_into_synesthesias_folder_is_refused_and_offers_a_copy`,
`copy_to_my_library_makes_a_copy_strobe_may_write`; `intake.rs`
`copy_to_my_library_takes_a_free_name_and_leaves_no_staging`; `ui::tags`
`a_synesthesia_scene_shows_its_tags_and_offers_a_copy_instead_of_edit`;
`ui::settings` `the_synesthesia_switch_asks_the_app_and_changes_nothing_itself`;
`ui::state`, `prefs`, `cli`, `command_palette` and `show::resolve` tests;
five shell scenarios (a library with Synesthesia's scenes selected, the
same compact, playing one with a write refused, the Editor on one, the
palette listing them) at every size, through the overflow, BLACKOUT and
text-overlap tests. By hand on Xvfb: a fake install whose `release` is the
private corpus listed 90 scenes after the library's 12, and `--scene
SkaiWalkerOG` played it with `hg_sdf.glsl` found in the install's `lib`.
Fake installs only: no real Windows install has run it yet (I-05).

**Would reopen:** the EULA (Q7) saying Synesthesia's scenes may not be run
elsewhere, which would turn the default off; a Synesthesia version that
moves its scenes or settings; or a person wanting to write into a
Synesthesia install they own, which Strobe would still not do by default.
### D272: Chromatic Rise interprets a video with procedural marbling (2026-10-13)

The owner requested a shader matching their TouchDesigner scene at
https://youtu.be/w0JcYtmRr0U. Chrome playback showed rainbow marbling,
folded fine striations, dark contours and pearlescent surface patches.
The network and its parameters were not supplied, so this is an original
visual interpretation rather than an exact port. No frames or audio from
the reference are bundled. Five nested fractal-noise evaluations create
the surface; screen derivatives provide relief without additional noise
samples, buffers, media or scripts. Low octave counts keep broad flowing
islands readable and avoid the overly dense first draft. Eleven controls
expose shape, motion, color and surface, with optional modest bass response.

Evidence: native GL 3.3 and ANGLE GLES 3.0 rendered the scene at 1280x720
on the RX 580; BENCHMARKS.md records timing and the first-draft delta.
Revisit fidelity with the owner's feedback or the original network.

### D275: Desktop controllers operate the host UI through a scoped raster stream (2026-10-13)

The owner requested the entire desktop UI, including scene editing and file
management, rather than a larger performance-command list. Mirror the host's
actual control-window framebuffer and translate normalized pointer, key,
committed text and clipboard input into its ordinary egui pass. This preserves
the existing editor, settings, file protections and action paths without a
second UI model or arbitrary filesystem/script RPC. Paths and saves refer to
the host. Operating-system windows and local file uploads are outside this
Strobe-window surface.

Full access is separately approved: `pair_desktop` requires the host's
runtime-only switch and labels the pending approval **Full desktop**. The
grant is held alongside that client's token digest, in memory only (see the
review hardening below); records without it default to performance-only access. Ordinary `pair` clients never gain
desktop permission when the switch is enabled, including after token rotation.
One authenticated connection leases input. Disabling, stop, revoke, app loss,
disconnect and input overflow discard pending input and request releases for
held keys/buttons. Old phone wire messages remain byte-for-byte compatible.

Compression and decoding live on app-owned workers; socket I/O stays on remote
workers. Capture is due only for an active lease, at most every 200 ms, with
one pending RGBA frame and one shared latest JPEG. The explicit capture is an
asynchronous, fenced GPU readback after control-window paint; ordinary/off frames neither
capture nor allocate desktop input buffers. JPEG is bounded at 2 MiB, dimensions
at 4096 per edge, input batches at 32 and 4096 JSON bytes, host input queue at
128 events, and text clipboard at 64 KiB. Slow peers retain write timeouts;
stale images disable controller input. This prioritizes full UI coverage and
old-hardware limits over video-rate remote preview. Enabled streaming overhead
still needs the T480 soak; the renderer bench cannot measure the app UI path.

Native credentials are bound to the explicit private IP:port and held only in
memory. Reconnect is explicit, uses ordinary rotating authentication, and
never retries old clicks. Copy/Cut returns only text a host widget explicitly
emitted, without reading the host OS clipboard. Tab remains routed to the host;
Ctrl/Command+Shift+F12 and the local Return button disconnect the viewer.
Images are shared as `Arc<ColorImage>` and recreated for a new egui context,
so graphics recovery cannot leave the viewer referring to an old texture ID.

Evidence: desktop bridge/client/unit tests, real-socket server tests, native
controller-to-host-widget editing and atomic save/backup test, Windows RX 580
native GL visual fixtures (`target/desktop-remote-qa/`). No real two-machine
LAN, minimized-window, ANGLE-streaming or older-GPU soak claim. Revisit a
structured render/input protocol or video codec only with measured latency,
bandwidth and hardware evidence. Benchmark before/after is in BENCHMARKS.md.

**Review hardening (2026-10-14).** A security review of the first cut
changed six things:

- **A desktop grant lives only as long as its switch.** Desktop records are
  never written to `remote-clients.json`, a file record claiming one is
  dropped on read, and switching full desktop access off or stopping the
  remote revokes every desktop pairing and closes its connection
  (`Clients::revoke_desktop`, `Server::revoke_desktop`). The first cut kept
  the grant on disk, so a token sniffed from the plain-HTTP link (D252)
  became full UI control again the next time the operator enabled the
  switch, even after restarts. The native controller holds its token in
  memory only, so nothing usable was lost.
- **A remote cannot widen who reaches the host.** A UI pass that carried
  remote input drops the actions that pair or approve a device, start a
  listener, switch desktop access on, rebind OSC (saved, unauthenticated and
  possibly on every interface) or make the host a controller of a third
  machine (`desktop::host_only`), with a banner saying to do it at the host.
  Narrowing actions (Stop, Deny, Revoke, switching access off) stay allowed.
  Everything else in the window stays reachable, as the owner asked.
- **Capture never stalls the show.** The control window is read into a
  pixel-pack buffer behind a fence and collected on a later control frame
  once the fence has signalled, the pattern of `strobe_gfx::readback`. The
  first cut used `glReadPixels` into client memory, which on the shared
  context waits for the output's queued frame too, five times a second.
  The pack buffer and pixel buffers are reused, not allocated per capture.
  A minimised window (size 0) is skipped rather than ending the session.
- **Clipboard text goes only where it was asked for.** The host forwards a
  copy only from a pass that carried the controller's input, so what the
  operator copies at the host stays there; the controller takes host text
  only within 3 s of sending input, so a host cannot write into the
  controller's clipboard unasked.
- **Leaving never blocks the controller's frame loop.** `Controller::disconnect`
  joins the worker only if it has finished. A worker still connecting has no
  socket to interrupt, and joining it held the UI thread for up to 8 s.
- **Operator visibility.** The title bar reads *Desktop controlled remotely*,
  as a warning, while a lease is held; the approval banner reads
  *Full desktop: … wants full control, files included*.

**Known limitations, not fixed here:**

- **Plain HTTP.** Frames (everything the window shows, including pairing
  codes and the import listener's token in Settings), input and the desktop
  token cross the LAN in clear. Anyone on the network can watch the session
  and, while the switch is on and the lease free, replay a sniffed desktop
  token. The switch's lifetime now bounds that window; TLS would close it.
- **No proxy or TLS for the native client.** `client::Connection` speaks only
  `ws://` to a private IP and does not verify which host answered. With
  `--remote-proxy` (D258) the plain listener is loopback-only, so desktop
  control from another machine is not available in that mode.
- **Full UI means full UI.** A controller can edit and save scenes, type any
  host path into Import, the watched folder or MEDIA, and Quit. That is the
  scope the operator approves.
- **Covered or minimised host windows.** Capture runs when the control
  window is redrawn. Whether Windows delivers `RedrawRequested`, and whether
  the back buffer holds the window's pixels, while a fullscreen output
  covers it is unverified on real hardware; the controller then shows its
  image as paused and refuses input.
- **A slow controller holds its session thread.** A frame write makes
  progress byte by byte under the 2 s write timeout, so a peer that drains
  slowly delays that session's notice of a revoke (its input is already
  refused, and the lease released, by the bridge).
### D276: Chromatic Rise rebuilt as a feedback-advected, posterised marbling (2026-10-14)

**Context.** The D272 draft was smooth, uniform-scale lava-lamp marbling with a
glossy sheen. The owner's reference frame (URBAN ASSAULT - RISE) is
a TouchDesigner feedback look: fibrous, combed striations, clusters of small
ringed bubbles, large islands full of fine texture, harsh posterised neon and
a gritty edge-detected relief. No amount of retuning a stateless noise stack
gets there, because the look comes from history.

**Decision.** Two passes.
- **`field`**, a double-buffered RGBA16F pass at `$WIDTH` x `$HEIGHT`, so the
  render scale still applies. r is a height, g a tint, a marks "written". Each
  frame it samples the previous frame upstream along a curl-noise flow
  (three 2-octave noise reads plus a slow turning drag), smears three taps
  along the flow so streamlines become fibres, and sharpens across it
  (clamped). It then injects fine isotropic noise (the smear turns it into
  hair, which beats deriving a flow-aligned frame: that moired and an
  axis-aligned one drew rectangles), a weak broad height, a slow tint, and
  small bubbles (one disc per grid cell, born once per cycle by frame count).
  The tint also sets texture roughness and smear length, so each island has
  its own colour and its own texture. Height is folded with `_mirror`, not
  clamped, so detail survives at the extremes. A target that reads `a == 0`
  (first frame, or a resize) is re-seeded, which keeps adaptive resolution
  safe.
- **The final pass** takes five taps for an emboss and an edge, posterises
  `tint * 4 + height * bands`, picks hues by golden-ratio steps per island
  plus 70 degree steps per height band (hard jumps, never a gradient), and
  inks the step lines and strong edges. A little animated grit is added.
- **Audio.** Bass lengthens the drag and smear, a bass hit adds a kick;
  `syn_BassTime` jumps the colour phase; highs thicken the striations and the
  grit. At reactivity 0 or in silence it still flows (checked by rendering with
  the audio uniforms forced to 0).
- **Twelve controls** (flow, flow_scale, speed, reactivity, smear, fibers,
  bubbles, bands, hue, color_speed, relief, outline).

**Costs and limits.** Advection is per frame, as in 06-feedback-trails, so the
motion speed follows the frame rate: no delta-time uniform exists. Cost is
about 1.35x 05-domain-warp on llvmpipe at 1280x720 (BENCHMARKS.md), against
the 2x budget. The sharpen term is clamped because an unclamped one
grew row-aligned instabilities where the flow was exactly horizontal.

**Would reopen:** the owner finding the swirl too vortex-like next to the
reference's long drags (lower `flow_scale`, or add a shear term), or HD 4000
timing showing the full-resolution field is too dear (half resolution is the
first lever).

**Revised 2026-10-15 (Opus review before merge).**
- **The row artifact, found.** Where the flow ran along an axis, every
  cross-flow tap landed on a texel centre, so nothing filtered across the
  flow and the unsharp (gain 1.5 at the pixel period) grew one-pixel rows
  (horizontal flow) or columns (vertical flow) until the clamp held them.
  The centre is now two taps half a texel either side across the flow (a
  [1 2 1] blur) and the unsharp is 0.4, which puts every cross-flow
  frequency below gain 1 (0.9 at the pixel period, 0.95 at two pixels): it
  sharpens what advection blurs and grows nothing. The tap count is the same,
  since the seed check now reads the advected sample's alpha.
- **Long drags.** The flow is lanes of shear along a slowly turning
  direction plus the curl swirl, both divergence-free; a `drag` control
  (default 1) trades swirl for drag, and `flow_scale` defaults to 1.8. The
  swirl potential is one noise octave: two cost 15% of the scene and the
  drags now give the fine direction.
- **Colour.** A ten-slot acid palette (four greens and lemon, then cyan,
  cobalt, magenta, sky, violet) replaces the HSV hue steps, and the steps
  inside an island jump three slots, so every island mixes green and lemon
  with cool accents, as the reference does.
- **Bubbles** cluster under a slow noise mask, 20 cells per screen height,
  each born every 0.4 s by the clock instead of every 32 frames.
- **A one-texel clock**, as in 14-16, integrates `speed` and `color_speed`
  (a slider moved an hour into a set no longer jumps the pattern) and gives
  the frame delta, which now scales the advection: the drag covers the same
  distance per second at any frame rate. The smear and sharpen remain per
  frame.
- Cost is unchanged at about 1.35x 05-domain-warp on llvmpipe (1.37x at
  1280x720, 1.31x at 1920x1080; BENCHMARKS.md): the one-octave swirl pays
  for the clock and the drags. Stable at 60, 180 and 300 s.

### D277: Electric Bloom is an original procedural garden, rebuilt from a Codex draft (2026-10-15)

The owner asked for a scene from a floral image that is not available to us.
The Codex draft (recorded as D273 on the owner's machine, never on master) was a flat grid of sticker-like rainbow daisies, and
the owner said it needed improving massively. This is an original procedural
interpretation worked from the draft's written description (vivid layered
flowers, inked veins, black negative space), not a reproduction of the image.

- Three depth layers (far, middle, near), each with its own rotation, cell
  size, drift speed and brightness, so there is parallax and no grid; cells
  are jittered and warped, and each layer visits only the four nearest cells.
- Each flower has three petal rings whose outlines are approximate signed
  distances, so the ink rim, the veins and the antialiasing hold at any size;
  a beaded pollen centre; and a halo plus a hot centre glow.
- Petals unfurl on a slow per-flower cycle. Bass opens them further, mids stir
  the sway speed, highs sparkle the pollen and the bead centres, kicks shift
  the hue. With zero audio it still looks complete.
- A one-texel `FULL_PRECISION` clock integrates the sway speed, as the
  Shadertoy converter's `st_clock` does (D91), so mids change the speed
  without making the pattern jump.

**Would reopen:** the owner's floral image arriving, or a request for a
different composition.

**Revised 2026-10-15 (Opus review before merge).** The ink rim was 0.05
flower radii wide, which filled the narrow petal tips: in focus they read as
black dots, and in the blurred near layer as black crescents beside the
flowers. The rim is now 0.018 radii (a thin outline along the whole petal)
and fades with the layer's blur. Past 1.02 radii (plus the blur) only the
glow reaches, so the petal rings are skipped there: 9% less time on
llvmpipe, 1.48x 05-domain-warp at 1280x720 (was 1.74x).

### D278: Neon Ribbon Flow is five translucent ribbons, not contour lines (2026-10-15)

The Codex draft contoured a single warped field into uniform rainbow stripes
with moire seams and no focal structure. The owner's reference (a Lethal Larry
video, not available) is judged from two frames of the same artist's style:
liquid, saturated, glossy shapes on black. This is an original procedural
interpretation, rebuilt from the draft.

- Five analytic ribbons, far to near, each a sine-sum centreline with a
  true distance (slope-corrected), a twist that narrows it where it turns
  edge-on, iridescent colour that marbles across and along it, filament
  lines, two gloss reflections and a neon edge; layers drift at different
  speeds for parallax and soften with depth.
- Anti-aliasing: every edge is a `smoothstep` over the pixel footprint
  (wider for far layers) and the filaments fade when thinner than about six
  pixels, so there is nothing to alias at any size. Glow is added per layer
  and only where it is above a visible level, which is what made it cheap.
- No feedback buffer: the trail look came from glow and layering instead,
  which keeps it at 1.3x of `05-domain-warp`. The one-texel clock is the
  only extra pass.
- The `LICENSE` field is now present (the draft had none).

**Would reopen:** a real frame of the owner's video showing a structure
this does not have (for instance streaked trails), which would justify a
feedback pass.

**Revised 2026-10-15 (Opus review before merge).** Judged clean but tamer
and more regular than the liquid, glossy style of the two reference frames.
It is kept as the calm counterpart to 13 and 16: pushing it toward liquid
gloss would duplicate Acid Mirror. Two cheap changes: the neon ramp gets a
contrast curve (deep blue and magenta instead of lavender pastels), and
`curl` defaults to 1.8 for more liquid bends. A `smoothstep` with reversed
edges (undefined in GLSL; Mesa happened to draw it) is fixed.

### D279: Acid Mirror advects paint in a feedback buffer and mirrors it (2026-10-15)

The Codex draft was a static analytic Rorschach with rainbow contour stripes
and jagged edges. The owner's reference frame is TouchDesigner-style liquid:
displaced feedback, mirrored. This is an original procedural interpretation,
rebuilt from the draft, not a reconstruction of that network.

- A `DOUBLE_BUFFER` pass, half the output width (`$WIDTH/2`, so the render
  scale still works, D194; never `texelFetch`ed, D280), holds the left half
  of the picture. Each frame it semi-Lagrangian-advects along a curl-of-stream-
  function flow (area-preserving, so paint folds and stretches instead of
  piling up), reflects at the mirror axis, sharpens the colour index with a
  bounded shock filter, thresholds the ink density so edges stay crisp, and
  injects or carves fresh paint from slow noise and two moving pens that the
  mirror turns into eyes.
- The buffer stores scalars (colour index, ink density, accent, grain), not
  colour, so smearing never greys the palette and the black voids stay black.
  Over 5 minutes it neither blows out nor fades (checked at 2, 10, 30, 60,
  120, 240 and 300 s).
- The final pass folds x (optionally y), lights the height of the ink with a
  gradient normal, specular, rim and ink lines in the creases, and maps the
  index through an acid palette (greens and lemon, with magenta, violet,
  cobalt and cyan streaks).
- A one-texel `FULL_PRECISION` clock gives the advection a real frame delta
  and integrates the flow speed (mids), as D91 does for Shadertoy.

**Would reopen:** a resize clearing the buffer in a way that is visible on a
projector (the paint re-grows in about a second), or the owner wanting
more directional streaking than the grain channel gives.

**Revised 2026-10-15 (Opus review before merge).** Against the reference the
render was mostly green and lemon with soft, rounded edges and few voids.
- **Hairline streaks.** The final pass draws iso-lines of the colour index,
  which the flow folds and stretches along itself, so they run with the
  paint in nested bundles. Each line is about two pixels wide at any
  spacing (distance over the index's rate per pixel, measured across 2.5
  pixels so it is smooth), takes magenta, violet, cobalt, cyan or white in
  turn, and fades where lines would crowd into moire. A `streaks` control
  (default 0.8). The grain channel's iso-lines were tried first and read as
  a contour map, not streaks.
- **Edges and voids.** Coverage is one pixel over the ink edge (from
  `fwidth`), the dark rim is a tenth of the density ramp instead of half, the
  ink thickness bevels the normal half as much, and `ink` defaults to 0.4,
  so about a third of the frame is black.
- **A cleared buffer re-seeds at once.** A resize or a render-scale change
  clears it to zeros, which written paint never is; the paint pass then takes
  its sources whole, as on the first frame, instead of re-growing over
  seconds, at the flow time the cleared clock restarts from.
- Two `smoothstep`s with reversed edges fixed; the final pass reuses its
  centre tap (five reads, was six). 1.10x 05-domain-warp at 1280x720
  (was 1.00x); stable at 60, 180 and 300 s.

### D280: a scene that reads a root-sized absolute pass texel by texel renders its final pass at that size (2026-10-14)

**Context.** Synesthesia's `neon_city` declares a root `WIDTH: 1920, HEIGHT:
1080` and three FLOAT, nearest, clamped buffer passes of literally 1920x1080.
Under D194 those stay absolute whatever the render scale. Its final pass
reads them with `texelFetch(passN, ivec2(_xy), 0)`, which addresses the
buffer in pixels, so the final pass has to be 1920x1080 as well. Ours was the
window times the render scale, so it read only the bottom-left corner of the
buffers: a quarter at 960x540 (the RX 580 run at 1.0), a sixteenth at 240x135
(the "striped plane" after adaptive resolution cut 1.0 -> 0.5 -> 0.25).
Adaptive resolution made it worse twice over: the heavy passes did not
shrink, so GPU time stayed at 59 ms (59.4 ms at 1.0, 59.5 ms at 0.5) and
the controller kept cutting for nothing.

**Decision.**
- **The pin.** A scene's *frame size* is its root `WIDTH` x `HEIGHT` when
  it writes both and a buffer pass has a literal (absolute, D194; not
  `FIXED_SIZE: false`) size exactly equal to them (`SceneDef::frame_size`).
  The default 1920x1080 does not count: it is a guess, not something the
  scene said. The final pass is pinned to the frame size only when the
  shader also reads such a pass texel by texel (`strobe_scene::pin`): a
  `texelFetch` or `texelFetchOffset` whose sampler is a frame-sized pass,
  or is not one of the scene's other samplers (a pass of another size, an
  image, a cube map, a `syn_` texture), since a helper's `sampler2D`
  parameter may be handed the frame (`neon_city` blurs through one). The
  scan runs once, when the player is built, on the tokens of every pass
  source and of every `#define` body, comments skipped and `#include`s
  already expanded, so an Editor reload or a context rebuild decides it
  afresh. The player then renders the final pass at the pinned size
  whatever the host asks (`ScenePlayer::render_size` is what is drawn,
  `requested_size` what was asked), every `$WIDTH` pass follows it, and
  `_xy`, `_uv`, `RENDERSIZE` and the pass-size uniforms all describe the
  pinned size, so they stay consistent. Present, the post stage and the
  compositor already scale a frame of any size to the window, so the picture
  fills the window (letterboxed by `present` if the window's aspect differs
  from the scene's; stretched to the window's aspect while a playlist
  crossfade runs, as the compositor samples both scenes by uv). A resize
  for such a scene only records the request: nothing is reallocated and no
  feedback restarts. Parking or warming it in a playlist keeps the final
  pass at the pinned size.
- **Why texel reads, not every root-sized literal pass.** The first draft
  pinned all eleven of the 90 scenes with a root-sized literal pass. Nine
  of those sample it by `_uv` (or by `_xy / RENDERSIZE`, which in each pass
  is that pass's own size), which reads the whole buffer at any final-pass
  size, so they drew correctly at every scale and the pin only cost them
  the render-scale lever on their final pass, which is what keeps a heavy
  scene playable on the HD 4000 and the venue's Vega 3 (`bouncy_hexagons`
  was one of the slowest scenes on the T480). Only `texelFetch` takes
  integer pixel coordinates, so it is the one read that goes wrong from a
  smaller final pass. The first draft rejected a source test because
  includes hide the call; they do not, as `SceneFolder::load` expands them
  before anything reads the source. A scene computing uv from pixels itself
  (`texture(pass, _xy / vec2(1920.0, 1080.0))`) would misread too and is
  not caught; none of the 90 does it.
- **Rejected: a scalable final pass with coordinates in frame pixels.**
  Rendering the final pass small while `_xy` and `RENDERSIZE` describe the
  frame would keep the lever, but needs a new uniform in both dialects'
  assembly for every scene, and `texelFetch` at every second or fourth
  texel aliases instead of averaging (`neon_city` is a noisy path trace).
  For the two scenes pinned, it would also gain little: their buffers are
  literal 1920x1080 under D194 and are most of their cost (59.4 ms against
  59.5 ms at scales 1.0 and 0.5 on the RX 580).
- **Buffer passes of a pinned scene.** The two pinned scenes have only
  literal passes, which D194 keeps absolute at every scale, and no `$WIDTH`
  pass that the scale could lower. Letting adaptive resolution shrink
  literal passes is a D194 question, not reopened here.
- **Adaptive resolution leaves a pinned scene at the ceiling** and says so
  once at the load (`perf.rs`). The scale cannot change what it draws or
  costs.
- **A drop must pay for itself, for any scene.** After a drop the GPU time
  is measured over the cooldown, leaving out its first 10 frames (the timer
  queue still holds up to four frames drawn at the old size, and the first
  frames at a new size pay for its new targets), and the mean of the other
  20 is compared with the smoothed time before the drop. A halving of the
  scale must cut it by at least 15% (`1 - 0.85^halvings` for other drops,
  judged once the drop spans a quarter of a halving). If it did not, the
  controller goes back to the scale it left and logs the two times. GPU
  time follows the music too, and a heavy passage that starts just after a
  drop hides what it saved, so one verdict only goes back up: lowering is
  locked at that scale after two futile verdicts in a row, and a drop that
  pays in between clears the count. The lock expires after 1800 frames (30
  s at 60 Hz) so a scene whose cost changes, or a wrong verdict, gets
  another try; one more futile probe locks it again for twice as long, up to
  four minutes. A scene change lifts it. Raising always works. A fixed cost
  under a scalable one stops the descent where the gains run out instead of
  running to the floor. The price is two probes of half a second at the
  lower scale per such scene, then one per expired lock, each a resize
  that restarts feedback.

**Cost.** Two of Synesthesia's 90 scenes are pinned: `neon_city` and
`entombed_silicon_dreams`. Their final pass is always 1920x1080. The nine
other scenes with a root-sized literal pass (`SkaiWalkerOG`,
`audio_indicators`, `bouncy_hexagons`, `fractilian_foundation`, `hextunnel`,
`logo_grid_pulses`, `neuron_proximitors`, `thresholder`, `videofx_1`) are not
pinned and keep the render-scale lever on their final pass, as before
D280. `neuron_proximitors` was counted as a texel reader in the first draft,
but the one such read of a frame-sized pass is in a comment; its live
`texelFetch` calls read `BuffA`, a 5x512 data pass, which is absolute
(D194) and right at any final-pass size.

**Evidence.** `scene.rs` `a_literal_pass_at_the_root_size_is_the_scenes_frame`;
`pin.rs` `a_texel_fetch_of_a_frame_sized_pass_pins_the_final_pass`,
`sampling_by_uv_or_fetching_other_textures_leaves_the_final_pass_free`
and two more; `render.rs`
`a_final_pass_reading_a_root_sized_literal_pass_covers_it_at_any_render_scale`
(an original fixture in both dialects: the picture covers the whole
`_xy / vec2(1920, 1080)` gradient asked for at 960x540, 480x270 and 240x135)
and `a_scene_without_a_texel_read_root_sized_literal_pass_still_follows_the_render_scale`
(including the same fixture sampled by uv, which covers the gradient at
480x270 unpinned); `adaptive.rs`
`a_scene_whose_cost_ignores_the_scale_is_not_lowered_for_nothing`
(two strikes, the lock, its expiry and the doubled re-lock),
`one_heavy_passage_after_a_drop_does_not_lock_a_scene_that_scales`,
`the_frames_still_in_flight_after_a_drop_are_not_judged`,
`a_mostly_fixed_cost_scene_stops_lowering_when_the_gain_runs_out` and
more; `perf.rs` `a_scene_that_pins_its_final_pass_stays_at_the_ceiling`,
`a_drop_that_cuts_nothing_is_undone_and_says_so`. The private corpus: 90 of
90 in both dialects, and the pin decided for exactly the two scenes above.
On the real `neon_city` under Xvfb: see `hardware/AMD.md` A9 and the run
recorded with this change.

**Would reopen:** a scene that reads a root-sized pass by pixel without
`texelFetch` (computing uv from `_xy` and the literal size); a pinned scene
whose final pass is the cost on a slow GPU; or a window aspect that makes
the letterbox matter on a projector.

### D281: the control window is Synesthesia's three columns, and the rail switches the main view (2026-10-14)

**Context.** The owner asked for the UI to be "more similar to
Synesthesia's layout while keeping our modern functionality and improving
the UI overall". Synesthesia 1.25.4's window, measured on a real install,
is three columns under the title bar: a left icon rail (about 3% of the
width), the main view (about 66%), a right panel (about 31%), and a
full-width help bar along the bottom (about 3% of the height). The rail
switches the main view; there is no Live/Edit mode switch. The active rail
item is a filled tile, and the right panel stays whatever the main view is.
Strobe's first Contextual Perform shell (D130-D138) had a workspace per
rail item, with panes docked around a preview.

**Decision.**
- **Two docks, not a workspace.** The *main* dock holds the Library
  (default), Import (GET), Playlist (LIST), Editor (EDIT), Mappings (MAP),
  Diagnostics (DIAG) and Settings (SET), one at a time: the most recently
  opened. With none open the main view is the large live preview (PLAY).
  The *side* dock holds the panel (D283) alone. `Pane` keeps its eight
  members and gains `Dock`; `Panes::shown_in(Dock)` says what is drawn.
  Esc closes the topmost pane, so Library, then Diagnostics, then Esc is the
  Library again.
- **The side panel persists.** The rail never touches it. Open, it is
  docked at its width; closed, it is a 28-point handle strip that reopens
  it, with a red dot while BLACKOUT is on.
- **Its width follows the window until it is dragged.** Until then
  `arrange` uses 31% of the window (`SIDE_SHARE`), inside the panel's 280
  to 720 range (595 at 1920, 424 at 1366). A width the operator drags to is
  kept as dragged. *Follows* is a flag on `Panes` (`side_follows`), not a
  magic width, so a drag that lands on any width, the 440 default
  included, is kept; a double-click on the panel's edge makes it follow
  again. `ui.toml` stores 0 for a following panel, and its version is 2: a
  version-1 file's inspector width was the old inspector's (300 by
  default), so it is dropped on reading and the panel follows the window.
  (The first cut used the default width as the sentinel; a drag that landed
  on 440 snapped the panel back to 31%, and an existing file kept the old
  300.)
- **Narrow windows give way as D260 says, in this order.** The main view
  keeps `MAIN_MIN` (320). Below that the panel shrinks towards its minimum,
  then becomes a drawer over the main view, clipped to the surface and
  below the banners. The rail drops its labels below 1000 points.
- **Strobe's own parts keep their places.** The title bar and its health
  readouts, the transport strip with BLACKOUT, and the macro strip stay
  under the columns at every size; Ctrl+K, Settings > Layout (saved named
  layouts) and density (Comfortable and Compact) are unchanged. The only
  thing taken away is the old per-workspace arrangement.

**Not done on purpose.** Synesthesia's proportions are matched; its colours
are not (Strobe's theme, docs/branding.md). The rail is 60 points, 3.1% at
1920, rather than a percentage, so its icons and labels have a fixed size.

**Would reopen:** the owner finding the always-open panel too wide on a
1280 window (it is 397 points there), or a second main view wanted beside
the first.

### D282: the library is a grid of cards under a toolbar and a filter row (2026-10-14)

**Context.** Synesthesia's library has a toolbar (new scene, refresh, a
small, medium or large thumbnail switch), a filter row (the count, "88/88",
a wide search, a sort dropdown with a direction, a favourites-only toggle)
and a grid of thumbnail cards with a dark caption band: a bold title, the
author and a "..." menu. Strobe's browser was a list of rows (D211), then
a grid with the search and the health chips on two rows of their own.

**Decision.**
- **Toolbar.** *New scene* opens Import, which is how a scene starts in
  Strobe (a Shadertoy shader; a scene of Synesthesia's is copied with *Copy
  to my library*). It is not a scene wizard, and says so in its tooltip.
  Then a rescan, and at the right the S, M and L switch (saved). The label
  drops to an icon below 300 points.
- **Filter row.** The count, a wide search, the sort and its direction, the
  favourites star, and the health dropdown. Search matches names, authors
  and tags, every word, as before. Below 560 points the search keeps a row
  with the count and the rest take the next, so no control is ever dropped.
  The health filter (All, Ready, Needs work, Favourites) stays: the star
  toggles between Favourites and All, and the dropdown reaches the rest.
- **Sort.** Library order (the scan's: the folder's scenes by name, then
  Synesthesia's), Name, Author (the unsigned last) and Favourites first; the
  direction reverses. Saved in `ui.toml` (`browser.sort`, `browser.descending`;
  a file from before is library order, ascending). The order is part of the
  cached visible list and recomputed only when the query, filter, sort or
  library changes, never per frame.
- **Authors.** `Entry.author` is the scene's `CREDIT`, else the `SOURCE.author`
  of an imported shader, read with the tags at a scan.
- **Cards.** Picture, with a state chip (`PLAYING`, `PREPARING`, `PREVIEW`,
  `FAILED`) at its top-left and the star at its top-right; a caption band
  with the title, a "..." button that opens the right-click menu (Load,
  Preview, favourite, Open in Editor, and Copy to my library on a read-only
  scene), and the author. The author line begins with a check once the scene
  loaded, and says `Failed: <first line>` in its place when it did not; with
  no author it shows the tags.
- **D271 lives in the grid.** The author line ends with a faint
  "Synesthesia" when there is room (60 points left for the text first), the
  count says "88/88, 4 from Synesthesia" (just "88/88" in the narrow layout,
  with the sentence in its tooltip), the search word `synesthesia` finds
  them, and the card id is by name and folder, so a name in both places is
  two cards (egui had reported two widgets with one id before this).
- **No bold.** No bold face is bundled and adding one is a font choice for
  the theme (docs/branding.md), so the title is larger (body plus 1.5) and
  the brightest text on the card. Drawing the title twice half a point apart
  was tried and removed: `no_text_is_drawn_over_other_text_at_any_size`
  rightly reads it as two overlapping labels, and that test is not to be
  loosened.

**Would reopen:** a bold font in the theme; the owner wanting the health
filter as chips again.

### D283: the side panel is a preview, a status strip and four icon tabs (2026-10-14)

**Context.** Synesthesia's right panel is one 16:9 preview, a status strip
(output, fullscreen, an fps readout with a dot, a horizontal audio level
bar, snapshot) and icon tabs for Playlist, Controls and Edit.

**Decision.**
- **The preview** is the scene's own texture fitted to the panel's width (at
  most 38% of its height; left out below 64 points), with the LIVE,
  BLACKOUT and output-hidden badges. The PLAY view still has the large
  preview with the output toolbar (Show output, Fullscreen, the monitor
  menu).
- **The status strip**, in Synesthesia's order: Output (show or hide),
  Fullscreen, a dot and the frame rate (green at 50 and over, amber at 25),
  the audio icon and a level bar that takes the room left, the tempo (dropped
  below 300 points) and Snapshot at the right edge, which is *Use frame as
  picture* (`Action::CaptureThumbnail`). Snapshot is disabled with a reason when
  nothing plays or the scene is Synesthesia's (D271: never written).
- **Four tabs, icons with their names in tooltips**, and beside the icons
  from 400 points of panel width: Playlist, Controls, Presets, Edit. The
  chosen tab is saved (`side_tab`). *Controls* is the script state and the
  groups. *Edit* is what used to sit above the controls: tags, Use frame as
  picture, media, Generate controls, and a new *Open in Editor*. *Presets*
  is the preset row and the preset list.
- **The fav slots (D176) are a row above the tabs**, in reach from every
  tab, as Synesthesia keeps them and as they were at the top of the old
  inspector. A first cut put them inside the Presets tab; a performer on
  the Controls tab then had to change tabs to fire one, which is a
  regression for a live show. They hold scenes, so they work with or
  without one playing. A panel under 360 points high (a drawer in a short
  window) moves them to the top of the Presets tab, and a panel too short
  to give the preview 64 points at its 38% share leaves the preview out:
  at 427x320 the first cut's drawer was all preview, strip and tab bar,
  with no room left for a single control.
- **Playlist** is the loaded show's cue list with a jump on double-click and
  a button to LIST (unchanged).

**Would reopen:** a short window where the fav slots matter more than the
tab's body.

### D284: a help bar under the transport says what the control under the pointer does (2026-10-14)

**Context.** Synesthesia has a full-width bar along the bottom that explains
what is under the pointer. Strobe's controls have tooltips, which cover the
thing they describe.

**Decision.** `ui::help`: a 24-point bar below the transport strip. A
control that has a tooltip says it through `help::hovered(response, text)`,
which copies the text into a reused buffer while the pointer is on it; the
bar, drawn before the panels above it, shows the previous frame's text (a
frame behind, invisible) and, with the pointer on nothing, a sentence about
the main view. Wired into the icon, tool, transport and blackout buttons,
the rail, the tabs, the size switch and the cards. It carries nothing a
person must read (those are banners, D285) and is not drawn in a window
under 520 points high, so the transport strip and BLACKOUT keep their room.
It allocates nothing per frame beyond the text's own layout.

**Would reopen:** a control whose hint differs from its tooltip, or a
window height at which the bar should stay.

### D285: banners scroll past half the surface, and nothing can be wider or taller than what it is given (2026-10-14)

**Context.** The brief for this redesign named a hazard: long notices must
never push panels over BLACKOUT, for instance at 427x320. A long-notice
scenario, added to the shell tests, found two real defects, and looking at
its screenshots found a third that no test could see.

**Decision.**
- The banners above the main view share the surface: past half of its height
  (`BANNER_SHARE`) they scroll. Before, a tall banner grew the central panel
  past the space the transport strip had left, and the drawer was painted
  over BLACKOUT.
- The drawer's height is `max(0)`. At 427x320 with a long notice the surface
  below the banners was shorter than the drawer's margins and egui panicked
  on a negative height, which is an abort in a release build.
- A narrow banner puts its buttons in a wrapping row of their own. In
  `right_to_left` three buttons wider than the banner pushed it off the left
  edge, over the rail, at 320 points wide; no test saw it because the
  overflow test looks only at the right and bottom edges.
  `a_banner_with_three_buttons_stays_inside_a_narrow_surface` holds it.
- New scenarios: a long notice over the library and over PLAY, the Edit tab
  with and without a scene, and the library sorted by author with favourites
  only, comfortable and compact, with authors on the scenes. All run through
  the overflow, BLACKOUT and text-overlap tests at every size.

**Would reopen:** a notice that must be read in full in a small window; it
scrolls, and its Dismiss button is always in the first row.

### D286: the desktop viewer keeps its last image, gives up waiting, and says what to do next (2026-10-15)

**Context.** The first two-machine test of D275 (a T480 controlling an RX 580
host) worked end to end and found five rough edges: the viewer went blank
when the host dropped it though its text promised the last image; "Connecting;
approve this desktop on the host" waited for ever; a reconnect with a blank
code was refused with the phone-oriented "not paired, or revoked"; the
connect form was the last paragraph of Settings > Remote with a hint that
read as a typed address; and the host's title counted a refused socket as
"1 connected".

**Decision.**
- **The viewer has a stage, and the picture follows it** (`desktop::Stage`):
  *Connecting* shows a card with the seconds spent against the limit and what
  to do on the host; *Live* is the interactive host window, dimmed with a
  "waiting" card while frames are paused; *Ended* (timed out, refused, lost)
  keeps the last received frame, dimmed and read-only, under a card that says
  why. The worker no longer clears the image when it fails, so a rebuilt
  graphics context can still recreate the texture; `disconnect` releases it.
  The text claims the image only when one is drawn.
- **Waiting for approval ends after 60 s** (`APPROVAL_TIMEOUT`), enforced by
  the worker, which shuts its socket down however it ends: the interrupter
  the app holds is a second handle on the socket, so dropping the connection
  alone left it open, and the host went on listing a request it could still
  approve. The deadline is checked only after a poll that brought nothing,
  so an approval the host sent just before it is read rather than thrown
  away. A reconnect with a held pairing needs no approval and says so. The
  time label reads "Waiting 12 s of 60 s"; it is formatted once a second,
  and the viewer repaints when the count next changes.
- **Input that cannot reach the host ends the connection** (`End::Lost`),
  rather than leaving the viewer on a paused card waiting for a frame that
  cannot come; a held pairing offers Reconnect.
- **Refusals name the next step.** `Unauthorized` becomes "a new pairing code
  from the host is needed" with where it appears (Settings > Remote > Pair a
  device, with Allow full desktop control on); `PairingRejected` keeps the
  host's reason and says the same. The wire message ("not paired, or
  revoked") is unchanged, because phones read it too. A finished connection
  offers **Reconnect** only when the pairing is still held in memory;
  otherwise **Enter a new pairing code** (`Action::ControlAnotherDesktop`).
- **The form is its own Settings section, "Control another Strobe".** The
  address starts empty with the hint "Host address, e.g. 192.168.1.20", the
  code field hints "Pairing code", and a bare IP takes the default port
  (47812). Ctrl+K lists **Control another Strobe...** (category Remote), which
  opens Settings, scrolls to the section and focuses the address; where the
  pane is too short for the heading and the field together, it scrolls the
  field into view instead. From a remote desktop pass it only closes this
  computer's own viewer and opens the form: Connect stays host-only (D275).
  No rail item was added: the rail is for Synesthesia's seven views (D281)
  and this is a rarely used setting, so the palette and a section of its own
  fit better than a new icon.
- **"Connected" counts authenticated clients.** `LanView::connected` counts
  connections whose `client_id` is set (`ConnectionInfo::is_authenticated`);
  a socket that was refused or is still waiting for approval is open but not
  counted. Settings reads "N paired devices connected".

**Evidence.** Unit tests for the count, the address, the timeout and the
messages; real-socket tests for a connection nobody approves and a blank-code
reconnect after the host revoked; viewer states drawn at every size through
the overflow and text-over-text checks; screenshots of each state. The
T480 to RX 580 re-test of these states is still to do.

**Would reopen:** an operator wanting automatic reconnect (D275 chose explicit
reconnect, so a stale click is never replayed), or a viewer that must stay
useful with the host asleep.

### D287: a resize keeps every pass whose own size it does not change (2026-10-15)

**Context.** `resize` rebuilt every pass target and `resize_keeping` blitted
every one into a fresh target, whatever the pass's size. D91 accepted that
when a resize was rare. It is not: the adaptive-resolution controller probes,
undoes and probes again (D280), and a window drag resizes continuously. Each
one reset the 1x1 `st_clock` pass of a Shadertoy import and the mids-integrated
clock of scenes 14 Electric Bloom and 15 Neon Ribbon Flow to zero, so their
integrated time drifted away from `TIME` and the picture visibly jumped. A
literal-size data pass (D194) lost its state for no reason, since nothing about
it had changed.

**Decision.**
- `ScenePlayer::allocate_passes` computes each pass's size as before and
  compares it with the old pass's. When they are equal it allocates nothing
  for that pass; the pass is marked and, once every allocation has succeeded,
  takes the old pass's targets by move (`adopt_unchanged`). Texture,
  framebuffer, contents and which buffer of a double-buffered pair is current
  all carry over. No copy, no clear, no blit.
- This holds for both `resize` and `resize_keeping`. The passes that do change
  size (`$WIDTH`-relative passes and anything with a scale) behave as before:
  `resize` restarts them from black, `resize_keeping` blits them filtered
  (nearest for 32-bit float) into the new targets.
- **Except what a restarted pass reads.** On `resize`, an unchanged pass that
  a restarted pass samples (the final pass aside; transitively) restarts with
  it, so the two stay consistent. The case that forces this is a Shadertoy
  import: its `$WIDTH` buffers read `st_clock` for `iFrame`, and a buffer
  wiped to black needs `iFrame` back at 0 to run its `iFrame == 0`
  initialisation again (D91). Keeping the clock left such a buffer counting up
  from black forever. The set is decided from each program's active samplers
  once per resize. `resize_keeping` wipes nothing, so it keeps every unchanged
  pass.
- `ScenePlayer::restart` clears every pass at the current size, kept ones
  included. `strobe-autocontrol` used "resize to another size and back" to
  clear history between probes; that no longer clears a literal-size pass, so
  its probes call `restart` instead (which also clears a pinned scene, D280,
  that the resize trick never touched).
- The final pass and the two finished-frame targets always follow the render
  size, so they are rebuilt (or blitted) on every real change.
- Allocation happens before any player field is touched, the final-pass
  readback's included, so a failed `resize` or `resize_keeping` leaves the
  player unchanged.
- A context-loss rebuild builds a new `ScenePlayer`, so it still recreates
  everything.
- Nothing runs per frame: the comparison and the move happen once per resize.

**Test.** `a_resize_keeps_the_passes_whose_size_it_does_not_change` (both
dialects, both resize calls): a 1x1 double-buffered counter keeps counting
through three resizes and a no-op resize while a `$WIDTH` pass still follows
the new size. With the keep disabled the `resize` half fails with the counter
back at 1; the `resize_keeping` half passes either way, since its exact
same-size blit already kept such a pass, so there the keep saves a copy.
`a_resize_restarts_an_unchanged_pass_that_a_restarted_pass_reads` (strobe-gfx)
and `a_resize_that_wipes_the_buffers_restarts_iframe_so_they_initialise_again`
(strobe-shadertoy) fail without the exception above;
`restart_clears_every_pass_a_resize_would_keep` covers `restart`.

**Would reopen:** a pass whose size is unchanged but whose contents should
still be cleared by a resize (none known), or a driver that invalidates
textures on a viewport change.

### D288: venue mode, a trust file lets a controller take full desktop control with no click at the host (2026-10-15)

**Context.** The JaJa venue's mini-PC drives two mirrored projectors and has
no control screen. The owner runs the show from a laptop with the full desktop
remote (D275), whose pairing needs the host's *Allow full desktop control*
switch on and an operator clicking Approve at the host, and whose grants live
in memory, so a host restart ends them. With nobody at the machine none of
that can happen.

**Decision.**

- **Launch flags on the host.** `--remote` (existing), `--allow-desktop-control`
  (switches full desktop access on when the listener first starts; needs
  `--remote`; never saved; consumed once, so stopping and starting the remote
  by hand later does not turn it back on) and `--remote-trust FILE` (needs
  `--remote`). The trust file is read once at launch. There is no setting,
  button or saved state for it, so there is nothing a remote controller's UI
  pass could be made to widen (`desktop::host_only` is unchanged, and needs no
  new entry). A trust file that cannot be read is reported and the host runs
  without trusted pairing (fail closed); it does not stop the show.
  A launch-time `--remote` that cannot start (no network yet at boot) is
  retried every 5 s, with only the first failure reported.
- **The trust file.** `strobe --make-remote-trust FILE` writes
  `strobe-remote-trust 1` and `secret <64 hex>`: 32 bytes from the OS RNG,
  with comment lines saying what it is. It uses `create_new`, so it refuses to
  overwrite and does not write through a symlink; mode 0600 on Unix; on
  Windows it takes the folder's ACL, so keep it in your user profile (ACLs are
  not set, which would need `unsafe`). Reading takes one open handle, at most
  4 KiB, refuses a file other Unix users can read (as `ssh` does), a version
  it does not know, trailing lines and a secret with fewer than 8 distinct
  byte values.
- **Trusted pairing is challenge-response; the secret is never sent.** After
  `hello`, the controller sends `trust_start {controller_id}`; the host
  answers `trust_challenge {nonce}` (32 random bytes, hex); the controller
  sends `trust_proof {response}` with
  `HMAC-SHA256(secret, F(context) F(nonce) F(host) F(controller id))`,
  `F(x) = len16(x) || x` (so no two tuples encode alike). *Context* is
  `strobe-remote trust v1: full desktop control`; *host* is the `Host` header
  the controller dialled, which must name the address and port that this
  connection actually reached (or `localhost` on loopback, or the D258 proxy
  name from this computer). That is stricter than the listener's own `Host`
  check, which on every interface (`--remote-bind 0.0.0.0`) takes any IP
  literal; without it a relay at another address could forward a
  controller's handshake under its own `Host` and the proof would hold. So a
  proof made for one address is no use at another (it defeats a rogue host
  relaying a challenge to the real one from a different address); *controller id* is 1 to 48 of `[A-Za-z0-9._-]`. The host
  verifies with `hmac`'s `verify_slice` (constant time, length-checked). The
  challenge is a value held by its connection and consumed by `verify(self)`,
  so it answers once whether right or wrong, and expires after 10 s. Success
  sends the existing `authenticated` and `state`; no token is issued. The
  v1 and desktop messages are byte-for-byte unchanged (golden tests), and
  the three new messages are frozen the same way. A sniffer sees a nonce and a
  MAC bound to it; recovering a 256-bit key from that is not feasible and a
  recorded response is useless on the next nonce.
- **Required to succeed:** the host has a trust file *and* full desktop access
  is on. Otherwise the answer is `unavailable` with one text for both cases,
  so a scan cannot tell how the host was started. Phones, `pair`,
  `pair_desktop` and tokens are untouched; a trust file never gives a phone
  token desktop scope.
- **A trusted pairing is a property of its connection.** It is a desktop
  record in the same in-memory list (never written to
  `remote-clients.json`), labelled `Full desktop: trusted <id>, <device> at
  <ip>`, shown in Settings > Remote with a *trusted* chip. Its token digest is
  of a token nobody was given, so it cannot be authenticated with later. It is
  removed when its connection ends. A controller that reconnects (Wi-Fi drop;
  the host has not yet noticed the old socket) replaces its own earlier
  record: the old connection is closed and gives up the lease at once, so the
  new one can take it. A different controller id never takes the lease.
  This keeps the 32-client list from filling over a four-hour show.
- **Revocation is for the run.** Revoke on a trusted entry drops the
  connection and refuses that controller id until Strobe restarts (otherwise
  it would prove its way straight back in). Revoke all closes trusted pairing
  until restart, because a leaked file could otherwise change its id. The kill
  switch (switching full desktop access off, or Stop remote) still ends every
  desktop pairing, trusted ones included, and while it is off trusted pairing
  is refused; switching it on again (a host action) re-opens it. The session
  also ends itself on its next tick if the switch is off or its entry has
  left the list, so a revoke, revoke all or replacing reconnect that lands
  between the pairing and the connection being marked with its client id
  still ends it.
  The title bar still says *Desktop controlled remotely* because it follows
  the lease.
- **Failed proofs are limited and logged.** Per peer address: three free
  failures, then a pause of 2 s doubling to 5 min. Across all addresses: 20
  failures in a minute pause everyone for 30 s, doubling to 5 min while it
  repeats, forgiven after a quiet spell. A paused peer is answered `busy`
  and disconnected; each failure is a warning in the log with the peer, the
  controller id and the pause (never a nonce or proof). Each connection also
  keeps its existing five-failure limit and message bucket. The state is in
  memory. Through the D258 proxy every client shares the loopback address, so
  the per-peer limit is then global in effect.
- **Dependency.** `hmac` 0.13 (RustCrypto, MIT or Apache-2.0), on the same
  `digest` 0.11 as `sha2` and `sha1`. It brings `cmov` and `ctutils` (the
  constant-time primitives `digest` 0.11 uses) as new lockfile entries.

**Threat model, and what this does not do.**

- **Whoever can read the trust file has full control** of the host: scenes,
  files and quitting (D275 scope), with no click. Treat it like a key. It is
  not wiped from memory (that needs `unsafe` or a crate), it is on disk in
  clear, and a controller with the full UI could in principle write host
  files, including over the trust file, which the next launch would then
  read. Keep the file outside any folder Strobe writes to. Replace it if it
  may have leaked: make a new one and relaunch both ends.
- **LAN only, plain HTTP.** The secret is never on the wire, but frames,
  input and everything the window shows are still in the clear (D275), and an
  active attacker who can sit on the path can take over an established
  connection, or relay a handshake from the same address, without learning the
  secret. Only the proof of knowledge is protected; there is no
  confidentiality and no host authentication to the controller (a rogue host
  can show the controller a fake window). TLS would close those (backlog
  L-07).
- **A flood can pause new attempts** (the global limit), but not a session
  already open.
- **Windows file permissions are not checked or set**; only the user profile's
  inherited ACL protects the file.

**Not done in this change.** The controller side: a trust-file field in the
*Control another Strobe desktop* form, `--control HOST:PORT --remote-trust FILE`
opening straight into the viewer, and automatic reconnect with backoff after a
host restart or a Wi-Fi drop. It is to be built on the viewer's Stage and
Reconnect machinery from D286, which was not on this branch (see
STATUS_AUDIT). The host half and the protocol are tested over real sockets;
the controller can use `trust::TrustSecret::respond` and the messages above.
**Built in D294.**

**Evidence.** Unit tests: RFC 4231 HMAC vectors, a protocol vector computed
independently with Python, a wrong secret, a flipped nibble, wrong host,
controller and nonce, single-use and expiry, the file parser, no overwrite,
no write through a symlink, mode 0600, loose-permission refusal, and the
limiter's backoff, global trip and bound. Socket tests
(`strobe-remote/tests/server.rs`): trusted pairing with no approval, the
secret and proofs never on the wire or in the log, wrong secret/host/
controller, replay, no trust file or desktop control off, the kill switch,
operator revoke and revoke all, reconnect taking back the lease, per-address
backoff and recovery, phone pairing unaffected, and the v1 messages unchanged.
Not verified: two physical machines, a Windows host, a four-hour soak.

**Would reopen:** TLS for the native client (closing the plain-HTTP gaps); a
per-controller secret instead of one shared file; ACLs set on Windows.

### D290: the event loop runs the control frame when Windows withholds its paints (2026-10-15)

**Context.** On the JaJa mini PC (Ryzen 3 3200U, Vega 3, AMD WGL), a
scheduled-task show with `--playlist --output-monitor 1 --fullscreen` logged
"control window in view: redrawing at 30 Hz", then "control 1 redraws" in the
first minute and "control 0 redraws" in every minute after, with the output
at 60 fps. Ninety posted `InvalidateRect`/`WM_PAINT` nudges and three minutes
with the output moved off-screen drew nothing either, so it was not
occlusion. The playlist compiled one scene per control frame and stuck at 1
of 14 behind a black output; fca44a5 made the output frame compile when the
control window has been quiet for 500 ms. Everything else in
`redraw_control` (applying UI actions, the remote and LAN UI passes, the
D275 desktop capture, `ui_state.tick`, remote persist, preview loads) still
never ran, so the desktop remote could not have worked on that machine.

**Cause.** The control window was starved of `WM_PAINT`, not hidden from it.
- winit 0.30.13 on Windows emits `RedrawRequested` only from `WM_PAINT`
  (`request_redraw` is `RedrawWindow(.., RDW_INTERNALPAINT)`,
  `platform_impl/windows/window.rs:152`; the event is sent from the
  `WM_PAINT` arm, `event_loop.rs:1276`).
- Windows makes a `WM_PAINT` only when the queue is otherwise empty, and for
  the first dirty window of the thread found walking top-level windows in
  z-order from the top (ReactOS's `IntFindWindowToRepaint` models it).
- winit stops dispatching after every `RedrawRequested` and goes back to
  `about_to_wait` (`event_loop/runner.rs:214`, `interrupt_msg_dispatch`).
- `schedule` requests the output's redraw whenever `OUTPUT_PERIOD`
  (6.9 ms) has passed since the last request. On Vega 3 the fullscreen
  output's vsync wait lands inside its own swap, so its frame takes about
  16.7 ms and the output is dirty again on every pass.
- So with the output above the control window, the output is always the
  first dirty window and the control window's paint never comes. Nudges add
  update region to a window that is already waiting; moving the output
  off-screen changes neither its z-order nor its dirtiness.

This also explains TORNADO T11's "0 to 2 paints a minute" for a covered
control window (D206), and why a *windowed* output on NVIDIA left the
control window painted: there the vsync wait landed in the control window's
`make_current` (D216), so the output's own frame was short and it was not
always dirty. The covered/in-view state does not decide it: the app thought
the control window was in view here (it had been given focus), and z-order,
not focus, is what counts.

**Decision.**
- **`stats::ControlDrive`.** Every control request is still made as before.
  When one has gone unanswered for `PAINT_PATIENCE` (250 ms) and
  `PAINT_PASSES` (4) passes of the event loop, `about_to_wait` runs the
  control frame itself (`redraw_control(true)`): make-current on the
  control surface, the egui pass, paint, desktop capture, swap, actions.
  It keeps doing so at the control window's period until a paint answers a
  request within the patience, which hands the frames back. A late paint
  (an output frame that happened to let one through) leaves the loop in
  charge, so the two do not take turns.
- **Counting passes as well as time** keeps a loop that was merely blocked
  (a scene compiling for a second inside an output frame) from reading as a
  starved window: that is one or two passes, a starved window sees one per
  output frame.
- **While the loop drives, the window counts as unseen** (`control_unseen`):
  it redraws at `COVERED_PERIOD` (2 Hz), or `desktop::STREAM_PERIOD` (5 Hz)
  while a desktop remote holds the lease, and the pacer treats it as
  covered. Starvation means the output is above it, and a swap of a covered
  window is what D206 throttled.
- **The swap policy does not.** The output's D221 wait is skipped only
  while `control_covered`, as before. A driven window still takes the
  shared context two to five times a second, and on a machine where the
  wait has engaged, dropping it would move the flip's wait back into that
  make-current and cost the show a frame per driven redraw (TORNADO T15,
  T16), where before this fix the starved window cost nothing.
- **Not in screenshot mode, and not while minimised.** A screenshot captures
  what was painted and keeps its 10 s give-up; a minimised window has
  nothing to draw (`redraw_control` returns at once).
- **The perf line counts loop-driven redraws:** "control N redraws X ms
  (worst Y ms, M unpainted)". The first takeover is logged at info:
  "control window: no paint from Windows for N ms (takeover 1); the event
  loop runs its frames every 500 ms until one comes (D290)". Later
  takeovers and every hand-back are at debug.
- **fca44a5 stays** as the backstop for the playlist compile.
- **The desktop capture stays valid.** It reads the control window's back
  buffer right after egui paints it and before the swap. Windows composes
  every window through DWM (always on since Windows 8), so a covered window
  owns all its pixels and the read does not depend on the window being
  shown. The fenced readback on a real covered window is still to be seen
  on hardware (STATUS_AUDIT).

**Not chosen.**
- *Skip the output's request while a control paint is overdue*, so Windows
  picks the control window. The control frame would run in the same slot
  between two output frames as a driven one does, so it costs the same; but
  it still depends on Windows' paint order, and a paint that did not come
  (a minimised window, another window of the thread on top) would stall
  the output until a timeout, which is this fix's heuristic again.
- *Drive the control window from the loop always*, dropping
  `request_redraw`. It would change the paint behaviour everywhere it works
  today, and lose winit's expose redraws.
- *Raise the control window or post paints at it.* The field already showed
  nudges do nothing, and raising it would cover the projector.

**Test.** `stats::tests`:
`a_painted_control_window_is_never_driven_by_the_loop`,
`a_control_window_with_no_paints_is_run_by_the_loop_at_its_own_period`
(the mini PC's state),
`a_blocked_loop_is_not_a_window_without_paints` and
`the_loop_hands_back_only_to_a_prompt_paint`;
`a_minute_is_summarised_once_with_late_frames_counted` checks the perf line's
unpainted count. Linux and llvmpipe cannot reproduce the paint order, so the
acceptance test is the mini PC (MINIPC.md).

**Would reopen:** the mini PC's log showing the takeover line with the
control count still at 0 (then the frame itself fails, not its scheduling),
or a loop-driven 2 Hz control window costing the output frames.

### D291: one click on a scene card makes it live, and the library view flips between Scenes and Controls (2026-10-15)

**Context.** The owner: "UI still needs work". Picking a scene took a click
to select it and then a Load button over the selected card (or a
double-click); in Synesthesia one click on a scene makes it live, with no
preview step in the main flow. The playing scene's controls lived only in
the side panel's Controls tab, a 31% column, where Synesthesia gives them
the main area. D288 to D290 are taken on other branches (venue mode, the
AMD bench diagnostic, the control-frame fix), so this is D291.

**Decision.**
- **A click on a card loads it.** `browser::card` pushes `Action::Load` on
  the card's click, and also selects it (its tags show above the grid).
  Enter or Space on a keyboard-focused card is the same click (egui reports
  both as one), and a focused card shows a second accent ring inside its
  edge. No load is asked for the second click of a double-click (or a
  triple), for the scene that is live, or for the scene already preparing,
  so a double-click is one load request. Everything that guarded a load is
  unchanged: the request is still `App::request_load`, painted as
  PREPARING on the card first; a failure keeps the old scene and is shown
  on the card and as a banner; nothing blocks a frame beyond the compile
  that loading already costs.
- **Preview stays, out of the main flow.** The Load and Preview buttons
  over the selected card are gone. *Preview without loading* is in the
  card's "..." menu (also its right-click menu) and in the palette
  (`Preview: name`); the preview card above the grid keeps *Load this
  scene*. On the live card the menu offers *Reload from disk*, which was
  the selected live card's button. Enter in the search field still loads the
  highlighted scene.
- **The library view has two pages, Scenes and Controls**
  (`browser::LibraryPage`, saved in `ui.toml` as `library_page`). A
  segmented switch (`widgets::segmented`) sits at the top of the main
  view; on the Scenes page the library's toolbar shares its row when the
  view is 560 points wide or more, and takes the next row below that. The
  Controls page is `inspector::show` with `Part::Page`: the same groups,
  rows, pin menus, default and random, script state and meta section as the
  side panel's Controls tab, laid out in up to three columns (one per 340
  points, each group to the shortest column, from a fixed array) and drawn
  at `controls::Scale::Touch` (32-point rails with a thicker track and
  handle, 48x26 switches, 40-point bangs, an XY pad up to 280 wide, a
  56x30 colour swatch). No control logic is duplicated: `controls::rows`
  takes the scale.
- **The side panel keeps the preview and the output's state in view** and
  does not draw the same controls twice: while the main view is the
  Controls page and the panel is docked beside it, its Controls tab is
  dimmed and the panel shows Presets in its place (without changing the
  saved tab). A drawer covers the main view, so a drawer keeps its Controls
  tab.
- **Flipping is one key.** `toggle_controls_view`, "Switch Scenes /
  Controls", default **Ctrl+L**, Navigation, in the palette like every
  command and rebindable in Settings > Keyboard. On the library it flips
  the page; from any other main view it opens the library on Controls.
  Not mappable to a controller (a presentation command, like Ctrl+F).
  A plain letter was rejected because a command's key silently shadows a
  macro bound to the same key; Tab is egui's focus traversal.
- **Remote desktop passes gain nothing.** A click on a card from a remote
  pass is the `Action::Load` it already was (not host-only, D275), and the
  switch is presentation state only.

**Smaller fixes from the same pass.** The colour control's swatch had
zero width (the theme sets `interact_size.x` to 0 and egui sizes the swatch
by it), so it could not be clicked in the panel or the macro strip; it is
`controls::swatch` now, sized and outlined so black reads as a swatch. The
macro strip's XY button opens the Controls page rather than the side panel
on whatever tab it had. The title crumb names the rail's views (PLAY,
SCENES, CONTROLS) instead of PERFORM and LIBRARY; empty states say "click"
rather than "load", centre each wrapped line, and the empty library's
buttons sit under its words; the narrow filter row wraps instead of cutting
off the health menu at 320 points; the meta section's chevron points down
while it is open.

**Tests.** `one_click_on_a_card_makes_it_live_and_a_double_click_is_still_one_load`,
`enter_on_a_focused_card_makes_it_live`,
`the_library_flips_between_scenes_and_controls_by_switch_key_and_palette`,
`the_controls_page_draws_the_controls_once_with_the_preview_beside_them`,
the page's preference round trip, and five new scenarios (the Controls page
with and without a scene, compact, with the panel collapsed, and a focused
card) through the overflow, BLACKOUT and text-over-text tests at every size.

**Would reopen:** the owner wanting a preview step back in the main flow,
the controls wanted beside the grid rather than instead of it, or Ctrl+L
clashing with something an operator relies on.

### D292: the second backend is ANGLE's Vulkan and Metal, not a new renderer (2026-10-15)

This extends D255; D255 stands. The owner asked three things:

- run the G-03 tool over the private Synesthesia corpus;
- check whether a newer naga changes the no-go;
- find out whether ANGLE's other backends could be the second backend.

The evidence is in `docs/research/g03-frontends/README.md` (its 2026-10-15
follow-up) and `docs/research/angle-backends.md`.

**What was found:**

- **The full corpus confirms D255.**
  - The corpus is 120 scenes and 288 passes: the owner's 90 Synesthesia
    scenes, the 16 library scenes and the 14 Shadertoy fixtures.
  - naga's GLSL input takes 0 of 288.
  - glslang takes all 227 corpus passes, and 287 of 288 overall to Vulkan
    SPIR-V. SPIRV-Cross writes HLSL and MSL for all 287.
  - The tool also had to learn to pass a script's uniforms, as
    `Session::load` does, or 43 scripted scenes failed on names the app
    would have declared.
- **naga 30.0.1 is still the newest, and nothing changed.** Trunk has no
  combined-sampler work in either front end.
  - One new route was measured. `spirv-webgpu-transform`, pure Rust,
    splits combined samplers in SPIR-V. After it, naga takes 60 of 60
    library passes but only 56 of 227 corpus passes.
  - The rest fail naga's validator on what its own SPIR-V reader built
    (155 passes: an expression emitted twice). Those are naga bugs, not
    SSF constructs. wgpu stays a no-go.
- **ANGLE's Vulkan backend works with no code change.**
  - It is inside the Electron v38.2.0 build D12 pins, on Windows and
    Linux.
  - With `ANGLE_DEFAULT_PLATFORM=vulkan`, the unmodified
    `strobe-bench --context window --gl angle` rendered all 16 library
    scenes on Mesa's lavapipe. The pictures matched Mesa's own GLES on
    the same rasteriser (identical in 12, feedback noise in 4). Frame
    times were within the container's run-to-run noise: one run was 18%
    slower, an earlier one faster.
  - `strobe-corpus --dialect gles --context window` ran all 90 corpus
    scenes on it. One needed a rerun after the X server refused a
    connection. 47 of 90 checksums are identical to Mesa's, and the same
    7 scenes are flagged all-black or all-white on both.
  - SwiftShader's Vulkan also rendered all 16.
- **ANGLE's Metal backend fits on paper.**
  - It is ES 3.0 conformant, and it is the backend Apple's Safari uses for
    WebGL 2.
  - Everything Strobe's ES path uses is there: float targets, timer
    queries, `KHR_debug`, robustness, PBOs, fences.
  - Electron's darwin zips ship it (`libEGL.dylib`, `libGLESv2.dylib`,
    7 MB, macOS 12+).
- **But two things stand in the way on macOS.**
  - glutin compiles EGL out on Apple targets and never passes ANGLE's
    platform attributes.
  - ANGLE's own default on macOS is its OpenGL backend: GL is compiled in,
    and GL comes before Metal in its fallback order. So the Metal type
    must be requested explicitly.
- **Apple's OpenGL is deprecated (10.14, 2018), frozen at 4.1, and still
  shipped.** No removal has been announced for macOS 26 or 27. WWDC 2026
  ended Intel Macs, not GL. The risk is gradual.

**Decision.** The ranking, with what would trigger each option:

1. **Stay native GL first, with ANGLE-D3D11 as the fallback (today).**
   - Cost: none.
   - This remains the default everywhere it works. D1 and D2 are
     unchanged.
2. **Add ANGLE backend selection: Vulkan now, Metal with macOS.** This is
   the chosen second backend.
   - It gives Vulkan and Metal with no new renderer, no new shader
     pipeline (the ES dialect and `fixups` already pass the corpus) and no
     new egui painter.
   - Cost:
     - small on Windows and Linux: an environment variable today, then a
       `--gl` choice through a glutin patch or a leaf wrapper;
     - moderate on macOS: an EGL provider of our own in `gl/`, with a
       `CALayer` surface, plus fetching, hashing and re-signing the
       dylibs.
   - Risk: a translation layer's overhead on real GPUs, and Metal-specific
     behaviour (feedback passes, RGBA16F precision, present timing), all
     unmeasured until hardware runs it.
   - Triggers:
     - **Vulkan:** the T480 and TORNADO runs (G-05) showing it is at least
       as good as D3D11, or a machine where WGL and D3D11 both fail and
       Vulkan works.
     - **Metal:** any macOS port (G-07). A macOS port needs a new context
       provider anyway (D55), and ANGLE-Metal costs about the same as CGL
       without sitting on a deprecated API.
3. **glslang -> SPIR-V -> SPIRV-Cross, with a new renderer per API
   (G-04).**
   - Cost: large. It is a renderer per API, two C++ dependencies, about
     50 ms per pass of extra compile time, a second egui painter, and the
     seam turned into a trait.
   - Risk: fidelity, which cannot be measured before a renderer exists.
   - Trigger: a need ANGLE cannot meet, such as a D3D12-only or compute
     feature, ES 3.0 limits, or a target with no GLES.
4. **wgpu.**
   - No-go while naga cannot take the corpus, even after the split.
   - Trigger: a rerun of the G-03 tool showing naga's SPIR-V reader taking
     the corpus after `combimgsampsplitter`. Even then, it carries
     glslang.

**Backlog:**

- **G-05:** ANGLE-Vulkan on the T480 and TORNADO.
- **G-06:** a backend-type choice in `--gl`, offered only when ANGLE's
  client extensions advertise it.
- **G-07:** macOS through ANGLE-Metal.
- **G-04** narrows to the case ANGLE cannot serve.

**Would reopen:**

- ANGLE-Vulkan or ANGLE-Metal failing the corpus, or costing frames, on
  real hardware;
- Electron or ANGLE dropping a backend from its builds;
- Apple announcing OpenGL's removal, which moves G-07 up but does not
  change the choice;
- naga's SPIR-V reader taking the corpus.

### D293: global initializers run in `main` on desktop too (2026-10-15)

The corpus scene sprinkles rendered all black on native GL (RX 580, Intel
HD 620, Vega 3) and correctly on ANGLE. Its code declares a global set from
the pixel coordinate, `vec2 uv = _uv;`. Strobe's `_xy`, `_uv` and `_uvc` are
globals that `main` fills, and a desktop driver runs a non-constant global
initializer before `main`, so `uv` was (0, 0) on every pixel. ES never
showed it, because D192 already moves global initializers to the top of
`main`, after the coordinates are set.

`fixups::hoist_global_initializers` now runs for every dialect, with
`Branches::for_dialect(false, …)` on desktop so `#ifdef GL_ES` blocks are
skipped there. D192's argument for equivalence holds unchanged: a global can
only be read from a function, every function runs from `main`, and the
assignments keep their source order. `STROBE_DUMP_GLSL` writes each pass's
assembled source at load, which is how the two dialects were compared.

Found and fixed on the AMD desktop (task A13) and reviewed here.

**Evidence.** `assemble` test
`a_global_initializer_runs_after_the_coordinates_in_every_dialect`, and
`render.rs` `a_global_initialized_from_the_coordinates_sees_them_in_both_dialects`.
On the RX 580, sprinkles is no longer all black on desktop, and the
90-scene corpus stays 90/90 in both dialects with no other change to the
all-black lists.

**Not explained by this:** PLUS, black on ES only. After 600 frames sprinkles
on ANGLE shows a small centre ring where native fills the frame; that
divergence predates this change and is not yet understood.

**Would reopen:** a driver that rejects a global declared without an
initializer and assigned in `main` (none known), or a scene that relies on
a global initializer running before `main` (GLSL has no way to observe
that).

### D294: a laptop controls a venue's show machine with a trust file, and keeps reconnecting until told to stop (2026-10-15)

**Context.** D288 built the host and the protocol and left the controller to
D286's viewer stages. A four-hour show on a venue's Wi-Fi sees the host
restart, the access point drop the laptop for a few seconds, and the mini PC
come up after the laptop. A controller that needs a click to come back, or a
code the host no longer has, ends the show's remote control the first time it
blinks.

**Decision.**

- **Two ways in, one worker.** *Settings > Control another Strobe* has a
  checkbox, **Use a trust file (venue mode)**; ticked, the pairing-code field
  becomes a path field and Connect runs trust pairing instead. There is no
  file picker: the app has no file-dialog helper and a dialog would need a
  new dependency or PowerShell, so the path is typed or pasted (quotes from
  Windows' Copy as path are stripped). On the command line,
  `strobe --control HOST:PORT --remote-trust FILE` opens straight into the
  viewer; a bare IP takes port 47812. `--control` without the file opens the
  form with the address filled in and the trust option **off**, for a code: a
  remembered file never connects by itself. `--remote-trust` now needs
  `--remote` *or* `--control` (a clap group); with `--remote` it is the host's
  file as before, and the host reads it only when `--remote` is given.
  `--control` conflicts with `--remote`: one run is a host or a controller.
- **What is remembered.** `ui.toml` gets `[remote]`: the trust file's path,
  whether the form uses it, and `controller_id`. The secret is read from the
  file each time Connect is pressed and lives only in the connection's
  worker; it is never saved, logged, shown or in the viewer's text
  (`TrustSecret` prints as `***`, as D288 made it). The controller id is
  `computer-name-<8 hex of OS randomness>`, made once on first launch and
  kept, so the host's list and its revoke-by-name refer to one laptop for
  ever; it is cut to 48 bytes of `[A-Za-z0-9.-]` by `trust::controller_id_for`.
- **The proof is made only for our own question** (the security review's
  condition). `trusted::Handshake` is owned by one connection. It sends
  `trust_start` once, when the host's `hello` arrives, and answers exactly
  one `trust_challenge` after it. A challenge that arrives before our
  `trust_start`, on a connection that pairs by code, or a second one, is
  refused: the connection ends as *refused*, no proof is computed, and the
  viewer says the host sent one nobody asked for. So the controller can
  never be used to sign a nonce taken from somewhere else. The `host` in
  the proof is `client::host_header(address)`, the very function that
  writes the `Host` header, so the two cannot differ.
- **Reconnecting is a Stage, not a second path.** `Stage::Reconnecting(n)`
  sits beside Connecting, Live and Ended. The worker that owns a trusted
  connection loops: connect, and when it ends without the host having said
  no (a closed socket, a timeout, connection refused, a pause for failed
  proofs), set `Reconnecting(n)`, wait `trusted::backoff(n)` and try again.
  The schedule is 1 s, 2 s, 4 s, 8 s, then 15 s for ever. A connection that
  brought the host's picture starts it again at attempt 1; one that never got
  that far keeps counting. Being paired is not enough (review fix): a second
  laptop that the host pairs and then refuses the lease, because another
  controller holds it, would otherwise pair and drop once a second for the
  whole show. The viewer keeps the last image, dimmed and read-only (D286),
  under a card that says "Reconnecting... (attempt N)" and why, and the
  toolbar button reads **Disconnect**. The wait is in 50 ms slices that look
  at the stop flag, so Disconnect ends it at once; input typed while away is
  dropped, never replayed.
- **A refusal stops it, and says why.** Wrong file, revoked, trusted pairing
  unavailable (host without the file or with desktop control off), a bad
  request, a protocol mismatch, an unanswered challenge: `Ended(Refused)`
  with the next step in words, a **Reconnect** button (which proves the file
  again), and nothing retries. Retrying a wrong file would only feed the
  host's failed-proof limiter (D288). Only `busy` (the host pausing after
  failures, which ends by itself) is retried. A kill-switch drop on the host
  is retried once and then shows "does not offer trusted pairing right now".
- **Code-paired connections are unchanged**: they end in `Ended` and offer a
  manual Reconnect (D275, D286).
- **The retry costs a four-hour show nothing.** The loop sleeps in slices,
  holds no lock while asleep, runs on the worker thread (never the frame
  loop), and logs attempts 1 to 4 and every 20th, so a night of retries is
  a few lines.

**Evidence.** Unit tests: the proof against the protocol vector from D288
and against the host's own `Challenge::verify` (accepted for the host
dialled, refused for another address or port); an unsolicited, a repeated
and a malformed challenge refused; the backoff schedule as a pure function;
controller ids for odd computer names; the trust-file loader's sentences;
`--control` and `--remote-trust` parsing (both roles, the refused
combinations); the launch decisions (`Controller::at_launch`). Loopback tests
with a real host (`desktop::tests`): a trusted connect with no approval and
the label *trusted*; a wrong file refused once, the worker ended after the
first backoff would have fired; the host stopped and restarted on the same
port with the controller back by itself and the lease taken again; Disconnect
with the host back on the same port, and nobody connects; a host that
volunteers a challenge is not answered; a second laptop shut out of the lease
backs off rather than retrying every second; the kill switch retried once and
then refused. Shell tests for the form's option and
viewer states for Reconnecting and a refused file.

**Not verified:** two physical machines, a real Wi-Fi drop, Windows, a
four-hour soak. The retry schedule is the decision, not a measurement.

**Would reopen:** TLS (D288's list); a controller that should also retry a
refusal after a long wait (a host brought up late with the switch off); a
file picker if a dialog helper is added.

### D295: Linux is a documented platform: three build packages, a Nix flake, dlopen'd libraries on `LD_LIBRARY_PATH` (2026-10-15)

D291-D294 were taken on other branches; this entry is D295 so the
numbers do not collide at merge.

**Context.** CI's Linux job already built and tested Strobe, but the packages
it installs were never explained, the README said only "checked under Xvfb",
and a Nix user (Bonzupii) offered a dev-shell flake from another project. His
own note was that it was a dev shell, not an install flake, and that some of
its graphical dependencies were bloat for his game. The owner wants Linux in
the README and the repo, and the AMD desktop runs Debian on WSL2.

**What Strobe needs on Linux, from `Cargo.lock`, the build scripts and
`ldd`/`LD_DEBUG` on a real run.**
- **Linked (build time, headers and a `.pc` file): ALSA only.** `ldd` on the
  release binary lists `libasound.so.2` and libc. `alsa-sys` (cpal for
  capture, midir for MIDI) runs `pkg-config alsa`. Plus a C compiler:
  `rquickjs-sys` (QuickJS) and `ring` (rustls, for Shadertoy import) compile C.
  `build-essential pkg-config libasound2-dev` was enough on a clean Ubuntu
  24.04 (`debootstrap`, no `libxkbcommon-dev`, no Wayland headers, no clang, no
  git: `build.rs` reports the commit as `unknown` without it).
- **Loaded with `dlopen` (run time, nothing in `DT_NEEDED`):** libGL, libEGL
  and libGLX (glutin; the GPU's driver behind them), libX11, libXcursor, libXi,
  libXrandr and libxcb (winit through `x11-dl`), libxkbcommon and
  libxkbcommon-x11 (`xkbcommon-dl`), libwayland-client, -cursor and -egl
  (winit's `wayland-dlopen`, glutin). A build on a machine without them
  succeeds; the window fails to open. On Nix they must be on `LD_LIBRARY_PATH`.
- **Not needed:** OpenSSL (TLS is rustls), fontconfig and freetype (egui's fonts
  are compiled in), udev (MIDI is ALSA's sequencer), Vulkan (no wgpu, D1),
  libclang. Bonzupii's `vulkan-loader` and `cargo-llvm-cov` are dropped for
  that reason; his libxkbcommon, wayland, libGL, X11 and alsa-lib stay, and
  libxcb is added.

**Decision.**
- **`flake.nix` at the root** with `forAllSystems` over x86_64-linux and
  aarch64-linux.
  - `devShells.default`: nixpkgs' `cargo`, `rustc`, `clippy`, `rustfmt`,
    `pkg-config`, `git`, ALSA in `buildInputs`, and `LD_LIBRARY_PATH` from the
    dlopen list. **The toolchain is nixpkgs', not rust-overlay or fenix.** The
    pin in `rust-toolchain.toml` is 1.98.0, nixpkgs-unstable has 1.98.1 and the
    workspace's `rust-version` is 1.98, so a second toolchain source buys
    nothing. The shell ignores `rust-toolchain.toml` (no rustup), so
    `cargo xtask ci` skips the Windows cross-check there, and clippy is 1.98.1
    where CI is 1.98.0. If nixpkgs falls behind the pin, use
    `rust-bin.fromRustupToolchainFile`.
  - `devShells.software`: the default shell plus nixpkgs' Mesa, with
    `__EGL_VENDOR_LIBRARY_DIRS` and `LIBGL_DRIVERS_PATH` set, for a machine
    with no GL of its own (a VM, a container, CI). **Mesa is not in the default
    shell**: forcing it would override the system's driver (NVIDIA's
    proprietary one, or a newer Mesa) on a real GPU. libglvnd, which is what
    `libGL` is in nixpkgs, already looks in `/run/opengl-driver`, where NixOS
    puts the driver.
  - `packages.default`: `rustPlatform.buildRustPackage` with
    `cargoLock.lockFile` (every dependency is from crates.io, so no
    `outputHashes`), building `strobe-app` and `strobe-bench` in one
    invocation, `doCheck = false` (the tests need a GL context and walk the
    source tree; CI's Linux job is where they run). `makeWrapper` puts the
    dlopen list on `LD_LIBRARY_PATH` for `strobe`, `strobe-bench` and
    `strobe-corpus`. The scene library is installed to `share/strobe/scenes`
    and linked as `bin/scenes`, which is where `library::resolve` looks "beside
    the program" for the packaged build. `STROBE_GIT_HASH` carries the flake's
    revision, since a Nix build has no repository for `build.rs` to ask.
  - `strobe-bench` has no such lookup, so it needs `--library
    result/share/strobe/scenes`.
- **`flake.lock`.** `api.github.com` and `codeload.github.com` are blocked in
  the agent container, so `nixos-unstable` could not be fetched as `github:`.
  The lock was made by fetching the same revision (`7fd4004d0511`, the
  head of `nixos-unstable` on 2026-10-15, which the channel's tarball link also names) with `git+https://github.com/NixOS/nixpkgs`
  and rewriting the node to `type: github` with the same `rev`, `narHash` and
  `lastModified`. nixpkgs has no `export-ignore` or `export-subst`, so the git
  checkout and GitHub's tarball are the same tree, and Nix accepted the
  rewritten lock and matched the `narHash` against that store path; the
  GitHub tarball itself was not fetched. If `nix flake metadata` ever
  disagrees, `nix flake update` regenerates it.
- **README "Linux" section**, with Debian and Ubuntu packages (verified), Fedora
  and Arch names (checked against the Fedora 43 and Arch package databases,
  not built), the Nix route, how to choose Wayland or X11, graphics, audio and
  MIDI, and WSL2. `docs/COMPATIBILITY.md` gets four rows,
  `docs/USER_GUIDE.md` and `CONTRIBUTING.md` point to it.
- **Wayland or X11.** winit 0.30 picks Wayland when `WAYLAND_DISPLAY` or
  `WAYLAND_SOCKET` is non-empty, else X11 when `DISPLAY` is set.
  `WINIT_UNIX_BACKEND` no longer exists and Strobe adds no flag, so the way to
  force one is `env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET` or `env -u DISPLAY`.
- **ANGLE is Windows-only.** On Linux `--gl angle` is the system's EGL with an
  OpenGL ES 3.0 context, which is how the GLSL ES dialect is exercised, and
  `--gl auto` falls back to it.

**What was checked (2026-10-15, cloud container, no GPU, llvmpipe).**

| Claim | Result |
|---|---|
| Debian/Ubuntu build list | Clean Ubuntu 24.04 chroot, `build-essential pkg-config libasound2-dev`, `cargo build --release -p strobe-app` and `-p strobe-bench`: built. |
| Debian/Ubuntu run list | Same chroot plus `libegl1 libegl-mesa0 libgl1-mesa-dri`: `--diagnose` and the headless bench ran (16 scenes, Mesa 24.0.5). Plus the X libraries and Xvfb: control and output screenshots on X11 with desktop GL 3.3 core. Plus Weston headless and the Wayland libraries: both screenshots on Wayland. |
| Flake, x86_64-linux, Nix 2.34 | `nix develop -c cargo build -p strobe-app -p strobe-bench`: built. `nix build`: built. The result, from another directory: `--help`, `--diagnose` (Mesa 26.2.4), headless bench with `--library`, a screenshot under nixpkgs' Xvfb and under nixpkgs' Weston, the scene library found beside the program. `nix run . -- --help`. |
| Flake, aarch64-linux | `nix eval` of the package and both shells succeeds. Not built. |
| Default dev shell on this non-NixOS host | `--diagnose` finds no GL context: expected, there is no driver for libglvnd to load. `.#software` is the answer. |

**Not verified:** NixOS itself (`hardware.graphics`, the ALSA plugin path), any
real Linux GPU, ALSA, PipeWire or PulseAudio capture, MIDI hardware, GNOME or
KDE, WSL2 and WSLg (the hand-off has a test script for the AMD desktop's
Debian), and Fedora and Arch builds.

**Not chosen.**
- *rust-overlay or fenix*, for the reason above.
- *Mesa in the default shell*, for the reason above.
- *`patchelf --add-rpath` instead of a wrapper.* It would work for libraries
  that glutin and winit `dlopen` from the executable, but the request was a
  wrapper and a wrapper also covers the helper processes the app starts.
  The cost is that `LD_LIBRARY_PATH` is inherited by children (an opened link
  starts `xdg-open` with it), which has not caused a problem so far.
- *crane or naersk.* `buildRustPackage` is nixpkgs' own and the lock file has
  no git dependencies.

**Would reopen:** a nixpkgs bump that moves `rustc` below 1.98, a git
dependency in `Cargo.lock` (needs `outputHashes`), a dlopen'd library
the list misses (a window that will not open on a clean Nix or Debian box),
or a real Linux GPU that needs more than `hardware.graphics.enable`.
