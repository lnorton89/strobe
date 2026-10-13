// ============================================================================
// Module:       xtask::corpus
// Description:  `cargo xtask corpus`: runs strobe-app's strobe-corpus harness
//               in release mode, forwarding every argument to it unchanged.
//
// Dependencies: anyhow, std::process
// ============================================================================

//! `cargo xtask corpus -- --library <dir> [harness flags]`.
//!
//! A shortcut for `cargo run --release -p strobe-app --bin strobe-corpus --
//! <args>`, for the same reason `bench` exists: the release build is the
//! part people forget. The harness is a binary of `strobe-app` because it
//! loads scenes through the app's own path, script included (D195), and
//! `strobe-script` cannot be built for MSVC by the cross-check, so this crate
//! must not link it. Nothing is parsed here; the harness owns its flags.

use anyhow::{Context as _, Result, bail};
use std::process::Command;

/// Runs the corpus harness with `args` and fails if it does.
pub(crate) fn run(args: &[String]) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(&cargo)
        .args([
            "run",
            "--release",
            "-p",
            "strobe-app",
            "--bin",
            "strobe-corpus",
            "--",
        ])
        .args(args)
        .status()
        .context("failed to run cargo")?;
    if !status.success() {
        bail!("strobe-corpus failed ({status})");
    }
    Ok(())
}
