// ============================================================================
// Module:       strobe_conventions (library crate root)
// Description:  The workspace's house rules as tests: file headers, the unsafe
//               boundary, the pinned toolchain, pinned workflow actions, and
//               discarded results that say why, crate READMEs' known gaps
//               that point at the backlog, GL object owners, counted driver
//               queries, and the docs' links and decision numbers.
//
// Dependencies: anyhow, std::fs (walks every crate's sources and READMEs at
//               test time)
// ============================================================================

//! The workspace's house rules, as tests.
//!
//! A convention only holds if something checks it. This crate contains no
//! code anything links against; it exists so `cargo test --workspace`
//! walks every crate's sources and fails the build for a file that breaks
//! a rule the compiler cannot express. Each rule lives in its own module
//! with the reasoning beside it:
//!
//! - [`header`] — every source file opens with the banner naming its
//!   module, purpose and dependencies, and `Module:` matches where the
//!   file actually sits.
//! - [`unsafe_boundary`] — `unsafe` lives only in `strobe-gfx`'s `gl`
//!   module and `strobe-diag`'s `native.rs`, one operation per safe leaf
//!   wrapper, never in a test.
//! - [`toolchain`] — `rust-toolchain.toml` and every CI workflow name the
//!   same Rust version.
//!
//! - [`actions`] — every workflow pins its actions to full commit SHAs
//!   with a version comment, declares its token permissions, runs cargo
//!   `--locked`, and gives every job a timeout (D520).
//! - [`discarded`] — outside tests, a result thrown away with `let _ =` or
//!   `.ok();` carries a comment saying why (D267).
//! - [`known_gaps`] — every gap a crate README lists names its backlog
//!   item, open question or decision, and the backlog IDs it cites exist
//!   (D490).
//! - [`gl_objects`] — every GL object has an owning type with a `Drop` in
//!   the `gl` module, and no raw handle is stored outside it (D497).
//! - [`driver_queries`] — every driver state query counts itself, so a test
//!   can prove a playing scene makes none, and none is made outside `gl`
//!   (D497).
//! - [`docs`] — the Markdown's relative links resolve and its cited
//!   decisions exist (D497).
//!
//! The walking and path arithmetic shared by the Rust-source rules is in
//! [`sources`].

pub mod actions;
pub mod discarded;
pub mod docs;
pub mod driver_queries;
pub mod gl_objects;
pub mod header;
pub mod known_gaps;
pub mod sources;
pub mod toolchain;
pub mod unsafe_boundary;
