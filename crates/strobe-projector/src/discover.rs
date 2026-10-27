// ============================================================================
// Module:       strobe_projector::discover
// Description:  Finding the projectors: every host on the projector network
//               with PJLink's port open (plus the ARP table's neighbours),
//               what each says it is, and what is missing or misplaced.
//
// Dependencies: std::net, std::process (arp); crate::{config, error, pjlink}
// ============================================================================

//! `strobe-projector discover`.
//!
//! The scan connects to TCP 4352 on every host of the projector network,
//! and on every neighbour the ARP table lists on that interface (a unit on
//! a link-local or leftover DHCP address is in the table but not the
//! subnet), [`SCAN_THREADS`] at a time. Each open host is asked its name
//! and maker, trying no password and then each configured one. [`check`]
//! then compares what was found with the config and says, per projector,
//! what to set on the unit's own menu.

use crate::config::{Config, Subnet};
use crate::error::PjLinkCode;
use crate::pjlink::{self, PjLink};
use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// Connections tried at once.
const SCAN_THREADS: usize = 64;

/// How long a scan's connect waits.
const PROBE_TIMEOUT: Duration = Duration::from_millis(600);

/// What to set on a unit's own menu, by vendor.
const MANUAL_STEPS: [(&str, &str); 2] = [
    (
        "panasonic",
        "Panasonic PT-DZ6700: MENU > NETWORK > LAN SETTINGS (Wired LAN): DHCP = OFF, \
         IP ADDRESS = 192.168.50.10, SUBNET MASK = 255.255.255.0, (gateway blank) > STORE.",
    ),
    (
        "sony",
        "Sony VPL-PHZ10: MENU > Connection/Power > Network Settings: DHCP = Off, \
         IP Address = 192.168.50.11, Subnet Mask = 255.255.255.0 > Apply.",
    ),
];

/// A host with PJLink's port open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Its address.
    pub ip: Ipv4Addr,
    /// `NAME ?`.
    pub name: Option<String>,
    /// `INF1 ?` (the maker).
    pub inf1: Option<String>,
    /// `INF2 ?` (the model).
    pub inf2: Option<String>,
    /// Why it could not be asked, if it could not.
    pub error: Option<String>,
    /// `panasonic`, `sony` or `unknown`.
    pub vendor: &'static str,
}

impl Found {
    /// A host at `ip` not yet asked anything.
    pub const fn at(ip: Ipv4Addr) -> Self {
        Self {
            ip,
            name: None,
            inf1: None,
            inf2: None,
            error: None,
            vendor: "unknown",
        }
    }
}

/// Whether `ip:port` accepts a connection within the probe timeout.
pub fn port_open(ip: Ipv4Addr, port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from((ip, port)), PROBE_TIMEOUT).is_ok()
}

/// The dynamic IPv4 neighbours `arp -a`'s `output` lists on interfaces in
/// `subnet` (Windows' format: an `Interface: a.b.c.d` line opens each
/// section).
pub fn arp_neighbours(output: &str, subnet: Subnet) -> BTreeSet<Ipv4Addr> {
    let mut found = BTreeSet::new();
    let mut in_section = false;
    for line in output.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("Interface:") {
            in_section = rest
                .split_whitespace()
                .next()
                .and_then(|ip| ip.parse::<Ipv4Addr>().ok())
                .is_some_and(|ip| subnet.contains(ip));
            continue;
        }
        if !in_section || !line.starts_with(char::is_whitespace) {
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(ip), Some(mac), Some(kind)) = (words.next(), words.next(), words.next()) else {
            continue;
        };
        let mac_like = mac.len() == 17 && mac.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        if mac_like
            && kind.eq_ignore_ascii_case("dynamic")
            && let Ok(ip) = ip.parse()
        {
            found.insert(ip);
        }
    }
    found
}

/// The ARP table's neighbours on `subnet`, or none when `arp` cannot run.
fn neighbours(subnet: Subnet) -> BTreeSet<Ipv4Addr> {
    match std::process::Command::new("arp").arg("-a").output() {
        Ok(output) => arp_neighbours(&String::from_utf8_lossy(&output.stdout), subnet),
        Err(error) => {
            log::info!("arp -a: {error}");
            BTreeSet::new()
        }
    }
}

/// The hosts among `candidates` with `port` open, in address order.
fn open_hosts(candidates: Vec<Ipv4Addr>, port: u16) -> Vec<Ipv4Addr> {
    let queue = Mutex::new(candidates);
    let open = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..SCAN_THREADS {
            scope.spawn(|| {
                loop {
                    let next = queue.lock().ok().and_then(|mut queue| queue.pop());
                    let Some(ip) = next else {
                        break;
                    };
                    if port_open(ip, port)
                        && let Ok(mut open) = open.lock()
                    {
                        open.push(ip);
                    }
                }
            });
        }
    });
    let mut open = open.into_inner().unwrap_or_default();
    open.sort_unstable();
    open
}

/// Asks the host at `ip` its name, maker and model, trying each password.
fn identify(ip: Ipv4Addr, passwords: &[String]) -> Found {
    let mut found = Found::at(ip);
    for password in passwords {
        let client = PjLink::new(&ip.to_string(), password);
        let answers = client
            .name()
            .and_then(|name| Ok((name, client.inf1()?, client.inf2()?)));
        match answers {
            Ok((name, inf1, inf2)) => {
                found.name = Some(name);
                found.inf1 = Some(inf1);
                found.inf2 = Some(inf2);
                found.error = None;
                break;
            }
            Err(error) => {
                let wrong_password = error.pjlink_code() == Some(PjLinkCode::Erra);
                found.error = Some(error.to_string());
                if !wrong_password {
                    break;
                }
            }
        }
    }
    let said = [&found.name, &found.inf1, &found.inf2]
        .into_iter()
        .flatten()
        .map(|text| text.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    found.vendor = if said.contains("panasonic") {
        "panasonic"
    } else if said.contains("sony") {
        "sony"
    } else {
        "unknown"
    };
    found
}

/// Scans `subnet` (the projector network) and identifies every PJLink host
/// but this PC.
pub fn discover(config: &Config, subnet: Subnet) -> Vec<Found> {
    let mut candidates: BTreeSet<Ipv4Addr> = subnet.hosts().into_iter().collect();
    candidates.extend(neighbours(subnet));
    if let Ok(local) = config.network.ethernet_ip.parse() {
        candidates.remove(&local);
    }
    let open = open_hosts(candidates.into_iter().collect(), pjlink::PORT);
    let mut passwords: Vec<String> = vec![String::new()];
    for projector in config.projectors.values() {
        if !passwords.contains(&projector.pjlink_password) {
            passwords.push(projector.pjlink_password.clone());
        }
    }
    open.into_iter()
        .map(|ip| identify(ip, &passwords))
        .collect()
}

/// What is wrong, per configured projector: not found, or found somewhere
/// else. Each problem ends with the steps to fix it on the unit.
pub fn check(config: &Config, found: &[Found]) -> Vec<String> {
    let mut problems = Vec::new();
    for (key, projector) in &config.projectors {
        let vendor = if projector.vendor.is_empty() {
            key.as_str()
        } else {
            projector.vendor.as_str()
        };
        let at_address = found
            .iter()
            .find(|host| host.ip.to_string() == projector.ip);
        if at_address.is_some_and(|host| host.vendor == vendor || host.vendor == "unknown") {
            continue;
        }
        let steps = MANUAL_STEPS
            .iter()
            .find(|(name, _)| *name == vendor)
            .map_or("", |(_, steps)| steps);
        let elsewhere = found.iter().find(|host| host.vendor == vendor);
        problems.push(match elsewhere {
            Some(host) => format!(
                "{} found at {} but expected {}.\n  -> {steps}",
                projector.name, host.ip, projector.ip
            ),
            None => format!(
                "{} not found (expected at {}).\n  -> {steps}",
                projector.name, projector.ip
            ),
        });
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};

    const ARP: &str = "\r
Interface: 10.1.10.155 --- 0x7\r
  Internet Address      Physical Address      Type\r
  10.1.10.1             aa-bb-cc-dd-ee-ff     dynamic\r
\r
Interface: 192.168.50.1 --- 0xc\r
  Internet Address      Physical Address      Type\r
  169.254.20.7          04-5d-4b-96-32-7c     dynamic\r
  192.168.50.10         00-80-9f-00-00-01     dynamic\r
  192.168.50.255        ff-ff-ff-ff-ff-ff     static\r
";

    #[test]
    fn arp_neighbours_come_from_the_projector_interface_only() -> Result<()> {
        let subnet = Subnet::parse("192.168.50.0/24").context("subnet")?;
        let found: Vec<Ipv4Addr> = arp_neighbours(ARP, subnet).into_iter().collect();
        assert_eq!(
            found,
            vec![
                Ipv4Addr::new(169, 254, 20, 7),
                Ipv4Addr::new(192, 168, 50, 10)
            ],
            "the link-local Sony and the Panasonic, not the router or a static entry"
        );
        Ok(())
    }

    #[test]
    fn check_names_the_missing_and_the_misplaced() -> Result<()> {
        let text = include_str!("../../../tools/projector-control/config.example.json");
        let config: Config = serde_json::from_str(text)?;
        let panasonic = Found {
            ip: Ipv4Addr::new(192, 168, 50, 10),
            vendor: "panasonic",
            ..Found::at(Ipv4Addr::UNSPECIFIED)
        };
        let stray_sony = Found {
            ip: Ipv4Addr::new(192, 168, 50, 99),
            vendor: "sony",
            ..Found::at(Ipv4Addr::UNSPECIFIED)
        };
        assert_eq!(
            check(&config, std::slice::from_ref(&panasonic)).len(),
            1,
            "only the Sony is missing"
        );
        let problems = check(&config, &[panasonic, stray_sony]);
        let [problem] = problems.as_slice() else {
            anyhow::bail!("one problem expected: {problems:?}");
        };
        assert!(
            problem.contains("found at 192.168.50.99 but expected 192.168.50.11"),
            "{problem}"
        );
        assert!(problem.contains("Sony VPL-PHZ10: MENU"), "steps: {problem}");
        Ok(())
    }
}
