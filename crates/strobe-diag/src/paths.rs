// ============================================================================
// Module:       strobe_diag::paths
// Description:  Where logs and crash reports live on this machine.
//
// Dependencies: dirs (the per-user data directory)
// ============================================================================

//! Per-user diagnostic directories.
//!
//! Everything lands under one base directory so a user can be told a
//! single path: `%LOCALAPPDATA%\strobe` on Windows,
//! `~/.local/share/strobe` on Linux, `~/Library/Application Support/strobe`
//! on macOS. `STROBE_DIAG_DIR` replaces the base, which is how tests and
//! the crash demo avoid touching a real profile. If the platform reports no
//! data directory the system temp directory is used, so a report can still
//! be written.

use std::path::PathBuf;

/// Environment variable that replaces the base directory.
pub const BASE_ENV: &str = "STROBE_DIAG_DIR";

/// The base directory, not created.
pub fn base_dir() -> PathBuf {
    if let Some(overridden) = std::env::var_os(BASE_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(overridden);
    }
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("strobe")
}

/// Where session logs are written.
pub fn logs_dir() -> PathBuf {
    base_dir().join("logs")
}

/// Where crash reports (`crash-*.txt`) and minidumps (`crash-*.dmp`) go.
pub fn crashes_dir() -> PathBuf {
    base_dir().join("crashes")
}

/// Where the crash monitor's socket file lives (Windows and macOS; Linux
/// uses an abstract-namespace socket with no file).
///
/// Deliberately not the system temp directory. On Windows `%TEMP%` is often
/// an 8.3 short path (`C:\Users\EXALTE~1\...`), and an AF_UNIX connect
/// through a short path to a socket bound at the long path fails with
/// `WSAEINVAL` (10022), which is what happened on the T480. The known-folder
/// API behind [`base_dir`] returns the long form.
pub fn sockets_dir() -> PathBuf {
    base_dir().join("run")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn logs_and_crashes_sit_under_the_base_directory() -> Result<()> {
        let base = base_dir();
        assert!(logs_dir().starts_with(&base), "logs outside {base:?}");
        assert!(crashes_dir().starts_with(&base), "crashes outside {base:?}");
        assert_ne!(logs_dir(), crashes_dir(), "logs and crashes must differ");
        Ok(())
    }
}
