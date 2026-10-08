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
