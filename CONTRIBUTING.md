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
cargo test --workspace
cargo run --release -p strobe-bench -- --size 1280x720   # renders scenes/ headless
```

On Linux, the renderer tests and the bench need Mesa's EGL:
`apt-get install -y libegl1 libegl-mesa0`.

## Before you submit

One command runs everything CI runs, in CI's order, and stops at the
first failure:

```bash
cargo xtask ci
```

It is these, plus `cargo deny check` and the changelog check:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo clippy --workspace --all-targets --all-features --target x86_64-pc-windows-msvc -- -D warnings
```

A step that cannot run on your machine (the Windows target not
installed, `cargo-deny` missing) is reported as skipped with the command
that would enable it, never silently dropped. CI runs the checks on
Windows, Linux and macOS. A warning is a build failure.

### What CI can and can't prove

The Linux job compiles and renders every test shader on a real GLSL
compiler (Mesa llvmpipe), in both the desktop GL 3.3 and GLES 3.0
dialects. The Windows job proves the build, the context code, and the
release settings compile and link. **Neither runner has a GPU or a
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

## Releasing

Releases are tags, and the changelog is finished *before* the tag exists.
In this order:

1. On a release branch off `main`: bump `version` in the workspace
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

The tag push triggers `.github/workflows/release.yml`. It checks the tag
against `Cargo.toml`, creates a **draft** release whose notes are the
changelog section, builds `strobe.exe` on Windows, and packages
`strobe-<tag>-x86_64-pc-windows-msvc.zip` (the exe, the three ANGLE DLLs
and `LICENSES.chromium.html`, `scenes/`, README, LICENSE). Alongside it
go a separate `-symbols.zip` holding `strobe.pdb` (keep it: it is what
symbolicates a crash minidump from that exact build), a CycloneDX SBOM,
and SHA-256 checksums. A final job attests build provenance and the SBOM
(`gh attestation verify <file> -R lnorton89/strobe`) and only then
publishes the draft, so a failed build leaves an unpublished draft and
not a half-populated public release.

**Never move or re-push a tag, and never amend or rebase a commit after
tagging it.** The tag keeps pointing at the copy you discarded, which
then sits on no branch at all, and since each release is computed as
`<previous>..<tag>`, every range from that point on is wrong. The
changelog check fails on any version tag no branch can reach. A second
push of a `v*` tag also re-runs the release workflow, which ends in
`gh release upload --clobber`, rebuilding a published release's assets
under new checksums. If a release is wrong, cut the next patch version.

ANGLE is pinned to Electron 38.2.0 by `scripts/fetch-angle.ps1`, which
the release workflow runs. Do not bump it as a drive-by: newer Electron
no longer ships the DLLs (see `docs/AGENT_RESEARCH.md`, section 8).

## Dependency updates

Dependabot opens weekly PRs for crates and GitHub Actions
(`.github/dependabot.yml`). Only patch bumps are grouped; for the
egui/glow/glutin/winit stack the PRs for individual crates will not build
alone, so take them together as one deliberate upgrade. Actions are
pinned to full commit SHAs with a version comment, which
`strobe-conventions` enforces; `dtolnay/rust-toolchain` and
`rust-toolchain.toml` move together, by hand.
