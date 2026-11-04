# xtask

The workspace's own tooling, behind the `cargo xtask` alias: a local runner
for everything CI runs, the generators for `CHANGELOG.md`, `THIRD_PARTY.md`,
`docs/DECISIONS_BY_AREA.md` and the icons, the release package and its
checks, and shortcuts for the benchmark, the corpus and symbolication. It is a normal workspace crate, so it
is built, linted and tested by the same `--workspace` commands as the rest.

## Role in the workspace

- **Depends on:** `clap`, `anyhow`, `serde_json` (`cargo metadata` for
  `credits`), `sha2` (the native DLLs' hashes, D536) and `resvg` (the
  icons). It shells out to `cargo`, `rustc` and `git`, and optionally
  `cargo-deny`.
- **Depended on by:** nothing. It is a binary with no library API.
- **Does not:** link against any other Strobe crate, or parse
  `strobe-bench`'s flags (they are forwarded untouched). The `xtask` alias in
  `.cargo/config.toml` (`xtask = "run -p xtask --"`) is the whole install.

Every command runs from the workspace root regardless of where it is invoked
(two folders above the crate's manifest directory, as `cargo run` names it
at run time, so two checkouts sharing a target directory each check their
own tree), so `cargo xtask ci` works from inside `crates/strobe-gfx`. A path you
type is still taken from where you typed it.

## Main types / entry points

| Command | What |
|---|---|
| `cargo xtask ci [lanes] [--strict]` | The commands CI runs, as named lanes (`fmt`, `check`, `test`, `deny`, `generated`, `render`, `build`, `package`, `lint`, `conventions`; `all` by default) with each host's policy in Rust. CI's jobs call it with `--strict`; `doctor` lists what this machine can run (D533). Stops at the first failure. |
| `cargo xtask changelog` | Regenerates `CHANGELOG.md` from conventional-commit subjects. |
| `cargo xtask bench -- <args>` | `cargo run --release -p strobe-bench -- <args>`. |
| `cargo xtask corpus [--profile p] -- <args>` | Builds `strobe-corpus` and runs it over a folder of scenes (D195, D534). |
| `cargo xtask symbolicate <dump>` | A minidump as a stack with function names and `file:line`. |
| `cargo xtask brand`, `credits`, `decisions` | The icons from `assets/brand/*.svg` (D190), `THIRD_PARTY.md` and the release's notices (D491), and `docs/DECISIONS_BY_AREA.md` (D539); each takes `--check`. |
| `cargo xtask package`, `package-check`, `rc`, `sbom-native` | The release zip and its symbols zip, the checks on an unpacked zip, the release-candidate harness for a real Windows machine (D537), and the native DLLs added to the SBOM (D536). |

The code is in `src/main.rs` (the clap `Cli`) and one module per command:
`ci`, `changelog`, `bench`, `corpus`, `symbolicate`, `brand`, `credits`,
`decisions`, `package`, `rc` and `native`.

## Usage

```bash
cargo xtask ci                                # every lane CI runs on this kind of host
cargo xtask ci check test                     # just those lanes, as CI's jobs run them
cargo xtask ci doctor                         # what can run here, and how to install the rest
cargo xtask ci --strict                       # a missing tool or a changed tree fails, as in CI
cargo xtask ci --allow-gl-skip                # on a machine with no headless EGL
cargo xtask ci --no-deny                      # skip cargo-deny (it needs the network)
cargo xtask changelog                         # regenerate CHANGELOG.md
cargo xtask changelog --check                 # what CI runs; compares released sections
cargo xtask changelog --release v0.2.0        # write a release's section before tagging
cargo xtask bench -- --size 1280x720          # strobe-bench, in release
cargo xtask corpus -- --library scenes        # compatibility report (D195)
cargo xtask symbolicate crash-….dmp           # a crash dump as a readable stack
cargo xtask brand                             # icon PNGs and .ico from assets/brand/*.svg
cargo xtask brand --check                     # what CI runs; fails on a stale render
cargo xtask sbom-native --sbom <f.cdx.json> --dir <dlls>  # the release DLLs into its SBOM, checked against scripts/native-deps.json (D536)
cargo xtask package --profile release-fast   # the release zip and symbols zip from a build, into dist/ (Windows, D537)
cargo xtask package-check <zip> --symbols <zip>  # unpack outside the checkout and check it is what a user should get
cargo xtask rc --zip <zip> --symbols <zip>    # the release-candidate harness for the T480: draw, crash, symbolicate, evidence.json
```

Flags, from the clap definitions:

- `ci`: lanes (any of `all`, `doctor`, `fmt`, `check`, `test`, `deny`,
  `generated`, `render`, `build`, `package`, `lint`, `conventions`),
  `--strict`, `--shipping`
  (the `build` lane also builds the fat-LTO `release`), `--nextest` (the
  `test` lane runs the tests through cargo-nextest and the doctests through
  `cargo test --doc`, configured by `.config/nextest.toml`, D535),
  `--allow-gl-skip` and `--no-deny`. The lane table, what each host does,
  and the strict rules are in `src/ci.rs`'s module docs. The `build` lane
  passes `--timings`, so `target/cargo-timings/` says where its time went.
- `changelog`: `--check` and `--release <vX.Y.Z>`, which conflict with each
  other.
- `bench`: everything after `--` goes to `strobe-bench`.
- `symbolicate <dump>`: `--symbols <file>` (repeatable; the default is
  every PDB under `target/x86_64-pc-windows-msvc/release` (a packaged
  build), `target/release` and `target/release/examples`, or the
  release `strobe` and `crash_demo` binaries where there are no PDBs), and
  `--out <file>`. It needs `cargo install dump_syms minidump-stackwalk
  --locked`, and finds the tools on `PATH` or in cargo's `bin` folder. See
  [`docs/DEBUGGING.md`](../../docs/DEBUGGING.md).
- `brand`: `--check`. Renders `icon-small.svg` at 16-32 px and `icon.svg`
  at 48-512 px with resvg (no fonts, so the bytes depend only on
  `Cargo.lock`), and packs `icon.ico`. See D190 and
  [`assets/brand/README.md`](../../assets/brand/README.md).

Environment: `cargo xtask ci` sets `STROBE_REQUIRE_HEADLESS_GL=1` for the test
step on Linux (unless you already set it or pass `--allow-gl-skip`), so a
missing Mesa EGL fails loudly instead of every renderer test skipping. On Linux it
also sets `STROBE_TEST_LAN=1` (unless you set it), so the few tests that bind
every interface run; elsewhere they skip unless you set it, because on Windows
that bind makes the firewall ask about each new test binary (D503). It also
honours `CARGO` and `RUSTC` if set.

`clippy no-midi` is `cargo clippy -p strobe-io --no-default-features
--all-targets -- -D warnings`: the show-control crate without its `midi`
feature, which `--all-features` never builds. It runs on Linux only and is
skipped, with that reason, elsewhere.

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

The changelog generator's unit tests cover subject parsing, heading
placement, the `Unreleased` exemption, version parsing, the calendar
conversion and reading `[workspace.package]`; they do not run git or cargo.
Others cover the credits (licence files, bots left out), the decisions
index, the native-DLL pin, the package's PDB and version checks, and
symbolicate's paths.
`ci`'s planner is tested without running anything: every host's lanes,
`--locked` on every resolving command, missing tools and `--strict`, the
tree check, and the step timeout.

## Design notes

- **A crate, not examples or scripts** (D50): workspace tooling would
  otherwise be parked in one package, and a shell script would not run the same
  on Windows.
- **`CHANGELOG.md` is generated from conventional commits** (D51): released
  sections only are checked, changelog-only commits and merge commits are not
  listed, and the repository URL is read from `[workspace.package]`.
- **Draft, build, attest, undraft** (D52) is the release workflow the
  changelog feeds; the tag's section becomes the release notes.
- **Skipped is never silent**: `ci` reports a step that does not apply to
  this host as `n/a`, and one whose tool is missing as `MISSING`, which ends
  the run as "partial" (and fails it with `--strict`).
- **One list of commands** (D533): CI's jobs run lanes, so the commands,
  flags and per-host exclusions are written once, in `src/ci.rs`.
- **Windows cross-check excludes the app** (D63), because `strobe-app` depends
  on `strobe-script`, whose C sources need the MSVC toolchain.

## Status and known gaps

Open work for this crate is tracked in
[`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md) under the
IDs given here; a gap with no ID is by design, with its decision named.

- Working and used on every change ([`ROADMAP.md`](../../docs/ROADMAP.md)).
- The release workflow has run from its tags: v0.2.0 was built, attested
  and published by `release.yml`, and the downloaded archive ran on the T480
  (Task 50). `cargo xtask rc` passed there on a candidate (D544). Left: the
  checks a person makes at the T480 (backlog R-03).
- `cargo xtask ci` runs what CI runs on this kind of host (D533). The other
  hosts' lanes need those platforms: the `render` lane needs Linux with Mesa
  and Xvfb, the `build` lane Windows, by design.
