# Contributing to Strobe

Strobe is a Synesthesia-style VJ engine in Rust: a GL renderer for
audio-reactive GLSL scenes, built to run well on old laptops.

Start with [`CLAUDE.md`](CLAUDE.md), which covers the commands, the
rules the build enforces, and the map of planning docs. Then read
[`docs/ROADMAP.md`](docs/ROADMAP.md) to see what's in flight.

## Reporting bugs

Use the bug report form on the
[issue tracker](https://github.com/lnorton89/strobe/issues). For a
graphics problem it asks for your GPU and driver version, which context
backend ran (native OpenGL or the ANGLE fallback), the scene, and the
render resolution, because the same scene can be right on one and wrong
on the other. Attach the output of `strobe --diagnose`, the newest crash
report from `%LOCALAPPDATA%\strobe\crashes\` if it crashed, and the
latest log from `%LOCALAPPDATA%\strobe\logs\`. Security vulnerabilities
are *not* reported there; see [`SECURITY.md`](SECURITY.md).

## Building and running

The toolchain is pinned in `rust-toolchain.toml`, and rustup fetches it
automatically.

```bash
cargo run -p strobe-app                               # debug: quick to rebuild
cargo run --profile release-fast -p strobe-app        # optimised, incremental: the one to iterate with
cargo run --release -p strobe-app                     # what users get: fat LTO, slow to rebuild
cargo test --workspace
cargo run --release -p strobe-bench -- --size 1280x720   # renders scenes/ headless
```

`--release` rebuilds slowly by design: fat LTO over one codegen unit
re-optimises every dependency after any edit, about five minutes on a
fast desktop and longer on a laptop. `release-fast` (D487) has the same
optimisation level, `panic = "abort"` and symbols without the LTO, and
rebuilds in seconds. Measured on TORNADO (GTX 1080 Ti desktop) after a
one-character edit in `strobe-app`, up to a linked binary: `--release`
290 s, `release-fast` 9 s, debug 5 s; a clean `release-fast` build is
about two minutes. It is not the shipped binary, so benchmark and release
with `--release`.

Two more things help on Windows, and both are your call rather than the
repository's (as in rustaman):

- **`rust-lld`**, which ships with the toolchain. Set
  `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=rust-lld.exe`, or put
  `linker = "rust-lld.exe"` under `[target.x86_64-pc-windows-msvc]` in your
  user-level `~/.cargo/config.toml`. It still needs the MSVC build tools.
  It is not set here because CI and the release build use `link.exe`, and
  a linker change there would change what ships.
- **Keep `target/` out of real-time antivirus scanning**: a Dev Drive, or a
  Defender exclusion for the checkout. A build writes thousands of files,
  and scanning each one can cost more than compiling it.

On Linux, the renderer tests and the bench need Mesa's EGL:
`apt-get install -y libegl1 libegl-mesa0`. Building the app also needs
`build-essential pkg-config libasound2-dev`. The README's Linux section has
the full package list, and `nix develop` (see `flake.nix`) provides all of it.

## Before you submit

One command runs what CI runs, and stops at the first failure:

```bash
cargo xtask ci                 # every lane CI runs on this kind of host
cargo xtask ci check test      # just those lanes
cargo xtask ci doctor          # what can run here, and how to install the rest
```

CI's jobs run the very same lanes, `cargo --locked xtask ci --strict
<lanes>`, so the commands are written once, in `crates/xtask/src/ci.rs`
(D533): `fmt`, `check` (clippy; on Linux also `strobe-io` without MIDI
and the Windows cross-check), `test`, `deny`, `generated` (the changelog,
icon and credits checks), `render` (Linux: the app on Xvfb and the scene
library in both dialects) and `build` (Windows: the optimised binaries).

A step that does not apply to your host says `n/a`, as CI's job for that
host would. One whose tool is missing (the Windows target, `cargo-deny`,
`xvfb-run`) says `MISSING` with the command that installs it, and the run
ends as "partial", never as a pass. `--strict`, which CI passes, fails it
instead, and also fails a run that changed a tracked file. CI runs the
lanes on Windows, Linux and macOS, and its `ci-ok` check passes only when
every job that should have run passed. A warning is a build failure.

### What CI can and can't prove

The Linux job compiles and renders every test shader on a real GLSL
compiler (Mesa llvmpipe), in both the desktop GL 3.3 and GLES 3.0
dialects. The Windows job proves the build, the context code, and the
optimised settings compile and link: a pull request builds
`release-fast`, and the shipping `release` build (fat LTO, about six
minutes on a runner) runs once the change is on master, and again for a
tag (D504). **Neither runner has a GPU or a
display.** Anything visual, and anything that depends on a real driver
(WGL, ANGLE, a real GPU's timing), needs a run on a real machine, and a
screenshot or bench result in the pull request.

## Rules the codebase enforces

Lints and `crates/strobe-conventions` fail the build for:

- a source file without the module header banner, or with a `Module:`
  that doesn't match its path;
- `unwrap`, `expect`, `panic!`, `todo!`, or `unreachable!` anywhere,
  including tests;
- `unsafe` outside `crates/strobe-gfx/src/gl/`, more than one unsafe
  operation in a function, a `pub unsafe fn`, or unsafe in a test;
- `#[allow]` (use `#[expect(lint, reason = "…")]`) and wildcard enum
  match arms;
- a CI workflow whose Rust version disagrees with `rust-toolchain.toml`,
  an action not pinned to a full commit SHA with a version comment, or a
  workflow with no top-level `permissions:`.

## Scenes

`scenes/` is the base library and the benchmark set. Only commit scenes
you wrote, or scenes whose license allows redistribution, with the
license recorded in `scene.json`'s `CREDIT`/`DESCRIPTION`. If you change
a scene, re-run the bench and update `docs/BENCHMARKS.md`.

The corpus gate (D534) holds every library scene to an expected status in
`crates/strobe-app/tests/corpus-gate/library.json`, so a new scene needs a
line there (`"ok"` in both dialects), and a removed one loses its line. For
the scenes it lists under `images`, the final frame must also match the
baseline in `library.baselines/`. A change that is meant to alter one of
those frames takes the new frame from the `corpus-gate` artefact of the
pull request's CI run (`frames/<scene>.<dialect>.png`), since the baselines
are what Mesa's llvmpipe draws there. `strobe-corpus --expect <file>
--bless` rewrites the file and baselines from a local run, for when the
local driver is the one being checked.

## Branches

`master` is the default branch and always passes `cargo xtask ci`.

- **Outside contributors and dependabot:** work on a topic branch and
  reach `master` through a pull request whose checks pass.
- **The owner's agent sessions:** develop on their `claude/*` branch, then
  fast-forward `master` to it once CI passes there (D121).

Delete a branch once it is merged. Never force-push `master`, and never
move a release tag (see Releasing).

## Commit messages

Use conventional prefixes: `feat:`, `fix:`, `perf:`, `refactor:`,
`docs:`, `test:`, `build:`, `ci:`. They are not decoration: they build
`CHANGELOG.md`.

## The changelog

`CHANGELOG.md` is **generated** from the commit history by
`cargo xtask changelog`: every entry is a conventional-commit subject,
grouped under the release tag that shipped it, so do not edit it by
hand (released sections are rewritten on every run). A subject that does
not parse still appears, but under "Other". Merge commits are not listed;
the commits they bring in are. CI runs the check form:

```bash
cargo xtask changelog --check
```

It compares the **released** sections only. `Unreleased` is exempt on
purpose: it moves with every merge, and a rebase or squash rewrites the
very hashes it pins.

A subject that turned out wrong after it was pushed (a decision number
that moved before the merge, say) is not fixed by rewording the commit,
which would rewrite every hash after it. Add a line to
`CHANGELOG.corrections` instead, `<full hash> <corrected subject>`, and
the generator uses that subject (D544).

## Releasing

Releases are tags, and the changelog is finished *before* the tag exists.
In this order:

1. On a release branch off `master`: bump `version` in the workspace
   `Cargo.toml` (`[workspace.package]`) and commit.
2. Write the release's section: `cargo xtask changelog --release v0.2.0`,
   and commit it. The generator refuses a version that does not match
   `Cargo.toml`, already has a tag, or is not newer than the latest
   release.
3. Open the pull request and let the checks pass. The changelog check
   recognises a leading section whose tag does not exist yet and
   validates it as the release being cut, rather than calling it drift.
4. Merge (a merge commit, never a squash, since the changelog pins
   individual commit hashes), then tag the merge commit, annotated:
   `git tag -a v0.2.0 -m "strobe v0.2.0"`, and push the tag.

For the first tag, [`docs/RELEASE_PLAN.md`](docs/RELEASE_PLAN.md)
sequences this order around the checks the workflow has never exercised,
including a dry run on a throwaway tag before the real one exists.

CI builds and checks the package on every pull request (the Windows job's
`package` lane, D537), but a runner has no GPU. Before publishing, run the
draft's own archive on the T480:

```bash
cargo xtask rc --zip strobe-v0.2.0-x86_64-pc-windows-msvc.zip --symbols strobe-v0.2.0-x86_64-pc-windows-msvc-symbols.zip
```

It runs the package check, draws on native GL and ANGLE-D3D11, crashes the
program on purpose and symbolicates the dump against the archived PDB, and
writes `target/rc/evidence.json`. That file goes with the T480's record in
`docs/hardware/`. The checks it lists as manual are still done by hand.

The tag push triggers `.github/workflows/release.yml`. It resolves the tag
to one commit and works from that SHA throughout, checks the tag
against `Cargo.toml`, runs all of `ci.yml` on that commit (D521), creates a **draft** release whose notes are the
changelog section, builds `strobe.exe` on Windows, and packages
`strobe-<tag>-x86_64-pc-windows-msvc.zip` (the exe, the three ANGLE DLLs and the Vulkan loader for `--gl angle-vulkan`,
and `LICENSES.chromium.html`, `scenes/`, README, LICENSE, `THIRD_PARTY.md`, and
`THIRD-PARTY-NOTICES.txt` with every crate's licence text, written by
`cargo xtask credits --notices`). Alongside it
go a separate `-symbols.zip` holding `strobe.pdb` (keep it: it is what
symbolicates a crash minidump from that exact build), a CycloneDX SBOM,
and SHA-256 checksums. A final job attests build provenance and the SBOM
(`gh attestation verify <file> -R lnorton89/strobe`) and only then, once
the build and every CI job have passed, publishes the draft, so a failed
build or a red test leaves an unpublished draft and not a half-populated
or untested public release. Re-running the workflow resumes a draft; it
refuses a release that is already published. To run it by hand, start it
from the tag itself, so the provenance names the commit that was built:
`gh workflow run release.yml --ref v0.2.0 -f tag=v0.2.0`.

**Never move or re-push a tag, and never amend or rebase a commit after
tagging it.** The tag keeps pointing at the copy you discarded, which
then sits on no branch at all, and since each release is computed as
`<previous>..<tag>`, every range from that point on is wrong. The
changelog check fails on any version tag no branch can reach. A second
push of a `v*` tag also re-runs the release workflow; since D521 that run
stops before building if the release is already published, but a tag
moved while its draft is still unpublished rebuilds the draft from the new
commit. If a release is wrong, cut the next patch version.

ANGLE is pinned to Electron 38.2.0 in `scripts/native-deps.json` (D536):
version, source, and the archive's and each DLL's SHA-256. Everything
that touches the DLLs reads that one file:
- `scripts/fetch-angle.ps1`, which the release workflow runs;
- `strobe-gfx`'s build script, which runs the fetch script on a Windows dev
  build whose DLLs are missing, or whose `angle.stamp` names another
  version (D498);
- the release's SBOM, through `cargo xtask sbom-native`;
- `strobe --diagnose`, which says whether the DLLs beside the program match
  the pin.

A bump is one commit to the manifest, and every profile's DLLs refresh on
its next build. The Scheduled workflow checks the pinned archive weekly
and names newer releases in the same line. Do not bump it as a drive-by:
newer Electron no longer ships the DLLs (see `docs/AGENT_RESEARCH.md`,
section 8).

## Dependency updates

Dependabot opens weekly PRs for crates and GitHub Actions
(`.github/dependabot.yml`). Only patch bumps are grouped; for the
egui/glow/glutin/winit stack the PRs for individual crates will not build
alone, so take them together as one deliberate upgrade. Actions are
pinned to full commit SHAs with a version comment, which
`strobe-conventions` enforces; `dtolnay/rust-toolchain` and
`rust-toolchain.toml` move together, by hand.

A bump that adds, removes or relicenses a crate changes `THIRD_PARTY.md`
(the About page's credits, D491): run `cargo xtask credits` and commit the
result, or CI's `credits --check` fails. A patch bump alone changes nothing
there.
