// ============================================================================
// Module:       xtask::bench
// Description:  `cargo xtask bench`: runs strobe-bench in release mode,
//               forwarding every argument to it unchanged.
//
// Dependencies: anyhow, std::process
// ============================================================================

//! `cargo xtask bench -- <strobe-bench arguments>`.
//!
//! A shortcut for `cargo run --release -p strobe-bench -- <args>`. It is
//! worth a subcommand only because the `--release` is the part people
//! forget, and a debug-build benchmark is a number that means nothing: the
//! default dev profile is unoptimised for this workspace's own crates, so
//! the CPU side of every frame is several times slower than a shipped
//! build.
//!
//! Nothing is parsed here. strobe-bench owns its flags (`--size`,
//! `--api`, `--shared`, `--out` …), and a copy of them in this crate would
//! be one more thing to keep in step with it.

use anyhow::{Context as _, Result, bail};
use std::process::Command;

/// Runs strobe-bench with `args` and fails if it does.
pub(crate) fn run(args: &[String]) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(&cargo)
        .args(["run", "--release", "-p", "strobe-bench", "--"])
        .args(args)
        .status()
        .context("failed to run cargo")?;
    if !status.success() {
        bail!("strobe-bench failed ({status})");
    }
    Ok(())
}
