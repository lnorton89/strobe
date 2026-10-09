// ============================================================================
// Module:       strobe_diag::report
// Description:  Renders and writes crash-YYYYMMDD-HHMMSS.txt reports, and
//               claims the shared file stem for a report and its minidump.
//
// Dependencies: strobe_diag::{diag, clock}
// ============================================================================

//! Crash report text and files.
//!
//! One report layout serves both capture paths: the panic hook (which has
//! a backtrace and a message) and the native crash monitor (which has a
//! minidump and the context the crashed process sent before dying). The
//! sections are fixed so a person or an agent can find the log tail or the
//! backtrace by heading.
//!
//! A report and its minidump share a stem, `crash-YYYYMMDD-HHMMSS`, in the
//! crashes directory: `crash-….txt` is for reading, `crash-….dmp` is for a
//! debugger. When two crashes land in the same second the second gets a
//! `-2` suffix rather than overwriting the first.

use crate::clock::Utc;
use crate::diag;
use std::fmt::Write;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Everything a crash report says.
#[derive(Debug, Clone, Default)]
pub struct CrashReport {
    /// What happened, for example `panic` or `native crash (signal 11)`.
    pub kind: String,
    /// One-line summary: the panic message or the exception.
    pub headline: String,
    /// Extra `key: value` facts (location, thread, dump file).
    pub details: Vec<(String, String)>,
    /// The session context.
    pub context: Vec<(String, String)>,
    /// Recent structured failures, already rendered.
    pub failures: Vec<String>,
    /// The log tail, oldest first.
    pub log: Vec<String>,
    /// The captured backtrace, when there is one.
    pub backtrace: Option<String>,
    /// Whether to probe the machine now (safe only in a healthy process).
    pub probe_system: bool,
}

impl CrashReport {
    /// The report as text.
    pub fn render(&self) -> String {
        let mut text = String::new();
        writeln!(text, "STROBE CRASH REPORT").ok();
        writeln!(text, "===================").ok();
        writeln!(text, "kind:    {}", self.kind).ok();
        writeln!(text, "time:    {}", Utc::now().iso()).ok();
        writeln!(text, "summary: {}", self.headline).ok();
        for (key, value) in &self.details {
            writeln!(text, "{key}: {value}").ok();
        }
        text.push('\n');
        if self.probe_system {
            text.push_str(&diag::system_report(&self.context));
        } else {
            text.push_str(&diag::system_report_now(&self.context));
        }

        text.push_str("\nRecent failures\n---------------\n");
        if self.failures.is_empty() {
            text.push_str("  (none)\n");
        }
        for failure in &self.failures {
            for line in failure.lines() {
                writeln!(text, "  {line}").ok();
            }
            text.push('\n');
        }

        writeln!(text, "\nRecent log (last {} entries)", self.log.len()).ok();
        text.push_str("---------------------------\n");
        if self.log.is_empty() {
            text.push_str("  (logger was not running)\n");
        }
        for line in &self.log {
            writeln!(text, "{line}").ok();
        }

        text.push_str("\nBacktrace\n---------\n");
        match &self.backtrace {
            Some(backtrace) => text.push_str(backtrace),
            None => text.push_str("  (see the minidump; native crashes have no Rust backtrace)\n"),
        }
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }
}

/// Picks `crash-YYYYMMDD-HHMMSS` (or `…-2`, `…-3`) so that neither the
/// `.txt` nor the `.dmp` exists yet, creating `dir` if needed.
pub fn claim_stem(dir: &Path) -> io::Result<String> {
    fs::create_dir_all(dir)?;
    let base = format!("crash-{}", Utc::now().file_stamp());
    let mut stem = base.clone();
    let mut attempt = 1;
    while dir.join(format!("{stem}.txt")).exists() || dir.join(format!("{stem}.dmp")).exists() {
        attempt += 1;
        stem = format!("{base}-{attempt}");
    }
    Ok(stem)
}

/// Writes `text` to `dir/<stem>.txt`, returning the path.
pub fn write_text(dir: &Path, stem: &str, text: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{stem}.txt"));
    fs::write(&path, text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("strobe-diag-test-{}-{name}", std::process::id()))
    }

    fn sample() -> CrashReport {
        CrashReport {
            kind: "panic".to_string(),
            headline: "index out of bounds".to_string(),
            details: vec![("location".to_string(), "src/player.rs:10:5".to_string())],
            context: vec![("scene.name".to_string(), "09-sdf-cathedral".to_string())],
            failures: vec!["SHADER\nphase=link\nmessage=bad".to_string()],
            log: vec!["   1.000s INFO  [main] t: hello".to_string()],
            backtrace: Some("   0: main\n".to_string()),
            probe_system: false,
        }
    }

    #[test]
    fn the_report_has_every_section_in_order() -> Result<()> {
        let text = sample().render();
        let mut at = 0;
        for heading in [
            "STROBE CRASH REPORT",
            "summary: index out of bounds",
            "location: src/player.rs:10:5",
            "Build",
            "scene.name",
            "Recent failures",
            "SHADER",
            "Recent log (last 1 entries)",
            "hello",
            "Backtrace",
            "0: main",
        ] {
            let found = text[at..].find(heading);
            assert!(
                found.is_some(),
                "`{heading}` missing or out of order in:\n{text}"
            );
            at += found.unwrap_or(0);
        }
        Ok(())
    }

    #[test]
    fn a_native_report_points_at_the_minidump() -> Result<()> {
        let report = CrashReport {
            backtrace: None,
            ..sample()
        };
        assert!(report.render().contains("minidump"), "no minidump pointer");
        Ok(())
    }

    #[test]
    fn stems_do_not_collide() -> Result<()> {
        let dir = scratch("stems");
        let first = claim_stem(&dir)?;
        write_text(&dir, &first, "one")?;
        let second = claim_stem(&dir)?;
        fs::remove_dir_all(&dir)?;
        assert_ne!(
            first, second,
            "second crash in the same second reused a stem"
        );
        assert!(first.starts_with("crash-"), "stem was {first}");
        Ok(())
    }
}
