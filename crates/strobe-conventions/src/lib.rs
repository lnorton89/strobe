// ============================================================================
// Module:       strobe_conventions (library crate root)
// Description:  The workspace's house rules as tests: file headers, the unsafe
//               boundary, the pinned toolchain, and pinned workflow actions.
//
// Dependencies: anyhow, std::fs (walks every crate's sources at test time)
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
//!   module, one operation per safe leaf wrapper, never in a test.
//! - [`toolchain`] — `rust-toolchain.toml` and every CI workflow name the
//!   same Rust version.
//!
//! - [`actions`] — every workflow pins its actions to full commit SHAs
//!   with a version comment, and declares its token permissions.
//!
//! The walking and path arithmetic shared by the Rust-source rules is in
//! [`sources`].

pub mod actions;
pub mod header;
pub mod sources;
pub mod toolchain;
pub mod unsafe_boundary;
