// ============================================================================
// Module:       crash_demo (example)
// Description:  Installs the whole diagnostics pipeline and then fails on
//               purpose (panic, abort, stack overflow) so it can be tested.
//
// Dependencies: strobe-diag
// ============================================================================

//! Crash demo: `cargo run --release --example crash_demo -- <mode>`.
//!
//! Modes:
//!
//! - `panic`: an out-of-bounds index. The panic hook writes `crash-*.txt`.
//! - `abort`: `std::process::abort()`. The monitor writes `crash-*.dmp`
//!   and `crash-*.txt`.
//! - `overflow`: unbounded safe recursion. Same files as `abort`.
//! - `panic-abort-profile`: same as `panic`; run it from a build with
//!   `panic = "abort"` (release) to see the panic report and the minidump
//!   of the abort that follows share one file stem.
//! - `ok`: exits normally, leaving only a session log.
//!
//! Output goes to `STROBE_DIAG_DIR` if set, else the per-user data
//! directory. Set `STROBE_LOG=debug` for more log.
//!
//! `STROBE_DEMO_NOTIFY_HOLD_MS=<ms>` makes the notifier wait that long
//! before it returns, as a message box does until someone clicks it. The
//! crash pipeline test uses it to check that a crashed app is not kept
//! alive while the monitor's notice is up.

use std::time::Duration;
use strobe_diag::context;
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_diag::notify::CrashKind;

/// Recurses until the stack is gone. The condition is opaque to the
/// optimiser so it can neither remove the recursion nor warn about it.
fn recurse(depth: u64) -> u64 {
    let pad = [depth as u8; 4096];
    let deeper = if std::hint::black_box(true) {
        recurse(depth + 1)
    } else {
        0
    };
    deeper + u64::from(std::hint::black_box(pad)[0])
}

fn main() {
    strobe_diag::notify::set_notifier(|notice| {
        let kind = match notice.kind {
            CrashKind::Panic => "panic",
            CrashKind::Native => "native crash",
        };
        eprintln!(
            "NOTIFIER ({kind}): {}\n  report: {:?}\n  dump:   {:?}",
            notice.summary, notice.report, notice.dump
        );
        let hold = std::env::var("STROBE_DEMO_NOTIFY_HOLD_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        if let Some(millis) = hold {
            std::thread::sleep(Duration::from_millis(millis));
        }
    });
    if strobe_diag::crash::run_monitor_if_requested(std::env::args()) {
        return;
    }
    let session = strobe_diag::init();
    let capture = strobe_diag::crash::start_monitor();
    let connected = capture
        .as_ref()
        .is_some_and(|handle| handle.wait_connected(Duration::from_secs(5)));
    eprintln!(
        "log file: {:?}; native capture connected: {connected}",
        session.log_file
    );

    context::set(context::GL_RENDERER, "demo renderer");
    context::set(context::GL_BACKEND, "native");
    context::set(context::SCENE, "09-sdf-cathedral");
    context::set(context::RENDER_SIZE, "1280x720");
    context::note_frame(4242);
    context::note_pass(1);
    log::info!("demo is about to fail");
    failure::report(
        Failure::new(Subsystem::Shader, "link")
            .scene("09-sdf-cathedral")
            .detail("file", "main.glsl")
            .detail("line", 142)
            .message("unexpected token")
            .action("scene skipped; previous scene keeps playing"),
    );

    let mode = std::env::args().nth(1).unwrap_or_else(|| "ok".to_string());
    match mode.as_str() {
        "panic" | "panic-abort-profile" => {
            let values = std::hint::black_box(vec![1_u8, 2, 3]);
            let index = std::hint::black_box(values.len() + 4);
            eprintln!("value: {}", values[index]);
        }
        "abort" => std::process::abort(),
        "overflow" => eprintln!("depth sum: {}", recurse(0)),
        _ => log::info!("demo finished normally"),
    }
    strobe_diag::logger::flush(Duration::from_secs(2));
}
