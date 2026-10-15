// ============================================================================
// Module:       strobe_app::corpus::driver
// Description:  Runs a library through worker processes, one per scene and
//               dialect, with a time budget; and the worker side that
//               measures one scene and writes its result to a file.
//
// Dependencies: std::process; serde_json; strobe-gfx (offscreen);
//               crate::corpus::{run, capture, cli}
// ============================================================================

//! The parent and the worker.
//!
//! ## Why a process per scene and dialect
//!
//! The requirement is that one scene never stops the run. A scene can fail
//! in four ways: it can be rejected (an error value, easy), it can hang (a
//! driver compile or a shader loop that never returns), it can crash the
//! driver (an llvmpipe segfault), or it can take the process down some
//! other way. Only a process boundary survives the last three: a GL context
//! cannot be abandoned from another thread, and a thread stuck inside a
//! driver call cannot be cancelled. So the parent starts the same
//! executable as `strobe-corpus --worker ...`, waits with a deadline, and
//! kills it when the deadline passes.
//!
//! The cost is a context per scene (tens of milliseconds on llvmpipe), which
//! also gives every scene a clean GL state, so one scene's leftovers can
//! never change the next one's result.
//!
//! ## The hand-off
//!
//! The worker writes a [`WorkerResult`] as JSON to the file named by
//! `--result` and exits. Its stderr goes to a file beside it. If the result
//! is missing or unreadable the worker died, and the parent says how
//! (the exit status and the last line the process printed) as a `gl_error`.

use super::capture;
use super::cli::{ContextArg, WORKER_FLAG, WorkerCli};
use super::run::run_scene;
use super::{Dialect, Driver, Record, Settings, Status, WorkerResult};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use strobe_gfx::GlChoice;
use strobe_gfx::offscreen::{self, Request};

/// How often the parent looks at a running worker.
const POLL: Duration = Duration::from_millis(5);

/// What a run needs.
#[derive(Debug, Clone)]
pub struct Options {
    /// The executable to start as a worker: `std::env::current_exe()` for
    /// the binary, the built binary's path for a test.
    pub exe: PathBuf,
    /// The folder of scene folders.
    pub library: PathBuf,
    /// Only scenes whose name contains this.
    pub filter: Option<String>,
    /// What each scene is measured with.
    pub settings: Settings,
    /// The dialects to measure, in order.
    pub dialects: Vec<Dialect>,
    /// The time one scene may take in one dialect.
    pub budget: Duration,
    /// Where the context comes from.
    pub context: ContextArg,
    /// A shown tiny window rather than a hidden one (window contexts).
    pub show_window: bool,
    /// A folder for the workers' result and stderr files, created here and
    /// removed when the run ends.
    pub work: PathBuf,
}

/// Why a run could not even start.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// The library folder could not be listed.
    #[error("{0}")]
    Library(#[from] strobe_scene::SceneError),
    /// The work folder could not be made.
    #[error("cannot create {path}: {source}")]
    Work {
        /// The folder.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// Nothing in the library matched.
    #[error("no scene folders (a folder with main.glsl) in {0}{1}")]
    Empty(PathBuf, String),
}

/// Everything a run produced.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// One record per scene and dialect, scenes in name order.
    pub records: Vec<Record>,
    /// The driver each dialect ran on (the first worker's report).
    pub drivers: Vec<Driver>,
}

/// Measures every scene in the library, calling `progress` with each record
/// as it arrives.
///
/// # Errors
///
/// [`DriverError`] when the library cannot be listed or has no scenes, or
/// the work folder cannot be made. A scene that fails is a record, not an
/// error.
pub fn run_library(
    options: &Options,
    mut progress: impl FnMut(&Record),
) -> Result<Outcome, DriverError> {
    let scenes: Vec<PathBuf> = strobe_scene::load::scan(&options.library)?
        .into_iter()
        .filter(|dir| {
            options.filter.as_ref().is_none_or(|filter| {
                strobe_scene::load::folder_title(dir).contains(filter.as_str())
            })
        })
        .collect();
    if scenes.is_empty() {
        let note = options
            .filter
            .as_ref()
            .map_or_else(String::new, |filter| format!(" matching `{filter}`"));
        return Err(DriverError::Empty(options.library.clone(), note));
    }
    std::fs::create_dir_all(&options.work).map_err(|source| DriverError::Work {
        path: options.work.clone(),
        source,
    })?;

    let mut outcome = Outcome::default();
    for (index, dir) in scenes.iter().enumerate() {
        for dialect in &options.dialects {
            let (record, driver) = run_worker_process(options, dir, *dialect, index);
            progress(&record);
            if let Some(driver) = driver
                && !outcome.drivers.iter().any(|d| d.dialect == driver.dialect)
            {
                outcome.drivers.push(driver);
            }
            outcome.records.push(record);
        }
    }
    // Best effort: the work folder only holds this run's scratch files.
    let _ = std::fs::remove_dir_all(&options.work);
    Ok(outcome)
}

/// Starts one worker and waits for it, up to the budget.
fn run_worker_process(
    options: &Options,
    dir: &Path,
    dialect: Dialect,
    index: usize,
) -> (Record, Option<Driver>) {
    let name = strobe_scene::load::folder_title(dir);
    let stem = format!("{index:04}-{}", dialect.label());
    let result_path = options.work.join(format!("{stem}.json"));
    let stderr_path = options.work.join(format!("{stem}.err"));
    // A stale result from an earlier run must not be read; a missing file is
    // the normal case.
    let _ = std::fs::remove_file(&result_path);

    let died = |why: String| {
        (
            Record::failed(&name, dialect, Status::GlError, &why),
            None::<Driver>,
        )
    };
    let Ok(stderr) = File::create(&stderr_path) else {
        return died(format!("cannot create {}", stderr_path.display()));
    };
    let mut command = Command::new(&options.exe);
    command.arg(WORKER_FLAG);
    if options.show_window {
        command.arg("--show-window");
    }
    let child = command
        .arg("--scene")
        .arg(dir)
        .args(["--dialect", dialect.label()])
        .args(["--frames", &options.settings.frames.to_string()])
        .args([
            "--size",
            &format!("{}x{}", options.settings.size[0], options.settings.size[1]),
        ])
        .args(["--context", options.context.label()])
        .arg("--result")
        .arg(&result_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            return died(format!(
                "cannot start the worker {}: {error}",
                options.exe.display()
            ));
        }
    };

    let deadline = Instant::now() + options.budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // The worker is being abandoned; it may already have exited,
                // and the record says why it stopped.
                let _ = child.kill();
                // Reaps the killed worker; its status is not needed.
                let _ = child.wait();
                let mut record = Record::failed(
                    &name,
                    dialect,
                    Status::Timeout,
                    &format!(
                        "no result within {} s; worker killed",
                        options.budget.as_secs()
                    ),
                );
                record.has_script = dir.join("script.js").is_file();
                return (record, None);
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(error) => {
                // The worker is being abandoned; it may already have exited,
                // and the record says why it stopped.
                let _ = child.kill();
                // Reaps the killed worker; its status is not needed.
                let _ = child.wait();
                return died(format!("cannot wait for the worker: {error}"));
            }
        }
    };

    let parsed = std::fs::read_to_string(&result_path)
        .ok()
        .and_then(|text| serde_json::from_str::<WorkerResult>(&text).ok());
    match parsed {
        Some(result) => (result.record, Some(result.driver)),
        None => {
            let last = last_line(&stderr_path);
            let mut why = format!("the worker died ({status})");
            if let Some(last) = last {
                why.push_str(": ");
                why.push_str(&last);
            }
            let (mut record, driver) = died(why);
            record.has_script = dir.join("script.js").is_file();
            (record, driver)
        }
    }
}

/// The last non-empty line of a file, capped, if it can be read.
fn last_line(path: &Path) -> Option<String> {
    let text = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&text);
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(200).collect())
}

/// The worker: opens the context, measures one scene, writes the result.
/// Returns whether the result was written.
pub fn run_worker(cli: &WorkerCli) -> bool {
    capture::install();
    let dialect = cli.dialect;
    let settings = Settings {
        frames: cli.frames,
        size: cli.size,
    };
    let request = Request {
        mode: cli.context.mode(),
        api: Some(dialect.api()),
        gl: GlChoice::Auto,
        tiny: cli.show_window,
        window_size: None,
    };
    let name = strobe_scene::load::folder_title(&cli.scene);
    let measured = offscreen::run(
        &request,
        |why| log::warn!("{why}"),
        |gl, caps, context| WorkerResult {
            record: run_scene(gl, caps, &cli.scene, dialect, &settings),
            driver: Driver {
                dialect,
                renderer: caps.info.renderer.clone(),
                version: caps.info.version.clone(),
                vendor: caps.info.vendor.clone(),
                context: context.to_string(),
            },
        },
    );
    let result = measured.unwrap_or_else(|error| WorkerResult {
        record: Record::failed(
            &name,
            dialect,
            Status::GlError,
            &format!("no GL context: {error}"),
        ),
        driver: Driver {
            dialect,
            renderer: "unavailable".to_string(),
            version: "unavailable".to_string(),
            vendor: "unavailable".to_string(),
            context: "none".to_string(),
        },
    });
    match serde_json::to_string(&result) {
        Ok(json) => std::fs::write(&cli.result, json).is_ok(),
        Err(_) => false,
    }
}
