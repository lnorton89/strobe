// ============================================================================
// Module:       strobe_diag::panic_hook
// Description:  The panic hook: message, location, thread, backtrace, session
//               context and log tail written to crash-*.txt before the
//               process aborts.
//
// Dependencies: strobe_diag::{report, context, logger, failure, notify, paths}
// ============================================================================

//! The panic hook.
//!
//! Release builds use `panic = "abort"`: a panic runs the hook and then the
//! process aborts, with no unwinding and no `Drop`. The hook therefore does
//! all the work, in this order, so the most valuable evidence is on disk
//! first:
//!
//! 1. capture the backtrace (`force_capture`, regardless of
//!    `RUST_BACKTRACE`) and the context;
//! 2. write `crash-*.txt` and log the panic;
//! 3. flush the session log;
//! 4. run the previous hook (the default one prints the message and
//!    location to stderr);
//! 5. run the app's notifier.
//!
//! A panic inside the hook (or a second thread panicking while the first is
//! reporting) skips straight to the previous hook, so reporting can never
//! recurse or deadlock.
//!
//! When the process then aborts, the native handler in [`crate::crash`]
//! fires for the abort signal and writes a minidump. It reuses this
//! report's file stem so the pair is `crash-X.txt` plus `crash-X.dmp`,
//! rather than two reports for one crash. See [`last_report`].

use crate::notify::{self, CrashKind, CrashNotice};
use crate::report::{self, CrashReport};
use crate::{context, failure, logger, paths};
use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

/// Log records included in a crash report.
pub const LOG_TAIL: usize = 200;

/// Structured failures included in a crash report.
const FAILURE_TAIL: usize = 20;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static REPORTING: AtomicBool = AtomicBool::new(false);
static LAST_REPORT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Where the most recent panic report was written, if a panic has been
/// reported. Never blocks: the native crash handler calls this from a
/// compromised context.
pub fn last_report() -> Option<PathBuf> {
    LAST_REPORT.try_lock().ok()?.clone()
}

/// Installs the panic hook, chaining to whatever hook was set before.
/// Calling it again does nothing.
pub fn install_panic_hook() {
    install_with(paths::crashes_dir);
}

/// [`install_panic_hook`] with the crashes directory chosen by `dir`, so
/// tests can write somewhere disposable.
pub(crate) fn install_with(dir: impl Fn() -> PathBuf + Send + Sync + 'static) {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if REPORTING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            previous(info);
            return;
        }
        let notice = handle(info, &dir());
        previous(info);
        if let Some(notice) = notice {
            notify::notify(&notice);
        }
        REPORTING.store(false, Ordering::SeqCst);
    }));
}

/// The panic message: the payload when it is a string, otherwise a note.
fn message_of(info: &PanicHookInfo<'_>) -> String {
    info.payload_as_str()
        .map_or_else(|| "(non-string panic payload)".to_string(), str::to_string)
}

fn handle(info: &PanicHookInfo<'_>, dir: &std::path::Path) -> Option<CrashNotice> {
    let backtrace = Backtrace::force_capture().to_string();
    let message = message_of(info);
    let location = info
        .location()
        .map_or_else(|| "unknown".to_string(), ToString::to_string);
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>").to_string();

    log::error!(target: "strobe_diag::panic", "panic on thread `{thread_name}` at {location}: {message}");
    let report = CrashReport {
        kind: "panic".to_string(),
        headline: message.clone(),
        details: vec![
            ("location".to_string(), location),
            ("thread".to_string(), thread_name),
        ],
        context: context::snapshot(),
        failures: failure::recent(FAILURE_TAIL)
            .iter()
            .map(ToString::to_string)
            .collect(),
        log: logger::recent_lines(LOG_TAIL),
        backtrace: Some(backtrace),
        probe_system: false,
    };

    let written =
        report::claim_stem(dir).and_then(|stem| report::write_text(dir, &stem, &report.render()));
    let path = match written {
        Ok(path) => {
            log::error!(target: "strobe_diag::panic", "crash report written to {}", path.display());
            *LAST_REPORT.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone());
            Some(path)
        }
        Err(error) => {
            log::error!(target: "strobe_diag::panic", "could not write crash report in {}: {error}", dir.display());
            None
        }
    };
    logger::flush(logger::CRASH_FLUSH_TIMEOUT);
    Some(CrashNotice {
        kind: CrashKind::Panic,
        summary: message,
        report: path,
        dump: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use std::fs;

    #[test]
    fn a_panic_leaves_a_report_with_message_location_thread_and_backtrace() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("strobe-diag-test-{}-panic", std::process::id()));
        let target = dir.clone();
        install_with(move || target.clone());
        context::set("test.panic", "context-survives");

        let outcome = std::thread::Builder::new()
            .name("render-test".to_string())
            .spawn(|| {
                std::panic::catch_unwind(|| {
                    // A real panic, raised without the panic macros the
                    // workspace denies: an out-of-bounds index.
                    let values = std::hint::black_box(vec![1_u8]);
                    std::hint::black_box(values[std::hint::black_box(7)]);
                })
            })?
            .join();
        assert!(outcome.is_ok(), "the worker thread itself should not die");

        let report_path = last_report().ok_or_else(|| anyhow!("no report recorded"))?;
        let text = fs::read_to_string(&report_path)?;
        fs::remove_dir_all(&dir)?;
        for needle in [
            "kind:    panic",
            "index out of bounds",
            "thread: render-test",
            "location:",
            "panic_hook.rs",
            "test.panic",
            "context-survives",
            "Backtrace",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        Ok(())
    }
}
