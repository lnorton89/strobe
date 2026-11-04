// ============================================================================
// Module:       strobe_app::corpus_main
// Description:  `strobe-corpus`: measures every scene in a library in both
//               GLSL dialects and writes report.json and report.md. Also the
//               worker process it starts for each scene.
//
// Dependencies: clap, anyhow; strobe_app::corpus (driver, report, cli)
// ============================================================================

//! `strobe-corpus`: the compatibility harness (D195).
//!
//! ```text
//! cargo xtask corpus -- --library /path/to/scenes
//! cargo run -p strobe-app --bin strobe-corpus -- --library scenes --frames 5
//! ```
//!
//! Run with a library, it starts a worker (this same executable, with a
//! hidden `--worker` flag) per scene and dialect, collects what they write,
//! and produces `<out>/report.json` and `<out>/report.md`. A scene that
//! fails, hangs or crashes the driver is a line in the report, never the end
//! of the run. See [`strobe_app::corpus`] for the design.
//!
//! Without `--expect` the exit status is 0 when the run completed, whatever
//! the scenes did: a measuring tool, not a gate. It is nonzero when the
//! library has no scenes or a file cannot be written. With
//! `--expect <file.json>` it is the gate (D534): the run is compared with
//! the committed expectations and baseline frames, `gate.md` and `diff/`
//! say what differs, and any difference is a nonzero exit.
//!
//! A CLI may print, so this file uses `println!` for progress. The library
//! code beneath it logs.

use anyhow::{Context as _, Result, bail};
use clap::Parser as _;
use std::ffi::OsString;
use std::process::ExitCode;
use strobe_app::corpus::cli::{Cli, WORKER_FLAG, WorkerCli};
use strobe_app::corpus::driver::{self, Options};
use strobe_app::corpus::gate::{self, Expectations};
use strobe_app::corpus::report::Report;
use strobe_app::corpus::{Dialect, Record, Status};

fn main() -> ExitCode {
    // First, in the driver and in each worker: `--gl angle-*` sets ANGLE's
    // backend through the environment before any thread exists (D485).
    let _ = strobe_gfx::gl::angle::choose_from_args(std::env::args_os());
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_default();
    let rest: Vec<OsString> = args.collect();
    if rest.first().is_some_and(|first| first == WORKER_FLAG) {
        let cli = WorkerCli::parse_from(std::iter::once(program).chain(rest.into_iter().skip(1)));
        return if driver::run_worker(&cli) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    match run(&Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The whole run: workers, then the two files, then a summary.
fn run(cli: &Cli) -> Result<()> {
    std::fs::create_dir_all(&cli.out)
        .with_context(|| format!("cannot create {}", cli.out.display()))?;
    let options = Options {
        exe: std::env::current_exe().context("cannot find this executable")?,
        library: cli.library.clone(),
        filter: cli.filter.clone(),
        settings: cli.settings(),
        dialects: cli.dialect.dialects(),
        budget: cli.budget(),
        context: cli.context,
        show_window: cli.show_window,
        gl: cli.gl.clone(),
        work: cli.out.join(".work"),
        frames: cli.expect.as_ref().map(|_| cli.out.join("frames")),
    };
    // A stale diff from an earlier run must not be read as this one's.
    let _ = std::fs::remove_dir_all(cli.out.join("diff"));
    let outcome = driver::run_library(&options, progress)?;
    let report = Report::new(
        &cli.library.display().to_string(),
        cli.settings(),
        cli.timeout,
        outcome.drivers,
        outcome.records,
    );

    let json = cli.out.join("report.json");
    std::fs::write(&json, report.to_json()?)
        .with_context(|| format!("cannot write {}", json.display()))?;
    println!("\nwrote {}", json.display());
    if !cli.json_only {
        let markdown = cli.out.join("report.md");
        std::fs::write(&markdown, report.to_markdown())
            .with_context(|| format!("cannot write {}", markdown.display()))?;
        println!("wrote {}", markdown.display());
    }
    if report.records.is_empty() {
        bail!("nothing was measured");
    }
    for dialect in Dialect::ALL {
        let of = |status: Status| {
            report
                .records
                .iter()
                .filter(|r| r.dialect == dialect && r.status == status)
                .count()
        };
        let total: usize = Status::ALL.iter().map(|s| of(*s)).sum();
        if total > 0 {
            let counts: Vec<String> = Status::ALL
                .iter()
                .filter(|s| of(**s) > 0)
                .map(|s| format!("{} {}", of(*s), s.label()))
                .collect();
            println!(
                "{:<8} {total} scenes: {}",
                dialect.label(),
                counts.join(", ")
            );
        }
    }
    match &cli.expect {
        Some(file) => gate(cli, file, &report),
        None => Ok(()),
    }
}

/// `--expect`: compares the run with the file, or with `--bless` rewrites
/// the file and the baselines from it (D534).
fn gate(cli: &Cli, file: &std::path::Path, report: &Report) -> Result<()> {
    let expectations = Expectations::read(file).map_err(anyhow::Error::msg)?;
    let frames = cli.out.join("frames");
    let Some(baselines) = cli.baselines() else {
        bail!("--expect has no baselines folder");
    };
    if cli.bless {
        let blessed = gate::blessed(report, &expectations);
        std::fs::write(file, blessed.to_json()?)
            .with_context(|| format!("cannot write {}", file.display()))?;
        println!("blessed {}", file.display());
        for written in
            gate::bless_frames(&blessed, &frames, &baselines).map_err(anyhow::Error::msg)?
        {
            println!("blessed {}", written.display());
        }
        return Ok(());
    }
    let mut verdict = gate::check_statuses(report, &expectations);
    gate::check_images(
        &expectations,
        report,
        &frames,
        &baselines,
        &cli.out.join("diff"),
        &mut verdict,
    );
    let markdown = cli.out.join("gate.md");
    std::fs::write(&markdown, verdict.to_markdown(file))
        .with_context(|| format!("cannot write {}", markdown.display()))?;
    println!("wrote {}", markdown.display());
    if verdict.passed() {
        println!(
            "gate passed: {} status(es) and {} frame(s) match {}",
            verdict.matched,
            verdict.compared.len(),
            file.display()
        );
        return Ok(());
    }
    for problem in &verdict.problems {
        println!("GATE  {problem}");
    }
    bail!(
        "the corpus gate failed with {} problem(s) against {}; see {}",
        verdict.problems.len(),
        file.display(),
        markdown.display()
    )
}

/// One line per record, as it arrives.
fn progress(record: &Record) {
    println!(
        "{:<32} {:<8} {:<14} {}",
        record.scene,
        record.dialect.label(),
        record.status.label(),
        record.cause.as_deref().unwrap_or(""),
    );
}
