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
//!
//! The directory does not grow without bound: [`prune_crashes`] keeps the
//! newest [`KEEP_CRASHES`] pairs, by file modification time (D396). It runs
//! from [`crate::init`] at start-up, never during a show.

use crate::clock::Utc;
use crate::diag;
use crate::rotate;
use std::fmt::Write;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many crash pairs (`.txt` plus `.dmp`) are kept. A minidump is a few
/// megabytes, and twenty crashes is more history than anyone reads.
pub const KEEP_CRASHES: usize = 20;

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

/// Deletes all but the newest `keep` crash pairs in `dir`, returning how many
/// files were removed. A pair is every `crash-<stem>.txt` / `.dmp` sharing a
/// stem; its age is that of the member modified last (the monitor appends to
/// the `.txt` after the dump is written). Order is by modification time, not
/// by the timestamp in the name, which a clock that jumped backwards makes
/// wrong (D396). Other files are left alone and a failed delete is skipped.
pub fn prune_crashes(dir: &Path, keep: usize) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut pairs: Vec<(String, SystemTime)> = Vec::new();
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        let Some(stem) = crash_stem(&path) else {
            continue;
        };
        let time = rotate::modified(&path);
        match pairs.iter_mut().find(|(known, _)| *known == stem) {
            Some((_, newest)) => *newest = (*newest).max(time),
            None => pairs.push((stem, time)),
        }
    }
    let mut removed = 0;
    for stem in rotate::beyond_newest(pairs, keep) {
        for extension in ["dmp", "txt"] {
            let path = dir.join(format!("{stem}.{extension}"));
            if path.is_file() && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// `crash-…` for a `crash-….txt` or `crash-….dmp` path.
fn crash_stem(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?;
    if extension != "txt" && extension != "dmp" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    stem.starts_with("crash-").then(|| stem.to_string())
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

    fn touch(dir: &Path, name: &str, age_seconds: u64) -> Result<()> {
        let path = dir.join(name);
        fs::write(&path, "x")?;
        let when = SystemTime::now() - std::time::Duration::from_secs(age_seconds);
        fs::File::options()
            .write(true)
            .open(&path)?
            .set_modified(when)?;
        Ok(())
    }

    fn names(dir: &Path) -> Result<Vec<String>> {
        let mut names: Vec<String> = fs::read_dir(dir)?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        Ok(names)
    }

    #[test]
    fn only_the_newest_crash_pairs_survive_and_pairs_stay_whole() -> Result<()> {
        let dir = scratch("prune-crashes");
        fs::create_dir_all(&dir)?;
        // Thirty pairs, one per minute, the oldest first.
        for index in 0..30_u64 {
            let age = (30 - index) * 60;
            touch(&dir, &format!("crash-20261001-{index:06}.txt"), age)?;
            touch(&dir, &format!("crash-20261001-{index:06}.dmp"), age)?;
        }
        // A panic report has no dump; unrelated files are not ours to touch.
        touch(&dir, "crash-20261001-999999.txt", 1)?;
        touch(&dir, "notes.txt", 100_000)?;
        touch(&dir, "crash-notes.log", 100_000)?;
        let removed = prune_crashes(&dir, KEEP_CRASHES);
        let left = names(&dir)?;
        fs::remove_dir_all(&dir)?;
        // 31 pairs, 20 kept: the 11 oldest go, two files each.
        assert_eq!(removed, 22, "eleven pairs of two files: {left:?}");
        let crashes = left
            .iter()
            .filter(|name| name.starts_with("crash-2026"))
            .count();
        assert_eq!(crashes, 39, "19 pairs and one lone report: {left:?}");
        assert!(
            left.contains(&"notes.txt".to_string()),
            "unrelated: {left:?}"
        );
        assert!(
            left.contains(&"crash-notes.log".to_string()),
            "not a report: {left:?}"
        );
        for stem in ["crash-20261001-000010", "crash-20261001-000029"] {
            let txt = left.contains(&format!("{stem}.txt"));
            let dmp = left.contains(&format!("{stem}.dmp"));
            let expected = stem.ends_with("29");
            assert_eq!((txt, dmp), (expected, expected), "{stem} in {left:?}");
        }
        Ok(())
    }

    #[test]
    fn a_clock_that_went_backwards_does_not_cost_the_newest_crash() -> Result<()> {
        let dir = scratch("prune-crashes-clock");
        fs::create_dir_all(&dir)?;
        // The machine's clock was reset to 2001 for the last two crashes:
        // their names sort first but they are the newest files.
        for index in 0..22_u64 {
            touch(
                &dir,
                &format!("crash-20261001-{index:06}.txt"),
                10_000 - index,
            )?;
        }
        touch(&dir, "crash-20010101-000000.txt", 20)?;
        touch(&dir, "crash-20010101-000100.txt", 10)?;
        prune_crashes(&dir, KEEP_CRASHES);
        let left = names(&dir)?;
        fs::remove_dir_all(&dir)?;
        assert_eq!(left.len(), KEEP_CRASHES, "twenty kept: {left:?}");
        for newest in ["crash-20010101-000000.txt", "crash-20010101-000100.txt"] {
            assert!(
                left.contains(&newest.to_string()),
                "{newest} deleted: {left:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_pair_is_as_new_as_its_newest_member() -> Result<()> {
        let dir = scratch("prune-crashes-pair");
        fs::create_dir_all(&dir)?;
        // The dump is old but the monitor appended to the report later.
        touch(&dir, "crash-a.dmp", 5_000)?;
        touch(&dir, "crash-a.txt", 1)?;
        touch(&dir, "crash-b.txt", 100)?;
        touch(&dir, "crash-b.dmp", 100)?;
        prune_crashes(&dir, 1);
        let left = names(&dir)?;
        fs::remove_dir_all(&dir)?;
        assert_eq!(left, ["crash-a.dmp", "crash-a.txt"], "pair a is the newer");
        Ok(())
    }

    #[test]
    fn pruning_a_missing_directory_is_a_no_op() -> Result<()> {
        assert_eq!(prune_crashes(&scratch("never-made"), 20), 0, "nothing");
        Ok(())
    }
}
