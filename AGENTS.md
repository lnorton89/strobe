# Strobe agent instructions

Read [`CLAUDE.md`](CLAUDE.md) first. It holds the commands, the rules
the build enforces, and the map of planning docs.

Before you finish a piece of work:

1. Run the gates (`cargo xtask ci` runs all of them in CI's order):
   `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
   `cargo test --workspace`, and the Windows cross-check
   (`--target x86_64-pc-windows-msvc`).
2. If you touched a per-frame path, run `strobe-bench` before and after
   and record the delta in `docs/BENCHMARKS.md`.
3. Update `docs/ROADMAP.md` with what is now done and what is next.
4. Log any non-obvious choice in `docs/DECISIONS.md`. If you verified or
   disproved something in `docs/OPEN_QUESTIONS.md`, move it there.

Preserve unrelated changes in a dirty worktree, and never revert another
contributor's edits.
