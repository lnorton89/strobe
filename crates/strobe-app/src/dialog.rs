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
/// that would wait for a click. Call it before
/// `strobe_diag::crash::start_monitor`: the crash monitor is a second
/// process, and it learns this when it is started.
pub fn set_unattended() {
    UNATTENDED.store(true, Ordering::Relaxed);
    strobe_diag::notify::set_unattended();
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
    if UNATTENDED.load(Ordering::Relaxed) || !cfg!(target_os = "windows") {
        log::error!("{text}");
        return;
    }
    match message_box(text, Icon::Error).and_then(|mut child| child.wait()) {
        Ok(_) => {}
        Err(error) => log::error!("could not show the error dialog: {error}"),
    }
}

/// Shows `text` in an information box titled "Strobe" and returns at once,
/// leaving the box up after this process exits. For news that is not a
/// failure and needs no answer, such as where `--diagnose` saved its report:
/// waiting there looked like a hung process to anyone who could not see the
/// box, because it belongs to PowerShell, not to `strobe.exe` (T480 Task
/// 50). Only logs it where [`show_error`] would.
pub fn show_notice(text: &str) {
    if UNATTENDED.load(Ordering::Relaxed) || !cfg!(target_os = "windows") {
        log::info!("{text}");
        return;
    }
    // Not waited on, on purpose: the box outlives this process, and the OS
    // reaps PowerShell when it is dismissed.
    if let Err(error) = message_box(text, Icon::Information) {
        log::warn!("could not show the notice: {error}");
    }
}

/// The icon a box shows, as `System.Windows.MessageBoxImage` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    /// A failure.
    Error,
    /// News that is not a failure.
    Information,
}

/// The PowerShell that raises a box with `icon` showing `$env:STROBE_ERROR`.
/// A service's session has no desktop anyone sees, so there it exits at once
/// rather than raise a box that cannot be dismissed.
fn box_script(icon: Icon) -> String {
    let icon = match icon {
        Icon::Error => "Error",
        Icon::Information => "Information",
    };
    format!(
        "if (-not [Environment]::UserInteractive) {{ exit 0 }}; \
         Add-Type -AssemblyName PresentationFramework; \
         [void][System.Windows.MessageBox]::Show($env:STROBE_ERROR, 'Strobe', 'OK', '{icon}')"
    )
}

/// Starts the hidden PowerShell that raises the box.
fn message_box(text: &str, icon: Icon) -> std::io::Result<std::process::Child> {
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command"])
        .arg(box_script(icon))
        .env("STROBE_ERROR", text)
        .spawn()
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

    #[test]
    fn a_notice_is_information_and_a_failure_is_an_error() {
        let notice = box_script(Icon::Information);
        let error = box_script(Icon::Error);
        assert!(
            notice.ends_with("'OK', 'Information')"),
            "a notice is not styled as a failure:\n{notice}"
        );
        assert!(
            error.ends_with("'OK', 'Error')"),
            "a failure keeps its error icon:\n{error}"
        );
        for script in [&notice, &error] {
            assert!(
                script.starts_with("if (-not [Environment]::UserInteractive) { exit 0 };"),
                "no box in a session nobody sees (M2):\n{script}"
            );
        }
    }
}
