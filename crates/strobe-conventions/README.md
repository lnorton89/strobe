# strobe-conventions

The workspace's house rules, written as tests. Nothing links against this
crate. It exists so that `cargo test --workspace` walks every crate's
sources and every CI workflow and fails the build when one breaks a rule the
compiler cannot express: file headers, where `unsafe` may appear, who owns a
GL object, that a playing frame asks the driver nothing, the pinned Rust
toolchain, pinned GitHub Actions, and the Markdown's links and decision
numbers.

## Role in the workspace

- **Depends on:** `anyhow` only. It reads the source tree and
  `.github/workflows/` from disk at test time, locating the workspace root
  two directories above its own manifest.
- **Depended on by:** nothing. It is a workspace member, so any `--workspace`
  test run (and `cargo xtask ci`) includes it.
- **Does not:** compile or inspect Rust semantically (the checks are textual
  line scans), check Markdown beyond links, decision numbers and the known
  gaps, or enforce the Clippy lints. The lints
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
| `gl_objects` | `Owners` (what the `gl` module owns and drops) and `stray_handles(relative_path, text)`. |
| `driver_queries` | `violations(relative_path, text)`, the `QUERIES` glow methods and the `EXEMPT` leaves. |
| `docs` | `pages()`, `problems(page, defined, exists)`, `defined_decisions` and `cited_decisions`. |
| `sources` | `workspace_root()` and `workspace_sources()`, the walk over each crate's `src/`, `tests/`, `examples/` and `build.rs`; `enclosing_function`. |

## Usage

There is nothing to call. Run the tests:

```bash
cargo test -p strobe-conventions
```

A failure prints the offending `path:line` and the rule. The rules and the
fix for each are below.

## Testing

`cargo test -p strobe-conventions` currently runs 48 tests: one test per
workspace-walking rule (three for the toolchain) plus unit tests that feed each rule a good and a bad
input, so a rule that stopped detecting anything would itself fail. No GPU,
window or network is needed. `cargo xtask ci` runs the same tests as part of
`cargo test --workspace`.

## The rules, and how to fix a failure

### File headers (`header`)

Every `.rs` file under a crate's `src/`, `tests/` or `examples/`, and its
`build.rs`, opens with
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
  `strobe_bench (binary crate root)`, `tests/x.rs` is `x (integration test)`,
  `examples/x.rs` is `x (example)` and a `build.rs` is `build (build script)`.

**Fix:** copy the banner of a neighbouring file and rewrite all three
fields. A wrapped value continues on a line indented to the label column
(`//` plus a space, then 14 columns). The reasoning belongs in the `//!`
block below the banner, not in it.

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

### Pinned actions, permissions, lockfile and timeouts (`actions`)

Every `uses:` line in a workflow must name a full 40-hex commit SHA followed
by a `# <version>` comment, and every workflow must declare a top-level
`permissions:` key. Local actions (`uses: ./...`) are exempt (D54). Every
cargo command that resolves dependencies (`build`, `check`, `clippy`, `doc`,
`install`, `run`, `test`, `xtask`) must pass `--locked`, and every job must
set `timeout-minutes:`; a job that only calls a reusable workflow is exempt
from the timeout (D517).

**Fix:** resolve the tag to its commit SHA, write
`uses: owner/action@<sha> # vX.Y.Z`, and add
`permissions:\n  contents: read` at the top of the workflow, elevating inside
the job that needs more. Add `--locked` to the cargo command (before the
alias for `cargo --locked xtask …`), and give the job a `timeout-minutes:`
well above its slowest cold-cache run.

### GL objects have owners (`gl_objects`)

In `crates/strobe-gfx/src/gl/`, a struct with a field of a raw glow object
type (`raw: glow::Texture,`, or an `Option` of one) needs an `impl Drop for`
it, and every create call there (`create_texture(`, `fence_sync(`, ...) must
make a kind that such a struct owns. Anywhere else, a struct field of a raw
glow object type fails (D497).

**Fix:** wrap the handle in an owning type in `gl/mod.rs` beside `Texture`
and `Framebuffer`, with a `Drop` that deletes it unless the context is lost,
and hold that type instead.

### No driver queries in the frame (`driver_queries`)

Every function in `gl/` that calls a glow state query (`get_parameter*`,
`get_error`, `is_enabled`, `get_uniform_location`, `check_framebuffer_status`,
the compile and link status and logs; the list is `QUERIES`) must call
`self.count_query();` before it, unless it is one of the `EXEMPT` leaves
(the timer result, the timer's disjoint flag, debug-output set-up). A glow
query anywhere outside `gl/` fails. The counter, `Gl::state_queries`, is
what `strobe-gfx`'s tests assert stays still while a scene plays (D497).

**Fix:** add `self.count_query();` as the leaf's first line. If a frame
path is what made the query, cache the fact at load time instead
(`player::build_program`, `caps`). A new exemption needs a reason in
`EXEMPT` and a decision.

### Links and decision numbers (`docs`)

Every `.md` under `docs/`, at the root (not the generated `CHANGELOG.md`)
and each crate's `README.md`: a relative link or image must resolve from
the linking file's directory, and a cited `D<number>` must be a heading in
`docs/DECISIONS.md`. Fenced code blocks and inline code spans are skipped
for links; `D3D11` and `HDR` are not citations (D497).

**Fix:** correct the path, or the number. A number used somewhere other
than DECISIONS.md (an off-master draft, say) is written out as "decision
273", not cited.

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
- The walk covers `src/`, `tests/`, `examples/` and `build.rs` of crates
  under `crates/`. `scripts/` holds shell and PowerShell, which the Rust
  rules do not apply to, by design (D497).
- The driver-query rule proves that each query counts itself; that a
  playing scene makes none is proved by `strobe-gfx`'s GPU tests, which
  skip without headless EGL, so only CI's Linux job runs them, by design
  (D497).
