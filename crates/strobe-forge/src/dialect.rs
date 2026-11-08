// ============================================================================
// Module:       strobe_forge::dialect
// Description:  Compiles a scene in the other GLSL dialect (GLSL ES 3.00 when
//               the check ran on desktop GL, and back) in a child process.
//
// Dependencies: anyhow, serde, serde_json; strobe-gfx (offscreen contexts,
//               player), strobe-scene (folder, dialect)
// ============================================================================

//! The second dialect.
//!
//! A scene must compile both as desktop GLSL 330 and as GLSL ES 3.00, the
//! dialect ANGLE runs (the skill's step 5). The check's own context is one
//! of the two, so the other needs a second context, and on Windows that is
//! a second window: winit allows one event loop per process
//! ([`strobe_gfx::offscreen`]). So the check runs this binary again with
//! the hidden `dialect` subcommand, which opens the other context (headless
//! when the check ran headless, an ANGLE or native window otherwise),
//! compiles, and prints one JSON line. A machine with no such context (no
//! ANGLE beside the binary, no headless GLES) gives "not checked", never a
//! failure.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use strobe_gfx::offscreen::{self, ContextMode, Request};
use strobe_gfx::{Api, Backend, Caps, GlChoice, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::assemble::Dialect;

/// How a dialect's compile went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    /// It compiled and linked.
    Ok,
    /// It did not.
    Failed,
    /// No context for it here.
    NotChecked,
}

impl Status {
    /// For the report.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ok => "compiles",
            Self::Failed => "FAILS",
            Self::NotChecked => "not checked",
        }
    }
}

/// One dialect's result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DialectResult {
    /// `desktop GLSL 330` or `GLSL ES 3.00`.
    pub(crate) dialect: String,
    /// How it went.
    pub(crate) status: Status,
    /// The renderer that compiled it.
    pub(crate) renderer: Option<String>,
    /// The compile log, or why it was not checked.
    pub(crate) error: Option<String>,
}

impl DialectResult {
    /// It compiled on `renderer`.
    pub(crate) fn ok(dialect: &str, renderer: &str) -> Self {
        Self {
            dialect: dialect.to_owned(),
            status: Status::Ok,
            renderer: Some(renderer.to_owned()),
            error: None,
        }
    }

    /// It failed on `renderer` with `error`.
    pub(crate) fn failed(dialect: &str, renderer: &str, error: String) -> Self {
        Self {
            dialect: dialect.to_owned(),
            status: Status::Failed,
            renderer: Some(renderer.to_owned()),
            error: Some(error),
        }
    }

    /// No context for it, because `why`.
    pub(crate) fn not_checked(dialect: &str, why: &str) -> Self {
        Self {
            dialect: dialect.to_owned(),
            status: Status::NotChecked,
            renderer: None,
            error: Some(why.to_owned()),
        }
    }
}

const DESKTOP: &str = "desktop GLSL 330";
const ES: &str = "GLSL ES 3.00";

/// Whether `dialect` is the GLSL ES one.
fn is_es(dialect: Dialect) -> bool {
    match dialect {
        Dialect::Es300 => true,
        Dialect::Glsl330 | Dialect::Glsl410 => false,
    }
}

/// The report's name for `dialect`.
pub(crate) fn name(dialect: Dialect) -> &'static str {
    if is_es(dialect) { ES } else { DESKTOP }
}

/// The report's name for the dialect `dialect` is not.
pub(crate) fn other_name(dialect: Dialect) -> &'static str {
    if is_es(dialect) { DESKTOP } else { ES }
}

/// Compiles `dir` in the dialect other than `here`, in a child process of
/// this binary. `context` is the check's own context line: a headless one
/// asks for a headless child.
pub(crate) fn check_other(dir: &Path, here: Dialect, context: &str) -> DialectResult {
    let dialect = other_name(here);
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            return DialectResult::not_checked(dialect, &format!("no executable path: {error}"));
        }
    };
    let mode = if context.starts_with("headless") {
        "headless"
    } else {
        "window"
    };
    let api = if is_es(here) { "desktop" } else { "gles" };
    let output = run_with_deadline(
        std::process::Command::new(exe)
            .arg("dialect")
            .arg(dir)
            .args(["--api", api, "--context", mode]),
    );
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return DialectResult::not_checked(
                dialect,
                &format!("could not run the child: {error}"),
            );
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .rev()
        .find(|line| line.starts_with('{'))
        .and_then(|line| serde_json::from_str::<DialectResult>(line).ok())
        .unwrap_or_else(|| {
            DialectResult::not_checked(
                dialect,
                &format!(
                    "the child said nothing usable ({}): {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            )
        })
}

/// How long the child may take. ANGLE on Direct3D 11 can spend minutes
/// on a heavy shader (backlog G-08); past this it is "not checked", not a
/// hang.
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(240);

/// Runs `command` with its output captured, killing it past [`DEADLINE`].
fn run_with_deadline(command: &mut std::process::Command) -> Result<std::process::Output, String> {
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map_err(|e| e.to_string()),
            Ok(None) if start.elapsed() > DEADLINE => {
                // Best effort: a child that cannot be killed or reaped has already
                // gone, and the answer is "not checked" either way.
                let _ = child.kill();
                let _ = child.wait(); // Reaps it; as above.
                return Err(format!("timed out after {} s", DEADLINE.as_secs()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// The `dialect` subcommand: compile `dir` on a context of `api` and print
/// the result as one JSON line. Main thread only (it may open a window).
pub(crate) fn child(dir: &Path, gles: bool, headless: bool) -> Result<()> {
    let (api, backend, dialect) = if gles {
        (Api::Gles30, Backend::Angle, ES)
    } else {
        (Api::DesktopCore33, Backend::Native, DESKTOP)
    };
    let request = Request {
        mode: if headless {
            ContextMode::Headless
        } else {
            ContextMode::Window
        },
        api: Some(api),
        gl: GlChoice::Only(backend),
        tiny: false,
        window_size: None,
    };
    let result = match SceneFolder::load(dir) {
        Err(error) => DialectResult::failed(dialect, "none", error.to_string()),
        Ok(scene) => offscreen::run(
            &request,
            |_| {},
            |gl, caps: &Caps, _context| {
                gl.clear_errors();
                let extras = strobe_autocontrol::script::uniforms(&scene);
                match ScenePlayer::with_extras(
                    gl,
                    caps,
                    &scene,
                    [64, 36],
                    Options::default(),
                    &extras,
                ) {
                    Ok(_) => DialectResult::ok(dialect, &caps.info.renderer),
                    Err(error) => {
                        DialectResult::failed(dialect, &caps.info.renderer, error.to_string())
                    }
                }
            },
        )
        .unwrap_or_else(|error| DialectResult::not_checked(dialect, &error.to_string())),
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
