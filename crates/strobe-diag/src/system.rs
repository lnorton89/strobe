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

use std::process::Command;
use std::sync::OnceLock;

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
    SystemInfo {
        os: os_version(),
        cpu_model: cpu_model(),
        cpu_cores: std::thread::available_parallelism()
            .ok()
            .map(std::num::NonZeroUsize::get),
        ram_bytes: ram_bytes(),
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

/// Runs a tool and returns its trimmed stdout, or `None` on any failure.
fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    hide_console(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
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

fn os_version() -> Option<String> {
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
        run(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$o = Get-CimInstance Win32_OperatingSystem; \
                 \"$($o.Caption) $($o.Version) build $($o.BuildNumber) $($o.OSArchitecture)\"",
            ],
        )
        .or_else(|| run("cmd", &["/c", "ver"]))
    } else if cfg!(target_os = "macos") {
        run("sw_vers", &["-productVersion"]).map(|version| format!("macOS {version}"))
    } else {
        Some(std::env::consts::OS.to_string())
    }
}

fn cpu_model() -> Option<String> {
    if cfg!(target_os = "linux") {
        let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        proc_field(&text, "model name").or_else(|| proc_field(&text, "Hardware"))
    } else if cfg!(windows) {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    } else if cfg!(target_os = "macos") {
        run("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        None
    }
}

fn ram_bytes() -> Option<u64> {
    if cfg!(target_os = "linux") {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        parse_meminfo_total(&text)
    } else if cfg!(windows) {
        run(
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
        run("sysctl", &["-n", "hw.memsize"])?.parse().ok()
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

    #[test]
    fn the_probe_finds_at_least_the_core_count() -> Result<()> {
        let probed = probe();
        assert!(
            probed.cpu_cores.is_some_and(|cores| cores >= 1),
            "core count missing: {probed:?}"
        );
        Ok(())
    }
}
