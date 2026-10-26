// ============================================================================
// Module:       strobe_gfx::gl::angle
// Description:  Which of ANGLE's backends a process asks for (G-06, D485):
//               D3D11, Vulkan, OpenGL or Metal, chosen from `--gl angle-*`
//               before any thread starts, and checked against the renderer
//               string once a context exists.
//
// Dependencies: std only
// ============================================================================

//! ANGLE's backend choice (G-06, D485).
//!
//! ANGLE implements GLES on several native APIs and picks one itself when
//! nobody says: Direct3D 11 on Windows. glutin 0.32 asks for an ANGLE
//! display with no platform attributes and has no way to add one
//! (research/angle-backends.md, section 3), so the choice is made the way
//! ANGLE documents for exactly this case: the `ANGLE_DEFAULT_PLATFORM`
//! environment variable, which its display reads when the caller passes no
//! type.
//!
//! Setting the environment is only sound while no other thread can read it,
//! so [`choose_from_args`] is the **first call in each binary's `main`**,
//! before the logger, the crash monitor or anything else starts a thread.
//! It reads `--gl` from the raw arguments itself, ahead of the real parser,
//! for the same reason. With no `--gl angle-*` it does nothing, and ANGLE
//! picks as it always has.
//!
//! ANGLE ignores a value for a backend it was not built with and falls back
//! to its own default, silently. So once the context exists,
//! [`check_renderer`] compares the renderer string with the backend asked
//! for, and a mismatch is a failure that says what is missing, rather than
//! a run on a backend nobody chose.
//!
//! The fail-over order of D66 is unchanged: native, then ANGLE's default.
//! A process that asked for one ANGLE backend asked for ANGLE only.

use std::ffi::OsString;
use std::sync::OnceLock;

/// The variable ANGLE's display reads for its backend when the caller gives
/// none (`src/libANGLE/Display.cpp`, `GetDisplayTypeFromEnvironment`).
const ENV: &str = "ANGLE_DEFAULT_PLATFORM";

/// One of ANGLE's backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnglePlatform {
    /// Direct3D 11: ANGLE's default on Windows.
    D3d11,
    /// Vulkan, through the system's `vulkan-1` loader and the GPU's driver.
    Vulkan,
    /// The platform's desktop OpenGL underneath ANGLE.
    Gl,
    /// Metal, on macOS.
    Metal,
}

impl AnglePlatform {
    /// Every backend, in the order `--gl` lists them.
    pub const ALL: [Self; 4] = [Self::D3d11, Self::Vulkan, Self::Gl, Self::Metal];

    /// The `--gl` value that asks for this backend.
    #[must_use]
    pub const fn gl_arg(self) -> &'static str {
        match self {
            Self::D3d11 => "angle-d3d11",
            Self::Vulkan => "angle-vulkan",
            Self::Gl => "angle-gl",
            Self::Metal => "angle-metal",
        }
    }

    /// The backend a `--gl` value names, if it names one.
    #[must_use]
    pub fn from_gl_arg(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|platform| platform.gl_arg().eq_ignore_ascii_case(value))
    }

    /// `ANGLE_DEFAULT_PLATFORM`'s value for this backend.
    const fn env_value(self) -> &'static str {
        match self {
            Self::D3d11 => "d3d11",
            Self::Vulkan => "vulkan",
            Self::Gl => "gl",
            Self::Metal => "metal",
        }
    }

    /// What ANGLE's renderer string carries on this backend: "ANGLE
    /// (NVIDIA, ... Direct3D11 vs_5_0 ps_5_0, D3D11)", "ANGLE (NVIDIA,
    /// Vulkan 1.3 (...), NVIDIA)", "ANGLE (..., OpenGL 4.5 core)".
    const fn renderer_marker(self) -> &'static str {
        match self {
            Self::D3d11 => "D3D11",
            Self::Vulkan => "Vulkan",
            Self::Gl => "OpenGL",
            Self::Metal => "Metal",
        }
    }

    /// The backend's name in a sentence.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::D3d11 => "Direct3D 11",
            Self::Vulkan => "Vulkan",
            Self::Gl => "OpenGL",
            Self::Metal => "Metal",
        }
    }

    /// What the backend needs besides ANGLE itself, for the failure.
    const fn needs(self) -> &'static str {
        match self {
            Self::D3d11 => "a Direct3D 11 GPU driver",
            Self::Vulkan => "a Vulkan driver and its loader (vulkan-1.dll on Windows)",
            Self::Gl => "a desktop OpenGL driver",
            Self::Metal => "macOS and an ANGLE built with Metal",
        }
    }
}

/// The backend this process asked for, once [`choose_from_args`] has run.
static CHOSEN: OnceLock<AnglePlatform> = OnceLock::new();

/// Reads `--gl angle-*` from `args` (`--gl X` or `--gl=X`) and, if it names
/// one of ANGLE's backends, asks ANGLE for it and remembers the choice.
///
/// **Call it first in `main`, before anything starts a thread**: it sets an
/// environment variable, which is only sound while no other thread can read
/// the environment. Every other `--gl` value, and none, leaves everything as
/// it was. Returns the backend chosen.
pub fn choose_from_args(args: impl IntoIterator<Item = OsString>) -> Option<AnglePlatform> {
    let platform = gl_value(args).and_then(|value| AnglePlatform::from_gl_arg(&value))?;
    set_platform_env(platform.env_value());
    // Already set means a second call with the same arguments: the first
    // choice stands.
    let _ = CHOSEN.set(platform);
    Some(platform)
}

/// The backend this process asked for, if it asked for one.
#[must_use]
pub fn chosen() -> Option<AnglePlatform> {
    CHOSEN.get().copied()
}

/// Whether an ANGLE context's `renderer` string is on the backend this
/// process asked for. Nothing asked for is always right; otherwise a
/// mismatch says what ANGLE started on instead and what the asked-for
/// backend needs.
pub fn check_renderer(renderer: &str) -> Result<(), String> {
    match chosen() {
        None => Ok(()),
        Some(platform) => check(platform, renderer),
    }
}

/// [`check_renderer`] for a given backend.
fn check(platform: AnglePlatform, renderer: &str) -> Result<(), String> {
    if renderer.contains(platform.renderer_marker()) {
        return Ok(());
    }
    Err(format!(
        "--gl {} asked ANGLE for {}, and it started on another backend ({renderer}): \
         this ANGLE was built without it, or the computer lacks {}",
        platform.gl_arg(),
        platform.label(),
        platform.needs()
    ))
}

/// The value given to `--gl`, from raw arguments.
fn gl_value(args: impl IntoIterator<Item = OsString>) -> Option<String> {
    let mut args = args
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned());
    while let Some(arg) = args.next() {
        if arg == "--gl" {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix("--gl=") {
            return Some(value.to_string());
        }
    }
    None
}

/// Sets `ANGLE_DEFAULT_PLATFORM`.
fn set_platform_env(value: &str) {
    // SAFETY: `set_var` is unsafe because another thread may read or write
    // the environment at the same moment. Its only caller,
    // `choose_from_args`, is documented as the first call in each binary's
    // `main` and is called there before the logger, the crash monitor or
    // any other thread is started, so this thread is the only one.
    unsafe { std::env::set_var(ENV, value) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_gl_value_is_read_in_both_spellings_and_only_from_gl() -> Result<()> {
        assert_eq!(
            gl_value(args(&["strobe", "--gl", "angle-vulkan"])).as_deref(),
            Some("angle-vulkan"),
            "--gl X"
        );
        assert_eq!(
            gl_value(args(&["strobe", "--scene", "x", "--gl=angle-d3d11"])).as_deref(),
            Some("angle-d3d11"),
            "--gl=X"
        );
        assert_eq!(
            gl_value(args(&["strobe", "--glow", "angle-vulkan"])),
            None,
            "another flag"
        );
        assert_eq!(gl_value(args(&["strobe", "--gl"])), None, "no value");
        Ok(())
    }

    #[test]
    fn every_backend_round_trips_through_its_gl_value() -> Result<()> {
        for platform in AnglePlatform::ALL {
            assert_eq!(
                AnglePlatform::from_gl_arg(platform.gl_arg()),
                Some(platform),
                "{platform:?}"
            );
        }
        assert_eq!(AnglePlatform::from_gl_arg("angle"), None, "plain angle");
        assert_eq!(AnglePlatform::from_gl_arg("native"), None, "native");
        Ok(())
    }

    #[test]
    fn the_renderer_string_says_which_backend_ran() -> Result<()> {
        let d3d11 = "ANGLE (NVIDIA, NVIDIA GeForce GTX 1080 Ti Direct3D11 vs_5_0 ps_5_0, D3D11)";
        let vulkan = "ANGLE (NVIDIA, Vulkan 1.3.277 (NVIDIA GeForce GTX 1080 Ti (0x00001B06)), \
                      NVIDIA-582.78.0.0)";
        assert!(check(AnglePlatform::D3d11, d3d11).is_ok(), "D3D11 on D3D11");
        assert!(
            check(AnglePlatform::Vulkan, vulkan).is_ok(),
            "Vulkan on Vulkan"
        );
        let refused = check(AnglePlatform::Vulkan, d3d11)
            .err()
            .unwrap_or_default();
        assert!(
            refused.contains("angle-vulkan") && refused.contains("vulkan-1.dll"),
            "a silent fall-back is a failure that names what is missing: {refused}"
        );
        Ok(())
    }
}
