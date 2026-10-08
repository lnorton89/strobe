// ============================================================================
// Module:       strobe_diag::native
// Description:  The only unsafe code in strobe-diag: one leaf wrapper that
//               builds the crash handler's callback object.
//
// Dependencies: crash-handler
// ============================================================================

//! The native-crash unsafe leaf.
//!
//! `crash-handler` exposes exactly one unsafe entry point that this crate
//! needs: `make_crash_event`, which turns a closure into the object that
//! the signal handler (Linux/macOS) or exception filter (Windows) calls.
//! Attaching that object to the process, [`crash_handler::CrashHandler::attach`],
//! is a safe function, and so is everything else in [`crate::crash`].
//!
//! This file follows the same leaf rules as `strobe-gfx/src/gl/`: one
//! named, private-to-the-crate, **safe** function per unsafe operation;
//! the block holds that one call and nothing else; the `SAFETY` comment
//! states the real reasoning. `strobe_conventions::unsafe_boundary`
//! allows unsafe in this file and in the GL module and nowhere else.

use crash_handler::{CrashContext, CrashEvent, CrashEventResult};

/// Wraps `on_crash` as the object `CrashHandler::attach` takes.
///
/// The closure is invoked in a compromised context: on a signal handler's
/// alternate stack on Unix, inside an exception filter on Windows, after
/// the process has already violated some invariant. Callers must keep it
/// to work that cannot deadlock or depend on state the crash may have
/// corrupted.
pub(crate) fn crash_event<F>(on_crash: F) -> Box<dyn CrashEvent>
where
    F: Fn(&CrashContext) -> CrashEventResult + Send + Sync + 'static,
{
    // SAFETY: `make_crash_event` is unsafe only because the closure runs
    // in a compromised context, where calling non-async-signal-safe
    // functions or taking locks the crashed thread holds can deadlock or
    // misbehave. Its sole caller, `crash::start_monitor`, passes a closure
    // that takes no lock without `try_lock`, performs no blocking I/O
    // except writes to an already-connected local socket and a log flush
    // that is bounded by a timeout, and never unwinds. The function itself
    // has no other preconditions: the closure is `Send + Sync + 'static`
    // by its bounds, so it may be called from any thread for the life of
    // the process.
    unsafe { crash_handler::make_crash_event(on_crash) }
}
