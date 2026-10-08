// ============================================================================
// Module:       strobe_diag (library crate root)
// Description:  Logging, session context, panic reports and out-of-process
//               native crash capture, so a failed show leaves usable evidence.
//
// Dependencies: log, dirs, crash-handler, minidumper
// ============================================================================

//! Diagnostics for a show that cannot be retried.
//!
//! Strobe runs fullscreen on a projector, often on an old laptop nobody is
//! sitting at. When it hangs or dies, the only evidence is what it left
//! behind, so this crate makes sure that is enough to fix the bug:
//!
//! - [`logger`]: a `log` backend with timestamps, thread names, a rotating
//!   file per session, and a ring buffer the UI console reads. Level from
//!   the `STROBE_LOG` environment variable ([`filter`]).
//! - [`context`]: facts about the session (GL renderer, scene, frame
//!   count) that every report includes.
//! - [`failure`]: structured, user-visible failures (subsystem, scene,
//!   phase, root error, recovery action).
//! - [`panic_hook`]: a panic writes `crash-*.txt` with a backtrace, the
//!   context and the log tail, even under `panic = "abort"`.
//! - [`crash`]: native crashes (driver access violations, stack overflows,
//!   aborts) are caught and a minidump is written by a second process.
//! - [`diag`]: the `--diagnose` report.
//!
//! [`init`] wires up the first four in the right order. The only unsafe
//! code is one call in [`native`].
//!
//! ## Integration, in order
//!
//! ```text
//! fn main() {
//!     // 1. Register how a person is told (message box). Before step 2,
//!     //    because the monitor process reports native crashes with it.
//!     strobe_diag::notify::set_notifier(|notice| show_message_box(notice));
//!     // 2. If this process is the crash monitor, run it and exit.
//!     if strobe_diag::crash::run_monitor_if_requested(std::env::args()) { return; }
//!     // 3. Logging, panic hook, background system probe.
//!     let session = strobe_diag::init();
//!     // 4. `--diagnose` prints `strobe_diag::diag::system_report(&[])` and exits.
//!     // 5. On the main thread, arm native crash capture; hold the handle.
//!     let _crash_capture = strobe_diag::crash::start_monitor();
//!     // 6. After the GL context exists: strobe_diag::context::set(GL_RENDERER, …).
//!     // … run …
//! }
//! ```

pub mod buildinfo;
pub mod clock;
pub mod context;
pub mod crash;
pub mod diag;
pub mod failure;
pub mod filter;
pub mod logger;
mod native;
pub mod notify;
pub mod panic_hook;
pub mod paths;
pub mod report;
pub mod system;

pub use panic_hook::install_panic_hook;

use std::path::PathBuf;

/// What [`init`] set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The session log file, or `None` if it could not be opened (the
    /// logger then keeps lines in memory and on stderr only).
    pub log_file: Option<PathBuf>,
}

/// Starts the clock, installs the logger and the panic hook, logs the
/// build and starts the background system probe. Safe to call once, early;
/// a second call (or another global logger) leaves the first in place and
/// reports no log file.
pub fn init() -> Session {
    clock::start();
    let log_file = logger::init().ok().flatten();
    install_panic_hook();
    system::probe_in_background();
    log::info!("strobe {} starting", buildinfo::summary());
    Session { log_file }
}
