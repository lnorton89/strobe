// ============================================================================
// Module:       strobe_app::awake
// Description:  Keeps the computer awake and the display on while the output
//               window is fullscreen, so a show never sleeps mid-set.
//
// Dependencies: strobe-diag (power); log
// ============================================================================

//! Keeping the show awake (D556).
//!
//! Windows sleeps, turns the display off or starts the screen saver after its
//! idle timeouts, and a show can run for hours with nobody touching the
//! keyboard or mouse. While the output window is fullscreen, [`KeepAwake`]
//! holds a request that the system and the display stay on: Windows'
//! `SetThreadExecutionState` with `ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED`
//! ([`strobe_diag::power`]). It lets the request go when the output
//! leaves fullscreen or Strobe exits, so the machine's own power settings
//! apply again the moment the show is not up.
//!
//! - **Fullscreen is the signal.** It is how the output goes on the
//!   projector, from the command, F11, a layout or the command line, so
//!   checking it once a control-window frame catches every path, and an
//!   unchanged state costs nothing.
//! - **The request belongs to the thread that made it.** It is made and
//!   released on the main thread, which runs the event loop for the whole run.
//! - **A refusal is said once.** If Windows refuses, it is logged and not
//!   asked again until the output next goes fullscreen.
//! - **Elsewhere, nothing yet.** macOS and Linux have their own mechanisms
//!   (IOKit, systemd-inhibit over D-Bus), which wait for their releases
//!   (R-08, G-07); there the request is refused once, in the log.

/// What a held request is.
type Guard = strobe_diag::power::AwakeRequest;

/// Holds the keep-awake request while the output is fullscreen (D556).
pub struct KeepAwake<G = Guard> {
    held: Option<G>,
    /// The last attempt was refused, so it is not tried again until the
    /// output leaves fullscreen and returns.
    refused: bool,
}

impl<G> Default for KeepAwake<G> {
    fn default() -> Self {
        Self {
            held: None,
            refused: false,
        }
    }
}

impl<G> std::fmt::Debug for KeepAwake<G> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeepAwake")
            .field("holding", &self.held.is_some())
            .field("refused", &self.refused)
            .finish()
    }
}

impl KeepAwake {
    /// Holds the request while `fullscreen`, and lets it go otherwise.
    /// Called once a control-window frame; only a change does anything.
    pub fn follow(&mut self, fullscreen: bool) {
        self.sync(fullscreen, hold);
    }
}

impl<G> KeepAwake<G> {
    /// Whether the request is held now.
    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    /// [`KeepAwake::follow`] with the request made by `request`, so a test
    /// can count the requests without asking Windows.
    pub fn sync(&mut self, fullscreen: bool, request: impl FnOnce() -> Result<G, String>) {
        match (fullscreen, self.held.is_some()) {
            (true, false) if !self.refused => match request() {
                Ok(guard) => {
                    log::info!(
                        "keeping the computer and display awake while the output is fullscreen"
                    );
                    self.held = Some(guard);
                }
                Err(error) => {
                    log::warn!("could not keep the computer awake while fullscreen: {error}");
                    self.refused = true;
                }
            },
            (false, true) => {
                self.held = None;
                log::info!("the output left fullscreen; the computer may sleep again");
            }
            (false, false) => self.refused = false,
            (true, false) | (true, true) => {}
        }
    }
}

/// Asks for the system and the display to stay on.
fn hold() -> Result<Guard, String> {
    strobe_diag::power::AwakeRequest::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::cell::Cell;

    #[test]
    fn the_request_is_held_only_while_fullscreen_and_asked_for_once() -> Result<()> {
        let asked = Cell::new(0);
        let request = || {
            asked.set(asked.get() + 1);
            Ok(())
        };
        let mut awake = KeepAwake::<()>::default();
        awake.sync(false, request);
        assert!(!awake.holding(), "windowed: nothing held");
        for _ in 0..3 {
            awake.sync(true, request);
        }
        assert!(awake.holding(), "fullscreen: held");
        assert_eq!(asked.get(), 1, "asked once, not once a frame");
        awake.sync(false, request);
        assert!(!awake.holding(), "out of fullscreen: let go");
        awake.sync(true, request);
        assert_eq!(asked.get(), 2, "asked again the next time");
        Ok(())
    }

    #[test]
    fn a_refusal_is_not_retried_until_fullscreen_comes_back() -> Result<()> {
        let asked = Cell::new(0);
        let refuse = || -> Result<(), String> {
            asked.set(asked.get() + 1);
            Err("refused".to_string())
        };
        let mut awake = KeepAwake::<()>::default();
        for _ in 0..5 {
            awake.sync(true, refuse);
        }
        assert!(!awake.holding(), "nothing held after a refusal");
        assert_eq!(asked.get(), 1, "a refusal is not asked again every frame");
        awake.sync(false, refuse);
        awake.sync(true, refuse);
        assert_eq!(asked.get(), 2, "tried again once fullscreen comes back");
        Ok(())
    }
}
