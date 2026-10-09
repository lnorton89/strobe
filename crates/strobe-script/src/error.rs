// ============================================================================
// Module:       strobe_script::error
// Description:  The ways a script host can fail to start. Failures while a
//               scene is running are faults, reported in the frame output.
//
// Dependencies: thiserror, rquickjs
// ============================================================================

//! Script host errors.
//!
//! Only *building* a host can fail. Once a script is running, an exception
//! is expected input (a script is edited by hand mid-show), so it is
//! reported through [`crate::FrameOutput::status`] and the console rather
//! than as an error the caller must route.

/// Why a [`crate::ScriptHost`] could not be built.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// The interpreter or its context could not be created, or its
    /// built-in functions could not be installed.
    #[error("cannot start the script engine: {0}")]
    Engine(#[from] rquickjs::Error),

    /// `script.js` threw, was interrupted, or did not parse. `message`
    /// carries the exception text and, when known, the line.
    #[error("script.js failed to load: {message}")]
    Load {
        /// What the engine reported.
        message: String,
    },

    /// `setup()` threw or was interrupted.
    #[error("setup() failed: {message}")]
    Setup {
        /// What the engine reported.
        message: String,
    },
}
