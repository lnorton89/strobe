// ============================================================================
// Module:       strobe_projector (binary crate root)
// Description:  The `strobe-projector` command (`pj` once installed): power,
//               status, discovery, raw PJLink, the headless web service, and
//               the DHCP reservation responder.
//
// Dependencies: clap, serde_json, log; strobe-diag (logger);
//               strobe_projector (the library)
// ============================================================================

//! The CLI and the headless service (D502).
//!
//! ```text
//! pj status   [all|key]          power, input, lamp/laser hours, error status
//! pj on       [all|key] [--wait] --wait polls until on (and a lamp reports lit)
//! pj off      [all|key] [--wait] --wait polls through cooling until off
//! pj discover [--subnet CIDR]    scan + ARP; on problems, listen for DHCP clients
//! pj raw      key "%1NAME ?"     send a raw PJLink command
//! pj serve                       the web page, for the Scheduled Task
//! pj dhcp-reserve [--seconds N]  answer the reserved projectors' DHCP
//! pj -v ...                      verbose (retries, fallbacks)
//! ```
//!
//! Exit codes, as the Python tool's: `0` ok, `1` a projector or command
//! failed, `2` a usage or configuration error.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use strobe_projector::audit::Audit;
use strobe_projector::config::{self, Config, Subnet};
use strobe_projector::server::Shared;
use strobe_projector::service::Service;
use strobe_projector::unit::{self, Timing, Unit};
use strobe_projector::{dhcp, discover};

/// Projector power control (PJLink / NTCONTROL).
#[derive(Debug, Parser)]
#[command(name = "pj", version)]
struct Cli {
    /// Log retries and fallbacks.
    #[arg(short, long, global = true)]
    verbose: bool,
    /// The configuration file (default: projectors.json in Strobe's data
    /// directory).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Switch on.
    On {
        /// `all` or a projector's key.
        #[arg(default_value = "all")]
        target: String,
        /// Poll until the target state is reached.
        #[arg(long)]
        wait: bool,
    },
    /// Switch off.
    Off {
        /// `all` or a projector's key.
        #[arg(default_value = "all")]
        target: String,
        /// Poll until the target state is reached.
        #[arg(long)]
        wait: bool,
    },
    /// Power, input, hours and errors.
    Status {
        /// `all` or a projector's key.
        #[arg(default_value = "all")]
        target: String,
    },
    /// Scan the projector network for PJLink hosts.
    Discover {
        /// The network to scan (default: network.projector_subnet).
        #[arg(long)]
        subnet: Option<String>,
    },
    /// Send a raw PJLink command, e.g. `pj raw sony "%1NAME ?"`.
    Raw {
        /// A projector's key.
        target: String,
        /// The command.
        command: String,
    },
    /// Serve the web page until stopped (the Scheduled Task's command).
    Serve,
    /// Answer DHCP for projectors with a `mac` in the config.
    DhcpReserve {
        /// Stop after this many seconds (default: run until stopped).
        #[arg(long)]
        seconds: Option<f64>,
    },
}

/// A failure and the exit code it means.
enum Failure {
    /// A usage or configuration error (exit 2).
    Usage(String),
    /// A projector or command failed (exit 1).
    Command(String),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let config_path = cli.config.clone().unwrap_or_else(config::default_path);
    start_logging(cli.verbose, &cli.command, &config_path);
    match run(&cli, &config_path) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(Failure::Command(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
        Err(Failure::Usage(message)) => {
            eprintln!("config error: {message}");
            ExitCode::from(2)
        }
    }
}

/// The logger: stderr at warn (info with `-v`); `serve` also keeps a
/// session file beside the config, since a Scheduled Task has no console.
fn start_logging(verbose: bool, command: &Command, config_path: &std::path::Path) {
    use strobe_diag::filter::Filter;
    use strobe_diag::logger::{self, LoggerConfig};
    let serving = matches!(command, Command::Serve | Command::DhcpReserve { .. });
    let level = if verbose || serving {
        log::LevelFilter::Info
    } else {
        log::LevelFilter::Warn
    };
    let dir = serving.then(|| {
        config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(strobe_projector::audit::DIR)
    });
    let installed = logger::init_with(LoggerConfig {
        filter: Filter::parse("", level),
        dir,
        stderr: true,
        max_file_bytes: logger::MAX_FILE_BYTES,
    });
    if installed.is_err() {
        eprintln!("warning: the log could not be set up");
    }
}

fn load(path: &std::path::Path) -> Result<Config, Failure> {
    Config::load(path).map_err(|error| Failure::Usage(error.to_string()))
}

/// Runs the command; `Ok(false)` is a failure already reported.
fn run(cli: &Cli, config_path: &std::path::Path) -> Result<bool, Failure> {
    let config = load(config_path)?;
    let audit = Audit::beside(config_path);
    let source = format!(
        "cli:{}",
        std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "?".into())
    );
    let units = |target: &str| -> Result<Vec<Unit>, Failure> {
        unit::units_for(&config, target).map_err(|error| Failure::Usage(error.to_string()))
    };
    match &cli.command {
        Command::On { target, wait } => Ok(power(units(target)?, true, *wait, &audit, &source)),
        Command::Off { target, wait } => Ok(power(units(target)?, false, *wait, &audit, &source)),
        Command::Status { target } => Ok(status(units(target)?)),
        Command::Raw { target, command } => {
            let units = units(target)?;
            let unit = units
                .first()
                .ok_or_else(|| Failure::Usage(format!("no projector called '{target}'")))?;
            let value = unit
                .pjlink()
                .raw(command)
                .map_err(|error| Failure::Command(error.to_string()))?;
            println!("{value}");
            Ok(true)
        }
        Command::Discover { subnet } => {
            let text = subnet
                .clone()
                .unwrap_or_else(|| config.network.projector_subnet.clone());
            let subnet = Subnet::parse(&text).ok_or_else(|| {
                Failure::Usage(format!("'{text}' is not a network like 192.168.50.0/24"))
            })?;
            Ok(run_discover(&config, subnet))
        }
        Command::Serve => {
            if let Some(problem) = config.api_problem() {
                return Err(Failure::Usage(format!("{problem}; refusing to start")));
            }
            let shared = Arc::new(Shared {
                config: Arc::new(RwLock::new(config)),
                audit,
                timing: Timing::default(),
            });
            Service::start(shared).run_forever();
            Ok(true)
        }
        Command::DhcpReserve { seconds } => {
            let duration = seconds
                .map(|seconds| {
                    Duration::try_from_secs_f64(seconds).map_err(|_| {
                        Failure::Usage(format!("--seconds {seconds} is not a duration"))
                    })
                })
                .transpose()?
                .filter(|duration| !duration.is_zero());
            dhcp::serve(&config, duration, &audit)
                .map_err(|error| Failure::Usage(error.to_string()))
        }
    }
}

fn power(units: Vec<Unit>, on: bool, wait: bool, audit: &Audit, source: &str) -> bool {
    let action = if on { "on" } else { "off" };
    let mut all_ok = true;
    for (unit, result) in unit::run_on(units, |unit| unit.set_power(on, wait)) {
        match result {
            Ok(result) => {
                let json = serde_json::to_string(&result).unwrap_or_default();
                audit.record(source, unit.key(), action, &format!("OK {json}"));
                let verb = if result.sent {
                    "command sent".to_string()
                } else {
                    format!("already {}, nothing sent", result.previous)
                };
                let reached = result
                    .reached
                    .as_ref()
                    .map(|state| format!(", reached {state} in {}s", result.elapsed_s))
                    .unwrap_or_default();
                println!(
                    "{}: {verb} (via {}){reached}",
                    unit.name(),
                    result.via.name()
                );
            }
            Err(error) => {
                all_ok = false;
                audit.record(source, unit.key(), action, &format!("FAIL {error}"));
                println!("{}: FAILED - {error}", unit.name());
            }
        }
    }
    all_ok
}

fn status(units: Vec<Unit>) -> bool {
    let mut all_ok = true;
    for (unit, result) in unit::run_on(units, |unit| Ok(unit.status())) {
        println!("== {} [{}] {}", unit.name(), unit.key(), unit.ip());
        let status = match result {
            Ok(status) => status,
            Err(error) => {
                all_ok = false;
                println!("   ERROR: {error}");
                continue;
            }
        };
        all_ok &= status.ok;
        let via = status.via.map(|via| via.name().to_string());
        for (label, value) in [
            ("power", Some(&status.power)),
            ("input", status.input.as_ref()),
            ("hours", status.hours.as_ref()),
            ("errors", status.errors.as_ref()),
            ("via", via.as_ref()),
        ] {
            if let Some(value) = value {
                println!("   {label:<7}: {value}");
            }
        }
    }
    all_ok
}

fn run_discover(config: &Config, subnet: Subnet) -> bool {
    /// How long to listen for units stuck in DHCP mode.
    const DHCP_LISTEN: Duration = Duration::from_secs(15);
    let found = discover::discover(config, subnet);
    for host in &found {
        let error = host
            .error
            .as_ref()
            .map(|error| format!("  ({error})"))
            .unwrap_or_default();
        println!(
            "{:<16} vendor={:<10} NAME={:?} INF1={:?} INF2={:?}{error}",
            host.ip.to_string(),
            host.vendor,
            host.name.as_deref().unwrap_or("?"),
            host.inf1.as_deref().unwrap_or("?"),
            host.inf2.as_deref().unwrap_or("?"),
        );
    }
    if found.is_empty() {
        println!("No hosts with TCP 4352 open.");
    }
    let problems = discover::check(config, &found);
    if !problems.is_empty() {
        match config.network.ethernet_ip.parse() {
            Ok(listen) => {
                println!(
                    "Listening {} s for DHCP clients on {listen} (units left in DHCP mode)...",
                    DHCP_LISTEN.as_secs()
                );
                for client in dhcp::sniff(listen, DHCP_LISTEN) {
                    let request = &client.request;
                    println!(
                        "DHCP client {} vendor={} hostname={:?} class={:?} requested={} \
                         ({} requests, no DHCP server here -> set a static IP on the unit)",
                        dhcp::mac_text(&request.mac),
                        client.vendor,
                        request.hostname.as_deref().unwrap_or("-"),
                        request.vendor_class.as_deref().unwrap_or("-"),
                        request
                            .requested_ip
                            .map_or_else(|| "-".to_string(), |ip| ip.to_string()),
                        client.count
                    );
                }
            }
            Err(_) => println!(
                "network.ethernet_ip '{}' is not an address; not listening for DHCP clients",
                config.network.ethernet_ip
            ),
        }
    }
    for problem in &problems {
        println!("PROBLEM: {problem}");
    }
    problems.is_empty()
}
