// ============================================================================
// Module:       strobe_app::corpus::capture
// Description:  A logger that keeps the error records a scene load raises
//               (structured failures, a script's console errors) so the
//               harness can read them back instead of losing them to a log.
//
// Dependencies: log; strobe-diag (the failure log target)
// ============================================================================

//! Capturing what a scene load reports.
//!
//! [`crate::session::Session::load`] does not return a script that failed
//! to start, or a cube map that did not decode: it reports them through
//! `strobe_diag::failure::report`, which logs them at error level, and
//! carries on without them, because that is what a show wants. A harness
//! wants the opposite, so it installs [`install`]'s logger and reads the
//! records back with [`take`].
//!
//! Only error-level records from the two sources that matter are kept:
//! structured failures (target `strobe_diag::failure`) and the script's own
//! console (target `script`).

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::sync::{Mutex, PoisonError};

/// The log target `strobe_diag::failure::report` writes under.
const FAILURE_TARGET: &str = "strobe_diag::failure";

/// The log target a scene's console output is written under.
const SCRIPT_TARGET: &str = "script";

/// What a captured record came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A structured failure with the `SCRIPT` subsystem tag: the script
    /// did not start, or faulted.
    ScriptFailure,
    /// Any other structured failure (an unreadable image, a cube map).
    OtherFailure,
    /// An error the script's own code printed or the host raised on its
    /// behalf (a missing callback, a refused late uniform).
    ScriptConsole,
}

/// One captured error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// Where it came from.
    pub source: Source,
    /// The root message: a failure's `message=` line, or the console line.
    pub message: String,
}

static SINK: Mutex<Vec<Captured>> = Mutex::new(Vec::new());

struct CaptureLogger;

static LOGGER: CaptureLogger = CaptureLogger;

impl Log for CaptureLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() == Level::Error
            && matches!(metadata.target(), FAILURE_TARGET | SCRIPT_TARGET)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let text = record.args().to_string();
        let captured = if record.target() == SCRIPT_TARGET {
            Captured {
                source: Source::ScriptConsole,
                message: text,
            }
        } else {
            from_failure(&text)
        };
        SINK.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(captured);
    }

    fn flush(&self) {}
}

/// Reads a rendered `strobe_diag::failure::Failure` (a tag line, then
/// `key=value` lines).
fn from_failure(text: &str) -> Captured {
    let tag = text.lines().next().unwrap_or_default();
    let message = text
        .lines()
        .find_map(|line| line.strip_prefix("message="))
        .unwrap_or(tag)
        .to_string();
    Captured {
        source: if tag == "SCRIPT" {
            Source::ScriptFailure
        } else {
            Source::OtherFailure
        },
        message,
    }
}

/// Installs the capturing logger. Returns whether this call did it; a
/// process that already has a logger keeps it, and then [`take`] sees
/// nothing.
pub fn install() -> bool {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LevelFilter::Error);
        true
    } else {
        false
    }
}

/// Everything captured since the last call, oldest first.
pub fn take() -> Vec<Captured> {
    std::mem::take(&mut *SINK.lock().unwrap_or_else(PoisonError::into_inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rendered_failure_gives_its_message_and_subsystem() {
        let script = from_failure(
            "SCRIPT\nscene=x\nasset=script.js\nphase=update\nmessage=boom at line 3\naction=a",
        );
        assert_eq!(script.source, Source::ScriptFailure, "tag SCRIPT");
        assert_eq!(script.message, "boom at line 3", "the message= line");
        let io = from_failure("IO\nphase=asset\nmessage=cannot decode");
        assert_eq!(io.source, Source::OtherFailure, "any other tag");
    }
}
