# Contributing to Strobe

Strobe is a Synesthesia-style VJ engine in Rust: a GL renderer for
audio-reactive GLSL scenes, built to run well on old laptops.

Start with [`CLAUDE.md`](CLAUDE.md), which covers the commands, the
rules the build enforces, and the map of planning docs. Then read
[`docs/ROADMAP.md`](docs/ROADMAP.md) to see what's in flight.

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

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo clippy --workspace --exclude strobe-script --all-targets --all-features --target x86_64-pc-windows-msvc -- -D warnings
```

CI runs these on Windows and Linux, plus `cargo deny check`. A warning
is a build failure.

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
- a CI workflow whose Rust version disagrees with `rust-toolchain.toml`.

## Scenes

`scenes/` is the base library and the benchmark set. Only commit scenes
you wrote, or scenes whose license allows redistribution, with the
license recorded in `scene.json`'s `CREDIT`/`DESCRIPTION`. If you change
a scene, re-run the bench and update `docs/BENCHMARKS.md`.

## Commit messages

Use conventional prefixes: `feat:`, `fix:`, `perf:`, `refactor:`,
`docs:`, `test:`, `build:`, `ci:`.
