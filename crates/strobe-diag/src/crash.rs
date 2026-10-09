// ============================================================================
// Module:       strobe_diag::crash
// Description:  Out-of-process native crash capture: a monitor process that
//               writes the minidump and companion report when the app
//               faults inside a driver, overflows its stack, or aborts.
//
// Dependencies: crash-handler, minidumper; strobe_diag::{native, report,
//               context, logger, failure, notify, panic_hook, paths}
// ============================================================================

//! Native crash capture.
//!
//! A panic is easy: the panic hook runs in a healthy process. The failures
//! that kill a VJ show are not panics. They are access violations inside a
//! GPU driver, stack overflows, and `abort()` from a C++ runtime, and the
//! process that suffers them cannot be trusted to write its own report.
//! So the work is split across two processes:
//!
//! ```text
//!   app process                              monitor process
//!   -----------                              ---------------
//!   start_monitor() ── spawns exe ─────────▶ run_monitor_if_requested()
//!   attach CrashHandler                       minidumper Server on a local socket
//!   (signal handler / exception filter)
//!        │  crash!
//!        ├─ send context, failures, log ───▶ stores them
//!        └─ request_dump(CrashContext) ────▶ reads the app's memory,
//!                                            writes crash-*.dmp and crash-*.txt,
//!                                            runs the notifier (message box)
//! ```
//!
//! The monitor is the same executable started as
//! `strobe.exe --strobe-crash-monitor <socket>`. It never opens a window or
//! a GL context. It exits when the app disconnects (normal shutdown or
//! death), or after sixty seconds if the app never connected.
//!
//! ## What the app must do
//!
//! 1. First thing in `main()`, before anything else:
//!    `if run_monitor_if_requested(std::env::args()) { return; }`
//! 2. On the **main thread**, call [`start_monitor`] and keep the returned
//!    handle alive until exit. Attaching on the main thread is what gives
//!    the main thread a stack-overflow-safe signal stack.
//!
//! [`start_monitor`] never blocks startup and never fails the app: if the
//! monitor cannot be spawned the app logs a warning and runs without
//! native capture. The connection to the monitor is made on a background
//! thread; a crash in the first moments, before it connects, is not
//! captured. `STROBE_NO_CRASH_MONITOR=1` turns the whole mechanism off,
//! which is what to do under a debugger.
//!
//! ## Platform notes
//!
//! The socket is a Linux abstract-namespace socket, or a Unix-domain socket
//! file in the temp directory elsewhere. On Windows that needs Windows 10
//! version 1803 or later. `std::process::abort()` on Windows compiles to a
//! fast-fail that bypasses exception filters, so an abort there leaves no
//! minidump; a *panic* still leaves its text report, which is written
//! before the abort.

use crate::notify::{self, CrashKind, CrashNotice};
use crate::report::{self, CrashReport};
use crate::{context, failure, logger, native, panic_hook, paths};
use crash_handler::{CrashContext, CrashEventResult, CrashHandler};
use minidumper::{Client, LoopAction, MinidumpBinary, Server, ServerHandler, SocketName};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// The hidden argument that makes the executable run as the monitor. It is
/// followed by the socket name.
pub const MONITOR_ARG: &str = "--strobe-crash-monitor";

/// `STROBE_NO_CRASH_MONITOR=1` disables native crash capture.
pub const DISABLE_ENV: &str = "STROBE_NO_CRASH_MONITOR";

/// How long the app keeps trying to reach a freshly spawned monitor.
const CONNECT_BUDGET: Duration = Duration::from_secs(10);

/// Pause between connection attempts.
const CONNECT_RETRY: Duration = Duration::from_millis(25);

/// How long a monitor waits for its app before giving up and exiting.
const MONITOR_PATIENCE: Duration = Duration::from_secs(60);

/// Structured failures sent with a crash.
const FAILURE_TAIL: usize = 20;

/// Separates failure blocks on the wire; they contain newlines themselves.
const FAILURE_SEPARATOR: char = '\u{1e}';

/// Message kinds on the minidumper channel (user kinds start at 0).
const KIND_CONTEXT: u32 = 1;
const KIND_FAILURES: u32 = 2;
const KIND_LOG: u32 = 3;
const KIND_PANIC_REPORT: u32 = 4;
const KIND_CRASH_INFO: u32 = 5;

// ----------------------------------------------------------------- the wire

/// `key=value` per line, values flattened to one line.
fn encode_pairs(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{key}={}", value.replace(['\n', '\r'], " ")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// What the monitor has been told by the app so far.
#[derive(Debug, Default)]
struct Received {
    context: Vec<(String, String)>,
    failures: Vec<String>,
    log: Vec<String>,
    panic_report: Option<PathBuf>,
    info: Option<String>,
}

impl Received {
    fn apply(&mut self, kind: u32, buffer: &[u8]) {
        let text = String::from_utf8_lossy(buffer);
        match kind {
            KIND_CONTEXT => self.context = decode_pairs(&text),
            KIND_FAILURES => {
                self.failures = text
                    .split(FAILURE_SEPARATOR)
                    .filter(|block| !block.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            KIND_LOG => self.log = text.lines().map(str::to_string).collect(),
            KIND_PANIC_REPORT => self.panic_report = Some(PathBuf::from(text.as_ref())),
            KIND_CRASH_INFO => self.info = Some(text.into_owned()),
            _ => {}
        }
    }

    fn report(&self, dump: Option<&Path>, dump_error: Option<&str>) -> CrashReport {
        let mut details = Vec::new();
        match (dump, dump_error) {
            (Some(path), _) => details.push(("minidump".to_string(), file_name(path))),
            (None, Some(error)) => {
                details.push(("minidump".to_string(), format!("NOT WRITTEN: {error}")));
            }
            (None, None) => {}
        }
        if self.context.is_empty() {
            details.push((
                "note".to_string(),
                "the app could not send its session context before dying".to_string(),
            ));
        }
        CrashReport {
            kind: "native crash".to_string(),
            headline: self
                .info
                .clone()
                .unwrap_or_else(|| "native crash (details are in the minidump)".to_string()),
            details,
            context: self.context.clone(),
            failures: self.failures.clone(),
            log: self.log.clone(),
            backtrace: None,
            // The monitor is healthy and idle; it can afford the probe.
            probe_system: true,
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

// ------------------------------------------------------------- the sockets

/// The socket name the app will pass to its monitor.
fn socket_name_for_this_process() -> String {
    let pid = std::process::id();
    if cfg!(target_os = "linux") {
        format!("strobe-crash-{pid}")
    } else {
        std::env::temp_dir()
            .join(format!("strobe-crash-{pid}.sock"))
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn socket<R>(name: &str, then: impl FnOnce(SocketName<'_>) -> R) -> R {
    then(SocketName::abstract_namespace(name))
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn socket<R>(name: &str, then: impl FnOnce(SocketName<'_>) -> R) -> R {
    // A stale socket file from a dead process would make bind fail.
    fs::remove_file(name).ok();
    then(SocketName::path(name))
}

// ------------------------------------------------------------- the monitor

/// Runs the monitor if `args` asks for it, blocking until it is done.
/// Returns `true` when this process was a monitor, in which case `main`
/// must return immediately.
pub fn run_monitor_if_requested(args: impl IntoIterator<Item = String>) -> bool {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == MONITOR_ARG {
            match args.next() {
                Some(name) => run_monitor(&name),
                None => write_stderr("strobe crash monitor: missing socket name"),
            }
            return true;
        }
    }
    false
}

/// The monitor has no logger of its own (it must not rotate the app's
/// session logs away), so its own trouble goes to stderr.
fn write_stderr(message: &str) {
    writeln!(std::io::stderr(), "{message}").ok();
}

fn run_monitor(name: &str) {
    let server = socket(name, Server::with_name);
    let mut server = match server {
        Ok(server) => server,
        Err(error) => {
            write_stderr(&format!("strobe crash monitor: cannot listen: {error}"));
            return;
        }
    };
    let shutdown = Arc::new(AtomicBool::new(false));
    let connected = Arc::new(AtomicBool::new(false));
    spawn_watchdog(Arc::clone(&shutdown), Arc::clone(&connected));

    let handler = MonitorHandler {
        dir: paths::crashes_dir(),
        connected,
        state: Mutex::new(MonitorState::default()),
    };
    if let Err(error) = server.run(Box::new(handler), &shutdown, None) {
        write_stderr(&format!("strobe crash monitor: server stopped: {error}"));
    }
    if !cfg!(target_os = "linux") {
        fs::remove_file(name).ok();
    }
}

/// Stops a monitor whose app never showed up.
fn spawn_watchdog(shutdown: Arc<AtomicBool>, connected: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("strobe-monitor-watchdog".to_string())
        .spawn(move || {
            std::thread::sleep(MONITOR_PATIENCE);
            if !connected.load(Ordering::SeqCst) {
                shutdown.store(true, Ordering::SeqCst);
            }
        })
        .ok();
}

#[derive(Debug, Default)]
struct MonitorState {
    received: Received,
    stem: Option<String>,
}

struct MonitorHandler {
    dir: PathBuf,
    connected: Arc<AtomicBool>,
    state: Mutex<MonitorState>,
}

impl MonitorHandler {
    fn state(&self) -> std::sync::MutexGuard<'_, MonitorState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the companion report (or annotates the panic report) and
    /// tells the person.
    fn finish(&self, result: Result<MinidumpBinary, minidumper::Error>) {
        let (dump, dump_error) = match result {
            Ok(mut binary) => {
                binary.file.flush().ok();
                (Some(binary.path), None)
            }
            Err(error) => (None, Some(error.to_string())),
        };
        let state = self.state();

        // An abort that followed a panic: the panic hook already wrote and
        // announced the report. Point it at the dump and stay quiet.
        if let Some(panic_report) = &state.received.panic_report {
            if let Some(dump) = &dump {
                append(
                    panic_report,
                    &format!(
                        "\nMinidump of the abort that followed: {}\n",
                        file_name(dump)
                    ),
                );
            }
            return;
        }

        let report = state
            .received
            .report(dump.as_deref(), dump_error.as_deref());
        let written = match (&state.stem, &dump) {
            (Some(stem), _) => report::write_text(&self.dir, stem, &report.render()),
            (None, Some(dump)) => report::write_text(
                &self.dir,
                &dump
                    .file_stem()
                    .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned()),
                &report.render(),
            ),
            (None, None) => report::claim_stem(&self.dir)
                .and_then(|stem| report::write_text(&self.dir, &stem, &report.render())),
        };
        let report_path = written
            .map_err(|error| write_stderr(&format!("strobe crash monitor: report: {error}")))
            .ok();
        notify::notify(&CrashNotice {
            kind: CrashKind::Native,
            summary: report.headline,
            report: report_path,
            dump,
        });
    }
}

fn append(path: &Path, text: &str) {
    if let Ok(mut file) = OpenOptions::new().append(true).open(path) {
        file.write_all(text.as_bytes()).ok();
    }
}

impl ServerHandler for MonitorHandler {
    fn create_minidump_file(&self) -> Result<(File, PathBuf), std::io::Error> {
        let mut state = self.state();
        let stem = match state
            .received
            .panic_report
            .as_deref()
            .and_then(Path::file_stem)
        {
            Some(stem) => stem.to_string_lossy().into_owned(),
            None => report::claim_stem(&self.dir)?,
        };
        let path = self.dir.join(format!("{stem}.dmp"));
        let file = File::create(&path)?;
        state.stem = Some(stem);
        Ok((file, path))
    }

    fn on_minidump_created(&self, result: Result<MinidumpBinary, minidumper::Error>) -> LoopAction {
        self.finish(result);
        LoopAction::Exit
    }

    fn on_message(&self, kind: u32, buffer: Vec<u8>) {
        self.state().received.apply(kind, &buffer);
    }

    fn on_client_connected(&self, _num_clients: usize) -> LoopAction {
        self.connected.store(true, Ordering::SeqCst);
        LoopAction::Continue
    }

    fn on_client_disconnected(&self, num_clients: usize) -> LoopAction {
        if num_clients == 0 {
            LoopAction::Exit
        } else {
            LoopAction::Continue
        }
    }
}

// ----------------------------------------------------------------- the app

/// Keeps native crash capture alive. Drop it at the end of `main`.
pub struct MonitorHandle {
    _handler: CrashHandler,
    child: Child,
    client: Arc<OnceLock<Client>>,
}

impl MonitorHandle {
    /// Whether the link to the monitor is up, so a crash now would be
    /// captured. Becomes true within milliseconds of [`start_monitor`] on a
    /// healthy machine.
    pub fn is_connected(&self) -> bool {
        self.client.get().is_some()
    }

    /// Waits until [`MonitorHandle::is_connected`] or `timeout` passes.
    /// For tests and the crash demo; the app never needs to wait.
    pub fn wait_connected(&self, timeout: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if self.is_connected() {
                return true;
            }
            std::thread::sleep(CONNECT_RETRY);
        }
        self.is_connected()
    }

    /// The monitor process id.
    pub fn monitor_pid(&self) -> u32 {
        self.child.id()
    }
}

/// Starts the monitor and attaches the crash handler. Call on the main
/// thread, after [`run_monitor_if_requested`]. Returns `None` (after
/// logging why) if capture could not be set up; the app should carry on.
pub fn start_monitor() -> Option<MonitorHandle> {
    if std::env::var(DISABLE_ENV).is_ok_and(|value| value == "1") {
        log::info!("native crash capture disabled by {DISABLE_ENV}=1");
        return None;
    }
    let exe = std::env::current_exe()
        .map_err(|error| log::warn!("native crash capture off: cannot find own exe: {error}"))
        .ok()?;
    let name = socket_name_for_this_process();
    let mut child = spawn_monitor_process(&exe, &name)
        .map_err(|error| log::warn!("native crash capture off: cannot start monitor: {error}"))
        .ok()?;

    let client: Arc<OnceLock<Client>> = Arc::new(OnceLock::new());
    let for_crash = Arc::clone(&client);
    let attached = CrashHandler::attach(native::crash_event(move |crash| {
        on_crash(&for_crash, crash)
    }));
    let handler = match attached {
        Ok(handler) => handler,
        Err(error) => {
            log::warn!("native crash capture off: cannot attach handler: {error}");
            child.kill().ok();
            return None;
        }
    };
    set_ptracer(&handler, child.id());
    spawn_connector(Arc::clone(&client), name);
    log::info!("native crash capture armed (monitor pid {})", child.id());
    Some(MonitorHandle {
        _handler: handler,
        child,
        client,
    })
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_ptracer(handler: &CrashHandler, monitor_pid: u32) {
    // Under Yama's ptrace_scope a process may only be traced by its
    // ancestors; naming the monitor makes the dump work regardless.
    handler.set_ptracer(Some(monitor_pid));
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn set_ptracer(_handler: &CrashHandler, _monitor_pid: u32) {}

fn spawn_monitor_process(exe: &Path, name: &str) -> std::io::Result<Child> {
    let mut command = Command::new(exe);
    command
        .arg(MONITOR_ARG)
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    command.spawn()
}

/// Starts the monitor without a console window flashing over the show.
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`.
    const FLAGS: u32 = 0x0800_0000;
    command.creation_flags(FLAGS);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

/// Connects to the monitor in the background so startup never waits.
fn spawn_connector(slot: Arc<OnceLock<Client>>, name: String) {
    let spawned = std::thread::Builder::new()
        .name("strobe-crash-connect".to_string())
        .spawn(move || {
            let started = Instant::now();
            while started.elapsed() < CONNECT_BUDGET {
                if let Ok(client) = socket(&name, Client::with_name) {
                    // First connection wins; there is only ever one.
                    slot.set(client).ok();
                    log::debug!("connected to crash monitor after {:?}", started.elapsed());
                    return;
                }
                std::thread::sleep(CONNECT_RETRY);
            }
            log::warn!("crash monitor never accepted a connection; native capture is off");
        });
    if let Err(error) = spawned {
        log::warn!("native crash capture off: connector thread failed: {error}");
    }
}

/// Runs in the compromised context. Sends what the monitor needs to write
/// a useful report, then asks it for the minidump. It takes no lock it
/// might have to wait for (`try_*` everywhere), and never logs.
fn on_crash(client: &OnceLock<Client>, crash: &CrashContext) -> CrashEventResult {
    let Some(client) = client.get() else {
        return CrashEventResult::Handled(false);
    };
    client.send_message(KIND_CRASH_INFO, describe(crash)).ok();
    if let Some(pairs) = context::try_snapshot() {
        client.send_message(KIND_CONTEXT, encode_pairs(&pairs)).ok();
    }
    let failures: Vec<String> = failure::try_recent(FAILURE_TAIL)
        .iter()
        .map(ToString::to_string)
        .collect();
    client
        .send_message(KIND_FAILURES, failures.join(&FAILURE_SEPARATOR.to_string()))
        .ok();
    client
        .send_message(
            KIND_LOG,
            logger::try_recent_lines(panic_hook::LOG_TAIL).join("\n"),
        )
        .ok();
    if let Some(path) = panic_hook::last_report() {
        client
            .send_message(KIND_PANIC_REPORT, path.to_string_lossy().as_bytes())
            .ok();
    }
    // Messages and the dump request travel separately on some platforms;
    // a ping makes sure the messages above are processed first.
    client.ping().ok();
    logger::flush(logger::CRASH_FLUSH_TIMEOUT);
    CrashEventResult::Handled(client.request_dump(crash).is_ok())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn describe(crash: &CrashContext) -> String {
    let signal = crash.siginfo.ssi_signo;
    format!("native crash: {} ({signal})", signal_name(signal))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn signal_name(signal: u32) -> &'static str {
    match signal {
        4 => "SIGILL, illegal instruction",
        6 => "SIGABRT, abort",
        7 => "SIGBUS, bus error",
        8 => "SIGFPE, arithmetic error",
        11 => "SIGSEGV, invalid memory access or stack overflow",
        5 => "SIGTRAP, trap",
        _ => "signal",
    }
}

#[cfg(windows)]
fn describe(crash: &CrashContext) -> String {
    // The cast reinterprets the NTSTATUS bits for hex display.
    format!(
        "native crash: exception code 0x{:08X}",
        crash.exception_code as u32
    )
}

#[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
fn describe(_crash: &CrashContext) -> String {
    "native crash: mach exception".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    #[test]
    fn pairs_survive_the_wire_and_lose_newlines() -> Result<()> {
        let pairs = vec![
            ("scene.name".to_string(), "a=b".to_string()),
            ("note".to_string(), "line one\nline two".to_string()),
        ];
        let decoded = decode_pairs(&encode_pairs(&pairs));
        assert_eq!(
            decoded,
            vec![
                ("scene.name".to_string(), "a=b".to_string()),
                ("note".to_string(), "line one line two".to_string()),
            ],
            "decoded pairs"
        );
        Ok(())
    }

    #[test]
    fn received_messages_become_a_native_report() -> Result<()> {
        let mut received = Received::default();
        received.apply(KIND_CRASH_INFO, b"native crash: SIGSEGV");
        received.apply(KIND_CONTEXT, b"scene.name=cathedral\nframe=900");
        received.apply(
            KIND_FAILURES,
            format!("SHADER\nphase=link{FAILURE_SEPARATOR}AUDIO\nphase=capture").as_bytes(),
        );
        received.apply(KIND_LOG, b"line one\nline two");
        received.apply(99, b"unknown kinds are ignored");

        let dump = PathBuf::from("/crashes/crash-20261008-120000.dmp");
        let report = received.report(Some(&dump), None);
        let text = report.render();
        assert_eq!(report.failures.len(), 2, "two failure blocks");
        for needle in [
            "kind:    native crash",
            "summary: native crash: SIGSEGV",
            "minidump: crash-20261008-120000.dmp",
            "scene.name",
            "cathedral",
            "line two",
            "see the minidump",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        Ok(())
    }

    #[test]
    fn a_missing_context_and_dump_failure_are_stated() -> Result<()> {
        let report = Received::default().report(None, Some("ptrace denied"));
        let text = report.render();
        assert!(text.contains("NOT WRITTEN: ptrace denied"), "got:\n{text}");
        assert!(
            text.contains("could not send its session context"),
            "got:\n{text}"
        );
        Ok(())
    }

    #[test]
    fn the_monitor_argument_is_recognised_only_with_its_flag() -> Result<()> {
        assert!(
            !run_monitor_if_requested(["strobe.exe".to_string(), "--diagnose".to_string()]),
            "ordinary arguments must not start a monitor"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_signals_are_named() -> Result<()> {
        assert!(signal_name(11).starts_with("SIGSEGV"), "11 is SIGSEGV");
        assert!(signal_name(6).starts_with("SIGABRT"), "6 is SIGABRT");
        Ok(())
    }

    #[test]
    fn a_dump_after_a_panic_reuses_the_panic_stem() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("strobe-diag-test-{}-stem", std::process::id()));
        fs::create_dir_all(&dir)?;
        let panic_txt = dir.join("crash-20261008-120000.txt");
        fs::write(&panic_txt, "panic report\n")?;
        let handler = MonitorHandler {
            dir: dir.clone(),
            connected: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(MonitorState::default()),
        };
        handler.on_message(
            KIND_PANIC_REPORT,
            panic_txt.to_string_lossy().as_bytes().to_vec(),
        );
        let (_file, dump_path) = handler.create_minidump_file()?;
        let name = dump_path
            .file_name()
            .ok_or_else(|| anyhow!("dump has no name"))?
            .to_string_lossy()
            .into_owned();
        fs::remove_dir_all(&dir)?;
        assert_eq!(
            name, "crash-20261008-120000.dmp",
            "dump shares the panic stem"
        );
        Ok(())
    }
}
