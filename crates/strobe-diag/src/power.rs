// ============================================================================
// Module:       strobe_diag::power
// Description:  A request that the system and display stay awake, held while
//               a show is up and released when it is dropped.
//
// Dependencies: crate::native (Windows' SetThreadExecutionState)
// ============================================================================

//! Keeping the machine awake (D556).
//!
//! [`AwakeRequest::new`] asks Windows to keep the system out of sleep and
//! the display on, with the screen saver held off, until the request is
//! dropped: `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED |
//! ES_DISPLAY_REQUIRED)`, and `ES_CONTINUOUS` alone to let go. The request
//! belongs to the calling thread, so it must be made and dropped on the same
//! thread, one that lives as long as the request is wanted. `AwakeRequest`
//! is `!Send` to make that true.
//!
//! Off Windows there is none yet: `new` says so, and the caller carries on.
//! macOS (IOKit) and Linux (systemd-inhibit over D-Bus) wait for their
//! releases (R-08, G-07).

use std::marker::PhantomData;

/// The system and display are kept awake until this is dropped (D556).
#[derive(Debug)]
pub struct AwakeRequest {
    /// `!Send` and `!Sync`: the request belongs to the thread that made it.
    _thread: PhantomData<*const ()>,
}

/// `ES_CONTINUOUS`: the request lasts until it is changed, not one call;
/// `ES_SYSTEM_REQUIRED`: no sleep for idleness; `ES_DISPLAY_REQUIRED`: the
/// display stays on and the screen saver does not start.
#[cfg(windows)]
mod flags {
    pub(super) use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS as CONTINUOUS, ES_DISPLAY_REQUIRED as DISPLAY, ES_SYSTEM_REQUIRED as SYSTEM,
    };
}

impl AwakeRequest {
    /// Asks for the system and display to stay awake. An `Err` says why
    /// not: refused by Windows, or not available on this platform.
    #[cfg(windows)]
    pub fn new() -> Result<Self, String> {
        let previous = crate::native::set_thread_execution_state(
            flags::CONTINUOUS | flags::SYSTEM | flags::DISPLAY,
        );
        if previous == 0 {
            return Err("Windows refused the request (SetThreadExecutionState)".to_string());
        }
        Ok(Self {
            _thread: PhantomData,
        })
    }

    /// No such request on this platform yet (D556).
    #[cfg(not(windows))]
    pub fn new() -> Result<Self, String> {
        Err("not available on this platform yet".to_string())
    }
}

#[cfg(windows)]
impl Drop for AwakeRequest {
    fn drop(&mut self) {
        // Back to the machine's own power settings. A failure here leaves
        // the request until the thread exits, which is when the process
        // does.
        let _ = crate::native::set_thread_execution_state(flags::CONTINUOUS);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_request_is_granted_and_released() -> Result<()> {
        // Windows grants it to any process; dropping it must not fail, and a
        // second request after the first is let go is granted too.
        let first = AwakeRequest::new().map_err(anyhow::Error::msg)?;
        drop(first);
        let second = AwakeRequest::new().map_err(anyhow::Error::msg)?;
        drop(second);
        Ok(())
    }
}
