// ============================================================================
// Module:       strobe_gfx::gl::debug
// Description:  KHR_debug driver messages routed to the `log` crate with
//               severity mapping and per-message-id rate limiting.
//
// Dependencies: glow (debug output); log; strobe_gfx::gl (Gl)
// ============================================================================

//! GL driver debug output.
//!
//! Drivers explain themselves through `KHR_debug` (core in GL 4.3 and
//! GLES 3.2): an invalid enum, a shader recompiled behind your back, a
//! pixel-transfer stall. [`enable_debug_output`] turns that on and routes
//! every message to the `log` crate under the target `gl_driver`, so
//! `STROBE_LOG=gl_driver=debug` shows them next to everything else and a
//! crash report's log tail contains the messages that led up to it.
//!
//! - **Severity** maps to log level: high is `error`, medium `warn`, low
//!   `info`, notification `debug`. [`DebugLevel`] also tells the driver not
//!   to generate the severities below the chosen level at all.
//! - **Synchronous in debug builds.** `GL_DEBUG_OUTPUT_SYNCHRONOUS` makes
//!   the driver call back on the offending call, so a breakpoint in the
//!   callback has the guilty stack. Release builds leave it asynchronous,
//!   which is cheaper but may report on another thread.
//! - **Rate limited.** A message repeated every frame would bury the log
//!   and cost a formatted string per frame. Each distinct
//!   (source, type, id) is passed through at most [`MAX_PER_WINDOW`] times
//!   per [`WINDOW`]; the next message after a quiet window reports how
//!   many were suppressed.
//! - **Never panics.** The callback runs inside the driver, across an FFI
//!   boundary; a panic there would abort. It takes poisoned locks as they
//!   are, and is wrapped in `catch_unwind` regardless.
//!
//! The newest messages are also kept in a small ring, readable with
//! [`recent_messages`], for a UI console.
//!
//! ## Why `&mut Gl`
//!
//! `glow` installs the callback through `&mut Context`, and `Gl` holds its
//! context in an `Arc` so it can be cloned. A mutable borrow exists only
//! while the `Gl` is unshared, so call this immediately after creating the
//! context, **before** cloning the `Gl` or handing [`Gl::glow`] to an
//! egui painter. If the context is already shared the function returns
//! `false` and says why in the log. `glow` also panics if a callback is
//! installed twice, so a context that already has debug output enabled is
//! refused rather than risked.

use super::Gl;
use glow::HasContext;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The log target for driver messages.
pub const TARGET: &str = "gl_driver";

/// Times one distinct message passes through per [`WINDOW`].
pub const MAX_PER_WINDOW: u32 = 5;

/// The rate-limit window.
pub const WINDOW: Duration = Duration::from_secs(1);

/// Driver messages kept for the UI.
const KEPT: usize = 64;

/// Distinct messages the limiter tracks before it forgets them all.
const LIMITER_CAPACITY: usize = 256;

/// How much the driver should report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugLevel {
    /// High severity only: errors and undefined behavior.
    Errors,
    /// High and medium: adds performance warnings and deprecations.
    Warnings,
    /// Adds low severity.
    Info,
    /// Everything, including notifications.
    All,
}

impl DebugLevel {
    /// The severities this level asks the driver *not* to generate.
    fn excluded(self) -> &'static [u32] {
        match self {
            Self::Errors => &[
                glow::DEBUG_SEVERITY_MEDIUM,
                glow::DEBUG_SEVERITY_LOW,
                glow::DEBUG_SEVERITY_NOTIFICATION,
            ],
            Self::Warnings => &[glow::DEBUG_SEVERITY_LOW, glow::DEBUG_SEVERITY_NOTIFICATION],
            Self::Info => &[glow::DEBUG_SEVERITY_NOTIFICATION],
            Self::All => &[],
        }
    }
}

/// How serious the driver says a message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Errors, undefined behavior, dangerous.
    High,
    /// Performance warnings, deprecated use.
    Medium,
    /// Minor.
    Low,
    /// Informational.
    Notification,
}

impl Severity {
    fn from_raw(raw: u32) -> Self {
        match raw {
            glow::DEBUG_SEVERITY_HIGH => Self::High,
            glow::DEBUG_SEVERITY_MEDIUM => Self::Medium,
            glow::DEBUG_SEVERITY_LOW => Self::Low,
            _ => Self::Notification,
        }
    }

    /// The log level for this severity.
    pub fn log_level(self) -> log::Level {
        match self {
            Self::High => log::Level::Error,
            Self::Medium => log::Level::Warn,
            Self::Low => log::Level::Info,
            Self::Notification => log::Level::Debug,
        }
    }
}

/// One message from the driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverMessage {
    /// Where it came from: `API`, `shader compiler`, `window system`…
    pub source: &'static str,
    /// What it is about: `error`, `performance`, `deprecated`…
    pub kind: &'static str,
    /// The driver's message id.
    pub id: u32,
    /// How serious it is.
    pub severity: Severity,
    /// The driver's text.
    pub text: String,
}

impl DriverMessage {
    fn from_raw(source: u32, kind: u32, id: u32, severity: u32, text: &str) -> Self {
        Self {
            source: source_name(source),
            kind: kind_name(kind),
            id,
            severity: Severity::from_raw(severity),
            text: text.trim_end().to_string(),
        }
    }

    /// The level a message is logged at: its severity's, but never below
    /// `warn` for something the driver typed as an error.
    pub fn level(&self) -> log::Level {
        let level = self.severity.log_level();
        if self.kind == "error" {
            level.min(log::Level::Warn)
        } else {
            level
        }
    }
}

fn source_name(raw: u32) -> &'static str {
    match raw {
        glow::DEBUG_SOURCE_API => "API",
        glow::DEBUG_SOURCE_WINDOW_SYSTEM => "window system",
        glow::DEBUG_SOURCE_SHADER_COMPILER => "shader compiler",
        glow::DEBUG_SOURCE_THIRD_PARTY => "third party",
        glow::DEBUG_SOURCE_APPLICATION => "application",
        glow::DEBUG_SOURCE_OTHER => "other",
        _ => "unknown",
    }
}

fn kind_name(raw: u32) -> &'static str {
    match raw {
        glow::DEBUG_TYPE_ERROR => "error",
        glow::DEBUG_TYPE_DEPRECATED_BEHAVIOR => "deprecated",
        glow::DEBUG_TYPE_UNDEFINED_BEHAVIOR => "undefined behavior",
        glow::DEBUG_TYPE_PORTABILITY => "portability",
        glow::DEBUG_TYPE_PERFORMANCE => "performance",
        glow::DEBUG_TYPE_MARKER => "marker",
        glow::DEBUG_TYPE_PUSH_GROUP => "push group",
        glow::DEBUG_TYPE_POP_GROUP => "pop group",
        glow::DEBUG_TYPE_OTHER => "other",
        _ => "unknown",
    }
}

/// What the limiter decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Log it; `suppressed` earlier repeats were dropped.
    Pass {
        /// Messages with this key dropped since the last one logged.
        suppressed: u32,
    },
    /// Drop it.
    Drop,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    passed: u32,
    suppressed: u32,
}

/// Allows [`MAX_PER_WINDOW`] messages per key per [`WINDOW`]. Time is
/// passed in so the behavior is testable.
#[derive(Debug, Default)]
pub struct RateLimiter {
    windows: HashMap<(u32, u32, u32), Window>,
}

impl RateLimiter {
    /// Decides whether a message with `key` at time `now` is logged.
    pub fn admit(&mut self, key: (u32, u32, u32), now: Instant) -> Admit {
        if self.windows.len() >= LIMITER_CAPACITY && !self.windows.contains_key(&key) {
            self.windows.clear();
        }
        let window = self.windows.entry(key).or_insert(Window {
            started: now,
            passed: 0,
            suppressed: 0,
        });
        if now.duration_since(window.started) >= WINDOW {
            let suppressed = window.suppressed;
            *window = Window {
                started: now,
                passed: 1,
                suppressed: 0,
            };
            return Admit::Pass { suppressed };
        }
        if window.passed < MAX_PER_WINDOW {
            window.passed += 1;
            Admit::Pass { suppressed: 0 }
        } else {
            window.suppressed += 1;
            Admit::Drop
        }
    }
}

static LIMITER: Mutex<Option<RateLimiter>> = Mutex::new(None);
static RECENT: Mutex<VecDeque<DriverMessage>> = Mutex::new(VecDeque::new());

/// Handles one driver message: rate limit, log, remember. Called from the
/// driver's callback, so it must not panic and must not call GL.
fn on_message(source: u32, kind: u32, id: u32, severity: u32, text: &str) {
    let admitted = {
        let mut guard = LIMITER.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .get_or_insert_with(RateLimiter::default)
            .admit((source, kind, id), Instant::now())
    };
    let Admit::Pass { suppressed } = admitted else {
        return;
    };
    let message = DriverMessage::from_raw(source, kind, id, severity, text);
    log::log!(
        target: TARGET,
        message.level(),
        "{}/{} id={}: {}{}",
        message.source,
        message.kind,
        message.id,
        message.text,
        if suppressed > 0 {
            format!(" ({suppressed} identical messages suppressed)")
        } else {
            String::new()
        }
    );
    let mut recent = RECENT.lock().unwrap_or_else(PoisonError::into_inner);
    if recent.len() == KEPT {
        recent.pop_front();
    }
    recent.push_back(message);
}

/// The newest `count` driver messages that passed the rate limiter, oldest
/// first.
pub fn recent_messages(count: usize) -> Vec<DriverMessage> {
    let recent = RECENT.lock().unwrap_or_else(PoisonError::into_inner);
    let skip = recent.len().saturating_sub(count);
    recent.iter().skip(skip).cloned().collect()
}

/// Turns on driver debug output and routes it to the `log` crate.
///
/// Returns `true` if it is now on. Returns `false`, having logged why at
/// `info` or `warn`, when the driver lacks `KHR_debug` (GL below 4.3 with
/// no extension, which includes some old Intel drivers), when the context
/// is shared (see the module docs), or when debug output was already on.
/// Call right after the context is created.
pub fn enable_debug_output(gl: &mut Gl, level: DebugLevel) -> bool {
    let Some(context) = Arc::get_mut(&mut gl.raw) else {
        log::warn!(
            target: TARGET,
            "GL debug output not enabled: the Gl is already shared; call enable_debug_output right after creating the context"
        );
        return false;
    };
    if !context.supports_debug() {
        log::info!(target: TARGET, "GL debug output unavailable: the driver lacks KHR_debug and GL 4.3");
        return false;
    }
    if is_enabled(context, glow::DEBUG_OUTPUT) {
        log::warn!(target: TARGET, "GL debug output was already enabled; leaving it alone");
        return false;
    }
    set_callback(context, on_message);
    enable(context, glow::DEBUG_OUTPUT);
    if cfg!(debug_assertions) {
        enable(context, glow::DEBUG_OUTPUT_SYNCHRONOUS);
    }
    for severity in level.excluded() {
        message_control(context, *severity);
    }
    log::info!(
        target: TARGET,
        "GL debug output enabled ({level:?}, {})",
        if cfg!(debug_assertions) { "synchronous" } else { "asynchronous" }
    );
    true
}

// ------------------------------------------------------------ unsafe leaves
//
// Same rules as `mod.rs`: one driver call per function, current-context
// invariant stated in the SAFETY comment.

fn is_enabled(context: &glow::Context, capability: u32) -> bool {
    // SAFETY: current-context invariant (module docs of `gl`): `context`
    // belongs to a `Gl`, which is `!Send` and was built on the thread where
    // its context is current; the caller holds that `Gl` by `&mut`, so this
    // is that thread. `capability` is a GL enum; an unknown one is a GL
    // error, not undefined behavior.
    unsafe { context.is_enabled(capability) }
}

fn enable(context: &glow::Context, capability: u32) {
    // SAFETY: current-context invariant, as for `is_enabled`.
    unsafe { context.enable(capability) }
}

fn message_control(context: &glow::Context, severity: u32) {
    // SAFETY: current-context invariant, as for `is_enabled`. The id list
    // is empty, so glow passes a null pointer with a zero count and the
    // driver reads no memory.
    unsafe { context.debug_message_control(glow::DONT_CARE, glow::DONT_CARE, severity, &[], false) }
}

fn set_callback(context: &mut glow::Context, handler: fn(u32, u32, u32, u32, &str)) {
    // SAFETY: current-context invariant, as for `is_enabled`. glow stores
    // the boxed closure and passes the driver a pointer to it that stays
    // valid until the context is dropped. The closure is `Send + Sync +
    // 'static` (a function pointer captured by value), as glow requires
    // because drivers may call back from any thread, and it never panics:
    // `handler` is wrapped in `catch_unwind`. glow panics if a callback is
    // already set; the caller has checked that debug output is off, and
    // this is the only place that installs one.
    unsafe {
        context.debug_message_callback(move |source, kind, id, severity, text| {
            // A panic must not unwind into the driver; the handler already
            // logs what it can.
            std::panic::catch_unwind(|| handler(source, kind, id, severity, text)).ok();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::context::{Api, Headless};
    use anyhow::{Result, anyhow};

    fn key() -> (u32, u32, u32) {
        (glow::DEBUG_SOURCE_API, glow::DEBUG_TYPE_ERROR, 1)
    }

    #[test]
    fn severity_maps_to_log_level() -> Result<()> {
        assert_eq!(Severity::High.log_level(), log::Level::Error, "high");
        assert_eq!(Severity::Medium.log_level(), log::Level::Warn, "medium");
        assert_eq!(Severity::Low.log_level(), log::Level::Info, "low");
        assert_eq!(
            Severity::Notification.log_level(),
            log::Level::Debug,
            "notification"
        );
        Ok(())
    }

    #[test]
    fn an_error_typed_message_is_at_least_a_warning() -> Result<()> {
        let message = DriverMessage::from_raw(
            glow::DEBUG_SOURCE_API,
            glow::DEBUG_TYPE_ERROR,
            7,
            glow::DEBUG_SEVERITY_NOTIFICATION,
            "x\n",
        );
        assert_eq!(message.level(), log::Level::Warn, "error type raises debug");
        assert_eq!(message.text, "x", "trailing newline trimmed");
        let perf = DriverMessage::from_raw(
            glow::DEBUG_SOURCE_API,
            glow::DEBUG_TYPE_PERFORMANCE,
            7,
            glow::DEBUG_SEVERITY_LOW,
            "slow",
        );
        assert_eq!(perf.level(), log::Level::Info, "performance stays low");
        assert_eq!(perf.kind, "performance", "kind name");
        Ok(())
    }

    #[test]
    fn the_limiter_passes_a_burst_then_drops() -> Result<()> {
        let mut limiter = RateLimiter::default();
        let start = Instant::now();
        for _ in 0..MAX_PER_WINDOW {
            assert_eq!(
                limiter.admit(key(), start),
                Admit::Pass { suppressed: 0 },
                "within the allowance"
            );
        }
        assert_eq!(
            limiter.admit(key(), start),
            Admit::Drop,
            "over the allowance"
        );
        assert_eq!(limiter.admit(key(), start), Admit::Drop, "still over");
        Ok(())
    }

    #[test]
    fn the_limiter_reports_suppressed_counts_after_the_window() -> Result<()> {
        let mut limiter = RateLimiter::default();
        let start = Instant::now();
        for _ in 0..(MAX_PER_WINDOW + 3) {
            limiter.admit(key(), start);
        }
        assert_eq!(
            limiter.admit(key(), start + WINDOW),
            Admit::Pass { suppressed: 3 },
            "next window reports the three dropped"
        );
        assert_eq!(
            limiter.admit(key(), start + WINDOW),
            Admit::Pass { suppressed: 0 },
            "and starts counting afresh"
        );
        Ok(())
    }

    #[test]
    fn different_ids_are_limited_independently() -> Result<()> {
        let mut limiter = RateLimiter::default();
        let start = Instant::now();
        for _ in 0..(MAX_PER_WINDOW * 2) {
            limiter.admit(key(), start);
        }
        let other = (glow::DEBUG_SOURCE_API, glow::DEBUG_TYPE_ERROR, 2);
        assert_eq!(
            limiter.admit(other, start),
            Admit::Pass { suppressed: 0 },
            "a different id is unaffected"
        );
        Ok(())
    }

    /// Captures `gl_driver` records so the test can assert they were logged.
    struct Capture;

    static CAPTURED: Mutex<Vec<(log::Level, String)>> = Mutex::new(Vec::new());

    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.target() == TARGET
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                CAPTURED
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((record.level(), record.args().to_string()));
            }
        }

        fn flush(&self) {}
    }

    static CAPTURE: Capture = Capture;

    /// A headless context, or `None` (after saying so) where none exists,
    /// following `tests/render.rs`: `STROBE_REQUIRE_HEADLESS_GL` turns the
    /// skip into a failure.
    fn headless() -> Result<Option<Headless>> {
        match Headless::new(Api::DesktopCore33) {
            Ok(headless) => Ok(Some(headless)),
            Err(error) if std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_none() => {
                crate::skip::note(format_args!("no headless context ({error})"));
                Ok(None)
            }
            Err(error) => Err(anyhow!("headless context required: {error}")),
        }
    }

    #[test]
    fn a_driver_error_reaches_the_log_and_the_recent_ring() -> Result<()> {
        let Some(mut headless) = headless()? else {
            return Ok(());
        };
        log::set_logger(&CAPTURE).map_err(|error| anyhow!("test logger: {error}"))?;
        log::set_max_level(log::LevelFilter::Trace);

        let enabled = enable_debug_output(headless.gl_mut(), DebugLevel::All);
        let gl = headless.gl();
        if !enabled {
            // Mesa llvmpipe has KHR_debug; anything else is a real failure.
            return Err(anyhow!(
                "debug output should be available on Mesa; renderer: {}",
                gl.info().renderer
            ));
        }
        // A negative viewport is GL_INVALID_VALUE.
        gl.viewport(-1, -1);
        gl.finish();
        // Drain the error so later tests on this thread are not confused.
        gl.error();

        let messages = recent_messages(KEPT);
        let found = messages
            .iter()
            .find(|message| message.kind == "error")
            .ok_or_else(|| anyhow!("no error message from the driver; got {messages:?}"))?;
        eprintln!("driver said: {found:?}");
        assert!(
            !found.text.is_empty(),
            "driver message text was empty: {found:?}"
        );
        let logged = CAPTURED.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(
            logged
                .iter()
                .any(|(level, text)| *level <= log::Level::Warn && text.contains(&found.text)),
            "the message was not logged at warn or above: {logged:?}"
        );

        // A second call on the same context must refuse, not panic in glow.
        drop(logged);
        assert!(
            !enable_debug_output(headless.gl_mut(), DebugLevel::All),
            "enabling twice must be refused"
        );
        Ok(())
    }

    #[test]
    fn a_shared_gl_is_refused_with_a_warning() -> Result<()> {
        let Some(mut headless) = headless()? else {
            return Ok(());
        };
        let _extra = headless.gl().clone();
        assert!(
            !enable_debug_output(headless.gl_mut(), DebugLevel::Errors),
            "a cloned Gl cannot be given a callback"
        );
        Ok(())
    }
}
