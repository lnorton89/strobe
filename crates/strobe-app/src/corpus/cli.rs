// ============================================================================
// Module:       strobe_app::corpus::cli
// Description:  The command lines of `strobe-corpus`: the one a person types
//               and the hidden one its worker processes are started with.
//
// Dependencies: clap; strobe-gfx (offscreen::ContextMode)
// ============================================================================

//! `strobe-corpus`'s flags.
//!
//! ```text
//! cargo xtask corpus -- --library /path/to/scenes
//! cargo xtask corpus -- --library scenes --frames 5 --size 320x180 --filter tunnel
//! ```
//!
//! The worker's flags are built by [`super::driver`] and read back by
//! [`WorkerCli`]; they are public only so the two stay one type apart.

use super::{Dialect, Settings};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use std::time::Duration;
use strobe_gfx::offscreen::ContextMode;

/// The flag that turns the executable into a worker. It is first on the
/// worker's command line and nowhere in `--help`.
pub const WORKER_FLAG: &str = "--worker";

/// Where the GL context comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ContextArg {
    /// Headless when an EGL device exists, else one hidden window.
    Auto,
    /// An EGL device with no window system.
    Headless,
    /// One hidden window.
    Window,
}

impl ContextArg {
    /// The renderer's name for this choice.
    pub fn mode(self) -> ContextMode {
        match self {
            Self::Auto => ContextMode::Auto,
            Self::Headless => ContextMode::Headless,
            Self::Window => ContextMode::Window,
        }
    }

    /// The command-line spelling.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Headless => "headless",
            Self::Window => "window",
        }
    }
}

/// Which dialects to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DialectArg {
    /// Desktop OpenGL 3.3 core and OpenGL ES 3.0.
    Both,
    /// Desktop OpenGL 3.3 core only.
    Desktop,
    /// OpenGL ES 3.0 only.
    Gles,
}

impl DialectArg {
    /// The dialects this choice runs.
    pub fn dialects(self) -> Vec<Dialect> {
        match self {
            Self::Both => Dialect::ALL.to_vec(),
            Self::Desktop => vec![Dialect::Desktop],
            Self::Gles => vec![Dialect::Gles],
        }
    }
}

/// Parses `WIDTHxHEIGHT` into two positive edges.
///
/// # Errors
///
/// A message naming what was wrong, for clap to show.
pub fn parse_size(text: &str) -> Result<[i32; 2], String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("size must be WIDTHxHEIGHT, got `{text}`"))?;
    let edge = |part: &str| {
        part.trim()
            .parse::<i32>()
            .ok()
            .filter(|edge| (1..=8192).contains(edge))
            .ok_or_else(|| format!("`{part}` is not an edge between 1 and 8192"))
    };
    Ok([edge(w)?, edge(h)?])
}

/// The command line a person types.
#[derive(Parser, Debug)]
#[command(
    name = "strobe-corpus",
    about = "Load, compile and render every scene in a library in both GLSL dialects, \
             and write a machine-readable compatibility report"
)]
pub struct Cli {
    /// Folder of scene folders (`*.synScene`, or any folder with a
    /// `main.glsl`). Read in place; nothing is copied from it.
    #[arg(long)]
    pub library: PathBuf,
    /// Where `report.json` and `report.md` go.
    #[arg(long, default_value = "target/corpus")]
    pub out: PathBuf,
    /// Frames of script `update` and rendering per scene.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=10_000))]
    pub frames: u32,
    /// Render size, WIDTHxHEIGHT.
    #[arg(long, default_value = "160x90", value_parser = parse_size)]
    pub size: [i32; 2],
    /// Only scenes whose folder name contains this.
    #[arg(long)]
    pub filter: Option<String>,
    /// Write `report.json` only.
    #[arg(long)]
    pub json_only: bool,
    /// Which dialects to measure.
    #[arg(long, value_enum, default_value_t = DialectArg::Both)]
    pub dialect: DialectArg,
    /// Seconds one scene (one dialect) may take, context creation
    /// included, before its worker is killed and it is recorded as a
    /// timeout.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout: u64,
    /// Where the GL context comes from.
    #[arg(long, value_enum, default_value_t = ContextArg::Auto)]
    pub context: ContextArg,
}

impl Cli {
    /// What each scene is measured with.
    pub fn settings(&self) -> Settings {
        Settings {
            frames: self.frames,
            size: self.size,
        }
    }

    /// The per-scene time budget.
    pub fn budget(&self) -> Duration {
        Duration::from_secs(self.timeout)
    }
}

/// What a worker process is started with. Built by the driver, never typed.
#[derive(Parser, Debug)]
#[command(name = "strobe-corpus --worker", hide = true)]
pub struct WorkerCli {
    /// The scene folder.
    #[arg(long)]
    pub scene: PathBuf,
    /// The dialect to measure.
    #[arg(long, value_enum)]
    pub dialect: Dialect,
    /// Frames to draw.
    #[arg(long)]
    pub frames: u32,
    /// Render size.
    #[arg(long, value_parser = parse_size)]
    pub size: [i32; 2],
    /// Where the context comes from.
    #[arg(long, value_enum)]
    pub context: ContextArg,
    /// Where to write the result JSON.
    #[arg(long)]
    pub result: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_and_reject_nonsense() {
        assert_eq!(parse_size("160x90"), Ok([160, 90]), "plain");
        assert_eq!(parse_size(" 320 X 180 "), Ok([320, 180]), "spaces, capital");
        assert!(parse_size("160").is_err(), "no separator");
        assert!(parse_size("0x90").is_err(), "zero edge");
        assert!(parse_size("axb").is_err(), "not numbers");
        assert!(parse_size("99999x90").is_err(), "too large");
    }

    #[test]
    fn the_defaults_are_the_documented_ones() {
        let cli = Cli::parse_from(["strobe-corpus", "--library", "scenes"]);
        assert_eq!(cli.frames, 3, "frames");
        assert_eq!(cli.size, [160, 90], "size");
        assert_eq!(cli.out, PathBuf::from("target/corpus"), "out");
        assert!(!cli.json_only, "markdown too");
        assert_eq!(cli.dialect.dialects(), Dialect::ALL.to_vec(), "both");
    }
}
