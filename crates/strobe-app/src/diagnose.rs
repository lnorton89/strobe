// ============================================================================
// Module:       strobe_app::diagnose
// Description:  `strobe --diagnose`: the system report plus whatever GL (and
//               later audio) facts can be found without opening a window.
//
// Dependencies: strobe-diag (diag, context, paths); strobe-gfx (Headless)
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

use crate::dialog;
use std::path::PathBuf;
use strobe_diag::context::{GL_BACKEND, GL_RENDERER, GL_VENDOR, GL_VERSION};
use strobe_diag::{diag, paths};
use strobe_gfx::{Api, Headless};

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

/// Builds the report: system, build and environment, then the GL probe.
pub fn report() -> String {
    diag::system_report(&gl_rows())
}

/// Prints the report, saves it, and (in a release build, which has no
/// console on Windows) shows where it was saved, in a box that does not
/// hold the process open (D499).
pub fn run() {
    let text = report();
    println!("{text}");
    let path = report_path();
    let saved = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, &text));
    match saved {
        Ok(()) => println!("Saved to {}", path.display()),
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
    fn the_report_is_saved_beside_the_logs() -> Result<()> {
        assert_eq!(
            report_path().parent(),
            Some(paths::base_dir().as_path()),
            "diagnose.txt lives in the base directory"
        );
        Ok(())
    }
}
