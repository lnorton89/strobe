// ============================================================================
// Module:       window (integration test)
// Description:  Runs the strobe-autocontrol binary on a headless context and
//               on a hidden-window context and requires the same report.
//
// Dependencies: anyhow; the strobe-autocontrol binary (spawned)
// ============================================================================

//! The window context for auto-controls (D140).
//!
//! Intel's Windows driver has no EGL devices, so the CLI scores on a hidden
//! winit window there. This test runs the real binary both ways on one
//! scene and requires the printed report (every candidate, its score and
//! its verdict) to be identical, which is what shows the two contexts feed
//! the engine the same pictures.
//!
//! It spawns the binary, and does not call the library from the test
//! thread, for a reason: winit builds an event loop only on the main
//! thread and only once per process, and a test harness gives a test
//! neither.
//!
//! The window run needs a display server. Without one (`DISPLAY` and
//! `WAYLAND_DISPLAY` unset) it reports a skip and passes, unless
//! `STROBE_REQUIRE_WINDOW_GL` is set, which CI's Xvfb step does:
//!
//! ```text
//! STROBE_REQUIRE_HEADLESS_GL=1 STROBE_REQUIRE_WINDOW_GL=1 \
//!     xvfb-run -a cargo test -p strobe-autocontrol --test window
//! ```

use anyhow::{Context as _, Result, bail, ensure};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCENE_JSON: &str = r#"{ "TITLE": "Window probe", "CREDIT": "Strobe" }"#;

/// A scene with literals worth measuring: a time scale, two colours and a
/// coordinate scale.
const SCENE_GLSL: &str = "\
vec4 renderMain(void) {
    vec2 p = _uvc * 2.5;
    float t = TIME * 0.2;
    vec3 col = mix(vec3(0.10, 0.30, 0.50), vec3(0.90, 0.50, 0.20),
                   0.5 + 0.5 * sin(p.x * 3.0 + p.y * 2.0 + t));
    return vec4(col, 1.0);
}
";

fn scratch(tag: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("autocontrol-window-{tag}"))
}

fn write_scene(tag: &str) -> Result<PathBuf> {
    let dir = scratch(tag).join("probe.synScene");
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("scene.json"), SCENE_JSON)?;
    std::fs::write(dir.join("main.glsl"), SCENE_GLSL)?;
    Ok(dir)
}

fn run(scene: &Path, args: &[&str], with_display: bool) -> Result<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_strobe-autocontrol"));
    command
        .arg(scene)
        .args(args)
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LOG");
    if !with_display {
        command
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET");
    }
    command.output().context("running strobe-autocontrol")
}

/// Whether a window can be opened: always, except on a Linux session with
/// neither an X nor a Wayland display.
fn has_display() -> bool {
    !cfg!(target_os = "linux")
        || std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn required(variable: &str) -> bool {
    std::env::var_os(variable).is_some()
}

/// The report as printed, without the line naming the context (the one line
/// that is allowed to differ) or the paths written.
fn report(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.starts_with("context:") && !line.starts_with("wrote "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn window_scores_the_same_as_headless() -> Result<()> {
    let scene = write_scene("same")?;
    let flags = ["--report", "--dry-run"];

    let headless = run(
        &scene,
        &[&["--context", "headless"], &flags[..]].concat(),
        true,
    )?;
    if !headless.status.success() {
        if required("STROBE_REQUIRE_HEADLESS_GL") {
            bail!("headless context required: {}", stderr(&headless));
        }
        strobe_gfx::skip::note(format_args!("no headless context ({})", stderr(&headless)));
        return Ok(());
    }
    if !has_display() && !required("STROBE_REQUIRE_WINDOW_GL") {
        strobe_gfx::skip::note(format_args!("no display server for a window context"));
        return Ok(());
    }
    let window = run(
        &scene,
        &[&["--context", "window"], &flags[..]].concat(),
        true,
    )?;
    ensure!(
        window.status.success(),
        "--context window failed: {}{}",
        String::from_utf8_lossy(&window.stdout),
        stderr(&window)
    );

    let (headless_report, window_report) = (report(&headless), report(&window));
    ensure!(
        headless_report.contains("kept") && !headless_report.contains(" 0 kept"),
        "the probe scene should keep controls:\n{headless_report}"
    );
    ensure!(
        headless_report == window_report,
        "reports differ\n--- headless ---\n{headless_report}\n--- window ---\n{window_report}"
    );
    let context = String::from_utf8_lossy(&window.stdout);
    ensure!(
        context.contains("context: window"),
        "the window run should say where its context came from:\n{context}"
    );
    Ok(())
}

#[test]
fn a_contradiction_between_api_and_gl_is_an_error() -> Result<()> {
    let scene = write_scene("conflict")?;
    for (api, gl) in [("gles", "native"), ("desktop", "angle")] {
        for context in ["auto", "headless", "window"] {
            let output = run(
                &scene,
                &["--api", api, "--gl", gl, "--context", context, "--dry-run"],
                true,
            )?;
            ensure!(
                !output.status.success(),
                "--api {api} --gl {gl} --context {context} should be refused"
            );
            ensure!(
                stderr(&output).contains("cannot run on"),
                "--api {api} --gl {gl}: unexpected error: {}",
                stderr(&output)
            );
        }
    }
    Ok(())
}

#[test]
fn analyze_only_needs_no_context_at_all() -> Result<()> {
    let scene = write_scene("analyze")?;
    let output = run(&scene, &["--analyze-only"], false)?;
    ensure!(
        output.status.success(),
        "--analyze-only should not need a display: {}",
        stderr(&output)
    );
    ensure!(
        String::from_utf8_lossy(&output.stdout).contains("candidates"),
        "--analyze-only should list candidates"
    );
    Ok(())
}

#[test]
fn a_window_without_a_display_says_why() -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    let scene = write_scene("nodisplay")?;
    let output = run(&scene, &["--context", "window", "--dry-run"], false)?;
    ensure!(
        !output.status.success(),
        "a window context cannot open with no display server"
    );
    ensure!(
        stderr(&output).contains("window system"),
        "the error should name the window system: {}",
        stderr(&output)
    );
    Ok(())
}
