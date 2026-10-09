// ============================================================================
// Module:       strobe_diag::logger
// Description:  The `log` backend: filtered, timestamped lines to a rotating
//               session file and stderr via a writer thread, plus an
//               in-memory ring for the UI console and crash reports.
//
// Dependencies: log; strobe_diag::{filter, clock, paths, buildinfo}
// ============================================================================

//! Logging.
//!
//! [`init`] installs a [`Logger`] as the global `log` backend. The cost
//! on the thread that logs is small and bounded on purpose, because that
//! thread may be the render thread:
//!
//! 1. the filter check runs first, so disabled records cost a comparison
//!    (and the `log` macros skip even that above `max_level`);
//! 2. an enabled record is formatted once into a shared string;
//! 3. that string goes into the ring buffer (a mutex held for one
//!    `push_back`) and into a bounded channel (`try_send`, never blocking).
//!
//! No file or console I/O happens on the logging thread. A writer thread
//! drains the channel, writes the session file, and flushes whenever it
//! catches up. If the channel is full (the disk stalled) lines are counted
//! and dropped, and the writer reports the count when it resumes, rather
//! than stalling a frame. [`flush`] is the synchronous escape hatch the
//! crash paths use: it asks the writer to flush and waits, with a timeout.
//!
//! The ring holds the last [`RING_LINES`] records. A record that spans
//! several lines (a failure report) is one entry. The UI console reads it
//! with [`recent_lines`], and so do crash reports.
//!
//! Line format:
//!
//! ```text
//!    12.345s INFO  [main] strobe_gfx::player: compiled 3 passes
//! ```
//!
//! Time is seconds since process start; the session header line at the top
//! of each file carries the wall clock, in UTC.

use crate::clock::{self, Utc};
use crate::filter::Filter;
use crate::{buildinfo, paths};
use log::{Level, Log, Metadata, Record};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

/// Records kept in the in-memory ring.
pub const RING_LINES: usize = 2000;

/// Session logs kept on disk, newest first, including the current one.
pub const KEEP_LOGS: usize = 10;

/// Records that may wait for the writer thread before new ones are dropped.
const CHANNEL_DEPTH: usize = 8192;

/// How long [`flush`] callers on the crash path wait for the writer.
pub const CRASH_FLUSH_TIMEOUT: Duration = Duration::from_millis(500);

/// What the logger writes and where.
#[derive(Debug, Clone)]
pub struct LoggerConfig {
    /// Which records pass.
    pub filter: Filter,
    /// Directory for the session file; `None` keeps logs in memory only.
    pub dir: Option<PathBuf>,
    /// Whether to mirror lines to stderr.
    pub stderr: bool,
}

impl LoggerConfig {
    /// The production configuration: `STROBE_LOG` for the filter, the
    /// per-user logs directory, and stderr in debug builds (or when
    /// `STROBE_LOG_STDERR=1`, for a console build on a test laptop).
    pub fn from_env() -> Self {
        Self {
            filter: Filter::from_env(),
            dir: Some(paths::logs_dir()),
            stderr: cfg!(debug_assertions)
                || std::env::var("STROBE_LOG_STDERR").is_ok_and(|value| value == "1"),
        }
    }
}

enum Message {
    Line(Arc<str>),
    Flush(SyncSender<()>),
}

/// The `log::Log` implementation. Use [`init`] to install the global one;
/// construct directly only in tests.
#[derive(Debug)]
pub struct Logger {
    filter: Filter,
    ring: Mutex<VecDeque<Arc<str>>>,
    sink: Option<SyncSender<Message>>,
    dropped: Arc<AtomicU64>,
    /// Records ever pushed, so a reader can tell the ring changed.
    written: AtomicU64,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Line(_) => f.write_str("Line"),
            Self::Flush(_) => f.write_str("Flush"),
        }
    }
}

impl Logger {
    /// Builds a logger, opens the session file, and starts the writer
    /// thread. Failure to open the file degrades to memory and stderr.
    pub fn new(config: LoggerConfig) -> Self {
        clock::start();
        let dropped = Arc::new(AtomicU64::new(0));
        let mut notes = Vec::new();
        let opened = config
            .dir
            .as_deref()
            .and_then(|dir| match open_session_file(dir) {
                Ok(opened) => Some(opened),
                Err(error) => {
                    notes.push(format!(
                        "log file unavailable in {}: {error}",
                        dir.display()
                    ));
                    None
                }
            });
        let path = opened.as_ref().map(|(_, path)| path.clone());
        let file = opened.map(|(file, _)| BufWriter::new(file));

        let sink = if file.is_some() || config.stderr {
            spawn_writer(file, config.stderr, Arc::clone(&dropped))
                .map_err(|error| notes.push(format!("log writer thread failed: {error}")))
                .ok()
        } else {
            None
        };

        let logger = Self {
            filter: config.filter,
            ring: Mutex::new(VecDeque::with_capacity(RING_LINES)),
            sink,
            dropped,
            written: AtomicU64::new(0),
            path,
        };
        logger.write_header(&notes);
        logger
    }

    /// The session file, if one could be opened.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The filter in force.
    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// The newest `count` records, oldest first.
    pub fn recent(&self, count: usize) -> Vec<String> {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        let skip = ring.len().saturating_sub(count);
        ring.iter()
            .skip(skip)
            .map(|line| line.to_string())
            .collect()
    }

    /// Like [`Logger::recent`] but never waits for the ring's lock: the
    /// crash handler may be running on the thread that holds it.
    pub fn try_recent(&self, count: usize) -> Option<Vec<String>> {
        let ring = self.ring.try_lock().ok()?;
        let skip = ring.len().saturating_sub(count);
        Some(
            ring.iter()
                .skip(skip)
                .map(|line| line.to_string())
                .collect(),
        )
    }

    /// Asks the writer thread to flush the file and waits up to `timeout`.
    /// Returns whether the flush completed.
    pub fn flush_within(&self, timeout: Duration) -> bool {
        let Some(sink) = &self.sink else {
            return true;
        };
        let (ack_tx, ack_rx) = sync_channel(1);
        if sink.try_send(Message::Flush(ack_tx)).is_err() {
            return false;
        }
        match ack_rx.recv_timeout(timeout) {
            Ok(()) => true,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => false,
        }
    }

    /// Records pushed since the logger started, including those the ring
    /// has since evicted. One relaxed load.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    fn push(&self, text: Arc<str>) {
        {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            if ring.len() == RING_LINES {
                ring.pop_front();
            }
            ring.push_back(Arc::clone(&text));
            self.written.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(sink) = &self.sink
            && sink.try_send(Message::Line(text)).is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn write_header(&self, notes: &[String]) {
        let started = Utc::now().iso();
        self.push(
            format!("=== Strobe session started {started} (log times are seconds since start) ===")
                .into(),
        );
        self.push(format!("build {}", buildinfo::summary()).into());
        self.push(format!("log filter: {}", self.filter.describe()).into());
        for problem in self.filter.problems() {
            self.push(format!("STROBE_LOG: ignored {problem}").into());
        }
        for note in notes {
            self.push(format!("diag: {note}").into());
        }
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.filter.enabled(metadata.target(), metadata.level())
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let text = format_record(
            clock::uptime(),
            record.level(),
            std::thread::current().name().unwrap_or("?"),
            record.target(),
            &record.args().to_string(),
        );
        self.push(text.into());
    }

    fn flush(&self) {
        self.flush_within(Duration::from_secs(2));
    }
}

/// Formats one record. Continuation lines of a multi-line message are
/// indented so a record stays visually one unit.
fn format_record(
    uptime: Duration,
    level: Level,
    thread: &str,
    target: &str,
    message: &str,
) -> String {
    let body = if message.contains('\n') {
        message.replace('\n', "\n    ")
    } else {
        message.to_string()
    };
    format!(
        "{:>9.3}s {:<5} [{thread}] {target}: {body}",
        uptime.as_secs_f64(),
        level.as_str()
    )
}

/// Creates the session file, first deleting the oldest logs so at most
/// [`KEEP_LOGS`] remain afterwards.
fn open_session_file(dir: &Path) -> std::io::Result<(File, PathBuf)> {
    fs::create_dir_all(dir)?;
    prune_old_logs(dir, KEEP_LOGS.saturating_sub(1));
    let name = format!(
        "strobe-{}-{}.log",
        Utc::now().file_stamp(),
        std::process::id()
    );
    let path = dir.join(name);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    Ok((file, path))
}

/// Deletes all but the newest `keep` session logs in `dir`. Names embed a
/// UTC timestamp, so name order is age order. Returns how many were
/// deleted; deletion failures (a log still open elsewhere) are skipped.
pub fn prune_old_logs(dir: &Path, keep: usize) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("strobe-") && name.ends_with(".log"))
        })
        .collect();
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    logs.iter()
        .take(excess)
        .filter(|path| fs::remove_file(path).is_ok())
        .count()
}

fn spawn_writer(
    file: Option<BufWriter<File>>,
    stderr: bool,
    dropped: Arc<AtomicU64>,
) -> std::io::Result<SyncSender<Message>> {
    let (tx, rx) = sync_channel(CHANNEL_DEPTH);
    std::thread::Builder::new()
        .name("strobe-log".to_string())
        .spawn(move || writer_loop(&rx, file, stderr, &dropped))?;
    Ok(tx)
}

fn writer_loop(
    rx: &Receiver<Message>,
    mut file: Option<BufWriter<File>>,
    stderr: bool,
    dropped: &AtomicU64,
) {
    while let Ok(first) = rx.recv() {
        let mut next = Some(first);
        while let Some(message) = next.take() {
            handle(message, file.as_mut(), stderr);
            next = rx.try_recv().ok();
        }
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0 {
            let note = format!("diag: {lost} log lines dropped while the writer was behind");
            handle(Message::Line(note.into()), file.as_mut(), stderr);
        }
        if let Some(file) = file.as_mut() {
            file.flush().ok();
        }
    }
}

fn handle(message: Message, file: Option<&mut BufWriter<File>>, stderr: bool) {
    match message {
        Message::Line(text) => {
            if let Some(file) = file {
                writeln!(file, "{text}").ok();
            }
            if stderr {
                writeln!(std::io::stderr(), "{text}").ok();
            }
        }
        Message::Flush(ack) => {
            if let Some(file) = file {
                file.flush().ok();
            }
            ack.send(()).ok();
        }
    }
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Installs the production logger (see [`LoggerConfig::from_env`]).
/// Returns the session file path, or an error if a global logger already
/// exists.
pub fn init() -> Result<Option<PathBuf>, log::SetLoggerError> {
    init_with(LoggerConfig::from_env())
}

/// Installs a logger with an explicit configuration.
pub fn init_with(config: LoggerConfig) -> Result<Option<PathBuf>, log::SetLoggerError> {
    let logger = LOGGER.get_or_init(|| Logger::new(config));
    log::set_logger(logger)?;
    log::set_max_level(logger.filter.max_level());
    Ok(logger.path.clone())
}

/// The session file, once [`init`] has run.
pub fn log_file() -> Option<PathBuf> {
    LOGGER.get()?.path.clone()
}

/// The newest `count` log records, oldest first. Empty before [`init`].
/// For the UI console.
pub fn recent_lines(count: usize) -> Vec<String> {
    LOGGER
        .get()
        .map_or_else(Vec::new, |logger| logger.recent(count))
}

/// How many records have been logged since [`init`]; zero before it. A UI
/// console compares this with the value it last saw and calls
/// [`recent_lines`] only when it moved, so an idle console costs one
/// atomic load per repaint.
pub fn records_written() -> u64 {
    LOGGER.get().map_or(0, Logger::written)
}

/// [`recent_lines`] that never blocks, for crash handlers.
pub fn try_recent_lines(count: usize) -> Vec<String> {
    LOGGER
        .get()
        .and_then(|logger| logger.try_recent(count))
        .unwrap_or_default()
}

/// Flushes the session file, waiting up to `timeout`. Returns whether the
/// writer confirmed. For crash paths and clean shutdown.
pub fn flush(timeout: Duration) -> bool {
    LOGGER
        .get()
        .is_none_or(|logger| logger.flush_within(timeout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    use log::LevelFilter;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("strobe-diag-test-{}-{name}", std::process::id()))
    }

    fn memory_logger(spec: &str) -> Logger {
        Logger::new(LoggerConfig {
            filter: Filter::parse(spec, LevelFilter::Info),
            dir: None,
            stderr: false,
        })
    }

    fn emit(logger: &Logger, level: Level, target: &str, message: &str) {
        logger.log(
            &Record::builder()
                .level(level)
                .target(target)
                .args(format_args!("{message}"))
                .build(),
        );
    }

    #[test]
    fn a_record_has_time_level_thread_target_and_message() -> Result<()> {
        let line = format_record(
            Duration::from_millis(12_345),
            Level::Warn,
            "main",
            "strobe_gfx::player",
            "slow frame",
        );
        assert_eq!(
            line, "   12.345s WARN  [main] strobe_gfx::player: slow frame",
            "line layout"
        );
        Ok(())
    }

    #[test]
    fn continuation_lines_are_indented() -> Result<()> {
        let line = format_record(Duration::ZERO, Level::Error, "t", "x", "SHADER\nscene=a");
        assert!(line.ends_with("SHADER\n    scene=a"), "got {line:?}");
        Ok(())
    }

    #[test]
    fn the_written_count_moves_with_every_kept_record() -> Result<()> {
        let logger = memory_logger("info");
        let before = logger.written();
        emit(&logger, Level::Info, "t", "one");
        emit(&logger, Level::Trace, "t", "filtered out");
        emit(&logger, Level::Warn, "t", "two");
        assert_eq!(logger.written(), before + 2, "only kept records count");
        Ok(())
    }

    #[test]
    fn disabled_records_are_not_kept() -> Result<()> {
        let logger = memory_logger("warn,strobe_gfx=trace");
        emit(&logger, Level::Info, "winit", "hidden");
        emit(&logger, Level::Trace, "strobe_gfx::gl", "shown");
        let kept = logger.recent(RING_LINES);
        assert!(
            kept.iter().all(|line| !line.contains("hidden")),
            "filtered record leaked: {kept:?}"
        );
        assert!(
            kept.iter().any(|line| line.contains("shown")),
            "enabled record missing: {kept:?}"
        );
        Ok(())
    }

    #[test]
    fn the_ring_keeps_only_the_newest_records() -> Result<()> {
        let logger = memory_logger("info");
        for index in 0..(RING_LINES + 50) {
            emit(&logger, Level::Info, "t", &format!("record {index}"));
        }
        let all = logger.recent(usize::MAX);
        assert_eq!(all.len(), RING_LINES, "ring is bounded");
        let last = all.last().ok_or_else(|| anyhow!("ring is empty"))?;
        assert!(
            last.ends_with(&format!("record {}", RING_LINES + 49)),
            "newest record last, got {last}"
        );
        assert!(
            all.iter().all(|line| !line.ends_with("record 0")),
            "oldest records were evicted"
        );
        let tail = logger.recent(3);
        assert_eq!(tail.len(), 3, "recent(n) returns n records");
        Ok(())
    }

    #[test]
    fn the_session_file_gets_the_lines_after_a_flush() -> Result<()> {
        let dir = scratch("file");
        let logger = Logger::new(LoggerConfig {
            filter: Filter::parse("info", LevelFilter::Info),
            dir: Some(dir.clone()),
            stderr: false,
        });
        emit(&logger, Level::Info, "t", "written to disk");
        assert!(
            logger.flush_within(Duration::from_secs(5)),
            "flush timed out"
        );
        let path = logger.path().ok_or_else(|| anyhow!("no log file"))?;
        let text = fs::read_to_string(path)?;
        fs::remove_dir_all(&dir)?;
        assert!(text.contains("written to disk"), "file was: {text}");
        assert!(
            text.contains("Strobe session started"),
            "header missing: {text}"
        );
        Ok(())
    }

    #[test]
    fn only_the_newest_logs_survive_pruning() -> Result<()> {
        let dir = scratch("prune");
        fs::create_dir_all(&dir)?;
        for index in 0..14 {
            fs::write(
                dir.join(format!("strobe-20260101-0000{index:02}-1.log")),
                "x",
            )?;
        }
        fs::write(dir.join("notes.txt"), "keep me")?;
        let removed = prune_old_logs(&dir, 10);
        let mut left: Vec<String> = fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        left.sort();
        fs::remove_dir_all(&dir)?;
        assert_eq!(removed, 4, "four oldest removed");
        assert_eq!(left.len(), 11, "ten logs plus the unrelated file: {left:?}");
        assert!(
            left.contains(&"notes.txt".to_string()),
            "unrelated file kept"
        );
        assert!(
            !left.iter().any(|name| name.ends_with("000000-1.log")),
            "oldest log was kept: {left:?}"
        );
        Ok(())
    }

    #[test]
    fn opening_a_session_keeps_the_total_at_ten() -> Result<()> {
        let dir = scratch("rotate");
        fs::create_dir_all(&dir)?;
        for index in 0..12 {
            fs::write(
                dir.join(format!("strobe-20250101-0000{index:02}-1.log")),
                "x",
            )?;
        }
        let (_file, path) = open_session_file(&dir)?;
        let count = fs::read_dir(&dir)?.count();
        fs::remove_dir_all(&dir)?;
        assert_eq!(count, KEEP_LOGS, "ten logs including {}", path.display());
        Ok(())
    }
}
