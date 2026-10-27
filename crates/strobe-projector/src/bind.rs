// ============================================================================
// Module:       strobe_projector::bind
// Description:  Which address the web page binds: Tailscale's IPv4 when it
//               is up, else the Wi-Fi adapter's, and never every interface,
//               loopback, link-local or the projector network.
//
// Dependencies: std::process (tailscale, powershell); crate::config
// ============================================================================

//! The bind address.
//!
//! The page switches projectors, so it listens on one address a person
//! reaches deliberately: the tailnet (remote access) or the venue Wi-Fi,
//! never `0.0.0.0`, never the projector network, never loopback (a page
//! nobody can open is a configuration mistake worth saying so). The address
//! is asked of the system rather than typed into the config, because a
//! typed address goes stale with the next DHCP lease; [`crate::service`]
//! asks again every 30 s and moves.
//!
//! Tailscale's address comes from `tailscale ip -4`; the Wi-Fi adapter's
//! from PowerShell's `Get-NetIPAddress` for the configured alias, skipping
//! a self-assigned `169.254` address. Both are Windows-first; elsewhere
//! the commands are missing and no address is found.

use crate::config::{Bind, Config};
use std::net::Ipv4Addr;
use std::process::Command;

/// Tailscale's address range (RFC 6598's shared space, which Tailscale
/// assigns from).
const TAILNET: (Ipv4Addr, u8) = (Ipv4Addr::new(100, 64, 0, 0), 10);

/// Where Tailscale installs on Windows, when it is not on the PATH (the
/// SYSTEM account's PATH does not always have it).
const TAILSCALE_EXE: &str = r"C:\Program Files\Tailscale\tailscale.exe";

/// Which interface an address came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The tailnet.
    Tailscale,
    /// The Wi-Fi adapter.
    Wifi,
}

impl Kind {
    /// The interface's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tailscale => "tailscale",
            Self::Wifi => "wifi",
        }
    }
}

/// Stops a child process from flashing a console window over the output
/// when Strobe (a windowed program) asks.
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`.
    const FLAGS: u32 = 0x0800_0000;
    command.creation_flags(FLAGS);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

/// Runs `program` with `args` and returns its standard output, or `None`
/// when it cannot run or fails.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    hide_console(&mut command);
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether `ip` is a tailnet address.
pub fn in_tailnet(ip: Ipv4Addr) -> bool {
    let (network, prefix) = TAILNET;
    let mask = u32::MAX << (32 - u32::from(prefix));
    u32::from(ip) & mask == u32::from(network)
}

/// The first tailnet address in `tailscale ip -4`'s output.
pub fn parse_tailscale(output: &str) -> Option<Ipv4Addr> {
    output
        .split_whitespace()
        .filter_map(|word| word.parse().ok())
        .find(|ip| in_tailnet(*ip))
}

/// This machine's tailnet address, if Tailscale is installed and up.
pub fn tailscale_ip() -> Option<Ipv4Addr> {
    let args = ["ip", "-4"];
    output("tailscale", &args)
        .or_else(|| output(TAILSCALE_EXE, &args))
        .and_then(|text| parse_tailscale(&text))
}

/// The usable IPv4 address of the adapter called `alias`.
pub fn wifi_ip(alias: &str) -> Option<Ipv4Addr> {
    // A single quote in the alias is doubled: PowerShell's own escape.
    let alias = alias.replace('\'', "''");
    let script = format!(
        "(Get-NetIPAddress -InterfaceAlias '{alias}' -AddressFamily IPv4 \
         -ErrorAction SilentlyContinue | Where-Object {{ $_.AddressState -eq 'Preferred' \
         -and $_.IPAddress -notlike '169.254.*' }} | Select-Object -First 1).IPAddress"
    );
    output(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .and_then(|text| text.trim().parse().ok())
}

/// The address to bind under `config`, and where it came from; `None`
/// when that interface has no address yet.
pub fn resolve(config: &Config) -> (Option<Ipv4Addr>, Kind) {
    resolve_with(config, tailscale_ip, wifi_ip)
}

/// [`resolve`] with the two lookups given (tests).
pub fn resolve_with(
    config: &Config,
    tailscale: impl Fn() -> Option<Ipv4Addr>,
    wifi: impl Fn(&str) -> Option<Ipv4Addr>,
) -> (Option<Ipv4Addr>, Kind) {
    match config.api.bind {
        Bind::Auto => match tailscale() {
            Some(ip) => (Some(ip), Kind::Tailscale),
            None => (wifi(&config.network.wifi_alias), Kind::Wifi),
        },
        Bind::Tailscale => (tailscale(), Kind::Tailscale),
        Bind::Wifi => (wifi(&config.network.wifi_alias), Kind::Wifi),
    }
}

/// Whether the page may listen on `ip`: not every interface, loopback,
/// link-local, or the projector network.
pub fn safe(ip: Ipv4Addr, config: &Config) -> bool {
    let projector_network = config.subnet().is_some_and(|subnet| subnet.contains(ip));
    !(ip.is_unspecified() || ip.is_loopback() || ip.is_link_local() || projector_network)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailscale_output_yields_the_tailnet_address() {
        assert_eq!(
            parse_tailscale("100.101.7.3\n"),
            Some(Ipv4Addr::new(100, 101, 7, 3)),
            "plain"
        );
        assert_eq!(parse_tailscale("10.0.0.1\n"), None, "not a tailnet address");
        assert_eq!(parse_tailscale("Tailscale is stopped.\n"), None, "down");
    }

    #[test]
    fn only_a_reachable_lan_address_is_safe() {
        let config = Config::default();
        assert!(
            safe(Ipv4Addr::new(10, 1, 10, 155), &config),
            "the venue Wi-Fi"
        );
        assert!(safe(Ipv4Addr::new(100, 101, 7, 3), &config), "the tailnet");
        for ip in [
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::new(169, 254, 1, 1),
            Ipv4Addr::new(192, 168, 50, 1),
        ] {
            assert!(!safe(ip, &config), "{ip} must not be bound");
        }
    }

    #[test]
    fn auto_prefers_tailscale_and_falls_back_to_wifi() {
        let mut config = Config::default();
        let tailnet = Ipv4Addr::new(100, 64, 1, 2);
        let wifi = Ipv4Addr::new(10, 1, 10, 155);
        let both = resolve_with(&config, || Some(tailnet), |_| Some(wifi));
        assert_eq!(
            both,
            (Some(tailnet), Kind::Tailscale),
            "auto with Tailscale"
        );
        let no_tailscale = resolve_with(&config, || None, |_| Some(wifi));
        assert_eq!(no_tailscale, (Some(wifi), Kind::Wifi), "auto without");
        config.api.bind = Bind::Tailscale;
        let strict = resolve_with(&config, || None, |_| Some(wifi));
        assert_eq!(strict, (None, Kind::Tailscale), "Tailscale only waits");
    }
}
