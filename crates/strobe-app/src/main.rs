// ============================================================================
// Module:       strobe_app (binary crate root)
// Description:  The `strobe` desktop app: a control window with a scene
//               browser and controls, and an output window for the show.
//
// Dependencies: winit (event loop), egui (control UI), clap (command line);
//               strobe-gfx (GL), strobe-scene (scenes and controls),
//               strobe-diag (logging, crash capture)
// ============================================================================

//! `strobe`: the desktop app.
//!
//! ```text
//! cargo run --release -p strobe-app -- --scene 05-domain-warp
//! cargo run --release -p strobe-app -- --gl angle --render-scale 0.5
//! cargo run --release -p strobe-app -- --scene 05 --screenshot target/shot --frames 60
//! cargo run --release -p strobe-app -- --diagnose
//! ```
//!
//! Two windows share one GL context. The *output* window shows the scene
//! letterboxed and waits for vertical blank; the *control* window runs
//! egui at a lower rate and shows a live preview next to the controls.
//! See [`strobe_app::app`] for the frame loop.
//!
//! ## Start-up order
//!
//! This is the order `strobe_diag`'s crate docs prescribe, and it matters:
//!
//! 1. register the notifier (how a person is told about a crash);
//! 2. if this process is the crash *monitor* (the same executable, started
//!    with a hidden argument), run that and exit;
//! 3. `strobe_diag::init()`: logger, panic hook, background system probe;
//! 4. `--diagnose` prints the report and exits;
//! 5. arm native crash capture and hold the handle for the whole run;
//! 6. open the windows (the GL context's facts go into the session
//!    context), then the event loop.
//!
//! ## Startup failures
//!
//! The release build is a Windows GUI application with no console, so a
//! failure to start (no usable OpenGL, no display) is reported as a
//! structured failure in the session log *and* shown in a dialog, the same
//! dialog the crash notifier uses.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context as _, Result};
use clap::Parser as _;
use std::process::ExitCode;
use std::time::Duration;
use strobe_app::{app, cli, diagnose, dialog, generate, library};
use strobe_diag::failure::{self, Failure, Subsystem};
use winit::event_loop::EventLoop;

/// How long to wait for the log writer at exit.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    // 1 and 2: before anything that can fail, because the monitor process
    // reports native crashes with the notifier.
    strobe_diag::notify::set_notifier(|notice| dialog::show_error(&dialog::crash_text(notice)));
    if strobe_diag::crash::run_monitor_if_requested(std::env::args()) {
        return ExitCode::SUCCESS;
    }
    // 3.
    let session = strobe_diag::init();

    let cli = match cli::Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if error.use_stderr() => {
            report_failure(&error.to_string(), false, session.log_file.as_deref());
            return ExitCode::FAILURE;
        }
        // --help and --version: print and stop.
        Err(error) => error.exit(),
    };
    // 4.
    if cli.diagnose {
        diagnose::run();
        return ExitCode::SUCCESS;
    }
    if let Some(args) = generate_worker(&cli) {
        let wrote = generate::worker(&args);
        strobe_diag::logger::flush(FLUSH_TIMEOUT);
        return if wrote {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    // 5. Held until `main` returns; dropping it detaches the handler.
    let _crash_capture = strobe_diag::crash::start_monitor();

    let outcome = run(cli);
    strobe_diag::logger::flush(FLUSH_TIMEOUT);
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(Stopped { error, after_start }) => {
            report_failure(
                &format!("{error:#}"),
                after_start,
                session.log_file.as_deref(),
            );
            ExitCode::FAILURE
        }
    }
}

/// Why a run ended in failure, and whether the windows had opened by then.
/// The worker's arguments, when this process is a generate-controls worker
/// (D202).
fn generate_worker(cli: &cli::Cli) -> Option<generate::WorkerArgs> {
    let scene = cli.generate_controls.clone()?;
    let library = cli
        .generate_library
        .clone()
        .or_else(|| scene.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let report = cli
        .generate_report
        .clone()
        .unwrap_or_else(|| scene.with_extension("generate.json"));
    Some(generate::WorkerArgs {
        scene,
        library,
        report,
        merge: cli.generate_merge,
    })
}

struct Stopped {
    error: anyhow::Error,
    after_start: bool,
}

fn run(cli: cli::Cli) -> Result<(), Stopped> {
    let before_start = |error: anyhow::Error| Stopped {
        error,
        after_start: false,
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
    let event_loop = EventLoop::new()
        .context("could not start the window system")
        .map_err(before_start)?;
    let mut app = app::App::new(cli, library, scan_error);
    let looped = event_loop
        .run_app(&mut app)
        .context("the event loop failed");
    let after_start = app.started();
    looped.map_err(|error| Stopped { error, after_start })?;
    match app.failure() {
        Some(message) => Err(Stopped {
            error: anyhow::anyhow!("{message}"),
            after_start,
        }),
        None => Ok(()),
    }
}

/// Records a failure and shows it where the person will see it. A failure
/// after the windows opened (a context that could not be recovered, say)
/// is the app stopping, not a failed start, and the record says which.
fn report_failure(message: &str, after_start: bool, log_file: Option<&std::path::Path>) {
    let (phase, action) = if after_start {
        ("run", "Strobe stopped")
    } else {
        ("startup", "Strobe did not start")
    };
    failure::report(
        Failure::new(Subsystem::Context, phase)
            .message(message)
            .action(action),
    );
    let mut text = message.to_string();
    if let Some(path) = log_file {
        text.push_str(&format!("\n\nLog: {}", path.display()));
    }
    dialog::show_error(&text);
}
