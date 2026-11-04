// ============================================================================
// Module:       xtask::ci
// Description:  `cargo xtask ci`: the checks .github/workflows/ci.yml runs, as
//               named lanes with the host policy in Rust, a strict mode for CI
//               and a doctor that lists what this machine can run.
//
// Dependencies: clap, anyhow, std::process; crate::changelog, crate::brand
//               and crate::credits (the in-process changelog, icon and
//               credits checks)
// ============================================================================

//! `cargo xtask ci`: what CI runs, without the push, and what CI runs, full
//! stop (D533).
//!
//! The commands live here, not in the workflow. Each CI job installs what
//! its lanes need, restores its cache and runs
//! `cargo --locked xtask ci --strict <lanes>`; a developer runs the same
//! lanes with the same arguments. So the list of commands, their flags and
//! the per-platform exclusions are written once, in Rust, and a change to
//! them is a change to both.
//!
//! The lanes, and the CI job that runs each:
//!
//! | lane | what | CI job |
//! |---|---|---|
//! | `fmt` | `cargo fmt --all -- --check` | `rustfmt` |
//! | `check` | clippy `-D warnings` over the workspace; on Linux also `strobe-io` without MIDI and the Windows cross-check; on macOS the portable crates only | each platform job |
//! | `test` | `cargo test --workspace --all-features`; on macOS the portable crates | each platform job |
//! | `deny` | `cargo deny check` | `cargo-deny` |
//! | `generated` | the changelog, icon and credits checks | `changelog` |
//! | `render` | Linux: the app on Xvfb, auto-controls on a window context, the scene library in both GLSL dialects | `linux` |
//! | `build` | Windows: the `release-fast` binaries, and with `--shipping` the fat-LTO `release` ones | `windows` |
//! | `conventions` | `cargo test -p strobe-conventions` | `docs` (a docs-only pull request) |
//!
//! `all` (the default) is `fmt check test deny generated render build`:
//! every lane this host's CI job runs, plus the cheap shared ones.
//! `doctor` runs nothing and lists, for the lanes named (or all of them),
//! what this machine would run, what does not apply here, and what is
//! missing and how to get it.
//!
//! **A step that cannot run is one of two things.** *Not here*: the lane
//! does not apply to this host (the Xvfb render outside Linux), as in CI,
//! where the job for that host does not run it either. *Missing*: it
//! applies, but a prerequisite is absent (`cargo-deny`, the Windows
//! target, `xvfb-run`). Without `--strict` a missing step is reported as
//! such and the run ends with "partial", never with "CI should agree";
//! with `--strict` (what CI passes) it is a failure, so a runner image
//! that lost a tool fails the job instead of quietly checking less.
//!
//! `--strict` also snapshots `git status` before the first step and fails
//! if the tree differs after the last: a check that rewrote a generated
//! file, or a build that touched a tracked one, is caught here. Every cargo
//! command that resolves dependencies passes `--locked`, so a `Cargo.lock`
//! that does not match the manifests fails rather than being repaired.
//!
//! It **stops at the first failure**: later steps usually fail for the same
//! reason, and a wall of clippy errors under a formatting failure buries
//! the line that matters. Steps marked informational (macOS's
//! whole-workspace clippy) report a failure without stopping or failing.
//! Long steps have a timeout of their own, so a window that never gets its
//! frames ends the step in minutes, not the job in hours.
//!
//! Environment: on Linux the tests run with `STROBE_REQUIRE_HEADLESS_GL=1`
//! (a missing Mesa EGL fails rather than letting every shader test skip
//! itself; `--allow-gl-skip` opts out). The test step sets
//! `STROBE_SKIP_LOG` (D249) and the summary counts the GPU tests that passed
//! by skipping. Tests that bind every interface run when `STROBE_TEST_LAN`
//! is set, which this sets on Linux and on any CI runner (`CI`), where no
//! firewall asks about it (D503). Under GitHub Actions the summary is also
//! written to the job's step summary.

use crate::{brand, changelog, credits};
use anyhow::{Context as _, Result, bail};
use clap::{Args as ClapArgs, ValueEnum};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// The target the Windows cross-check type-checks for.
const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

/// The environment variable that turns a missing headless GL driver from a
/// skip into a failure.
const REQUIRE_GL: &str = "STROBE_REQUIRE_HEADLESS_GL";

/// The file GPU tests record their skips in (`strobe_gfx::skip::LOG_VAR`).
const SKIP_LOG: &str = "STROBE_SKIP_LOG";

/// The environment variable that lets tests bind beyond loopback (D503).
const LAN_TESTS: &str = "STROBE_TEST_LAN";

/// The crates macOS cannot build yet: `strobe-gfx` has no CGL context, and
/// the rest depend on it (strobe-autocontrol renders to measure, strobe-web
/// is the scene player). strobe-shadertoy's tests render, so it is checked
/// without them.
const NOT_ON_MACOS: [&str; 6] = [
    "strobe-gfx",
    "strobe-bench",
    "strobe-app",
    "strobe-autocontrol",
    "strobe-shadertoy",
    "strobe-web",
];

/// Options for `cargo xtask ci`.
#[derive(ClapArgs, Debug)]
pub(crate) struct Args {
    /// The lanes to run, in order. `all` (the default) is every lane CI
    /// runs on this kind of host; `doctor` lists instead of running.
    #[arg(value_enum)]
    lanes: Vec<Lane>,
    /// Fail when a step that applies here cannot run for want of a
    /// prerequisite, and when the tree changed during the run. CI passes it.
    #[arg(long)]
    strict: bool,
    /// In the `build` lane, also build the shipping `release` binaries (fat
    /// LTO, several minutes).
    #[arg(long)]
    shipping: bool,
    /// Do not require a headless GL driver on Linux; the renderer tests
    /// skip themselves instead of failing. CI never does this.
    #[arg(long)]
    allow_gl_skip: bool,
    /// Skip `cargo deny check` (it downloads the advisory database, so it
    /// needs a network connection).
    #[arg(long)]
    no_deny: bool,
}

/// A named group of steps, matching a CI job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Lane {
    /// Every lane below except `conventions`, which `test` covers.
    All,
    /// List what each lane would do on this machine, and run nothing.
    Doctor,
    /// rustfmt.
    Fmt,
    /// Clippy, with this host's exclusions and extra checks.
    Check,
    /// The test suite.
    Test,
    /// cargo-deny.
    Deny,
    /// The changelog, icon and credits checks.
    Generated,
    /// The app, auto-controls and scene library on a real GL driver (Linux).
    Render,
    /// The optimised binaries (Windows).
    Build,
    /// The conventions tests alone (links, decisions, known gaps).
    Conventions,
}

impl Lane {
    /// The lanes `all` stands for, in the order they run.
    const ALL: [Self; 7] = [
        Self::Fmt,
        Self::Check,
        Self::Test,
        Self::Deny,
        Self::Generated,
        Self::Render,
        Self::Build,
    ];

    /// The name on the command line.
    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Doctor => "doctor",
            Self::Fmt => "fmt",
            Self::Check => "check",
            Self::Test => "test",
            Self::Deny => "deny",
            Self::Generated => "generated",
            Self::Render => "render",
            Self::Build => "build",
            Self::Conventions => "conventions",
        }
    }
}

/// The kind of machine the lanes are planned for. CI has one job per kind,
/// and each lane does here what that job does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Host {
    Linux,
    Windows,
    MacOs,
    /// A BSD, say: the shared lanes run, the platform ones do not apply.
    Other,
}

impl Host {
    /// The host this binary was built for, which is the one it runs on.
    fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

/// What a step does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// Run a program (cargo, or a wrapper such as `xvfb-run`) with these
    /// arguments and these extra environment variables.
    Run {
        program: String,
        args: Vec<String>,
        env: Vec<(&'static str, String)>,
    },
    /// The in-process changelog check.
    Changelog,
    /// The in-process icon check.
    Brand,
    /// The in-process credits check.
    Credits,
    /// These files exist and are not empty.
    Nonempty(Vec<&'static str>),
    /// `git status` is what it was before the first step.
    CleanTree(String),
    /// The lane does not apply to this host, for this reason.
    NotHere(String),
    /// The step applies here but a prerequisite is missing: why, and the
    /// command that would fix it.
    Missing { why: String, fix: String },
}

/// One line of the plan.
#[derive(Debug)]
struct Step {
    /// The lane it belongs to.
    lane: Lane,
    /// A short name for the summary.
    name: &'static str,
    action: Action,
    /// How long it may run before it is killed and fails.
    timeout: Option<Duration>,
    /// A failure is reported but neither stops the run nor fails it.
    informational: bool,
}

impl Step {
    fn new(lane: Lane, name: &'static str, action: Action) -> Self {
        Self {
            lane,
            name,
            action,
            timeout: None,
            informational: false,
        }
    }

    fn timeout(mut self, minutes: u64) -> Self {
        self.timeout = Some(Duration::from_secs(minutes * 60));
        self
    }

    fn informational(mut self) -> Self {
        self.informational = true;
        self
    }
}

/// How one step ended.
#[derive(Debug)]
enum Outcome {
    Passed,
    Failed(String),
    /// An informational step failed.
    Warned(String),
    NotHere(String),
    Missing(String),
}

/// The facts about this machine the plan depends on, gathered once so the
/// plan itself is a pure function the tests can call.
#[derive(Debug, Clone)]
struct Machine {
    host: Host,
    /// The program that runs cargo (`$CARGO`, else `cargo`).
    cargo: String,
    /// `cargo deny --version` succeeds.
    deny: bool,
    /// The Windows target's standard library is installed.
    windows_target: bool,
    /// `xvfb-run` is on the path.
    xvfb: bool,
    /// Running under CI (`CI` is set).
    ci: bool,
    /// The headless GL requirement was already set by the caller.
    require_gl_set: bool,
    /// `STROBE_TEST_LAN` was already set by the caller.
    lan_set: bool,
    /// Where the GPU tests log their skips.
    skip_log: PathBuf,
}

impl Machine {
    /// Looks at this machine.
    fn detect() -> Result<Self> {
        let host = Host::current();
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        Ok(Self {
            host,
            deny: tool_works(&cargo, &["deny", "--version"]),
            windows_target: host != Host::Windows && windows_target_installed(),
            xvfb: host == Host::Linux && tool_works("xvfb-run", &["--help"]),
            ci: std::env::var_os("CI").is_some(),
            require_gl_set: std::env::var_os(REQUIRE_GL).is_some(),
            lan_set: std::env::var_os(LAN_TESTS).is_some(),
            skip_log: crate::workspace_root()?
                .join("target")
                .join("test-skips.log"),
            cargo,
        })
    }
}

/// Runs the lanes asked for, or lists them for `doctor`.
pub(crate) fn run(args: &Args) -> Result<()> {
    let machine = Machine::detect()?;
    let doctor = args.lanes.contains(&Lane::Doctor);
    let lanes = expand(&args.lanes);
    let tree = if args.strict && !doctor {
        Some(git_status().context(
            "--strict compares `git status` before and after the run, and git did not answer",
        )?)
    } else {
        None
    };
    let steps = plan(&lanes, args, &machine, tree);
    if doctor {
        return report_doctor(&steps, args.strict);
    }
    if lanes.contains(&Lane::Test) {
        // A fresh log per run; a missing one is already fresh.
        let _ = std::fs::remove_file(&machine.skip_log);
    }
    if lanes.contains(&Lane::Render) {
        // A screenshot left by an earlier run must not pass for this one.
        for stale in SCREENSHOTS {
            // Absent already is as good as removed.
            let _ = std::fs::remove_file(stale);
        }
    }
    execute_all(&steps, args.strict, &machine)
}

/// `all` stands for its lanes, `doctor` for nothing (or for `all` when it is
/// alone), and nothing at all for `all`. Duplicates are dropped.
fn expand(asked: &[Lane]) -> Vec<Lane> {
    let named: Vec<Lane> = asked
        .iter()
        .copied()
        .filter(|lane| *lane != Lane::Doctor)
        .collect();
    let named = if named.is_empty() {
        vec![Lane::All]
    } else {
        named
    };
    let mut out: Vec<Lane> = Vec::new();
    for lane in named {
        let members: &[Lane] = if lane == Lane::All {
            &Lane::ALL
        } else {
            std::slice::from_ref(&lane)
        };
        for member in members {
            if !out.contains(member) {
                out.push(*member);
            }
        }
    }
    out
}

/// The screenshots the render lane's app smoke test must produce.
const SCREENSHOTS: [&str; 2] = ["target/ci-shot-control.png", "target/ci-shot-output.png"];

/// The ordered steps for these lanes on this machine.
fn plan(lanes: &[Lane], args: &Args, machine: &Machine, tree: Option<String>) -> Vec<Step> {
    let mut steps = Vec::new();
    for lane in lanes {
        match lane {
            Lane::All | Lane::Doctor => {}
            Lane::Fmt => steps.push(Step::new(
                *lane,
                "rustfmt",
                cargo(machine, &["fmt", "--all", "--", "--check"]),
            )),
            Lane::Check => check_lane(&mut steps, machine),
            Lane::Test => steps.push(test_step(args, machine)),
            Lane::Deny => steps.push(Step::new(
                *lane,
                "cargo-deny",
                if args.no_deny {
                    Action::NotHere("--no-deny".to_owned())
                } else if machine.deny {
                    cargo(machine, &["deny", "check"])
                } else {
                    Action::Missing {
                        why: "cargo-deny is not installed".to_owned(),
                        fix: "cargo install cargo-deny --locked".to_owned(),
                    }
                },
            )),
            Lane::Generated => {
                steps.push(Step::new(*lane, "changelog", Action::Changelog));
                steps.push(Step::new(*lane, "brand", Action::Brand));
                steps.push(Step::new(*lane, "credits", Action::Credits));
            }
            Lane::Render => render_lane(&mut steps, args, machine),
            Lane::Build => build_lane(&mut steps, args, machine),
            Lane::Conventions => steps.push(Step::new(
                *lane,
                "conventions",
                cargo(machine, &["test", "--locked", "-p", "strobe-conventions"]),
            )),
        }
    }
    if let Some(before) = tree {
        steps.push(Step::new(
            Lane::All,
            "clean tree",
            Action::CleanTree(before),
        ));
    }
    steps
}

/// `cargo <args>`, with no extra environment.
fn cargo(machine: &Machine, args: &[&str]) -> Action {
    Action::Run {
        program: machine.cargo.clone(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: Vec::new(),
    }
}

/// `cargo <args>` under `xvfb-run` on a virtual screen of this size, or
/// missing without `xvfb-run`.
fn under_xvfb(
    machine: &Machine,
    screen: &str,
    args: &[&str],
    env: Vec<(&'static str, String)>,
) -> Action {
    if !machine.xvfb {
        return Action::Missing {
            why: "xvfb-run is not installed".to_owned(),
            fix: "sudo apt-get install -y xvfb".to_owned(),
        };
    }
    let mut full = vec![
        "-a".to_owned(),
        "-s".to_owned(),
        format!("-screen 0 {screen}"),
        machine.cargo.clone(),
    ];
    full.extend(args.iter().map(|arg| (*arg).to_owned()));
    Action::Run {
        program: "xvfb-run".to_owned(),
        args: full,
        env,
    }
}

/// `--exclude <crate>` for every crate macOS cannot build.
fn macos_excludes() -> Vec<&'static str> {
    NOT_ON_MACOS
        .iter()
        .flat_map(|krate| ["--exclude", *krate])
        .collect()
}

/// Clippy, and on Linux the two extra configurations CI checks there.
fn check_lane(steps: &mut Vec<Step>, machine: &Machine) {
    const LANE: Lane = Lane::Check;
    const DENY: [&str; 3] = ["--", "-D", "warnings"];
    let mut clippy = vec!["clippy", "--locked", "--workspace"];
    if machine.host == Host::MacOs {
        // macOS is a later target: the blocking tier is every crate with no
        // GL context in it, so the port is not made harder by drift.
        clippy.extend(macos_excludes());
    }
    clippy.extend(["--all-targets", "--all-features"]);
    clippy.extend(DENY);
    steps.push(Step::new(LANE, "clippy", cargo(machine, &clippy)));

    if machine.host == Host::MacOs {
        let mut shadertoy = vec![
            "clippy",
            "--locked",
            "-p",
            "strobe-shadertoy",
            "--lib",
            "--bins",
            "--all-features",
        ];
        shadertoy.extend(DENY);
        steps.push(Step::new(
            LANE,
            "clippy shadertoy",
            cargo(machine, &shadertoy),
        ));
        // Allowed to fail: its log is the running list of what the CGL port
        // has left. When it goes green, drop the exclusions above.
        let mut whole = vec![
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--all-features",
        ];
        whole.extend(DENY);
        steps.push(Step::new(LANE, "clippy whole", cargo(machine, &whole)).informational());
    }

    // The `midi` feature is on by default and `--all-features` keeps it on,
    // so without this nothing builds the crate the way a MIDI-less build
    // does, and dead code only the device thread reads would go unseen.
    // Linux only: the same Rust on every host, and one is enough.
    let mut no_midi = vec![
        "clippy",
        "--locked",
        "-p",
        "strobe-io",
        "--no-default-features",
        "--all-targets",
    ];
    no_midi.extend(DENY);
    steps.push(Step::new(
        LANE,
        "clippy no-midi",
        if machine.host == Host::Linux {
            cargo(machine, &no_midi)
        } else {
            Action::NotHere("Linux only; CI's Linux job runs it".to_owned())
        },
    ));

    // strobe-script is excluded: rquickjs compiles QuickJS's C sources,
    // which needs MSVC and the Windows SDK. strobe-app depends on it, so
    // cargo would build it anyway (its Windows branches are `cfg!` values,
    // type-checked on every host). strobe-shadertoy is checked by the next
    // step without its `net` feature, whose TLS (ring) compiles C for MSVC
    // (D110). A Windows host checks all of them natively.
    let mut cross = vec![
        "clippy",
        "--locked",
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
    ];
    cross.extend(DENY);
    let mut cross_shadertoy = vec![
        "clippy",
        "--locked",
        "-p",
        "strobe-shadertoy",
        "--no-default-features",
        "--all-targets",
        "--target",
        WINDOWS_TARGET,
    ];
    cross_shadertoy.extend(DENY);
    for (name, args) in [
        ("windows clippy", cross),
        ("windows shadertoy", cross_shadertoy),
    ] {
        let action = match machine.host {
            Host::Windows => Action::NotHere(format!(
                "the host is {WINDOWS_TARGET}, so `clippy` already checked it natively"
            )),
            Host::Linux if machine.windows_target => cargo(machine, &args),
            Host::Linux => Action::Missing {
                why: format!("the {WINDOWS_TARGET} standard library is not installed"),
                fix: format!("rustup target add {WINDOWS_TARGET}"),
            },
            Host::MacOs | Host::Other => {
                Action::NotHere("CI's Linux job runs the Windows cross-check".to_owned())
            }
        };
        steps.push(Step::new(LANE, name, action));
    }
}

/// The test suite, with this host's exclusions and environment.
fn test_step(args: &Args, machine: &Machine) -> Step {
    let mut env: Vec<(&'static str, String)> = Vec::new();
    if machine.host == Host::Linux && !args.allow_gl_skip && !machine.require_gl_set {
        env.push((REQUIRE_GL, "1".to_owned()));
    }
    if (machine.host == Host::Linux || machine.ci) && !machine.lan_set {
        env.push((LAN_TESTS, "1".to_owned()));
    }
    env.push((SKIP_LOG, machine.skip_log.display().to_string()));
    let mut test = vec!["test", "--locked", "--workspace"];
    if machine.host == Host::MacOs {
        test.extend(macos_excludes());
    }
    test.push("--all-features");
    Step::new(
        Lane::Test,
        "test",
        Action::Run {
            program: machine.cargo.clone(),
            args: test.iter().map(|arg| (*arg).to_owned()).collect(),
            env,
        },
    )
    .timeout(45)
}

/// Linux's real-driver checks: the app on a virtual display, the window
/// half of the auto-controls tests, and the scene library in both dialects.
fn render_lane(steps: &mut Vec<Step>, args: &Args, machine: &Machine) {
    const LANE: Lane = Lane::Render;
    if machine.host != Host::Linux {
        steps.push(Step::new(
            LANE,
            "render",
            Action::NotHere("Linux only: Mesa's EGL and Xvfb; CI's Linux job runs it".to_owned()),
        ));
        return;
    }
    // Starts the real app on a virtual X display, renders frames on both
    // windows through one shared GL context, and reads them back to PNGs.
    // A window that never gets its frames waits rather than fails, so this
    // and the next step have timeouts of their own.
    steps.push(
        Step::new(
            LANE,
            "app smoke",
            under_xvfb(
                machine,
                "1920x1080x24",
                &[
                    "run",
                    "--locked",
                    "-p",
                    "strobe-app",
                    "--",
                    "--scene",
                    "01",
                    "--screenshot",
                    "target/ci-shot",
                    "--frames",
                    "10",
                ],
                Vec::new(),
            ),
        )
        .timeout(15),
    );
    steps.push(Step::new(
        LANE,
        "app screenshots",
        Action::Nonempty(SCREENSHOTS.to_vec()),
    ));
    // The CLI scores a scene on an EGL device and on a hidden window, and
    // the two reports must match (D140). The plain test step has no
    // display, so it skips these.
    let mut window_env = vec![("STROBE_REQUIRE_WINDOW_GL", "1".to_owned())];
    if !args.allow_gl_skip {
        window_env.push((REQUIRE_GL, "1".to_owned()));
    }
    steps.push(
        Step::new(
            LANE,
            "auto-controls window",
            under_xvfb(
                machine,
                "1280x720x24",
                &[
                    "test",
                    "--locked",
                    "-p",
                    "strobe-autocontrol",
                    "--test",
                    "window",
                ],
                window_env,
            ),
        )
        .timeout(15),
    );
    // Every scene in the library, headless, once per dialect, frames kept
    // for review. A scene that fails to compile in either dialect fails
    // the step. `release-fast`: the frames are the renderer's, and fat LTO
    // only costs build time here (D504).
    for (name, extra) in [
        ("scenes desktop", &["--out", "target/bench"][..]),
        (
            "scenes gles",
            &["--api", "gles", "--out", "target/bench-gles"][..],
        ),
    ] {
        let mut bench = vec![
            "run",
            "--locked",
            "--profile",
            "release-fast",
            "-p",
            "strobe-bench",
            "--",
            "--size",
            "640x360",
            "--frames",
            "10",
            "--warmup",
            "60",
        ];
        bench.extend(extra);
        steps.push(Step::new(LANE, name, cargo(machine, &bench)).timeout(20));
    }
}

/// The optimised binaries on the shipping platform.
fn build_lane(steps: &mut Vec<Step>, args: &Args, machine: &Machine) {
    const LANE: Lane = Lane::Build;
    if machine.host != Host::Windows {
        steps.push(Step::new(
            LANE,
            "build",
            Action::NotHere(
                "Windows only, the shipping platform; CI's Windows job runs it".to_owned(),
            ),
        ));
        return;
    }
    // Proves the binaries compile and link with `panic = "abort"` and
    // opt-level 3. A pull request builds `release-fast` (no fat LTO); the
    // shipping build runs on master and for a release (D504).
    steps.push(
        Step::new(
            LANE,
            "release-fast",
            cargo(
                machine,
                &[
                    "build",
                    "--locked",
                    "--profile",
                    "release-fast",
                    "--workspace",
                    "--bins",
                ],
            ),
        )
        .timeout(40),
    );
    if args.shipping {
        steps.push(
            Step::new(
                LANE,
                "release",
                cargo(
                    machine,
                    &["build", "--locked", "--release", "--workspace", "--bins"],
                ),
            )
            .timeout(40),
        );
    }
}

/// Runs every step in order, stops at the first failure, and summarises.
fn execute_all(steps: &[Step], strict: bool, machine: &Machine) -> Result<()> {
    let total = steps.len();
    let started = Instant::now();
    let mut results: Vec<(&Step, Outcome, Duration)> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        println!(
            "\n==> [{}/{total}] {} {}: {}",
            index + 1,
            step.lane.label(),
            step.name,
            describe(&step.action)
        );
        let step_started = Instant::now();
        let outcome = match execute(&step.action, step.timeout, strict) {
            Outcome::Failed(why) if step.informational => Outcome::Warned(why),
            other @ (Outcome::Passed
            | Outcome::Failed(_)
            | Outcome::Warned(_)
            | Outcome::NotHere(_)
            | Outcome::Missing(_)) => other,
        };
        let stop = matches!(outcome, Outcome::Failed(_));
        results.push((step, outcome, step_started.elapsed()));
        if stop {
            break;
        }
    }

    let mut summary = format!(
        "==> summary ({:.1}s){}\n",
        started.elapsed().as_secs_f32(),
        if strict { ", strict" } else { "" }
    );
    let mut failure: Option<String> = None;
    let mut missing = 0;
    for (step, outcome, took) in &results {
        let seconds = took.as_secs_f32();
        let name = format!("{} {}", step.lane.label(), step.name);
        let line = match outcome {
            Outcome::Passed => format!("    ok       {name:<28} {seconds:>6.1}s"),
            Outcome::NotHere(why) => format!("    n/a      {name:<28} {why}"),
            Outcome::Missing(why) => {
                missing += 1;
                format!("    MISSING  {name:<28} {why}")
            }
            Outcome::Warned(why) => {
                format!("    warning  {name:<28} {seconds:>6.1}s (informational) {why}")
            }
            Outcome::Failed(why) => {
                failure = Some(format!("{name}: {why}"));
                format!("    FAILED   {name:<28} {seconds:>6.1}s")
            }
        };
        let _ = writeln!(summary, "{line}");
    }
    let not_run = total.saturating_sub(results.len());
    if failure.is_some() && not_run > 0 {
        let _ = writeln!(summary, "    not run  {not_run} step(s) after the failure");
    }
    if let Some(skips) = skips_note(&machine.skip_log) {
        summary.push_str(&skips);
    }
    println!("\n{summary}");
    step_summary(&summary);
    if let Some(why) = failure {
        bail!("CI would fail at `{why}`");
    }
    if missing > 0 {
        println!(
            "PARTIAL: {missing} step(s) could not run here for want of a prerequisite (MISSING above), so this is less than CI checks. `--strict` makes that a failure; `cargo xtask ci doctor` lists the fixes."
        );
    } else {
        println!("All {total} steps passed or do not apply to this host. CI should agree.");
    }
    Ok(())
}

/// Appends the summary to the GitHub Actions job summary, when there is one.
fn step_summary(summary: &str) {
    let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
        return;
    };
    let text = format!("```text\n{summary}```\n");
    // The console has the same text; a summary that cannot be written is not
    // a reason to fail checks that passed.
    let appended = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()));
    if let Err(error) = appended {
        eprintln!("note: could not write the job summary: {error}");
    }
}

/// How many tests skipped and why, from the log the GPU tests wrote
/// (`test\treason` per line), or nothing when none did. A skipped test
/// passed the harness, so this is the only place the difference shows.
fn skips_note(log: &Path) -> Option<String> {
    let text = std::fs::read_to_string(log).ok()?;
    let mut by_reason: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (test, reason) = line.split_once('\t').unwrap_or((line, ""));
        // The error text after the reason's first parenthesis varies by
        // driver; the reason before it is what groups.
        let reason = reason.split(" (").next().unwrap_or(reason).to_owned();
        by_reason.entry(reason).or_default().insert(test.to_owned());
    }
    let tests: BTreeSet<&String> = by_reason.values().flatten().collect();
    if tests.is_empty() {
        return None;
    }
    let mut out = format!(
        "    note     {} test(s) passed by skipping, not by running (see {}):\n",
        tests.len(),
        log.display()
    );
    for (reason, tests) in &by_reason {
        let _ = writeln!(out, "               {:>3} {reason}", tests.len());
    }
    Some(out)
}

/// `cargo xtask ci doctor`: what each step would do here.
fn report_doctor(steps: &[Step], strict: bool) -> Result<()> {
    let mut missing = 0;
    for step in steps {
        let name = format!("{} {}", step.lane.label(), step.name);
        match &step.action {
            Action::NotHere(why) => println!("n/a      {name:<28} {why}"),
            Action::Missing { why, fix } => {
                missing += 1;
                println!("MISSING  {name:<28} {why}: `{fix}`");
            }
            Action::Run { .. }
            | Action::Changelog
            | Action::Brand
            | Action::Credits
            | Action::Nonempty(_)
            | Action::CleanTree(_) => println!("ready    {name:<28} {}", describe(&step.action)),
        }
    }
    if missing == 0 {
        println!("\nEverything that applies to this host can run here.");
        return Ok(());
    }
    println!("\n{missing} step(s) cannot run here until their prerequisite is installed.");
    if strict {
        bail!("{missing} prerequisite(s) missing");
    }
    Ok(())
}

/// The command a step runs, as the line printed above its output.
fn describe(action: &Action) -> String {
    match action {
        Action::Run { program, args, env } => {
            let vars: String = env.iter().map(|(k, v)| format!("{k}={v} ")).collect();
            let program = Path::new(program)
                .file_stem()
                .map_or_else(|| program.clone(), |s| s.to_string_lossy().into_owned());
            format!("{vars}{program} {}", args.join(" "))
        }
        Action::Changelog => "cargo xtask changelog --check".to_owned(),
        Action::Brand => "cargo xtask brand --check".to_owned(),
        Action::Credits => "cargo xtask credits --check".to_owned(),
        Action::Nonempty(files) => format!("non-empty: {}", files.join(", ")),
        Action::CleanTree(_) => "git status is what it was before the run".to_owned(),
        Action::NotHere(why) => format!("does not apply here ({why})"),
        Action::Missing { why, fix } => format!("missing: {why} (`{fix}`)"),
    }
}

/// Runs one step.
fn execute(action: &Action, timeout: Option<Duration>, strict: bool) -> Outcome {
    let checked = |result: Result<()>| match result {
        Ok(()) => Outcome::Passed,
        Err(error) => Outcome::Failed(format!("{error:#}")),
    };
    match action {
        Action::NotHere(why) => Outcome::NotHere(why.clone()),
        Action::Missing { why, fix } if strict => {
            Outcome::Failed(format!("missing prerequisite: {why} (`{fix}`)"))
        }
        Action::Missing { why, fix } => Outcome::Missing(format!("{why} (`{fix}`)")),
        Action::Changelog => checked(changelog::run(true, None)),
        Action::Brand => checked(brand::run(true)),
        Action::Credits => checked(credits::run(true, None)),
        Action::Nonempty(files) => {
            let empty: Vec<&str> = files
                .iter()
                .copied()
                .filter(|file| std::fs::metadata(file).map_or(true, |m| m.len() == 0))
                .collect();
            if empty.is_empty() {
                Outcome::Passed
            } else {
                Outcome::Failed(format!("missing or empty: {}", empty.join(", ")))
            }
        }
        Action::CleanTree(before) => match git_status() {
            Ok(after) if after == *before => Outcome::Passed,
            Ok(after) => Outcome::Failed(format!(
                "the run changed the tree; before:\n{before}\nafter:\n{after}"
            )),
            Err(error) => Outcome::Failed(format!("{error:#}")),
        },
        Action::Run { program, args, env } => {
            let mut command = Command::new(program);
            command.args(args);
            for (key, value) in env {
                command.env(key, value);
            }
            match command.spawn() {
                Ok(child) => wait(child, timeout, program),
                Err(error) => Outcome::Failed(format!("could not start {program}: {error}")),
            }
        }
    }
}

/// Waits for a step's process, killing it at its timeout.
fn wait(mut child: Child, timeout: Option<Duration>, program: &str) -> Outcome {
    const POLL: Duration = Duration::from_millis(200);
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Outcome::Passed,
            Ok(Some(status)) => return Outcome::Failed(format!("{program} exited with {status}")),
            Ok(None) => {}
            Err(error) => return Outcome::Failed(format!("could not wait for {program}: {error}")),
        }
        if let Some(limit) = timeout
            && started.elapsed() > limit
        {
            // Killed because it ran too long; the failure says so whether or
            // not the kill itself worked.
            let _ = child.kill();
            // Reaps it; the outcome is the timeout either way.
            let _ = child.wait();
            return Outcome::Failed(format!(
                "{program} ran past its {} minute timeout and was stopped",
                limit.as_secs() / 60
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// `git status --porcelain`, the tree's state to compare.
fn git_status() -> Result<String> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .output()
        .context("failed to run git")?;
    if !output.status.success() {
        bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether the Windows target's standard library is installed.
fn windows_target_installed() -> bool {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    Command::new(rustc)
        .args(["--print", "target-libdir", "--target", WINDOWS_TARGET])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|dir| Path::new(dir.trim()).is_dir())
}

/// Whether `program args` runs and succeeds, with its output discarded.
fn tool_works(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(lanes: &[Lane]) -> Args {
        Args {
            lanes: lanes.to_vec(),
            strict: false,
            shipping: false,
            allow_gl_skip: false,
            no_deny: false,
        }
    }

    fn machine(host: Host) -> Machine {
        Machine {
            host,
            cargo: "cargo".to_owned(),
            deny: true,
            windows_target: true,
            xvfb: true,
            ci: false,
            require_gl_set: false,
            lan_set: false,
            skip_log: PathBuf::from("target/test-skips.log"),
        }
    }

    fn names(steps: &[Step]) -> Vec<String> {
        steps
            .iter()
            .map(|s| format!("{} {}", s.lane.label(), s.name))
            .collect()
    }

    fn command(step: &Step) -> String {
        describe(&step.action)
    }

    #[test]
    fn all_is_every_ci_lane_in_order() {
        assert_eq!(
            expand(&[]),
            Lane::ALL.to_vec(),
            "no lane named means all of them"
        );
        assert_eq!(
            expand(&[Lane::Test, Lane::All]),
            vec![
                Lane::Test,
                Lane::Fmt,
                Lane::Check,
                Lane::Deny,
                Lane::Generated,
                Lane::Render,
                Lane::Build
            ],
            "a lane named twice runs once, where it was first named"
        );
        assert_eq!(
            expand(&[Lane::Doctor]),
            Lane::ALL.to_vec(),
            "doctor alone looks at every lane"
        );
    }

    #[test]
    fn every_dependency_resolving_cargo_command_is_locked() {
        for host in [Host::Linux, Host::Windows, Host::MacOs] {
            let mut every = args(&[Lane::All, Lane::Conventions]);
            every.shipping = true;
            let steps = plan(&expand(&every.lanes), &every, &machine(host), None);
            for step in &steps {
                let Action::Run { args, .. } = &step.action else {
                    continue;
                };
                let resolving = args
                    .iter()
                    .any(|a| ["build", "clippy", "test", "run"].contains(&a.as_str()));
                assert!(
                    !resolving || args.iter().any(|a| a == "--locked"),
                    "{host:?} {}: {}",
                    step.name,
                    command(step)
                );
            }
        }
    }

    #[test]
    fn each_host_gets_its_ci_jobs_policy() {
        let all = expand(&[]);
        let linux = plan(&all, &args(&[]), &machine(Host::Linux), None);
        let windows = plan(&all, &args(&[]), &machine(Host::Windows), None);
        let macos = plan(&all, &args(&[]), &machine(Host::MacOs), None);
        let runs = |steps: &[Step], name: &str| {
            steps
                .iter()
                .any(|s| s.name == name && matches!(s.action, Action::Run { .. }))
        };
        assert!(runs(&linux, "clippy no-midi"), "Linux checks without MIDI");
        assert!(runs(&linux, "windows clippy"), "Linux cross-checks Windows");
        assert!(runs(&linux, "app smoke"), "Linux renders");
        assert!(!runs(&linux, "release-fast"), "Linux builds no release");
        assert!(
            runs(&windows, "release-fast"),
            "Windows builds release-fast"
        );
        assert!(!runs(&windows, "windows clippy"), "Windows checks natively");
        assert!(!runs(&windows, "app smoke"), "Windows has no Xvfb lane");
        let mac_clippy = macos.iter().find(|s| s.name == "clippy");
        assert!(
            mac_clippy.is_some_and(|s| command(s).contains("--exclude strobe-gfx")),
            "macOS checks the portable crates: {:?}",
            mac_clippy.map(command)
        );
        assert!(
            macos
                .iter()
                .any(|s| s.name == "clippy whole" && s.informational),
            "macOS's whole-workspace clippy is informational"
        );
    }

    #[test]
    fn a_missing_tool_is_missing_and_strict_makes_it_fail() {
        let mut bare = machine(Host::Linux);
        bare.deny = false;
        bare.xvfb = false;
        bare.windows_target = false;
        let steps = plan(&expand(&[]), &args(&[]), &bare, None);
        let missing: Vec<&str> = steps
            .iter()
            .filter(|s| matches!(s.action, Action::Missing { .. }))
            .map(|s| s.name)
            .collect();
        assert_eq!(
            missing,
            vec![
                "windows clippy",
                "windows shadertoy",
                "cargo-deny",
                "app smoke",
                "auto-controls window"
            ],
            "every step whose tool is absent is missing, not skipped"
        );
        let deny = Action::Missing {
            why: "cargo-deny is not installed".to_owned(),
            fix: "cargo install cargo-deny --locked".to_owned(),
        };
        assert!(
            matches!(execute(&deny, None, false), Outcome::Missing(_)),
            "without --strict a missing step is reported"
        );
        assert!(
            matches!(execute(&deny, None, true), Outcome::Failed(_)),
            "with --strict it fails"
        );
        let not_here = Action::NotHere("Linux only".to_owned());
        assert!(
            matches!(execute(&not_here, None, true), Outcome::NotHere(_)),
            "a lane that does not apply is never a failure"
        );
    }

    #[test]
    fn strict_ends_with_the_tree_check_and_shipping_adds_release() {
        let mut ship = args(&[Lane::Build]);
        ship.shipping = true;
        let steps = plan(
            &expand(&ship.lanes),
            &ship,
            &machine(Host::Windows),
            Some(String::new()),
        );
        assert_eq!(
            names(&steps),
            vec!["build release-fast", "build release", "all clean tree"],
            "the shipping build follows release-fast, and strict checks the tree last"
        );
        assert!(
            matches!(
                execute(&Action::CleanTree("not this".to_owned()), None, true),
                Outcome::Failed(_)
            ),
            "a tree that differs fails"
        );
    }

    #[test]
    fn ci_runners_bind_every_interface_and_linux_requires_gl() {
        let mut runner = machine(Host::Windows);
        runner.ci = true;
        let windows = test_step(&args(&[]), &runner);
        let linux = test_step(&args(&[]), &machine(Host::Linux));
        let desk = test_step(&args(&[]), &machine(Host::Windows));
        assert!(
            command(&windows).contains("STROBE_TEST_LAN=1"),
            "{}",
            command(&windows)
        );
        assert!(
            !command(&desk).contains("STROBE_TEST_LAN"),
            "a desk does not (D503)"
        );
        assert!(
            command(&linux).contains("STROBE_REQUIRE_HEADLESS_GL=1"),
            "{}",
            command(&linux)
        );
    }

    #[test]
    fn a_timeout_stops_a_step_that_hangs() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("powershell", &["-NoProfile", "-Command", "Start-Sleep 30"])
        } else {
            ("sleep", &["30"])
        };
        let Ok(child) = Command::new(program).args(args).spawn() else {
            // No such shell here; nothing to measure.
            return;
        };
        let started = Instant::now();
        let outcome = wait(child, Some(Duration::from_millis(300)), program);
        assert!(
            matches!(&outcome, Outcome::Failed(why) if why.contains("timeout")),
            "{outcome:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "it did not wait for the sleep"
        );
    }
}
