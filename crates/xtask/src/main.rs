// ============================================================================
// Module:       xtask (binary crate root)
// Description:  The workspace's own tooling behind `cargo xtask`: the local
//               CI runner, the changelog generator, the bench shortcut,
//               crash-dump symbolication, and the icon renderer.
//
// Dependencies: clap, anyhow; modules brand, changelog, ci, bench, corpus,
//               symbolicate
// ============================================================================

//! `cargo xtask`: the repository's tooling, written in the repository's
//! language.
//!
//! ```text
//! cargo xtask ci                              # everything CI runs, in CI's order
//! cargo xtask changelog                       # regenerate CHANGELOG.md
//! cargo xtask changelog --check               # what CI runs
//! cargo xtask changelog --release v0.2.0      # write the section before tagging
//! cargo xtask bench -- --size 1280x720        # strobe-bench, in release
//! cargo xtask corpus -- --library scenes     # compatibility report over a scene folder
//! cargo xtask symbolicate crash-….dmp         # a crash dump as a readable stack
//! cargo xtask brand [--check]                 # icon PNGs and .ico from the SVG sources
//! ```
//!
//! The `xtask` alias in `.cargo/config.toml` is the whole installation.
//!
//! Why a crate and not `cargo run --example` or a shell script: an example
//! belongs to one package, so tooling for the *workspace* ends up parked in
//! whichever crate happened to be convenient, and every example is compiled
//! into that crate's `--all-targets` builds for no reason but to keep it
//! from rotting. A crate of its own is built and linted by the same
//! `--workspace` commands as everything else, so it cannot rot either, it
//! can have unit tests (the changelog parser does), and it works the same
//! on Windows as on Linux, which a bash script would not. It is a binary
//! with no library API, so nothing links against it.
//!
//! Every command is run from the workspace root regardless of where it is
//! invoked: the root is baked in at compile time from this crate's
//! manifest directory, and the process changes into it before doing
//! anything. That is what lets `cargo xtask ci` work from inside
//! `crates/strobe-gfx`.

mod bench;
mod brand;
mod changelog;
mod ci;
mod corpus;
mod symbolicate;

use anyhow::{Context as _, Result, anyhow};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

/// The command line.
#[derive(Parser, Debug)]
#[command(
    name = "cargo xtask",
    about = "Strobe's workspace tooling",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// One subcommand per job.
#[derive(Subcommand, Debug)]
enum Command {
    /// Run locally exactly what CI runs: fmt, clippy, the no-MIDI strobe-io
    /// clippy (Linux), tests, the Windows cross-check, cargo-deny and the
    /// changelog check. Stops at the first failure and says which step it
    /// was.
    Ci(ci::Args),
    /// Regenerate CHANGELOG.md from the git history.
    Changelog {
        /// Compare the committed file's released sections against the
        /// history without rewriting anything. The `Unreleased` section is
        /// exempt.
        #[arg(long, conflicts_with = "release")]
        check: bool,
        /// Write the section for a release that is about to be tagged, so
        /// the tag lands on a tree whose changelog is already final.
        #[arg(long, value_name = "vX.Y.Z")]
        release: Option<String>,
    },
    /// Run strobe-bench in release mode. Arguments after `--` go to it
    /// unchanged, e.g. `cargo xtask bench -- --size 1280x720 --api gles`.
    Bench {
        /// Arguments forwarded to strobe-bench.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Measure a folder of scenes: load each the way the app does (script
    /// included), compile and render it in both GLSL dialects, and write
    /// `target/corpus/report.{json,md}` (D195). Arguments after `--` go to
    /// strobe-corpus, e.g. `cargo xtask corpus -- --library scenes`.
    Corpus {
        /// Arguments forwarded to strobe-corpus.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Turn a crash minidump into a readable stack with the build's own
    /// debug information. Needs `cargo install dump_syms minidump-stackwalk
    /// --locked`.
    Symbolicate(symbolicate::Args),
    /// Render the application icon PNGs and `icon.ico` from the SVG
    /// sources in `assets/brand`.
    Brand {
        /// Fail if a committed file differs from a fresh render instead of
        /// writing anything.
        #[arg(long)]
        check: bool,
    },
}

/// The workspace root: `crates/xtask/../..`.
///
/// `cargo run` names the manifest directory at run time, and that one wins
/// over the one compiled in: two checkouts sharing a target directory (a
/// worktree built with `CARGO_TARGET_DIR`) share one `xtask` binary, which
/// cargo does not rebuild when the sources match, so the compiled-in path
/// can name the other checkout and `ci` would check the wrong tree.
fn workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    manifest
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| anyhow!("{} has no workspace root above it", manifest.display()))
}

/// Parses the command line and runs one subcommand from the workspace root.
fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = workspace_root()?;
    // Paths a person typed are relative to where they typed them.
    let invoked_from = std::env::current_dir().context("could not read the current directory")?;
    std::env::set_current_dir(&root).with_context(|| {
        format!(
            "could not change into the workspace root {}",
            root.display()
        )
    })?;
    match cli.command {
        Command::Ci(args) => ci::run(&args),
        Command::Changelog { check, release } => changelog::run(check, release.as_deref()),
        Command::Bench { args } => bench::run(&args),
        Command::Corpus { args } => corpus::run(&args),
        Command::Symbolicate(args) => symbolicate::run(&args.anchored(&invoked_from)),
        Command::Brand { check } => brand::run(check),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `{:#}` prints the whole context chain on one line, which is
            // what turns "failed to run git" into "failed to run git: No
            // such file or directory".
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
