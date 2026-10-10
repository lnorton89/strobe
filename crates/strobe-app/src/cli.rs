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
//! The output window starts hidden; `--show-output` opens it at launch.
//! `--diagnose` prints the system report and exits (see [`crate::diagnose`]).

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

/// Where audio comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AudioArg {
    /// Capture from an input device; fall back to synthetic audio when
    /// there is none.
    Auto,
    /// Always synthetic audio: a fixed 120 BPM pattern, reproducible.
    Synthetic,
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
    /// Start with adaptive resolution off, so the render scale stays where
    /// `--render-scale` (or the slider) puts it.
    #[arg(long)]
    pub no_adaptive: bool,
    /// Audio source. `synthetic` makes frames reproducible, which is what
    /// the benchmark and screenshots want when a real device is present.
    #[arg(long, value_enum, default_value_t = AudioArg::Auto)]
    pub audio: AudioArg,
    /// Show the output window at launch. Without it only the control window
    /// opens, and the output is shown from the control window's button, F11,
    /// or the monitor picker (D96).
    #[arg(long)]
    pub show_output: bool,
    /// Accept shaders from the "Send to Strobe" browser userscript on
    /// 127.0.0.1 (off by default; the Import panel can switch it on too).
    #[arg(long)]
    pub listen: bool,
    /// The port the listener uses (127.0.0.1 only).
    #[arg(long, default_value_t = strobe_shadertoy::listener::DEFAULT_PORT)]
    pub listen_port: u16,
    /// Do not download Shadertoy textures when importing (they read black
    /// and are listed in the scene's images/pending-assets.json).
    #[arg(long)]
    pub no_texture_download: bool,
    /// Print a report of this machine, build, GL and audio setup, save it
    /// next to the logs, and exit. Attach it to bug reports.
    #[arg(long)]
    pub diagnose: bool,
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
    fn audio_defaults_to_auto_and_can_be_forced_synthetic() -> Result<()> {
        assert_eq!(
            Cli::try_parse_from(["strobe"])?.audio,
            AudioArg::Auto,
            "auto"
        );
        let forced = Cli::try_parse_from(["strobe", "--audio", "synthetic"])?;
        assert_eq!(forced.audio, AudioArg::Synthetic, "forced");
        Ok(())
    }

    #[test]
    fn adaptive_resolution_is_on_unless_switched_off() -> Result<()> {
        assert!(
            !Cli::try_parse_from(["strobe"])?.no_adaptive,
            "on by default"
        );
        assert!(
            Cli::try_parse_from(["strobe", "--no-adaptive"])?.no_adaptive,
            "off on request"
        );
        Ok(())
    }

    #[test]
    fn diagnose_is_a_plain_flag() -> Result<()> {
        assert!(
            Cli::try_parse_from(["strobe", "--diagnose"])?.diagnose,
            "set"
        );
        assert!(!Cli::try_parse_from(["strobe"])?.diagnose, "off by default");
        Ok(())
    }

    #[test]
    fn the_output_window_is_hidden_unless_asked_for() -> Result<()> {
        assert!(
            !Cli::try_parse_from(["strobe"])?.show_output,
            "hidden by default"
        );
        assert!(
            Cli::try_parse_from(["strobe", "--show-output"])?.show_output,
            "shown on request"
        );
        let shot = Cli::try_parse_from(["strobe", "--screenshot", "x", "--show-output"])?;
        assert!(
            shot.show_output && shot.screenshot.is_some(),
            "combines with screenshot mode"
        );
        Ok(())
    }

    #[test]
    fn the_listener_is_off_unless_asked_for_and_the_port_has_a_default() -> Result<()> {
        let off = Cli::try_parse_from(["strobe"])?;
        assert!(!off.listen, "off by default");
        assert_eq!(off.listen_port, 47811, "the documented port");
        assert!(!off.no_texture_download, "textures download by default");
        let on = Cli::try_parse_from(["strobe", "--listen", "--listen-port", "50000"])?;
        assert!(on.listen, "on when asked");
        assert_eq!(on.listen_port, 50000, "port");
        assert!(
            Cli::try_parse_from(["strobe", "--listen-port", "70000"]).is_err(),
            "not a port"
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
