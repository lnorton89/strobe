// ============================================================================
// Module:       strobe_gfx::skip
// Description:  How a GPU test says it skipped: to stderr, and as a line in
//               the file `STROBE_SKIP_LOG` names, so a run can count skips
//               apart from passes.
//
// Dependencies: std only
// ============================================================================

//! Skips that are not passes (backlog D-04, D249).
//!
//! A test that needs a GL context and finds none (a Windows runner has no
//! headless EGL, a cloud container may lack Mesa) returns early and passes:
//! the Rust test harness has no "skipped" result. Printing "skipping" is
//! not enough, because the harness captures a passing test's output, so a
//! run where every GPU test skipped looks exactly like one where they all
//! ran.
//!
//! [`note`] keeps the message, and when `STROBE_SKIP_LOG` names a file it
//! also appends one line to it: the test's name (the harness names each
//! test's thread after it) and the reason, separated by a tab. `cargo xtask
//! ci` and the CI jobs set the variable, then report how many tests skipped
//! and why, beside the harness's pass count. `STROBE_REQUIRE_HEADLESS_GL`
//! still turns a missing context into a failure where one must exist.
//!
//! Appending is one `write` of one short line to a file opened for append,
//! which the OS keeps whole when parallel test threads append at once.

use std::fmt::Display;
use std::io::Write as _;

/// The variable naming the skip log.
pub const LOG_VAR: &str = "STROBE_SKIP_LOG";

/// Records that the current test is skipping, and why. Never fails: a skip
/// log that cannot be written is said on stderr and otherwise ignored.
pub fn note(reason: impl Display) {
    let test = std::thread::current()
        .name()
        .unwrap_or("unnamed test")
        .to_string();
    eprintln!("skipping {test}: {reason}");
    let Some(path) = std::env::var_os(LOG_VAR) else {
        return;
    };
    let line = format!("{test}\t{reason}\n");
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(error) = written {
        eprintln!("could not record the skip in {}: {error}", path.display());
    }
}

/// The skips recorded in a log written by [`note`]: `(test, reason)`
/// pairs, in the order they were written. Lines that do not split are
/// kept whole as the test's name.
pub fn read(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match line.split_once('\t') {
            Some((test, reason)) => (test.to_string(), reason.to_string()),
            None => (line.to_string(), String::new()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_log_reads_back_as_test_and_reason() -> Result<()> {
        let skips = read("a::b\tno headless context\n\nc::d\tno timer\nloose line\n");
        assert_eq!(
            skips,
            vec![
                ("a::b".to_string(), "no headless context".to_string()),
                ("c::d".to_string(), "no timer".to_string()),
                ("loose line".to_string(), String::new()),
            ],
            "blank lines dropped, a line with no tab kept whole"
        );
        Ok(())
    }
}
