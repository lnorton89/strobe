# strobe-conventions

The workspace's house rules, written as tests. Nothing links against this
crate. It exists so that `cargo test --workspace` walks every crate's
sources and every CI workflow and fails the build when one breaks a rule the
compiler cannot express: file headers, where `unsafe` may appear, the pinned
Rust toolchain, and pinned GitHub Actions.

## Role in the workspace

- **Depends on:** `anyhow` only. It reads the source tree and
  `.github/workflows/` from disk at test time, locating the workspace root
  two directories above its own manifest.
- **Depended on by:** nothing. It is a workspace member, so any `--workspace`
  test run (and `cargo xtask ci`) includes it.
- **Does not:** compile or inspect Rust semantically (the checks are textual
  line scans), check Markdown, or enforce the Clippy lints. The lints
  (`unwrap_used`, `wildcard_enum_match_arm`, `undocumented_unsafe_blocks`,
  and the rest) are set in the root `Cargo.toml` and enforced by
  `cargo clippy`, not by this crate.

## Main types / entry points

The public items are the checks themselves, each in its own module, so a
failure message can be reproduced by calling them:

| Module | What |
|---|---|
| `header` | `problems(relative_path, text)` lists every banner problem in one file; `expected_module(path)` derives the `Module:` value a path must declare. |
| `unsafe_boundary` | `violations(relative_path, text)` and the `ALLOWED` list of places that may hold `unsafe`. |
| `toolchain` | `pinned_channel()` reads `rust-toolchain.toml`; `workflows()` loads every workflow file. |
| `actions` | `uses_problem(line)` and `problems(name, workflow_text)` for pinned actions and permissions. |
| `sources` | `workspace_root()` and `workspace_sources()`, the walk over each crate's `src/`, `tests/` and `examples/`. |

## Usage

There is nothing to call. Run the tests:

```bash
cargo test -p strobe-conventions
```

A failure prints the offending `path:line` and the rule. The rules and the
fix for each are below.

## Testing

`cargo test -p strobe-conventions` currently runs 24 tests: one test per
workspace-walking rule (three for the toolchain) plus unit tests that feed each rule a good and a bad
input, so a rule that stopped detecting anything would itself fail. No GPU,
window or network is needed. `cargo xtask ci` runs the same tests as part of
`cargo test --workspace`.

## The rules, and how to fix a failure

### File headers (`header`)

Every `.rs` file under a crate's `src/`, `tests/` or `examples/` opens with
a ruled banner (the exact `// ====...` rule line, 79 characters, first and
again to close it), then `Module:`, `Description:` and `Dependencies:`.
The check is structural. It fails when:

- the file does not open with the rule, or the rule is never closed;
- a field is missing, empty, or still starts with `[` (the template
  placeholder);
- a line inside the banner is not a plain `//` comment, or runs past 100
  columns;
- `Module:` disagrees with the file's path: `crates/strobe-scene/src/control/kind.rs`
  is `strobe_scene::control::kind`, a `mod.rs` documents its directory, a
  `lib.rs` is `strobe_scene (library crate root)`, a `main.rs` is
  `strobe_bench (binary crate root)`, `tests/x.rs` is `x (integration test)`
  and `examples/x.rs` is `x (example)`.

**Fix:** copy the banner of a neighbouring file and rewrite all three
fields. A wrapped value continues on a line indented to the label column
(`//` plus a space, then 14 columns). The reasoning belongs in the `//!`
block below the banner, not in it. `build.rs` files are outside the
walked directories.

### The unsafe boundary (`unsafe_boundary`)

`unsafe` may appear only in `crates/strobe-gfx/src/gl/` (a directory) and
the single file `crates/strobe-diag/src/native.rs` (decision D41). A line
that opens `unsafe {`, `unsafe fn`, `unsafe impl` or `unsafe extern` fails when:

- it is anywhere else;
- it is in `tests/` or after a `mod tests {` line;
- it is a `pub unsafe fn`;
- one function holds more than one such line (a leaf wrapper must hold
  exactly one unsafe operation).

**Fix:** move the operation into `gl/mod.rs` as a private, safe, one-call
leaf wrapper and call that wrapper. Adding a third place to `ALLOWED` is a
deliberate act that wants its own entry in
[`DECISIONS.md`](../../docs/DECISIONS.md), as D41 did. The `// SAFETY:`
comment on each block is required by Clippy's `undocumented_unsafe_blocks`,
not by this crate.

### One toolchain (`toolchain`)

Three tests. Every `toolchain:` line in `.github/workflows/*.yml` must equal
the `channel` in `rust-toolchain.toml`; no workflow may use
`rust-toolchain@stable`; and both `ci.yml` and `release.yml` must exist and
name a toolchain.

**Fix:** bump `rust-toolchain.toml` and every workflow's `toolchain:` value
in one commit, or replace a floating reference with the pinned version.

### Pinned actions and permissions (`actions`)

Every `uses:` line in a workflow must name a full 40-hex commit SHA followed
by a `# <version>` comment, and every workflow must declare a top-level
`permissions:` key. Local actions (`uses: ./...`) are exempt (D54).

**Fix:** resolve the tag to its commit SHA, write
`uses: owner/action@<sha> # vX.Y.Z`, and add
`permissions:\n  contents: read` at the top of the workflow, elevating inside
the job that needs more.

## Design notes

- **Conventions as tests, not as a document** (D4): the lints, banners,
  unsafe-leaf discipline, pinned toolchain and LF endings come from
  `rustaman`, and a rule nothing checks drifts.
- **One allowed list for `unsafe`** (D41): `ALLOWED` holds a directory and one
  file; an entry ending in `/` is a directory, anything else is one file.
- **Actions pinned by SHA, permissions declared** (D54): the workflow check is
  a line scan, which is exact for the simple YAML here and avoids a YAML
  dependency.
- **Textual, not semantic.** The checks scan lines (comments stripped for the
  unsafe scan) rather than parse Rust. That keeps the crate dependency-light;
  Clippy's lints are the other layer.
- **Windows-safe paths:** failures report workspace-relative paths with `/`
  separators so a message reads the same on every platform.

## Status and known gaps

Open work for this crate is tracked in
[`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md) under the
IDs given here; a gap with no ID is by design, with its decision named.

- Its tests run in `cargo test --workspace` and pass on the current tree.
- The walk covers `src/`, `tests/` and `examples/` of crates under
  `crates/`. Files elsewhere (`build.rs`, `scripts/`) are not checked yet
  (backlog D-05).
- Of the Markdown, only the crate READMEs' "Status and known gaps" sections
  are checked ([`known_gaps`](src/known_gaps.rs)); the `docs/` tree is not
  checked yet (backlog D-05).
- The "every GL object has an owning type with a `Drop`" and "no `glGet*`
  in the frame loop" rules in [`CLAUDE.md`](../../CLAUDE.md) are not
  enforced yet (backlog D-05).
