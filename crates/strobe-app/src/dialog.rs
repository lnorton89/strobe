// ============================================================================
// Module:       strobe_app::dialog
// Description:  Telling a person about a failure when the release build has
//               no console: a message box, and the text of a crash notice.
//
// Dependencies: strobe-diag (CrashNotice); std::process (PowerShell)
// ============================================================================

//! Failure dialogs.
//!
//! The release build is a Windows GUI application with no console, so a
//! failure that only reaches stderr is invisible. Every message a person
//! must see goes through [`show_error`]: the startup-failure path, and the
//! notifier that `strobe-diag` calls after a panic or a native crash.
//!
//! The box is raised through PowerShell rather than a Win32 call, because
//! `unsafe` is confined to the GL boundary (and one call in `strobe-diag`).
//! On other platforms there is a terminal, so the message is only logged.
//!
//! A modal box nobody can see blocks the process for ever. On the projector
//! mini-PC (M2), Strobe launched from a Windows service ran in session 0,
//! whose desktop is never shown: a failed start raised a box there and the
//! process never exited. So the box is skipped, and the text only logged,
//! when the run is unattended (`--screenshot`, [`set_unattended`]) or when
//! the process is not in an interactive session (checked by PowerShell's
//! `[Environment]::UserInteractive`, which is false in a service).

use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set when nobody is watching this run, so no box is raised.
static UNATTENDED: AtomicBool = AtomicBool::new(false);

/// Marks this run as unattended: failures are logged, never shown in a box
/// that would wait for a click.
pub fn set_unattended() {
    UNATTENDED.store(true, Ordering::Relaxed);
}
use strobe_diag::notify::{CrashKind, CrashNotice};

/// The words of a crash dialog: what happened, then where the evidence is.
pub fn crash_text(notice: &CrashNotice) -> String {
    let what = match notice.kind {
        CrashKind::Panic => "Strobe hit an internal error and has stopped.",
        CrashKind::Native => "Strobe crashed (most often a graphics driver fault).",
    };
    let mut text = format!("{what}\n\n{}", notice.summary);
    let mut file = |label: &str, path: Option<&Path>| {
        if let Some(path) = path {
            let _ = write!(text, "\n{label}: {}", path.display());
        }
    };
    file("Report", notice.report.as_deref());
    file("Minidump", notice.dump.as_deref());
    text.push_str("\n\nPlease attach these files when you report the problem.");
    text
}

/// Shows `text` in a modal error box titled "Strobe" and waits for it to be
/// dismissed. Only logs it off Windows, in an unattended run, and in a
/// session with no visible desktop.
pub fn show_error(text: &str) {
    if UNATTENDED.load(Ordering::Relaxed) {
        log::error!("{text}");
        return;
    }
    if cfg!(target_os = "windows") {
        // A service's session has no desktop anyone sees: exit at once
        // rather than wait on a box that cannot be dismissed.
        let script = "if (-not [Environment]::UserInteractive) { exit 0 }; \
                      Add-Type -AssemblyName PresentationFramework; \
                      [void][System.Windows.MessageBox]::Show($env:STROBE_ERROR, 'Strobe', 'OK', 'Error')";
        let shown = std::process::Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", script])
            .env("STROBE_ERROR", text)
            .status();
        if let Err(error) = shown {
            log::error!("could not show the error dialog: {error}");
        }
    } else {
        log::error!("{text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::path::PathBuf;

    #[test]
    fn a_crash_notice_names_what_happened_and_where_the_files_are() -> Result<()> {
        let text = crash_text(&CrashNotice {
            kind: CrashKind::Native,
            summary: "access violation".to_string(),
            report: Some(PathBuf::from("crashes/crash-1.txt")),
            dump: Some(PathBuf::from("crashes/crash-1.dmp")),
        });
        for needle in [
            "driver",
            "access violation",
            "crash-1.txt",
            "crash-1.dmp",
            "attach",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        Ok(())
    }

    #[test]
    fn a_panic_without_a_dump_omits_the_dump_line() -> Result<()> {
        let text = crash_text(&CrashNotice {
            kind: CrashKind::Panic,
            summary: "boom".to_string(),
            report: None,
            dump: None,
        });
        assert!(!text.contains("Minidump"), "no dump line expected:\n{text}");
        assert!(text.contains("internal error"), "panic wording:\n{text}");
        Ok(())
    }
}
