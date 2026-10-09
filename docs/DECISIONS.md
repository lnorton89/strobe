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
