// ============================================================================
// Module:       xtask::rc
// Description:  `cargo xtask rc`: the release-candidate harness for a real
//               Windows machine. Checks the exact zip, runs it on each GL
//               backend, crashes it on purpose and symbolicates the dump
//               against the archived PDB, and writes evidence as JSON.
//
// Dependencies: anyhow, clap, serde_json; crate::{native, package,
//               symbolicate}
// ============================================================================

//! The release-candidate run, on the machine the show runs on (D537).
//!
//! A GitHub runner has no GPU and no interactive session, so CI can prove
//! the package is complete but never that it draws. This harness is for the
//! T480 (or any real Windows machine), run by a person, against the exact
//! zip a release would publish:
//!
//! 1. everything `cargo xtask package-check` checks, on that zip and its
//!    symbols zip;
//! 2. each backend (`native` and `angle-d3d11` by default) draws a scene for
//!    `--frames` frames in a window and writes both screenshots;
//! 3. an injected crash (`STROBE_FAULT=crash@120`, a stack overflow, R-04)
//!    leaves a minidump, and `cargo xtask symbolicate` turns it into a stack
//!    that names Strobe's own functions, using the PDB from the symbols zip;
//! 4. optionally a soak: `--soak-frames` more frames on the native backend.
//!
//! `evidence.json` records the archive's SHA-256, the `--diagnose` report
//! (build, GPU, driver), each step's settings and result, and the checks
//! that stay manual (audio recovery, fullscreen changes, a playlist), so the
//! STATUS_AUDIT entry can cite one file. It never runs on a pull request:
//! an untrusted change does not get a personal or venue machine.

use crate::package::{self, CheckArgs};
use crate::{native, symbolicate};
use anyhow::{Context as _, Result, bail};
use clap::Args as ClapArgs;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The checks this harness cannot make, listed in the evidence so a reader
/// knows they were not made.
const MANUAL: [&str; 4] = [
    "a representative playlist plays through its transitions",
    "unplugging and replugging the audio device recovers analysis",
    "switching the output window to fullscreen and back keeps drawing",
    "STROBE_FAULT=lose-context@60 rebuilds the context in place",
];

/// Options for `cargo xtask rc`.
#[derive(ClapArgs, Debug)]
pub(crate) struct Args {
    /// The release zip to test, exactly as it would be published.
    #[arg(long)]
    zip: PathBuf,
    /// Its symbols zip.
    #[arg(long)]
    symbols: PathBuf,
    /// Where the screenshots, logs, dump, stack and evidence go.
    #[arg(long, default_value = "target/rc")]
    out: PathBuf,
    /// Frames each backend draws.
    #[arg(long, default_value_t = 600)]
    frames: u32,
    /// The backends to run (`--gl` values). Repeatable.
    #[arg(long = "backend", default_values_t = ["native".to_owned(), "angle-d3d11".to_owned()])]
    backends: Vec<String>,
    /// Frames of a soak on the native backend after the rest; 0 for none.
    /// 108000 is thirty minutes at 60 fps.
    #[arg(long, default_value_t = 0)]
    soak_frames: u32,
    /// Skip the injected crash and its symbolication (no dump_syms or
    /// minidump-stackwalk here).
    #[arg(long)]
    no_crash: bool,
}

impl Args {
    /// Paths a person typed, taken from where they typed them.
    pub(crate) fn anchored(self, invoked_from: &Path) -> Self {
        Self {
            zip: invoked_from.join(self.zip),
            symbols: invoked_from.join(self.symbols),
            out: invoked_from.join(self.out),
            ..self
        }
    }
}

/// Runs the harness and writes `evidence.json`.
pub(crate) fn run(args: &Args) -> Result<()> {
    if !cfg!(windows) {
        bail!(
            "the release candidate is a Windows package; run this on the Windows machine under test"
        );
    }
    // Each run's evidence starts empty; absent is fine.
    let _ = std::fs::remove_dir_all(&args.out);
    std::fs::create_dir_all(&args.out)?;
    let mut failures = 0;

    println!("==> checking the package");
    let work = package::scratch("rc");
    let checked = package::check(
        &CheckArgs::new(args.zip.clone(), Some(args.symbols.clone())),
        &work,
    )?;
    failures += checked.problems.len();
    for line in &checked.problems {
        println!("PROBLEM  {line}");
    }
    let exe = checked.root.join("strobe.exe");

    let mut backends = Vec::new();
    for backend in &args.backends {
        println!("==> {backend}: {} frames", args.frames);
        let result = draw(&exe, &args.out.join(backend), backend, args.frames, None)?;
        failures += usize::from(result["ok"] != json!(true));
        backends.push(result);
    }

    let soak = if args.soak_frames > 0 {
        println!("==> soak: {} frames on native", args.soak_frames);
        let result = draw(
            &exe,
            &args.out.join("soak"),
            "native",
            args.soak_frames,
            None,
        )?;
        failures += usize::from(result["ok"] != json!(true));
        result
    } else {
        json!({ "run": false })
    };

    let crash = if args.no_crash {
        json!({ "run": false })
    } else {
        println!("==> an injected crash, symbolicated");
        let pdb = find_pdb(&work)?;
        let result = crash(&exe, &args.out.join("crash"), &pdb)?;
        failures += usize::from(result["ok"] != json!(true));
        result
    };

    let evidence = json!({
        "tool": format!("cargo xtask rc {}", env!("CARGO_PKG_VERSION")),
        "recorded_unix": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "archive": { "path": args.zip.display().to_string(), "sha256": native::sha256(&args.zip)? },
        "symbols": { "path": args.symbols.display().to_string(), "sha256": native::sha256(&args.symbols)? },
        "diagnose": checked.diagnose,
        "package": { "passed": checked.passed, "problems": checked.problems },
        "settings": { "frames": args.frames, "soak_frames": args.soak_frames, "scene": "01" },
        "backends": backends,
        "soak": soak,
        "crash": crash,
        "manual": MANUAL,
        "verdict": if failures == 0 { "pass" } else { "fail" },
    });
    let path = args.out.join("evidence.json");
    std::fs::write(&path, serde_json::to_string_pretty(&evidence)?)
        .with_context(|| format!("cannot write {}", path.display()))?;
    println!("\nwrote {}", path.display());
    println!("still to check by hand:");
    for item in MANUAL {
        println!("  - {item}");
    }
    if failures > 0 {
        bail!("{failures} failure(s); see {}", path.display());
    }
    println!("The release candidate passed everything this harness checks.");
    Ok(())
}

/// Runs the program on `backend` for `frames` frames with screenshots, its
/// logs kept in `dir`, and an optional fault. Returns the step's evidence.
fn draw(exe: &Path, dir: &Path, backend: &str, frames: u32, fault: Option<&str>) -> Result<Value> {
    std::fs::create_dir_all(dir)?;
    let shot = dir.join("shot");
    let mut command = Command::new(exe);
    command
        .args([
            "--gl",
            backend,
            "--scene",
            "01",
            "--frames",
            &frames.to_string(),
        ])
        .arg("--screenshot")
        .arg(&shot)
        .current_dir(exe.parent().context("the program has no folder")?)
        .env("STROBE_DIAG_DIR", dir.join("diag"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(fault) = fault {
        command.env("STROBE_FAULT", fault);
    }
    let started = Instant::now();
    // A generous ceiling: ten frames a second is far below any GPU that
    // can run a show.
    let limit = Duration::from_secs(120 + u64::from(frames) / 10);
    let status = package::wait(command.spawn()?, limit);
    let seconds = started.elapsed().as_secs_f64();
    let shots: Vec<bool> = ["control", "output"]
        .iter()
        .map(|which| {
            std::fs::metadata(dir.join(format!("shot-{which}.png"))).is_ok_and(|m| m.len() > 0)
        })
        .collect();
    let exited = match &status {
        Ok(status) => status.to_string(),
        Err(error) => format!("{error:#}"),
    };
    let clean = status.as_ref().is_ok_and(std::process::ExitStatus::success);
    Ok(json!({
        "backend": backend,
        "frames": frames,
        "fault": fault,
        "seconds": seconds,
        "exit": exited,
        "screenshots": shots.iter().all(|s| *s),
        "renderer": renderer(&dir.join("diag").join("logs")),
        "ok": clean && shots.iter().all(|s| *s),
    }))
}

/// The backend, GL renderer and version the run drew with, from the newest
/// session log: the app logs `GL: <backend> | <renderer> | <version>` once
/// its context is up. On the first T480 run (D542) this looked for the word
/// "renderer", which no log line contains, and recorded nothing.
fn renderer(logs: &Path) -> Option<String> {
    let newest = std::fs::read_dir(logs)
        .ok()?
        .filter_map(Result::ok)
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?;
    let text = std::fs::read_to_string(newest.path()).ok()?;
    gl_line(&text)
}

/// [`renderer`]'s search: the text after the app's `GL: ` line, or after
/// the window code's `GL backend ` line when the app's is missing.
fn gl_line(log: &str) -> Option<String> {
    let after = |marker: &str| {
        log.lines().find_map(|line| {
            line.split_once(marker)
                .map(|(_, rest)| rest.trim().to_owned())
        })
    };
    after(" strobe_app::app: GL: ")
        .or_else(|| after(" strobe_gfx::gl::window: GL backend "))
        .map(|line| line.chars().take(240).collect())
}

/// The PDB from the unpacked symbols zip, beside the package's work folder.
fn find_pdb(work: &Path) -> Result<PathBuf> {
    let symbols = PathBuf::from(format!("{}-symbols", work.display()));
    let mut pending = vec![symbols];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder)?.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if entry.file_name() == "strobe.pdb" {
                return Ok(path);
            }
        }
    }
    bail!("no strobe.pdb in the unpacked symbols")
}

/// Crashes the program on purpose and symbolicates its dump against `pdb`.
fn crash(exe: &Path, dir: &Path, pdb: &Path) -> Result<Value> {
    let ran = draw(exe, dir, "auto", 600, Some("crash@120"))?;
    let crashes = dir.join("diag").join("crashes");
    let dump = std::fs::read_dir(&crashes)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "dmp"));
    let Some(dump) = dump else {
        return Ok(
            json!({ "run": true, "ok": false, "draw": ran, "why": "no minidump was written" }),
        );
    };
    let stack = dir.join("stack.txt");
    let symbolicated = symbolicate::run(&symbolicate::Args::new(
        dump.clone(),
        vec![pdb.to_path_buf()],
        Some(stack.clone()),
    ));
    let text = std::fs::read_to_string(&stack).unwrap_or_default();
    let named = text.contains("strobe_app") || text.contains("strobe::");
    Ok(json!({
        "run": true,
        "draw": ran,
        "dump": dump.display().to_string(),
        "stack": stack.display().to_string(),
        "symbolicated": symbolicated.as_ref().map_or_else(|e| format!("{e:#}"), |()| "ok".to_owned()),
        "names_strobe_functions": named,
        "ok": symbolicated.is_ok() && named,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_renderer_comes_from_the_apps_gl_line() {
        // Lines from the T480's first run (D542).
        let window = "0.790s INFO  [main] strobe_gfx::gl::window: GL backend native (OpenGL 3.3 core): Intel(R) UHD Graphics 620 | 3.3.0 - Build 30.0.101.1122";
        let app = "0.923s INFO  [main] strobe_app::app: GL: native (OpenGL 3.3 core) | Intel(R) UHD Graphics 620 | 3.3.0 - Build 30.0.101.1122";
        assert_eq!(
            gl_line(&format!("{window}\n{app}\n")).as_deref(),
            Some(
                "native (OpenGL 3.3 core) | Intel(R) UHD Graphics 620 | 3.3.0 - Build 30.0.101.1122"
            ),
            "the app's line, without its prefix"
        );
        assert!(
            gl_line(window).is_some_and(|line| line.contains("UHD Graphics 620")),
            "the window line when the app's is missing"
        );
        assert_eq!(gl_line("nothing here"), None, "no GL line, no renderer");
    }
}
