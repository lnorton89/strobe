// ============================================================================
// Module:       crash_pipeline (integration test)
// Description:  Runs the crash_demo example as a real process and checks the
//               files a panic, an abort and a stack overflow leave behind.
//
// Dependencies: anyhow; the crash_demo example binary built by cargo test
// ============================================================================

//! End-to-end crash capture.
//!
//! These tests start the `crash_demo` example, let it fail on purpose, and
//! read what it left in a scratch `STROBE_DIAG_DIR`. They exercise what
//! unit tests cannot: a real signal handler, a real second process writing
//! a real minidump, and the file stem shared by a report and its dump.
//!
//! `cargo test -p strobe-diag` builds the example first. Running only this
//! test target (`--test crash_pipeline`) does not, and the test says so.
//! The native-crash tests are Linux-only: they rely on `abort()` reaching
//! the signal handler, which on Windows it does not (a fast-fail bypasses
//! exception filters).

use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn demo_path() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("test binary has no profile directory: {}", exe.display()))?;
    let path = profile_dir
        .join("examples")
        .join(format!("crash_demo{}", std::env::consts::EXE_SUFFIX));
    if !path.exists() {
        return Err(anyhow!(
            "{} is missing; run `cargo test -p strobe-diag` (which builds examples), \
             or `cargo build -p strobe-diag --example crash_demo` first",
            path.display()
        ));
    }
    Ok(path)
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "strobe-diag-pipeline-{}-{name}",
        std::process::id()
    ))
}

/// Runs the demo in `mode` and returns the crashes directory.
fn run_demo(mode: &str, dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    Command::new(demo_path()?)
        .arg(mode)
        .env("STROBE_DIAG_DIR", dir)
        .env_remove("STROBE_LOG")
        .env_remove("STROBE_NO_CRASH_MONITOR")
        .output()
        .with_context(|| format!("running crash_demo {mode}"))?;
    Ok(dir.join("crashes"))
}

/// Files in `dir` with extension `ext`, waiting up to ten seconds for at
/// least one (the monitor writes its report just after the process dies).
fn wait_for(dir: &Path, ext: &str) -> Result<Vec<PathBuf>> {
    let started = Instant::now();
    loop {
        let found: Vec<PathBuf> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|found| found == ext))
                    .collect()
            })
            .unwrap_or_default();
        if !found.is_empty() {
            return Ok(found);
        }
        if started.elapsed() > Duration::from_secs(10) {
            return Err(anyhow!("no .{ext} appeared in {}", dir.display()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn first_text(dir: &Path) -> Result<String> {
    let reports = wait_for(dir, "txt")?;
    let path = reports
        .first()
        .ok_or_else(|| anyhow!("no report in {}", dir.display()))?;
    Ok(fs::read_to_string(path)?)
}

#[test]
fn a_panic_leaves_a_report_with_context_failures_log_and_backtrace() -> Result<()> {
    let dir = scratch("panic");
    let crashes = run_demo("panic", &dir)?;
    let text = first_text(&crashes)?;
    fs::remove_dir_all(&dir)?;
    for needle in [
        "kind:    panic",
        "index out of bounds",
        "scene.name",
        "09-sdf-cathedral",
        "SHADER",
        "message=unexpected token",
        "demo is about to fail",
        "Backtrace",
    ] {
        assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn an_abort_leaves_a_minidump_and_a_companion_report() -> Result<()> {
    let dir = scratch("abort");
    let crashes = run_demo("abort", &dir)?;
    let text = first_text(&crashes)?;
    let dumps = wait_for(&crashes, "dmp")?;
    let header = fs::read(dumps.first().ok_or_else(|| anyhow!("no dump"))?)?;
    fs::remove_dir_all(&dir)?;
    assert_eq!(header.get(..4), Some(&b"MDMP"[..]), "not a minidump");
    for needle in [
        "kind:    native crash",
        "SIGABRT",
        "09-sdf-cathedral",
        "frame",
    ] {
        assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn a_stack_overflow_leaves_a_minidump_and_a_companion_report() -> Result<()> {
    let dir = scratch("overflow");
    let crashes = run_demo("overflow", &dir)?;
    let text = first_text(&crashes)?;
    let dumps = wait_for(&crashes, "dmp")?;
    fs::remove_dir_all(&dir)?;
    assert!(!dumps.is_empty(), "no minidump written");
    assert!(text.contains("SIGSEGV"), "report was:\n{text}");
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn a_clean_run_leaves_a_log_and_no_crash_files() -> Result<()> {
    let dir = scratch("clean");
    let crashes = run_demo("ok", &dir)?;
    let logs = wait_for(&dir.join("logs"), "log")?;
    let text = fs::read_to_string(logs.first().ok_or_else(|| anyhow!("no log"))?)?;
    let crash_files = fs::read_dir(&crashes).map(Iterator::count).unwrap_or(0);
    fs::remove_dir_all(&dir)?;
    assert!(text.contains("demo finished normally"), "log was:\n{text}");
    assert_eq!(crash_files, 0, "a clean run must not leave crash files");
    Ok(())
}
