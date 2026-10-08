// ============================================================================
// Module:       strobe_app::cli
// Description:  The command line: library folder, starting scene, GL backend,
//               render scale, and the headless screenshot mode.
//
// Dependencies: clap; strobe-gfx (GlChoice, Backend)
// ============================================================================

//! The command line.
//!
//! `--screenshot <path> --frames <n>` is how the UI is verified with no
//! human present: the app renders `n` frames, reads both windows back to
//! `<path>-control.png` and `<path>-output.png`, and exits.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use strobe_gfx::{Backend, GlChoice};

/// Which GL stack to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GlArg {
    /// Native GL 3.3 first, then ANGLE if native fails.
    Auto,
    /// Native GL 3.3 only.
    Native,
    /// ANGLE (EGL, OpenGL ES 3.0) only.
    Angle,
}

impl GlArg {
    /// The renderer's own name for this choice.
    pub fn choice(self) -> GlChoice {
        match self {
            Self::Auto => GlChoice::Auto,
            Self::Native => GlChoice::Only(Backend::Native),
            Self::Angle => GlChoice::Only(Backend::Angle),
        }
    }
}

/// The command line.
#[derive(Parser, Debug)]
#[command(
    name = "strobe",
    version,
    about = "Strobe: an audio-reactive VJ engine"
)]
pub struct Cli {
    /// Folder of scene folders.
    #[arg(long, default_value = "scenes")]
    pub library: PathBuf,
    /// Scene to load at start: a folder name, or a unique prefix of one.
    #[arg(long)]
    pub scene: Option<String>,
    /// GL backend.
    #[arg(long, value_enum, default_value_t = GlArg::Auto)]
    pub gl: GlArg,
    /// Render resolution as a fraction of the output window, 0.25 to 1.0.
    #[arg(long, default_value_t = 1.0)]
    pub render_scale: f32,
    /// Render `--frames` frames, write `<PATH>-control.png` and
    /// `<PATH>-output.png`, and exit.
    #[arg(long, value_name = "PATH")]
    pub screenshot: Option<PathBuf>,
    /// Frames to render before the screenshot.
    #[arg(long, default_value_t = 60, requires = "screenshot")]
    pub frames: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn defaults_load_the_scenes_folder_with_auto_backend() -> Result<()> {
        let cli = Cli::try_parse_from(["strobe"])?;
        assert_eq!(cli.library, PathBuf::from("scenes"), "library");
        assert_eq!(cli.gl, GlArg::Auto, "backend");
        assert!(cli.screenshot.is_none(), "no screenshot");
        Ok(())
    }

    #[test]
    fn a_forced_backend_maps_to_one_attempt() -> Result<()> {
        let cli = Cli::try_parse_from(["strobe", "--gl", "angle"])?;
        assert_eq!(
            cli.gl.choice(),
            GlChoice::Only(Backend::Angle),
            "angle forced"
        );
        Ok(())
    }

    #[test]
    fn frames_without_a_screenshot_is_rejected() {
        assert!(
            Cli::try_parse_from(["strobe", "--frames", "5"]).is_err(),
            "--frames needs --screenshot"
        );
    }
}
