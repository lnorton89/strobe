// ============================================================================
// Module:       strobe_app (binary crate root)
// Description:  The `strobe` desktop app: a control window with a scene
//               browser and controls, and an output window for the show.
//
// Dependencies: winit (event loop), egui (control UI), clap (command line);
//               strobe-gfx (GL), strobe-scene (scenes and controls)
// ============================================================================

//! `strobe`: the desktop app.
//!
//! ```text
//! cargo run --release -p strobe-app -- --scene 05-domain-warp
//! cargo run --release -p strobe-app -- --gl angle --render-scale 0.5
//! cargo run --release -p strobe-app -- --scene 05 --screenshot target/shot --frames 60
//! ```
//!
//! Two windows share one GL context. The *output* window shows the scene
//! letterboxed and waits for vertical blank; the *control* window runs
//! egui at a lower rate and shows a live preview next to the controls.
//! See [`strobe_app::app`] for the frame loop.
//!
//! ## Startup failures
//!
//! The release build is a Windows GUI application with no console, so a
//! failure to start (no usable OpenGL, no display) is logged to
//! `strobe.log` in the temp folder *and* shown in a dialog. The dialog is
//! raised through PowerShell rather than a Win32 call, because `unsafe`
//! is confined to the GL boundary.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context as _, Result};
use clap::Parser as _;
use std::process::ExitCode;
use strobe_app::{app, cli, library, logging};
use winit::event_loop::EventLoop;

fn main() -> ExitCode {
    logging::init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report_failure(&format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = match cli::Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if error.use_stderr() => return Err(error.into()),
        // --help and --version: print and stop.
        Err(error) => error.exit(),
    };
    let (library, scan_error) = match library::Library::scan(&cli.library) {
        Ok(library) => (library, None),
        Err(error) => (
            library::Library {
                dir: cli.library.clone(),
                entries: Vec::new(),
            },
            Some(error),
        ),
    };
    let event_loop = EventLoop::new().context("could not start the window system")?;
    let mut app = app::App::new(cli, library, scan_error);
    event_loop
        .run_app(&mut app)
        .context("the event loop failed")?;
    match app.failure() {
        Some(message) => Err(anyhow::anyhow!("{message}")),
        None => Ok(()),
    }
}

/// Logs a startup failure and shows it where the person will see it.
fn report_failure(message: &str) {
    let log_file = logging::log_path();
    log::error!("{message} (log: {})", log_file.display());
    if cfg!(target_os = "windows") {
        let text = format!("{message}\n\nLog: {}", log_file.display());
        let script = "Add-Type -AssemblyName PresentationFramework; \
                      [void][System.Windows.MessageBox]::Show($env:STROBE_ERROR, 'Strobe', 'OK', 'Error')";
        let shown = std::process::Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", script])
            .env("STROBE_ERROR", text)
            .status();
        if let Err(error) = shown {
            log::error!("could not show the error dialog: {error}");
        }
    }
}
