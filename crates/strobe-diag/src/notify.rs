// ============================================================================
// Module:       strobe_diag::notify
// Description:  The app-supplied hook that tells a person a crash happened
//               (a message box on Windows), after the report is on disk.
//
// Dependencies: std::sync
// ============================================================================

//! Crash notification.
//!
//! This crate writes reports; it does not draw UI. The app registers one
//! notifier with [`set_notifier`], and it runs after the report (and, for
//! native crashes, the minidump) is safely on disk, so a notifier that
//! hangs or fails cannot lose the evidence.
//!
//! Where it runs matters. A panic notifies from the panicking thread of the
//! app process. A native crash notifies from the **monitor process**, not
//! the crashed one: the crashed process is in an unknown state and may not
//! be able to open a window, while the monitor is healthy. Because the
//! monitor is the same executable started with a hidden argument, the app
//! must call [`set_notifier`] before [`crate::crash::run_monitor_if_requested`].
//!
//! A run nobody watches (a screenshot run, a worker another process
//! started) calls [`set_unattended`] before [`crate::crash::start_monitor`].
//! Notices are then never handed to the notifier, in this process or in
//! its monitor, which is told by its command line: a message box that
//! waits for a click would otherwise hold the run, and anything waiting
//! for its process tree to exit, until someone happened to dismiss it.
//! The reports are written as usual.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// What kind of failure is being reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashKind {
    /// A Rust panic, caught by the panic hook.
    Panic,
    /// An access violation, stack overflow, abort or similar, caught by
    /// the native handler.
    Native,
}

/// What the notifier is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashNotice {
    /// What happened.
    pub kind: CrashKind,
    /// One line suitable for a message box.
    pub summary: String,
    /// The `crash-*.txt` report, when it was written.
    pub report: Option<PathBuf>,
    /// The `crash-*.dmp` minidump, when one was written.
    pub dump: Option<PathBuf>,
}

/// A notifier callback.
pub type Notifier = Box<dyn Fn(&CrashNotice) + Send + Sync>;

static NOTIFIER: OnceLock<Notifier> = OnceLock::new();

static UNATTENDED: AtomicBool = AtomicBool::new(false);

/// Registers the notifier. Only the first registration takes effect;
/// returns whether this one did.
pub fn set_notifier(notifier: impl Fn(&CrashNotice) + Send + Sync + 'static) -> bool {
    NOTIFIER.set(Box::new(notifier)).is_ok()
}

/// Marks this run as one nobody watches: crash notices are not handed to
/// the notifier, here or in a crash monitor started after this call. The
/// reports are still written.
pub fn set_unattended() {
    UNATTENDED.store(true, Ordering::Relaxed);
}

/// Whether [`set_unattended`] was called.
pub fn is_unattended() -> bool {
    UNATTENDED.load(Ordering::Relaxed)
}

/// Runs the notifier, if any and if someone is watching. A notifier that
/// panics must not take the crash path down with it, so the call is
/// isolated.
pub(crate) fn notify(notice: &CrashNotice) {
    if is_unattended() {
        return;
    }
    if let Some(notifier) = NOTIFIER.get() {
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| notifier(notice)));
        if attempt.is_err() {
            log::error!("crash notifier panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::sync::{Mutex, PoisonError};

    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

    #[test]
    fn the_first_notifier_wins_and_receives_notices() -> Result<()> {
        // Other tests (the panic hook's) notify too, so look for ours.
        let first = set_notifier(|notice| {
            SEEN.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(notice.summary.clone());
        });
        let second = set_notifier(|_| {});
        assert!(!second, "a later registration must never win");
        assert!(first, "this test registers first in its process");
        notify(&CrashNotice {
            kind: CrashKind::Panic,
            summary: "notify-test".to_string(),
            report: None,
            dump: None,
        });
        let seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(
            seen.iter().any(|summary| summary == "notify-test"),
            "notifier was not called: {seen:?}"
        );
        Ok(())
    }
}
