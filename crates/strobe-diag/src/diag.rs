// ============================================================================
// Module:       strobe_diag::diag
// Description:  The human-readable system report behind `--diagnose` and the
//               top of every crash report.
//
// Dependencies: strobe_diag::{buildinfo, system, paths, logger}
// ============================================================================

//! The diagnose report.
//!
//! [`system_report`] answers "what is this machine and this build?" in one
//! block of text: build identity, operating system, CPU and RAM, the
//! environment flags that change behaviour, and where logs and crash
//! reports go. The app's `--diagnose` flag prints it; crash reports start
//! with the same block so a report is self-describing.
//!
//! The caller passes `extra` pairs for things only it knows (the GL
//! renderer, the context backend, the audio device). They are listed under
//! "Session". Crash reports pass the whole session context there.

use crate::system::{self, SystemInfo};
use crate::{buildinfo, logger, paths};
use std::fmt::Write;

/// Environment variables worth reporting, set or not.
const ENV_FLAGS: [&str; 6] = [
    "STROBE_LOG",
    "STROBE_LOG_STDERR",
    "STROBE_DIAG_DIR",
    "STROBE_NO_CRASH_MONITOR",
    "STROBE_REQUIRE_HEADLESS_GL",
    "RUST_BACKTRACE",
];

/// The report, probing the machine now if that has not finished. May take
/// a moment on Windows (a PowerShell query); use it for `--diagnose`, not
/// on a crash path.
pub fn system_report(extra: &[(String, String)]) -> String {
    render(extra, Some(system::info()))
}

/// The report using only what has already been probed; never waits. For
/// panic and crash paths.
pub(crate) fn system_report_now(extra: &[(String, String)]) -> String {
    render(extra, system::cached())
}

fn render(extra: &[(String, String)], machine: Option<&SystemInfo>) -> String {
    let mut text = String::new();
    section(&mut text, "Build");
    row(&mut text, "version", buildinfo::VERSION);
    row(
        &mut text,
        "commit",
        &format!(
            "{}{}",
            buildinfo::GIT_HASH,
            if buildinfo::GIT_DIRTY {
                " (uncommitted changes)"
            } else {
                ""
            }
        ),
    );
    row(&mut text, "profile", buildinfo::PROFILE);
    row(&mut text, "target", buildinfo::TARGET);

    section(&mut text, "System");
    match machine {
        Some(info) => {
            row(&mut text, "os", info.os.as_deref().unwrap_or("unknown"));
            row(
                &mut text,
                "cpu",
                info.cpu_model.as_deref().unwrap_or("unknown"),
            );
            row(
                &mut text,
                "cores",
                &info
                    .cpu_cores
                    .map_or_else(|| "unknown".to_string(), |cores| cores.to_string()),
            );
            row(
                &mut text,
                "ram",
                &info
                    .ram_bytes
                    .map_or_else(|| "unknown".to_string(), system::format_bytes),
            );
        }
        None => row(&mut text, "probe", "still running when this was written"),
    }

    section(&mut text, "Environment");
    for name in ENV_FLAGS {
        let value = std::env::var(name).unwrap_or_else(|_| "(unset)".to_string());
        row(&mut text, name, &value);
    }

    section(&mut text, "Directories");
    row(&mut text, "logs", &paths::logs_dir().display().to_string());
    row(
        &mut text,
        "crashes",
        &paths::crashes_dir().display().to_string(),
    );
    row(
        &mut text,
        "this log",
        &logger::log_file().map_or_else(|| "(none)".to_string(), |path| path.display().to_string()),
    );

    if !extra.is_empty() {
        section(&mut text, "Session");
        for (key, value) in extra {
            row(&mut text, key, value);
        }
    }
    text
}

fn section(text: &mut String, title: &str) {
    if !text.is_empty() {
        text.push('\n');
    }
    writeln!(text, "{title}").ok();
    writeln!(text, "{}", "-".repeat(title.len())).ok();
}

fn row(text: &mut String, key: &str, value: &str) {
    writeln!(text, "  {key:<10} {value}").ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_report_names_build_machine_environment_and_directories() -> Result<()> {
        let report = system_report(&[("gl.renderer".to_string(), "llvmpipe".to_string())]);
        for needle in [
            "Build",
            "Environment",
            "Directories",
            "System",
            "STROBE_LOG",
            "gl.renderer",
            "llvmpipe",
            buildinfo::GIT_HASH,
        ] {
            assert!(report.contains(needle), "missing `{needle}` in:\n{report}");
        }
        Ok(())
    }

    #[test]
    fn the_non_blocking_report_works_before_the_probe_runs() -> Result<()> {
        let report = system_report_now(&[]);
        assert!(report.contains("Build"), "build section missing:\n{report}");
        assert!(
            !report.contains("Session"),
            "no extras means no session section:\n{report}"
        );
        Ok(())
    }
}
