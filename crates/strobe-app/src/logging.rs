// ============================================================================
// Module:       strobe_app::logging
// Description:  A minimal logger: stderr plus a log file, so a release build
//               with no console still leaves a trail to read.
//
// Dependencies: log
// ============================================================================

//! Logging.
//!
//! The release binary is a Windows GUI application with no console, so
//! `stderr` goes nowhere. Every line also lands in `strobe.log` in the
//! system temp folder, which is also where the startup-failure dialog
//! points the person looking at it. Strobe's own crates log at `info`;
//! everything else (winit, glutin, egui) only at `warn`.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

struct Logger {
    file: Mutex<Option<File>>,
}

static LOGGER: Logger = Logger {
    file: Mutex::new(None),
};

fn wanted(metadata: &Metadata<'_>) -> bool {
    if metadata.target().starts_with("strobe") {
        metadata.level() <= Level::Info
    } else {
        metadata.level() <= Level::Warn
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        wanted(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if !wanted(record.metadata()) {
            return;
        }
        let line = format!(
            "{:<5} {}: {}",
            record.level(),
            record.target(),
            record.args()
        );
        eprintln!("{line}");
        if let Ok(mut guard) = self.file.lock()
            && let Some(file) = guard.as_mut()
        {
            let _ = writeln!(file, "{line}");
        }
    }

    fn flush(&self) {}
}

/// Where the log file lives.
pub fn log_path() -> PathBuf {
    std::env::temp_dir().join("strobe.log")
}

/// Installs the logger. A second call changes nothing.
pub fn init() {
    if let Ok(mut guard) = LOGGER.file.lock() {
        *guard = File::create(log_path()).ok();
    }
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
}
