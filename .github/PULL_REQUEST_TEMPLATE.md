## Summary

What this change does and why, in one or two sentences.

## Test plan

What you ran, and on what. Say whether anything ran **on a real GPU /
real Windows machine**. CI has neither, so a visual or driver-dependent
change needs a screenshot or bench numbers from one.

## Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Windows cross-check: `cargo clippy … --target x86_64-pc-windows-msvc`
- [ ] No new `unwrap` / `expect` / `panic!`, not even in tests
- [ ] New files carry the header banner
- [ ] Any new `unsafe` is a one-call leaf in `strobe-gfx/src/gl/` with a `// SAFETY:` comment
- [ ] Per-frame path changed → bench before/after in `docs/BENCHMARKS.md`
- [ ] `docs/ROADMAP.md` updated; new decisions logged in `docs/DECISIONS.md`
