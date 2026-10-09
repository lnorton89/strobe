// ============================================================================
// Module:       strobe_app::fault
// Description:  STROBE_FAULT: injected GL faults, so context-loss recovery can
//               be exercised end to end without a broken driver.
//
// Dependencies: strobe-gfx (Backend)
// ============================================================================

//! Fault injection.
//!
//! Recovery from a lost GL context (see [`crate::recovery`]) only runs when
//! a driver misbehaves, and the machine that misbehaves is a laptop in
//! another room. `STROBE_FAULT` makes the app misbehave on purpose, on any
//! machine, so the whole path (detect, decide, tear down, rebuild, restore,
//! reload the scene) can be run under Xvfb and in CI.
//!
//! The variable is read **once**, at startup. It is a comma-separated list:
//!
//! | Item | Effect |
//! |---|---|
//! | `lose-context@60` | After 60 output frames have been presented, the context dies: from then on every `make_current`, on either window, fails with an injected error and the `Gl` is marked lost, as if the driver had reset. A rebuild gives the app a new, working context. |
//! | `lose-context@60,200` | Two losses, after 60 and after 200 output frames. A bare number after a `lose-context` item adds another frame. |
//! | `fail-rebuild:native` | Rebuilding on the native backend fails (every time). |
//! | `fail-rebuild:angle` | Rebuilding on ANGLE fails (every time). |
//!
//! Frames count presented output frames over the whole run, across
//! rebuilds, and each `lose-context` frame fires once. Examples:
//!
//! ```text
//! STROBE_FAULT=lose-context@60                    # one loss: rebuild in place
//! STROBE_FAULT=lose-context@60,120                # second loss within 60 s: fail over to ANGLE
//! STROBE_FAULT=lose-context@60,fail-rebuild:native  # the native rebuild fails: ANGLE
//! STROBE_FAULT=lose-context@60,fail-rebuild:native,fail-rebuild:angle   # give up
//! ```
//!
//! A malformed value is reported in the log and ignored, so a typo cannot
//! silently turn a test into a no-op that passes.

use strobe_gfx::Backend;

/// The environment variable.
pub const ENV: &str = "STROBE_FAULT";

/// The faults in force.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Faults {
    /// Output-frame counts at which the context dies, ascending, not yet
    /// fired.
    lose_at: Vec<u64>,
    /// Backends whose rebuild fails.
    fail_rebuild: Vec<Backend>,
}

impl Faults {
    /// Reads `STROBE_FAULT`. Unset gives no faults; a malformed value is
    /// logged and gives none.
    pub fn from_env() -> Self {
        let Ok(text) = std::env::var(ENV) else {
            return Self::default();
        };
        match Self::parse(&text) {
            Ok(faults) => {
                if !faults.is_empty() {
                    log::warn!(
                        "{ENV}={text}: fault injection is on ({})",
                        faults.describe()
                    );
                }
                faults
            }
            Err(error) => {
                log::error!("{ENV}={text}: ignored, {error}");
                Self::default()
            }
        }
    }

    /// Parses a `STROBE_FAULT` value.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut faults = Self::default();
        let mut after_lose = false;
        for item in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(frame) = item.strip_prefix("lose-context@") {
                faults.lose_at.push(frame_number(frame, item)?);
                after_lose = true;
            } else if let Some(backend) = item.strip_prefix("fail-rebuild:") {
                faults.fail_rebuild.push(match backend {
                    "native" => Backend::Native,
                    "angle" => Backend::Angle,
                    _ => {
                        return Err(format!("`{item}`: the backend must be `native` or `angle`"));
                    }
                });
                after_lose = false;
            } else if after_lose && item.bytes().all(|b| b.is_ascii_digit()) {
                faults.lose_at.push(frame_number(item, item)?);
            } else {
                return Err(format!(
                    "unrecognised item `{item}`; expected lose-context@<frame>[,<frame>...] or fail-rebuild:<native|angle>"
                ));
            }
        }
        faults.lose_at.sort_unstable();
        faults.lose_at.dedup();
        Ok(faults)
    }

    /// Whether no fault is configured.
    pub fn is_empty(&self) -> bool {
        self.lose_at.is_empty() && self.fail_rebuild.is_empty()
    }

    /// Whether the context should die now, with `frames` output frames
    /// presented so far. Fires each configured frame once.
    pub fn lose_context_due(&mut self, frames: u64) -> bool {
        let due = self.lose_at.iter().take_while(|at| **at <= frames).count();
        self.lose_at.drain(..due);
        due > 0
    }

    /// Whether rebuilding on `backend` should fail.
    pub fn rebuild_fails(&self, backend: Backend) -> bool {
        self.fail_rebuild.contains(&backend)
    }

    /// A one-line summary for the log.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.lose_at.is_empty() {
            let frames: Vec<String> = self.lose_at.iter().map(u64::to_string).collect();
            parts.push(format!(
                "lose the context after output frame {}",
                frames.join(", ")
            ));
        }
        for backend in &self.fail_rebuild {
            parts.push(format!("rebuilding on {backend} fails"));
        }
        parts.join("; ")
    }
}

fn frame_number(text: &str, item: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("`{item}`: `{text}` is not a frame number"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_single_loss_parses() -> Result<()> {
        let faults = Faults::parse("lose-context@60").map_err(anyhow::Error::msg)?;
        assert_eq!(faults.lose_at, [60], "one frame");
        assert!(faults.fail_rebuild.is_empty(), "nothing else");
        Ok(())
    }

    #[test]
    fn bare_numbers_extend_the_last_loss_and_the_list_is_sorted() -> Result<()> {
        let faults = Faults::parse("lose-context@120, 60 ,200").map_err(anyhow::Error::msg)?;
        assert_eq!(faults.lose_at, [60, 120, 200], "sorted");
        Ok(())
    }

    #[test]
    fn a_rebuild_failure_names_its_backend() -> Result<()> {
        let faults =
            Faults::parse("lose-context@5,fail-rebuild:native").map_err(anyhow::Error::msg)?;
        assert!(faults.rebuild_fails(Backend::Native), "native fails");
        assert!(!faults.rebuild_fails(Backend::Angle), "angle does not");
        Ok(())
    }

    #[test]
    fn a_bare_number_after_a_rebuild_item_is_an_error() {
        let result = Faults::parse("fail-rebuild:native,60");
        assert!(result.is_err(), "60 belongs to nothing: {result:?}");
    }

    #[test]
    fn nonsense_is_reported_with_the_grammar() {
        for bad in [
            "lose-context",
            "lose-context@soon",
            "fail-rebuild:vulkan",
            "explode@5",
        ] {
            let result = Faults::parse(bad);
            assert!(result.is_err(), "`{bad}` should not parse: {result:?}");
        }
        let message = Faults::parse("explode@5").err().unwrap_or_default();
        assert!(
            message.contains("lose-context@"),
            "grammar shown: {message}"
        );
    }

    #[test]
    fn empty_means_no_faults() -> Result<()> {
        assert!(
            Faults::parse("").map_err(anyhow::Error::msg)?.is_empty(),
            "empty"
        );
        assert!(
            Faults::parse(" , ").map_err(anyhow::Error::msg)?.is_empty(),
            "blanks"
        );
        Ok(())
    }

    #[test]
    fn each_loss_fires_once_when_its_frame_is_reached() -> Result<()> {
        let mut faults = Faults::parse("lose-context@60,120").map_err(anyhow::Error::msg)?;
        assert!(!faults.lose_context_due(59), "too early");
        assert!(faults.lose_context_due(60), "first");
        assert!(!faults.lose_context_due(60), "only once");
        assert!(!faults.lose_context_due(119), "between");
        assert!(faults.lose_context_due(120), "second");
        assert!(!faults.lose_context_due(10_000), "none left");
        Ok(())
    }

    #[test]
    fn a_late_check_still_fires_a_loss_that_was_passed() -> Result<()> {
        let mut faults = Faults::parse("lose-context@60").map_err(anyhow::Error::msg)?;
        assert!(faults.lose_context_due(75), "passed frames count");
        Ok(())
    }
}
