// ============================================================================
// Module:       strobe_diag::buildinfo
// Description:  Which build this is: version, git commit, dirty flag, target
//               and profile, captured by build.rs.
//
// Dependencies: build.rs (the STROBE_BUILD_* environment)
// ============================================================================

//! Build identity.
//!
//! A crash report that cannot say which commit produced the binary cannot
//! be reproduced. `build.rs` records the commit, whether tracked files had
//! uncommitted changes, the target triple and the profile; this module
//! exposes them as constants and as one summary line.
//!
//! The values are fixed when `strobe-diag` is compiled. Cargo rebuilds the
//! crate when `HEAD` or the index changes, so they track commits, but a
//! dirty flag can lag behind an edit made after the last build of this
//! crate. Release builds from a clean checkout are exact.

/// The workspace version (`[workspace.package] version`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Abbreviated commit hash, or `unknown` when git was unavailable.
pub const GIT_HASH: &str = env!("STROBE_BUILD_GIT_HASH");

/// Whether tracked files differed from `HEAD` when this was built.
pub const GIT_DIRTY: bool = matches!(env!("STROBE_BUILD_GIT_DIRTY").as_bytes(), b"1");

/// The target triple, such as `x86_64-pc-windows-msvc`.
pub const TARGET: &str = env!("STROBE_BUILD_TARGET");

/// The cargo profile, `debug` or `release`.
pub const PROFILE: &str = env!("STROBE_BUILD_PROFILE");

/// One line naming the build, such as
/// `0.1.0 (3f9a1c2b4d5e+dirty, release, x86_64-pc-windows-msvc)`.
pub fn summary() -> String {
    let dirty = if GIT_DIRTY { "+dirty" } else { "" };
    format!("{VERSION} ({GIT_HASH}{dirty}, {PROFILE}, {TARGET})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_summary_names_version_commit_profile_and_target() -> Result<()> {
        let line = summary();
        assert!(line.starts_with(VERSION), "summary was {line}");
        assert!(line.contains(GIT_HASH), "summary was {line}");
        assert!(line.contains(PROFILE), "summary was {line}");
        assert!(line.contains(TARGET), "summary was {line}");
        Ok(())
    }
}
