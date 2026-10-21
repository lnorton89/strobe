// ============================================================================
// Module:       exit_codes (integration test)
// Description:  The real `strobe` executable's exit status for a command line
//               it refuses, which a restart loop reads to stop rather than
//               start the same mistake again every few seconds.
//
// Dependencies: anyhow
// ============================================================================

//! A refused command line exits 2, a failed or stopped run 1 (D454).
//!
//! The venue launcher restarts Strobe 3 s after any exit. A run that crashed
//! or lost its GL context is worth restarting; a mistyped flag is refused
//! the same way every time. Only the status can tell them apart.
//!
//! Run on Unix only: on Windows a refused command line raises a message box
//! and waits for it, which a test cannot click. The code path is the same.

use anyhow::{Result, anyhow};
use std::process::{Command, Output};

/// Runs the executable with `args`, its logs in a scratch folder and no
/// crash monitor.
fn run(tag: &str, args: &[&str]) -> Result<Output> {
    let dir = std::env::temp_dir().join(format!("strobe-exit-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let output = Command::new(env!("CARGO_BIN_EXE_strobe"))
        .args(args)
        .env("STROBE_DIAG_DIR", &dir)
        .env("STROBE_NO_CRASH_MONITOR", "1")
        .output()?;
    // Best effort: the folder is scratch, and a leftover log harms nothing.
    let _ = std::fs::remove_dir_all(&dir);
    Ok(output)
}

#[test]
fn a_refused_command_line_exits_2_not_the_1_of_a_fault() -> Result<()> {
    if !cfg!(unix) {
        return Ok(());
    }
    for (tag, args) in [
        ("unknown", ["--audio", "off"].as_slice()),
        ("range", &["--fullscreen", "--output-monitor", "0"]),
        ("lonely", &["--remote-trust", "venue.trust"]),
    ] {
        let output = run(tag, args)?;
        let code = output
            .status
            .code()
            .ok_or_else(|| anyhow!("{args:?}: ended by a signal: {:?}", output.status))?;
        assert_eq!(
            code,
            i32::from(strobe_app::cli::REFUSED_EXIT),
            "{args:?}: a refused command line, stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.stderr.is_empty(),
            "{args:?}: clap says why on stderr"
        );
    }
    let version = run("version", &["--version"])?;
    assert_eq!(version.status.code(), Some(0), "--version is not a refusal");
    Ok(())
}
