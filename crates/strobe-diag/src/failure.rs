// ============================================================================
// Module:       strobe_diag::failure
// Description:  Structured user-visible failures: subsystem, scene/asset,
//               path, phase, root error and recovery action, logged in one
//               greppable shape and kept in a bounded ring for the UI.
//
// Dependencies: log; strobe_diag::context (via reports)
// ============================================================================

//! Actionable failures.
//!
//! Every failure a person can see should say what failed, where, in what
//! phase, why, and what the program did about it. [`Failure`] carries
//! exactly those fields and renders them as a block that is easy to read in
//! the console and easy to grep in a log:
//!
//! ```text
//! SHADER
//! scene=09-sdf-cathedral
//! phase=link
//! file=main.glsl
//! line=142
//! message=unexpected token
//! action=scene skipped; previous scene keeps playing
//! ```
//!
//! [`report`] logs the block at `error` level (so it lands in the session
//! file, the ring and every crash report) and also keeps the structured
//! record in a bounded list that the UI reads with [`recent`]. This is
//! deliberately off the render path: a failure is something that already
//! went wrong, and reporting one costs a formatted string and two short
//! locks. The renderer's hot path never calls it per frame.

use std::collections::VecDeque;
use std::fmt::{self, Display, Write};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

/// Failures kept for the UI and crash reports.
pub const KEPT: usize = 100;

/// Which part of the program failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsystem {
    /// Shader compile or link.
    Shader,
    /// Audio capture or analysis.
    Audio,
    /// Shadertoy or scene import.
    Import,
    /// GL context creation or loss.
    Context,
    /// The scene scripting host.
    Script,
    /// Playlist loading or scheduling.
    Playlist,
    /// Reading or writing files.
    Io,
    /// The command line, before anything started.
    Arguments,
}

impl Subsystem {
    /// The upper-case tag that opens the block.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Shader => "SHADER",
            Self::Audio => "AUDIO",
            Self::Import => "IMPORT",
            Self::Context => "CONTEXT",
            Self::Script => "SCRIPT",
            Self::Playlist => "PLAYLIST",
            Self::Io => "IO",
            Self::Arguments => "ARGUMENTS",
        }
    }
}

/// One failure, built with chained setters and reported with [`report`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    subsystem: Subsystem,
    phase: String,
    scene: Option<String>,
    asset: Option<String>,
    path: Option<String>,
    details: Vec<(String, String)>,
    message: String,
    action: Option<String>,
}

impl Failure {
    /// A failure in `subsystem` during `phase` (`link`, `capture`, `asset`…).
    pub fn new(subsystem: Subsystem, phase: &str) -> Self {
        Self {
            subsystem,
            phase: phase.to_string(),
            scene: None,
            asset: None,
            path: None,
            details: Vec::new(),
            message: String::new(),
            action: None,
        }
    }

    /// The scene involved.
    #[must_use]
    pub fn scene(mut self, scene: impl Display) -> Self {
        self.scene = Some(scene.to_string());
        self
    }

    /// The asset involved: a device, a shader id, a texture name.
    #[must_use]
    pub fn asset(mut self, asset: impl Display) -> Self {
        self.asset = Some(asset.to_string());
        self
    }

    /// The file path or URL involved.
    #[must_use]
    pub fn path(mut self, path: impl AsRef<Path>) -> Self {
        self.path = Some(path.as_ref().display().to_string());
        self
    }

    /// An extra `key=value` line, such as `line=142`.
    #[must_use]
    pub fn detail(mut self, key: &str, value: impl Display) -> Self {
        self.details.push((key.to_string(), value.to_string()));
        self
    }

    /// The root error, in the underlying library's words.
    #[must_use]
    pub fn message(mut self, message: impl Display) -> Self {
        self.message = message.to_string();
        self
    }

    /// What the program did about it, such as `retrying in 500ms`.
    #[must_use]
    pub fn action(mut self, action: impl Display) -> Self {
        self.action = Some(action.to_string());
        self
    }

    /// Which subsystem failed.
    pub fn subsystem(&self) -> Subsystem {
        self.subsystem
    }

    /// The phase it failed in.
    pub fn phase(&self) -> &str {
        &self.phase
    }

    /// The root error text.
    pub fn root_message(&self) -> &str {
        &self.message
    }

    /// The recovery action, if one was recorded.
    pub fn recovery(&self) -> Option<&str> {
        self.action.as_deref()
    }
}

impl Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.subsystem.tag())?;
        let line = |key: &str, value: &str, f: &mut fmt::Formatter<'_>| -> fmt::Result {
            f.write_char('\n')?;
            write!(f, "{key}={value}")
        };
        if let Some(scene) = &self.scene {
            line("scene", scene, f)?;
        }
        if let Some(asset) = &self.asset {
            line("asset", asset, f)?;
        }
        if let Some(path) = &self.path {
            line("path", path, f)?;
        }
        line("phase", &self.phase, f)?;
        for (key, value) in &self.details {
            line(key, value, f)?;
        }
        line("message", &self.message, f)?;
        if let Some(action) = &self.action {
            line("action", action, f)?;
        }
        Ok(())
    }
}

/// One remembered failure and how many times it was reported.
struct Kept {
    failure: Failure,
    times: u32,
}

impl Kept {
    /// The failure as the UI and reports see it: a repeated one carries a
    /// `repeated=N` line.
    fn shown(&self) -> Failure {
        if self.times > 1 {
            self.failure.clone().detail("repeated", self.times)
        } else {
            self.failure.clone()
        }
    }
}

static RECENT: Mutex<VecDeque<Kept>> = Mutex::new(VecDeque::new());

/// Logs `failure` at error level and remembers it for the UI.
///
/// A failure identical to one already kept (a device that drops out every
/// few seconds, a resize the driver keeps refusing) is not kept twice: the
/// earlier entry moves to the newest place and counts the repeat (D381).
/// So [`KEPT`] distinct failures survive one that keeps coming back.
pub fn report(failure: Failure) {
    log::error!(target: "strobe_diag::failure", "{failure}");
    let mut recent = RECENT.lock().unwrap_or_else(PoisonError::into_inner);
    let earlier = recent
        .iter()
        .position(|kept| kept.failure == failure)
        .and_then(|at| recent.remove(at));
    let times = match earlier {
        Some(kept) => kept.times.saturating_add(1),
        None => {
            if recent.len() >= KEPT {
                recent.pop_front();
            }
            1
        }
    };
    recent.push_back(Kept { failure, times });
}

/// The newest `count` distinct failures, oldest first.
pub fn recent(count: usize) -> Vec<Failure> {
    let recent = RECENT.lock().unwrap_or_else(PoisonError::into_inner);
    let skip = recent.len().saturating_sub(count);
    recent.iter().skip(skip).map(Kept::shown).collect()
}

/// [`recent`] that never waits for the lock, for crash handlers.
pub fn try_recent(count: usize) -> Vec<Failure> {
    let Ok(recent) = RECENT.try_lock() else {
        return Vec::new();
    };
    let skip = recent.len().saturating_sub(count);
    recent.iter().skip(skip).map(Kept::shown).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// The tests that fill the shared list take turns, so one cannot evict
    /// what another is checking.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn a_failure_renders_in_the_documented_shape() -> Result<()> {
        let failure = Failure::new(Subsystem::Shader, "link")
            .scene("09-sdf-cathedral")
            .detail("file", "main.glsl")
            .detail("line", 142)
            .message("unexpected token")
            .action("scene skipped");
        assert_eq!(
            failure.to_string(),
            "SHADER\nscene=09-sdf-cathedral\nphase=link\nfile=main.glsl\nline=142\n\
             message=unexpected token\naction=scene skipped",
            "block layout"
        );
        Ok(())
    }

    #[test]
    fn absent_fields_are_omitted_and_message_is_always_present() -> Result<()> {
        let failure = Failure::new(Subsystem::Audio, "capture")
            .asset("Default Output")
            .message("device disconnected")
            .action("retrying in 500ms");
        assert_eq!(
            failure.to_string(),
            "AUDIO\nasset=Default Output\nphase=capture\nmessage=device disconnected\n\
             action=retrying in 500ms",
            "block layout"
        );
        let bare = Failure::new(Subsystem::Io, "read");
        assert!(bare.to_string().contains("message="), "message key kept");
        Ok(())
    }

    #[test]
    fn a_path_is_rendered_with_its_separators() -> Result<()> {
        let failure = Failure::new(Subsystem::Import, "asset")
            .path("https://example.test/a.png")
            .message("HTTP 403")
            .action("left in needs-work");
        let text = failure.to_string();
        assert!(
            text.contains("path=https://example.test/a.png"),
            "got {text}"
        );
        Ok(())
    }

    #[test]
    fn reported_failures_are_kept_in_order_and_bounded() -> Result<()> {
        let _turn = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        for index in 0..(KEPT + 5) {
            report(Failure::new(Subsystem::Script, "eval").message(format!("boom {index}")));
        }
        let all = recent(usize::MAX);
        assert_eq!(all.len(), KEPT, "ring is bounded");
        assert_eq!(
            all.last().map(Failure::root_message),
            Some(format!("boom {}", KEPT + 4).as_str()),
            "newest last"
        );
        assert_eq!(recent(2).len(), 2, "recent(n) returns n");
        Ok(())
    }

    #[test]
    fn a_failure_that_keeps_coming_back_does_not_push_the_others_out() -> Result<()> {
        let _turn = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        report(
            Failure::new(Subsystem::Shader, "link")
                .scene("flood-test-cathedral")
                .message("the one that matters"),
        );
        let flapping = Failure::new(Subsystem::Audio, "capture")
            .asset("flood-test-usb")
            .message("device disconnected")
            .action("using synthetic audio");
        for _ in 0..(2 * KEPT) {
            report(flapping.clone());
        }
        let all = recent(KEPT);
        assert!(
            all.iter()
                .any(|kept| kept.root_message() == "the one that matters"),
            "an earlier failure was pushed out by one repeated failure"
        );
        let repeats: Vec<String> = all
            .iter()
            .filter(|kept| kept.to_string().contains("flood-test-usb"))
            .map(ToString::to_string)
            .collect();
        assert_eq!(repeats.len(), 1, "the repeats are one entry: {repeats:?}");
        assert!(
            repeats
                .iter()
                .any(|text| text.contains(&format!("repeated={}", 2 * KEPT))),
            "the entry says how often: {repeats:?}"
        );
        assert!(
            all.last()
                .is_some_and(|newest| newest.to_string().contains("flood-test-usb")),
            "a repeat counts as the newest"
        );
        Ok(())
    }
}
