// ============================================================================
// Module:       xtask::ci
// Description:  `cargo xtask ci`: runs locally, in order, the checks that
//               .github/workflows/ci.yml runs, and stops at the first failure.
//
// Dependencies: clap, anyhow, std::process; crate::changelog and crate::brand
//               (the in-process changelog and icon checks)
// ============================================================================

//! `cargo xtask ci`: what CI runs, without the push.
//!
//! The steps, in the order CI's Linux job runs them (cheapest first, so
//! the common failure — a formatting slip — costs a second, not a build):
//!
//! 1. `cargo fmt --all -- --check`
//! 2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
//! 3. `cargo clippy -p strobe-io --no-default-features --all-targets -- -D
//!    warnings`, on Linux only: the show-control crate without its MIDI
//!    device layer (the `midi` feature), which the all-features run above
//!    never compiles. Code that only the `midi` build reads is dead there,
//!    and `-D warnings` makes that a failure. One small crate, and the
//!    workspace dependencies are already built, so it costs seconds.
//! 4. `cargo test --workspace --all-features`
//! 5. the same clippy for `x86_64-pc-windows-msvc`, when that target's
//!    standard library is installed and the host is not already Windows
//!    (and `strobe-shadertoy` without its `net` feature, which needs a C
//!    compiler for MSVC)
//! 6. `cargo deny check`, when `cargo-deny` is installed
//! 7. the changelog check
//! 8. the brand check: the icon PNGs and `.ico` match a fresh render of
//!    their SVG sources (`cargo xtask brand --check`)
//!
//! It **stops at the first failure**. Later steps usually fail for the same
//! reason, and a wall of clippy errors under a formatting failure buries
//! the one line that matters. The summary names the step that failed and
//! the exact command to re-run it.
//!
//! A step that cannot run here (no Windows target, no cargo-deny) is
//! reported as *skipped, with the reason and the command that would enable
//! it*, never silently omitted: a green `cargo xtask ci` that quietly
//! skipped the Windows type-check would be a false promise, and the whole
//! point of this command is that green here means green there.
//!
//! On Linux the renderer tests are run with `STROBE_REQUIRE_HEADLESS_GL=1`,
//! as CI's Linux job does, so a missing Mesa EGL fails loudly rather than
//! letting every shader test skip itself and report success. Pass
//! `--allow-gl-skip` to opt out on a machine that cannot have one.
//! Elsewhere the variable is left alone, as in CI's Windows and macOS jobs.
//!
//! The test step also sets `STROBE_SKIP_LOG` (D249), and every GPU test
//! that skips for want of a context writes a line there
//! (`strobe_gfx::skip`). The summary then says how many tests skipped and
//! why, so a run where they skipped never reads as one where they ran.

use crate::{brand, changelog};
use anyhow::{Context as _, Result, bail};
use clap::Args as ClapArgs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// The target the Windows cross-check type-checks for.
const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

/// The environment variable that turns a missing headless GL driver from a
/// skip into a failure.
const REQUIRE_GL: &str = "STROBE_REQUIRE_HEADLESS_GL";

/// The file GPU tests record their skips in (`strobe_gfx::skip::LOG_VAR`).
const SKIP_LOG: &str = "STROBE_SKIP_LOG";

/// Options for `cargo xtask ci`.
#[derive(ClapArgs, Debug)]
pub(crate) struct Args {
    /// Do not require a headless GL driver on Linux; the renderer tests
    /// skip themselves instead of failing. CI never does this.
    #[arg(long)]
    allow_gl_skip: bool,
    /// Skip `cargo deny check` (it downloads the advisory database, so it
    /// needs a network connection).
    #[arg(long)]
    no_deny: bool,
}

/// What a step does.
enum Action {
    /// Run `cargo` with these arguments and these extra environment
    /// variables.
    Cargo {
        args: Vec<&'static str>,
        env: Vec<(&'static str, String)>,
    },
    /// The in-process changelog check.
    Changelog,
    /// The in-process icon check.
    Brand,
    /// Not runnable here, for this reason.
    Skip(String),
}

/// One line of the plan.
struct Step {
    name: &'static str,
    action: Action,
}

/// How one step ended.
enum Outcome {
    Passed,
    Failed(String),
    Skipped(String),
}

/// Runs every step in order and stops at the first failure.
pub(crate) fn run(args: &Args) -> Result<()> {
    let steps = plan(args)?;
    let total = steps.len();
    let started = Instant::now();
    let mut results: Vec<(&'static str, Outcome, Duration)> = Vec::new();

    for (index, step) in steps.iter().enumerate() {
        println!(
            "\n==> [{}/{total}] {}: {}",
            index + 1,
            step.name,
            describe(&step.action)
        );
        let step_started = Instant::now();
        let outcome = execute(&step.action);
        let failed = matches!(outcome, Outcome::Failed(_));
        results.push((step.name, outcome, step_started.elapsed()));
        if failed {
            break;
        }
    }

    println!("\n==> summary ({:.1}s)", started.elapsed().as_secs_f32());
    let mut failure: Option<String> = None;
    for (name, outcome, took) in &results {
        let seconds = took.as_secs_f32();
        match outcome {
            Outcome::Passed => println!("    ok       {name:<16} {seconds:>6.1}s"),
            Outcome::Skipped(reason) => println!("    skipped  {name:<16} {reason}"),
            Outcome::Failed(why) => {
                println!("    FAILED   {name:<16} {seconds:>6.1}s");
                failure = Some(format!("{name}: {why}"));
            }
        }
    }
    report_skips(&skip_log()?);
    let not_run = total.saturating_sub(results.len());
    if let Some(why) = failure {
        if not_run > 0 {
            println!("    not run  {not_run} step(s) after the failure");
        }
        bail!("CI would fail at `{why}`");
    }
    println!("\nAll {total} steps passed or were skipped with a reason. CI should agree.");
    Ok(())
}

/// Where the test step's skips are written: `target/test-skips.log` at
/// the workspace root, absolute, since each test runs in its own crate's
/// folder.
fn skip_log() -> Result<std::path::PathBuf> {
    Ok(crate::workspace_root()?
        .join("target")
        .join("test-skips.log"))
}

/// Says how many tests skipped and why, from the log the GPU tests wrote
/// (`test\treason` per line). Nothing when none skipped. A skipped test
/// passed the harness, so this is the only place the difference shows.
fn report_skips(log: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(log) else {
        return;
    };
    let mut by_reason: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (test, reason) = line.split_once('\t').unwrap_or((line, ""));
        // The error text after the reason's first parenthesis varies by
        // driver; the reason before it is what groups.
        let reason = reason.split(" (").next().unwrap_or(reason).to_owned();
        by_reason.entry(reason).or_default().insert(test.to_owned());
    }
    let tests: std::collections::BTreeSet<&String> = by_reason.values().flatten().collect();
    if tests.is_empty() {
        return;
    }
    println!(
        "    note     {} test(s) passed by skipping, not by running (see {}):",
        tests.len(),
        log.display()
    );
    for (reason, tests) in &by_reason {
        println!("               {:>3} {reason}", tests.len());
    }
}

/// The ordered list of steps for this machine.
fn plan(args: &Args) -> Result<Vec<Step>> {
    let mut test_env: Vec<(&'static str, String)> = Vec::new();
    if cfg!(target_os = "linux") && !args.allow_gl_skip && std::env::var_os(REQUIRE_GL).is_none() {
        test_env.push((REQUIRE_GL, "1".to_owned()));
    }
    let skip_log = skip_log()?;
    // A fresh log per run; a missing one is already fresh.
    let _ = std::fs::remove_file(&skip_log);
    test_env.push((SKIP_LOG, skip_log.display().to_string()));

    let mut steps = vec![
        Step {
            name: "rustfmt",
            action: Action::Cargo {
                args: vec!["fmt", "--all", "--", "--check"],
                env: Vec::new(),
            },
        },
        Step {
            name: "clippy",
            action: Action::Cargo {
                args: vec![
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--all-features",
                    "--",
                    "-D",
                    "warnings",
                ],
                env: Vec::new(),
            },
        },
        Step {
            name: "clippy no-midi",
            // The `midi` feature is on by default and `--all-features` keeps
            // it on, so without this step nothing builds the crate the way a
            // MIDI-less build does, and dead code there (a constant only the
            // device thread reads) would go unseen. Linux only: it is the
            // same Rust on every host, and one platform is enough to catch
            // it; the others would only pay for another build.
            action: if cfg!(target_os = "linux") {
                Action::Cargo {
                    args: vec![
                        "clippy",
                        "-p",
                        "strobe-io",
                        "--no-default-features",
                        "--all-targets",
                        "--",
                        "-D",
                        "warnings",
                    ],
                    env: Vec::new(),
                }
            } else {
                Action::Skip("Linux only; CI's Linux job runs it".to_owned())
            },
        },
        Step {
            name: "test",
            action: Action::Cargo {
                args: vec!["test", "--workspace", "--all-features"],
                env: test_env,
            },
        },
    ];

    steps.push(Step {
        name: "windows clippy",
        action: match windows_cross_check()? {
            Some(reason) => Action::Skip(reason),
            // strobe-script is excluded: rquickjs compiles QuickJS's C
            // sources, which needs MSVC and the Windows SDK. strobe-app is
            // excluded because it depends on strobe-script, so cargo would
            // build it anyway (its code has no `#[cfg(windows)]` of its own:
            // the Windows branches are `cfg!` values, type-checked on every
            // host). This step only runs on non-Windows hosts, so both are
            // checked for Windows by a native build (CI's windows job)
            // instead. strobe-shadertoy is excluded here and checked by the
            // next step without its `net` feature: the HTTP client's TLS
            // (ring) compiles C for MSVC (D110).
            None => Action::Cargo {
                args: vec![
                    "clippy",
                    "--workspace",
                    "--exclude",
                    "strobe-script",
                    "--exclude",
                    "strobe-app",
                    "--exclude",
                    "strobe-shadertoy",
                    "--all-targets",
                    "--all-features",
                    "--target",
                    WINDOWS_TARGET,
                    "--",
                    "-D",
                    "warnings",
                ],
                env: Vec::new(),
            },
        },
    });

    steps.push(Step {
        name: "windows shadertoy",
        action: match windows_cross_check()? {
            Some(reason) => Action::Skip(reason),
            // Everything in strobe-shadertoy except the `net` feature's
            // HTTP client, which needs a C compiler for MSVC. The client is
            // type-checked natively by CI's windows job and by the Linux
            // job's all-features clippy.
            None => Action::Cargo {
                args: vec![
                    "clippy",
                    "-p",
                    "strobe-shadertoy",
                    "--no-default-features",
                    "--all-targets",
                    "--target",
                    WINDOWS_TARGET,
                    "--",
                    "-D",
                    "warnings",
                ],
                env: Vec::new(),
            },
        },
    });

    steps.push(Step {
        name: "cargo-deny",
        action: if args.no_deny {
            Action::Skip("--no-deny".to_owned())
        } else if tool_works("cargo", &["deny", "--version"]) {
            Action::Cargo {
                args: vec!["deny", "check"],
                env: Vec::new(),
            }
        } else {
            Action::Skip(
                "cargo-deny is not installed (`cargo install cargo-deny --locked`)".to_owned(),
            )
        },
    });

    steps.push(Step {
        name: "changelog",
        action: Action::Changelog,
    });
    steps.push(Step {
        name: "brand",
        action: Action::Brand,
    });
    Ok(steps)
}

/// `None` when the Windows cross-check can run; otherwise why it cannot.
fn windows_cross_check() -> Result<Option<String>> {
    if host_triple()? == WINDOWS_TARGET {
        return Ok(Some(format!(
            "the host is {WINDOWS_TARGET}, so the clippy step already checked it"
        )));
    }
    let libdir = rustc_output(&["--print", "target-libdir", "--target", WINDOWS_TARGET]);
    let installed = libdir
        .as_deref()
        .map(str::trim)
        .map(Path::new)
        .is_some_and(|dir| dir.is_dir());
    if installed {
        Ok(None)
    } else {
        Ok(Some(format!(
            "the {WINDOWS_TARGET} standard library is not installed (`rustup target add {WINDOWS_TARGET}`)"
        )))
    }
}

/// The host triple, from `rustc -vV`.
fn host_triple() -> Result<String> {
    let verbose = rustc_output(&["-vV"]).context("failed to run `rustc -vV`")?;
    verbose
        .lines()
        .find_map(|line| line.strip_prefix("host:"))
        .map(|host| host.trim().to_owned())
        .context("`rustc -vV` printed no `host:` line")
}

/// Stdout of `rustc <args>` if it succeeded.
fn rustc_output(args: &[&str]) -> Option<String> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Whether `program args` runs and succeeds, with its output discarded.
fn tool_works(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// The command a step runs, as the line printed above its output.
fn describe(action: &Action) -> String {
    match action {
        Action::Cargo { args, env } => {
            let vars: String = env.iter().map(|(k, v)| format!("{k}={v} ")).collect();
            format!("{vars}cargo {}", args.join(" "))
        }
        Action::Changelog => "cargo xtask changelog --check".to_owned(),
        Action::Brand => "cargo xtask brand --check".to_owned(),
        Action::Skip(reason) => format!("skipped ({reason})"),
    }
}

/// Runs one step.
fn execute(action: &Action) -> Outcome {
    match action {
        Action::Skip(reason) => Outcome::Skipped(reason.clone()),
        Action::Changelog => match changelog::run(true, None) {
            Ok(()) => Outcome::Passed,
            Err(error) => Outcome::Failed(format!("{error:#}")),
        },
        Action::Brand => match brand::run(true) {
            Ok(()) => Outcome::Passed,
            Err(error) => Outcome::Failed(format!("{error:#}")),
        },
        Action::Cargo { args, env } => {
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let mut command = Command::new(cargo);
            command.args(args);
            for (key, value) in env {
                command.env(key, value);
            }
            match command.status() {
                Ok(status) if status.success() => Outcome::Passed,
                Ok(status) => {
                    Outcome::Failed(format!("`cargo {}` exited with {status}", args.join(" ")))
                }
                Err(error) => Outcome::Failed(format!("could not start cargo: {error}")),
            }
        }
    }
}
