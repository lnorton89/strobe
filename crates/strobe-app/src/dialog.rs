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

use std::fmt::Write as _;
use std::path::Path;
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
/// dismissed. Does nothing but log off Windows.
pub fn show_error(text: &str) {
    if cfg!(target_os = "windows") {
        let script = "Add-Type -AssemblyName PresentationFramework; \
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
