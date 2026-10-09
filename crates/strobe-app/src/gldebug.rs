// ============================================================================
// Module:       strobe_app::gldebug
// Description:  Decides how much GL driver debug output to ask for, from the
//               build profile and STROBE_GL_DEBUG, and switches it on.
//
// Dependencies: strobe-gfx (gl::debug)
// ============================================================================

//! GL driver debug output policy.
//!
//! `strobe_gfx::gl::debug` does the work; this module decides how loudly.
//!
//! | Build | `STROBE_GL_DEBUG` unset | `=1` or `all` |
//! |---|---|---|
//! | debug | warnings and errors, synchronous | everything, synchronous |
//! | release | errors only | everything |
//!
//! `0`/`off` turns it off, and `errors`, `warn`, `info` pick a level. Old
//! Intel drivers without `KHR_debug` simply report that it is unavailable.
//! Release defaults to errors only because that is the evidence a crash
//! report needs, at a cost of nothing on a driver that has nothing to say.

use strobe_gfx::gl::Gl;
use strobe_gfx::gl::debug::{DebugLevel, enable_debug_output};

/// The environment variable that overrides the default.
pub const ENV: &str = "STROBE_GL_DEBUG";

/// The level for a build and a setting; `None` means no debug output.
pub fn level(debug_build: bool, setting: Option<&str>) -> Option<DebugLevel> {
    match setting.map(str::trim) {
        Some("0" | "off" | "false") => None,
        Some("1" | "all" | "true") => Some(DebugLevel::All),
        Some("info") => Some(DebugLevel::Info),
        Some("warn" | "warning" | "warnings") => Some(DebugLevel::Warnings),
        Some("errors" | "error") => Some(DebugLevel::Errors),
        Some(_) | None if debug_build => Some(DebugLevel::Warnings),
        Some(_) | None => Some(DebugLevel::Errors),
    }
}

/// Turns driver debug output on per the policy. Call right after the
/// context is created, before the `Gl` is shared (see
/// [`enable_debug_output`]). Returns whether it is now on.
pub fn enable(gl: &mut Gl) -> bool {
    let setting = std::env::var(ENV).ok();
    match level(cfg!(debug_assertions), setting.as_deref()) {
        Some(level) => enable_debug_output(gl, level),
        None => {
            log::info!(
                "GL debug output off ({ENV}={})",
                setting.unwrap_or_default()
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn debug_builds_default_to_warnings_and_release_to_errors() -> Result<()> {
        assert_eq!(level(true, None), Some(DebugLevel::Warnings), "debug");
        assert_eq!(level(false, None), Some(DebugLevel::Errors), "release");
        assert_eq!(level(false, Some("")), Some(DebugLevel::Errors), "empty");
        assert_eq!(
            level(false, Some("nonsense")),
            Some(DebugLevel::Errors),
            "unknown value falls back to the build default"
        );
        Ok(())
    }

    #[test]
    fn the_setting_overrides_the_build() -> Result<()> {
        assert_eq!(level(false, Some("1")), Some(DebugLevel::All), "1");
        assert_eq!(level(true, Some("errors")), Some(DebugLevel::Errors), "e");
        assert_eq!(level(false, Some("info")), Some(DebugLevel::Info), "info");
        assert_eq!(level(true, Some("0")), None, "off");
        Ok(())
    }
}
