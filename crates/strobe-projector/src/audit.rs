// ============================================================================
// Module:       strobe_projector::audit
// Description:  The command log: one tab-separated line per power command,
//               status query, refused request and DHCP lease, rotated at
//               1 MB with ten kept.
//
// Dependencies: chrono, log
// ============================================================================

//! Who switched what, and when.
//!
//! Every power command, status query, request turned away for want of a
//! token, and DHCP lease is one line:
//! `time<TAB>src=...<TAB>target=...<TAB>action=...<TAB>result=...`, where
//! the source is `cli:<user>`, `strobe`, `dhcp` or the client's address.
//! The file is `commands.log` in the directory given (beside the config:
//! `projector-logs/`). When it passes [`MAX_BYTES`] it becomes
//! `commands.log.1`, the older ones move up one, and the oldest beyond
//! [`KEEP`] goes.
//!
//! A log that cannot be written must not stop a projector command: the line
//! goes to the session log instead.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The size at which the log rotates.
pub const MAX_BYTES: u64 = 1_000_000;

/// Rotated files kept.
pub const KEEP: u32 = 10;

/// The log's file name.
const FILE: &str = "commands.log";

/// The directory beside `projectors.json` that holds the log.
pub const DIR: &str = "projector-logs";

/// Serialises writers within this process, so lines never interleave and
/// a rotation never races a write.
static WRITING: Mutex<()> = Mutex::new(());

/// The command log in one directory.
#[derive(Clone, Debug)]
pub struct Audit {
    dir: Option<PathBuf>,
}

impl Audit {
    /// The log in `dir`.
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: Some(dir.to_path_buf()),
        }
    }

    /// The log beside `config` (`<its directory>/projector-logs`).
    pub fn beside(config: &Path) -> Self {
        let dir = config.parent().unwrap_or_else(|| Path::new("."));
        Self::new(&dir.join(DIR))
    }

    /// A log that only reaches the session log (tests).
    pub const fn none() -> Self {
        Self { dir: None }
    }

    /// The file lines go to, if any.
    pub fn path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|dir| dir.join(FILE))
    }

    /// Appends one line.
    pub fn record(&self, source: &str, target: &str, action: &str, result: &str) {
        let time = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%z");
        let line = format!(
            "{time}\tsrc={source}\ttarget={target}\taction={action}\tresult={result}",
            source = clean(source),
            target = clean(target),
            action = clean(action),
            result = clean(result),
        );
        log::info!(target: "projector::audit", "{line}");
        let Some(dir) = &self.dir else {
            return;
        };
        let written = WRITING
            .lock()
            .map_err(|_| std::io::Error::other("audit lock poisoned"))
            .and_then(|_guard| append(dir, &line));
        if let Err(error) = written {
            log::warn!("projector command log {}: {error}", dir.display());
        }
    }
}

/// `text` on one line: a tab or line break in a projector's answer would
/// split the record.
fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

fn append(dir: &Path, line: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(FILE);
    if fs::metadata(&path).is_ok_and(|meta| meta.len() >= MAX_BYTES) {
        rotate(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{line}")
}

/// `commands.log` to `.1`, `.1` to `.2`, ..., dropping the one beyond
/// [`KEEP`].
fn rotate(dir: &Path) -> std::io::Result<()> {
    let numbered = |n: u32| dir.join(format!("{FILE}.{n}"));
    let oldest = numbered(KEEP);
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }
    for n in (1..KEEP).rev() {
        let from = numbered(n);
        if from.exists() {
            fs::rename(&from, numbered(n + 1))?;
        }
    }
    fs::rename(dir.join(FILE), numbered(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn lines_are_one_record_each_and_rotate() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("strobe-projector-audit-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        let audit = Audit::new(&dir);
        audit.record("cli:me", "sony", "on", "OK\nsecond line");
        let path = audit.path().ok_or_else(|| anyhow::anyhow!("path"))?;
        let text = fs::read_to_string(&path)?;
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(
            text.contains("\tsrc=cli:me\ttarget=sony\taction=on\tresult=OK second line"),
            "{text}"
        );
        fs::write(&path, vec![b'x'; usize::try_from(MAX_BYTES)?])?;
        audit.record("dhcp", "04-5d-4b-96-32-7c", "lease", "ACK 192.168.50.11");
        assert!(dir.join("commands.log.1").exists(), "rotated");
        assert_eq!(fs::read_to_string(&path)?.lines().count(), 1, "fresh file");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
