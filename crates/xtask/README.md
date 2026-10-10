# xtask

The workspace's own tooling, behind the `cargo xtask` alias: a local runner
for everything CI runs, the generator for `CHANGELOG.md`, and a shortcut for
running the benchmark in release mode. It is a normal workspace crate, so it
is built, linted and tested by the same `--workspace` commands as the rest.

## Role in the workspace

- **Depends on:** `clap` and `anyhow` only. It shells out to `cargo`, `rustc`
  and `git`, and optionally `cargo-deny`.
- **Depended on by:** nothing. It is a binary with no library API.
- **Does not:** link against any other Strobe crate, or parse
  `strobe-bench`'s flags (they are forwarded untouched). The `xtask` alias in
  `.cargo/config.toml` (`xtask = "run -p xtask --"`) is the whole install.

Every command runs from the workspace root regardless of where it is invoked
(the root is fixed at compile time from the crate's manifest directory), so
`cargo xtask ci` works from inside `crates/strobe-gfx`.

## Main types / entry points

| Command | What |
|---|---|
| `cargo xtask ci` | Runs locally, in CI's order, and stops at the first failure: `rustfmt`, `clippy`, `test`, the Windows cross-check clippy, `cargo-deny`, and the changelog check. |
| `cargo xtask changelog` | Regenerates `CHANGELOG.md` from conventional-commit subjects. |
| `cargo xtask bench -- <args>` | `cargo run --release -p strobe-bench -- <args>`. |

The code is in `src/main.rs` (the clap `Cli`), `src/ci.rs`, `src/changelog.rs`
and `src/bench.rs`.

## Usage

```bash
cargo xtask ci                                # everything CI runs
cargo xtask ci --allow-gl-skip                # on a machine with no headless EGL
cargo xtask ci --no-deny                      # skip cargo-deny (it needs the network)
cargo xtask changelog                         # regenerate CHANGELOG.md
cargo xtask changelog --check                 # what CI runs; compares released sections
cargo xtask changelog --release v0.2.0        # write a release's section before tagging
cargo xtask bench -- --size 1280x720          # strobe-bench, in release
```

Flags, from the clap definitions:

- `ci`: `--allow-gl-skip` and `--no-deny`.
- `changelog`: `--check` and `--release <vX.Y.Z>`, which conflict with each
  other.
- `bench`: everything after `--` goes to `strobe-bench`.

Environment: `cargo xtask ci` sets `STROBE_REQUIRE_HEADLESS_GL=1` for the test
step on Linux (unless you already set it or pass `--allow-gl-skip`), so a
missing Mesa EGL fails loudly instead of every renderer test skipping. It also
honours `CARGO` and `RUSTC` if set.

Some steps skip with a stated reason and the command that enables them: the
Windows clippy when the `x86_64-pc-windows-msvc` standard library is not
installed (`rustup target add x86_64-pc-windows-msvc`; `rust-toolchain.toml`
lists the target) or the host is already that target, and `cargo-deny` when it
is not installed (`cargo install cargo-deny --locked`). The Windows step
excludes `strobe-script` and `strobe-app` (D63).

`CHANGELOG.md` is generated: never edit it by hand. The release order is in
[`CONTRIBUTING.md`](../../CONTRIBUTING.md): bump the version, write the
section with `--release`, merge, then tag, and never move a tag.

## Testing

```bash
cargo test -p xtask
```

The ten unit tests cover the changelog generator (subject parsing, heading
placement, the `Unreleased` exemption, version parsing, the calendar
conversion, reading `[workspace.package]`). They do not run git or cargo.
`cargo xtask ci` itself is exercised by running it.

## Design notes

- **A crate, not examples or scripts** (D50): workspace tooling would
  otherwise be parked in one package, and a shell script would not run the same
  on Windows.
- **`CHANGELOG.md` is generated from conventional commits** (D51): released
  sections only are checked, changelog-only commits and merge commits are not
  listed, and the repository URL is read from `[workspace.package]`.
- **Draft, build, attest, undraft** (D52) is the release workflow the
  changelog feeds; the tag's section becomes the release notes.
- **Skipped is never silent**: `ci` reports a step it cannot run as skipped
  with the reason, so a green run means what CI means.
- **Windows cross-check excludes the app** (D63), because `strobe-app` depends
  on `strobe-script`, whose C sources need the MSVC toolchain.

## Status and known gaps

- Working and used on every change ([`ROADMAP.md`](../../docs/ROADMAP.md)).
- The release workflow itself has not run end to end: it only runs on a tag
  (Q12 in [`OPEN_QUESTIONS.md`](../../docs/OPEN_QUESTIONS.md)).
- `cargo xtask ci` mirrors the Linux job's steps; CI's other jobs (the
  Windows build, macOS, the app screenshot under Xvfb, the bench frames
  upload) are not reproduced locally.
