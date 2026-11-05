// ============================================================================
// Module:       strobe_app::diagnose
// Description:  `strobe --diagnose`: the system report plus whatever GL (and
//               later audio) facts can be found without opening a window.
//
// Dependencies: strobe-diag (diag, context, paths); strobe-gfx (Headless);
//               sha2, serde_json (the ANGLE pin, scripts/native-deps.json)
// ============================================================================

//! The `--diagnose` report.
//!
//! `strobe-diag` knows the machine and the build; only the app knows the
//! graphics stack and the audio devices, so they are probed here and passed
//! in as the report's "Session" section.
//!
//! GL is probed with a **headless** context (an EGL device display, no
//! window), desktop 3.3 core first and then GLES 3.0. That answers "does
//! this machine have a GL driver at all, and which one", which is the
//! question a bug report needs. It cannot see a WGL-only driver on Windows;
//! there the windowed context's facts are in the session log header and in
//! every crash report instead, and the report says so.
//!
//! The release build has no console on Windows, so the report is also saved
//! as `diagnose.txt` beside the logs and its location is shown in a dialog.
//! The report before it is kept as `diagnose.previous.txt`, so two runs can
//! be compared and a second run never loses the first (R-09).

use crate::dialog;
use sha2::Digest as _;
use std::path::{Path, PathBuf};
use strobe_diag::context::{GL_BACKEND, GL_RENDERER, GL_VENDOR, GL_VERSION};
use strobe_diag::{diag, paths};
use strobe_gfx::offscreen::{self, ContextMode, Request};
use strobe_gfx::{Api, Backend, GlChoice, Headless};

/// The GL facts a headless probe can find, as report rows.
pub fn gl_rows() -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (api, label) in [
        (Api::DesktopCore33, "headless EGL, desktop 3.3 core"),
        (Api::Gles30, "headless EGL, OpenGL ES 3.0"),
    ] {
        match Headless::new(api) {
            Ok(context) => {
                let info = context.gl().info();
                rows.push((GL_BACKEND.to_string(), label.to_string()));
                rows.push((GL_VENDOR.to_string(), info.vendor));
                rows.push((GL_RENDERER.to_string(), info.renderer));
                rows.push((GL_VERSION.to_string(), info.version));
                return rows;
            }
            Err(error) => failures.push(format!("{label}: {error}")),
        }
    }
    rows.push((
        "gl.probe".to_string(),
        format!("no headless context ({})", failures.join("; ")),
    ));
    rows.push((
        "gl.note".to_string(),
        "a windowed driver (WGL) can still work; see the session log".to_string(),
    ));
    rows
}

/// Where the saved report goes.
pub fn report_path() -> PathBuf {
    paths::base_dir().join("diagnose.txt")
}

/// The report before this one, beside it. One is kept, so a second run (a
/// dev build after a release build, or a run after a fix) does not lose the
/// first, and the folder never holds more than two (T480 Task 50, R-09).
fn previous_path(path: &Path) -> PathBuf {
    path.with_file_name("diagnose.previous.txt")
}

/// Saves `text` at `path`, first moving a report already there to
/// [`previous_path`], replacing the one before it.
fn save(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let previous = previous_path(path);
        // `rename` does not replace an existing file on every Windows
        // filesystem, so the older copy goes first. Absent is fine.
        match std::fs::remove_file(&previous) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        std::fs::rename(path, &previous)?;
    }
    std::fs::write(path, text)
}

/// Builds the report: system, build and environment, then the GL probe and
/// the ANGLE DLLs beside the program.
pub fn report() -> String {
    let mut rows = gl_rows();
    if cfg!(windows)
        && let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        rows.push(("angle".to_string(), angle_status(&dir)));
        rows.push(("angle.loaded".to_string(), angle_loaded()));
    }
    diag::system_report(&rows)
}

/// Which ANGLE actually starts, from the version string of a headless
/// GLES context: ANGLE names itself and its build there (`OpenGL ES 3.0.0
/// (ANGLE 2.1.x git hash: …)`). The DLLs beside the program can match the
/// pin and still not be what loads, if a copy elsewhere on the search path
/// wins, so this is the other half of the `angle` row (D544).
///
/// ANGLE has no headless EGL device on Windows, so the probe is one hidden
/// window with ANGLE alone, as the corpus harness opens one (D195).
fn angle_loaded() -> String {
    let request = Request {
        mode: ContextMode::Window,
        api: Some(Api::Gles30),
        gl: GlChoice::Only(Backend::Angle),
        tiny: false,
        window_size: None,
    };
    match offscreen::run(&request, |_| {}, |gl, _, _| gl.info()) {
        Ok(info) if info.version.contains("ANGLE") => {
            format!("{} on {}", info.version, info.renderer)
        }
        Ok(info) => format!(
            "not ANGLE: the GLES context is {} ({})",
            info.version, info.renderer
        ),
        Err(error) => format!("ANGLE did not start ({error})"),
    }
}

/// The native dependency pin, as the build and the release read it (D536).
const NATIVE_DEPS: &str = include_str!("../../../scripts/native-deps.json");

/// Which ANGLE is in `dir`, judged by its files' SHA-256 against the pin:
/// the pinned version, verified; absent (native GL only); or which file is
/// not the pinned one. A DLL's name says nothing about its version, and a
/// stale or swapped ANGLE is the first thing to rule out when the fallback
/// misbehaves.
pub fn angle_status(dir: &Path) -> String {
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(NATIVE_DEPS) else {
        return "the built-in pin did not parse".to_string();
    };
    let angle = &manifest["angle"];
    let version = angle["version"].as_str().unwrap_or("?");
    let files: Vec<(&str, &str)> = angle["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|file| Some((file["name"].as_str()?, file["sha256"].as_str()?)))
        .collect();
    let mut missing = Vec::new();
    let mut wrong = Vec::new();
    for (name, pinned) in &files {
        match std::fs::read(dir.join(name)) {
            Ok(bytes) => {
                let digest = sha2::Sha256::digest(&bytes);
                let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
                if hex != *pinned {
                    wrong.push(format!("{name} ({})", hex.get(..12).unwrap_or(&hex)));
                }
            }
            Err(_) => missing.push(*name),
        }
    }
    if missing.len() == files.len() {
        return "not beside the program: native GL only".to_string();
    }
    if missing.is_empty() && wrong.is_empty() {
        return format!(
            "{version} (Electron), all {} files match the pin",
            files.len()
        );
    }
    let mut parts = Vec::new();
    if !wrong.is_empty() {
        parts.push(format!("not the pinned {version}: {}", wrong.join(", ")));
    }
    if !missing.is_empty() {
        parts.push(format!("missing: {}", missing.join(", ")));
    }
    parts.join("; ")
}

/// Prints the report, saves it, and (in a release build, which has no
/// console on Windows) shows where it was saved, in a box that does not
/// hold the process open (D499).
pub fn run() {
    let text = report();
    println!("{text}");
    let path = report_path();
    match save(&path, &text) {
        Ok(()) => println!(
            "Saved to {} (the run before is {})",
            path.display(),
            previous_path(&path).display()
        ),
        Err(error) => eprintln!("Could not save {}: {error}", path.display()),
    }
    if !cfg!(debug_assertions) {
        dialog::show_notice(&format!("Diagnostic report saved to\n{}", path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_probe_always_says_something_about_gl() -> Result<()> {
        let rows = gl_rows();
        assert!(
            rows.iter()
                .any(|(key, _)| key == GL_RENDERER || key == "gl.probe"),
            "either a renderer or the reason there is none: {rows:?}"
        );
        Ok(())
    }

    #[test]
    fn angle_is_judged_by_its_bytes_not_its_name() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-angle-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        assert_eq!(
            angle_status(&dir),
            "not beside the program: native GL only",
            "an empty folder has no ANGLE"
        );
        std::fs::write(dir.join("libGLESv2.dll"), b"not the pinned build")?;
        let stale = angle_status(&dir);
        // Scratch space; a leftover is harmless.
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            stale.contains("not the pinned ")
                && stale.contains(": libGLESv2.dll")
                && stale.contains("missing: "),
            "a file with the right name and the wrong bytes is named: {stale}"
        );
        Ok(())
    }

    #[test]
    fn the_report_is_saved_beside_the_logs() -> Result<()> {
        assert_eq!(
            report_path().parent(),
            Some(paths::base_dir().as_path()),
            "diagnose.txt lives in the base directory"
        );
        Ok(())
    }

    #[test]
    fn a_second_report_keeps_the_first_as_the_previous_one() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-diagnose-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("diagnose.txt");
        save(&path, "first")?;
        let first_alone = !previous_path(&path).exists();
        save(&path, "second")?;
        save(&path, "third")?;
        let current = std::fs::read_to_string(&path)?;
        let previous = std::fs::read_to_string(previous_path(&path))?;
        let files = std::fs::read_dir(&dir)?.count();
        // Scratch space; a leftover is harmless.
        let _ = std::fs::remove_dir_all(&dir);
        assert!(first_alone, "the first report has no previous one");
        assert_eq!(current, "third", "diagnose.txt is the newest report");
        assert_eq!(previous, "second", "the previous file is the run before");
        assert_eq!(files, 2, "only one older report is kept");
        Ok(())
    }
}
