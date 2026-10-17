// ============================================================================
// Module:       strobe_diag::system
// Description:  OS version, CPU model and core count, and installed RAM,
//               gathered without unsafe code or extra dependencies.
//
// Dependencies: std::process, std::fs (platform tools and /proc)
// ============================================================================

//! Machine facts for reports.
//!
//! These come from files and tools the OS already provides, so this crate
//! needs no FFI and no system-information dependency:
//!
//! | | OS version | CPU model | RAM |
//! |---|---|---|---|
//! | Linux | `/etc/os-release`, `/proc/sys/kernel/osrelease` | `/proc/cpuinfo` | `/proc/meminfo` |
//! | Windows | `Get-CimInstance Win32_OperatingSystem` (falls back to `cmd /c ver`) | `PROCESSOR_IDENTIFIER` | `Win32_ComputerSystem` |
//! | macOS | `sw_vers` | `sysctl machdep.cpu.brand_string` | `sysctl hw.memsize` |
//!
//! On Windows the OS version and RAM need a PowerShell process, which takes
//! a noticeable fraction of a second on an old laptop. [`probe_in_background`]
//! therefore runs the probe on a thread at startup; [`cached`] reads the
//! result without waiting (a crash report written before the probe finished
//! simply lacks those lines), and [`info`] waits for it (the `--diagnose`
//! report). Every field is optional: a failed probe is an absent line, not
//! an error. Child processes are started without a console window.
//!
//! The whole probe has a budget ([`PROBE_BUDGET`], ten seconds) shared by
//! every child process it starts. A PowerShell or WMI query that hangs is
//! killed when the budget runs out, the remaining queries are skipped, and
//! [`SystemInfo::timed_out`] is set so the report says "system probe timed
//! out" instead of the monitor waiting forever and never writing it (D395).

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long the probe may spend on child processes in total.
pub const PROBE_BUDGET: Duration = Duration::from_secs(10);

/// What the probe learned. Missing facts are `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemInfo {
    /// Human-readable OS name and version.
    pub os: Option<String>,
    /// CPU model string.
    pub cpu_model: Option<String>,
    /// Logical cores available to the process.
    pub cpu_cores: Option<usize>,
    /// Installed physical memory in bytes.
    pub ram_bytes: Option<u64>,
    /// A child process outlived [`PROBE_BUDGET`] and was killed, so fields
    /// that need one may be missing.
    pub timed_out: bool,
}

static CACHE: OnceLock<SystemInfo> = OnceLock::new();

/// Starts the probe on a background thread. Idempotent.
pub fn probe_in_background() {
    if CACHE.get().is_some() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("strobe-sysinfo".to_string())
        .spawn(|| {
            info();
        });
    if let Err(error) = spawned {
        // The facts are then gathered on first use instead, on the caller.
        log::warn!("system facts: no background probe ({error})");
    }
}

/// The probe result if it has finished; never blocks.
pub fn cached() -> Option<&'static SystemInfo> {
    CACHE.get()
}

/// The probe result, running the probe now if nothing has yet.
pub fn info() -> &'static SystemInfo {
    CACHE.get_or_init(probe)
}

fn probe() -> SystemInfo {
    probe_within(PROBE_BUDGET)
}

fn probe_within(budget: Duration) -> SystemInfo {
    let mut tools = Tools::new(budget);
    SystemInfo {
        os: os_version(&mut tools),
        cpu_model: cpu_model(&mut tools),
        cpu_cores: std::thread::available_parallelism()
            .ok()
            .map(std::num::NonZeroUsize::get),
        ram_bytes: ram_bytes(&mut tools),
        timed_out: tools.timed_out,
    }
}

/// Binary-prefixed size, such as `7.9 GiB`.
pub fn format_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else {
        format!("{:.0} MiB", value / MIB)
    }
}

/// Child processes the probe may start, sharing one deadline.
struct Tools {
    deadline: Instant,
    /// A child was killed at the deadline; nothing more is started.
    timed_out: bool,
}

impl Tools {
    fn new(budget: Duration) -> Self {
        Self {
            deadline: Instant::now() + budget,
            timed_out: false,
        }
    }

    /// Runs a tool and returns its trimmed stdout, or `None` on any failure
    /// or when the budget is spent. A child still running at the deadline is
    /// killed and reaped.
    fn run(&mut self, program: &str, args: &[&str]) -> Option<String> {
        if self.timed_out {
            return None;
        }
        let mut command = Command::new(program);
        command.args(args);
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        match run_command(command, remaining) {
            Ran::Output(text) => text,
            Ran::TimedOut => {
                log::warn!("system probe: `{program}` did not finish in time and was killed");
                self.timed_out = true;
                None
            }
        }
    }
}

/// How one child process ended.
#[derive(Debug, PartialEq, Eq)]
enum Ran {
    /// It finished (or never started): its trimmed stdout if it succeeded
    /// and printed something.
    Output(Option<String>),
    /// It was still running after the allowed time and was killed.
    TimedOut,
}

/// Runs `command` for at most `limit`. Stdout is read on a helper thread so
/// that a child which fills the pipe, or exits while a grandchild keeps the
/// pipe open, cannot hold this thread past the limit.
fn run_command(mut command: Command, limit: Duration) -> Ran {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let Ok(mut child) = command.spawn() else {
        return Ran::Output(None);
    };
    let Some(mut stdout) = child.stdout.take() else {
        kill_and_reap(&mut child);
        return Ran::Output(None);
    };
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("strobe-sysinfo-read".to_string())
        .spawn(move || {
            let mut bytes = Vec::new();
            // A read error ends the output early; what was read is used.
            stdout.read_to_end(&mut bytes).ok();
            // The receiver is gone after a timeout, which is fine.
            sender.send(bytes).ok();
        });
    if reader.is_err() {
        // The child's output cannot be read; do not leave it running.
        kill_and_reap(&mut child);
        return Ran::Output(None);
    }
    let started = Instant::now();
    let Ok(bytes) = receiver.recv_timeout(limit) else {
        kill_and_reap(&mut child);
        return Ran::TimedOut;
    };
    // The pipe closed, so the child is exiting; give it what is left of the
    // time to report its status.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < limit => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                kill_and_reap(&mut child);
                return Ran::TimedOut;
            }
            Err(_) => break None,
        }
    };
    if !status.is_some_and(|status| status.success()) {
        return Ran::Output(None);
    }
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    Ran::Output((!text.is_empty()).then_some(text))
}

/// Kills a child and reaps it so it does not linger as a zombie.
fn kill_and_reap(child: &mut Child) {
    // Killing a process that has just exited by itself fails harmlessly.
    child.kill().ok();
    // Only reaping matters; the status of a killed child is not needed.
    child.wait().ok();
}

/// Stops a child process from flashing a console window over the output.
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`.
    const FLAGS: u32 = 0x0800_0000;
    command.creation_flags(FLAGS);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

/// The value after `key:` in a `/proc`-style `key: value` text.
fn proc_field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().to_string())
}

/// Parses `/proc/meminfo`'s `MemTotal:  16384 kB`.
fn parse_meminfo_total(text: &str) -> Option<u64> {
    let value = proc_field(text, "MemTotal")?;
    let kib: u64 = value.split_whitespace().next()?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Parses the `PRETTY_NAME="…"` line of `/etc/os-release`.
fn parse_os_release(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|value| value.trim().trim_matches('"').to_string())
}

fn os_version(tools: &mut Tools) -> Option<String> {
    if cfg!(target_os = "linux") {
        let name = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| parse_os_release(&text));
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|text| text.trim().to_string());
        match (name, kernel) {
            (Some(name), Some(kernel)) => Some(format!("{name} (kernel {kernel})")),
            (Some(name), None) => Some(name),
            (None, Some(kernel)) => Some(format!("Linux kernel {kernel}")),
            (None, None) => None,
        }
    } else if cfg!(windows) {
        tools
            .run(
                "powershell",
                &[
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$o = Get-CimInstance Win32_OperatingSystem; \
                 \"$($o.Caption) $($o.Version) build $($o.BuildNumber) $($o.OSArchitecture)\"",
                ],
            )
            .or_else(|| tools.run("cmd", &["/c", "ver"]))
    } else if cfg!(target_os = "macos") {
        tools
            .run("sw_vers", &["-productVersion"])
            .map(|version| format!("macOS {version}"))
    } else {
        Some(std::env::consts::OS.to_string())
    }
}

fn cpu_model(tools: &mut Tools) -> Option<String> {
    if cfg!(target_os = "linux") {
        let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        proc_field(&text, "model name").or_else(|| proc_field(&text, "Hardware"))
    } else if cfg!(windows) {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    } else if cfg!(target_os = "macos") {
        tools.run("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        None
    }
}

fn ram_bytes(tools: &mut Tools) -> Option<u64> {
    if cfg!(target_os = "linux") {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        parse_meminfo_total(&text)
    } else if cfg!(windows) {
        tools
            .run(
                "powershell",
                &[
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
                ],
            )?
            .parse()
            .ok()
    } else if cfg!(target_os = "macos") {
        tools.run("sysctl", &["-n", "hw.memsize"])?.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    #[test]
    fn meminfo_total_is_converted_to_bytes() -> Result<()> {
        let text = "MemTotal:       16384000 kB\nMemFree:  100 kB\n";
        let bytes = parse_meminfo_total(text).ok_or_else(|| anyhow!("no MemTotal"))?;
        assert_eq!(bytes, 16_384_000 * 1024, "kB to bytes");
        Ok(())
    }

    #[test]
    fn os_release_pretty_name_is_unquoted() -> Result<()> {
        let text = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n";
        assert_eq!(
            parse_os_release(text).as_deref(),
            Some("Ubuntu 24.04.1 LTS"),
            "pretty name"
        );
        Ok(())
    }

    #[test]
    fn proc_fields_match_exact_keys() -> Result<()> {
        let text = "model name\t: Intel(R) Core(TM) i5-8350U\nmodel\t: 142\n";
        assert_eq!(
            proc_field(text, "model name").as_deref(),
            Some("Intel(R) Core(TM) i5-8350U"),
            "model name"
        );
        assert_eq!(proc_field(text, "model").as_deref(), Some("142"), "model");
        Ok(())
    }

    #[test]
    fn sizes_are_formatted_in_binary_units() -> Result<()> {
        assert_eq!(format_bytes(8 * 1024 * 1024 * 1024), "8.0 GiB", "GiB");
        assert_eq!(format_bytes(512 * 1024 * 1024), "512 MiB", "MiB");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_tool_is_killed_at_the_deadline() -> Result<()> {
        let mut command = Command::new("sleep");
        command.arg("30");
        let started = Instant::now();
        let ran = run_command(command, Duration::from_millis(300));
        let took = started.elapsed();
        assert_eq!(ran, Ran::TimedOut, "a 30 s sleep must time out");
        assert!(
            took < Duration::from_secs(5),
            "the call waited {took:?} for a hung child"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_finishes_returns_its_output() -> Result<()> {
        let mut command = Command::new("echo");
        command.arg("  hello  ");
        let ran = run_command(command, Duration::from_secs(10));
        assert_eq!(ran, Ran::Output(Some("hello".to_string())), "echo output");
        let mut failing = Command::new("false");
        failing.arg("ignored");
        assert_eq!(
            run_command(failing, Duration::from_secs(10)),
            Ran::Output(None),
            "a failing tool is an absent fact, not a timeout"
        );
        let missing = Command::new("strobe-no-such-tool-anywhere");
        assert_eq!(
            run_command(missing, Duration::from_secs(10)),
            Ran::Output(None),
            "a tool that cannot start is an absent fact"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn once_the_budget_is_spent_no_more_tools_are_started() -> Result<()> {
        let mut tools = Tools::new(Duration::from_millis(200));
        let started = Instant::now();
        assert_eq!(tools.run("sleep", &["30"]), None, "the hung tool");
        assert!(tools.timed_out, "the timeout is recorded");
        // `echo` would succeed, so a `None` here is the skipped start.
        assert_eq!(tools.run("echo", &["x"]), None, "no start after timeout");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the budget was not enforced"
        );
        Ok(())
    }

    #[test]
    fn the_probe_finds_at_least_the_core_count() -> Result<()> {
        let probed = probe();
        assert!(!probed.timed_out, "a healthy probe does not time out");
        assert!(
            probed.cpu_cores.is_some_and(|cores| cores >= 1),
            "core count missing: {probed:?}"
        );
        Ok(())
    }
}
