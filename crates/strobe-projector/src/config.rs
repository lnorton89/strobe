// ============================================================================
// Module:       strobe_projector::config
// Description:  `projectors.json`: the projectors (name, address,
//               passwords, fallback), the projector network, and the web
//               page's bind, port and password. Loaded, checked and saved.
//
// Dependencies: serde, serde_json; strobe-diag (paths)
// ============================================================================

//! The configuration file (D502).
//!
//! It holds the page's password and the projectors' passwords, so it is never in
//! the repository: it lives in Strobe's data directory as
//! `projectors.json` ([`default_path`]), where Settings > Projectors edits
//! it, and the headless service is pointed at the same file with
//! `--config`. The shape is the Python tool's `config.json`, unchanged, so
//! an existing file moves over as it is; [`Config::strobe`] is the one
//! addition, Strobe's own switches, which the headless service ignores.
//!
//! [`Config::problems`] is the check both loading and Settings' Save run:
//! at least one projector, each with a name and an address, and a password
//! of at least [`MIN_PASSWORD`] characters before the page may be served
//! (D516). A file from before D516 has `api.token` instead, which is read as
//! the password until one is set.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

/// The file's name in Strobe's data directory.
pub const FILE_NAME: &str = "projectors.json";

/// The shortest password the web page accepts (D516): something a person
/// types on a phone, not a token, but not a single word either.
pub const MIN_PASSWORD: usize = 6;

/// The default web page port.
pub const DEFAULT_PORT: u16 = 8780;

/// Where Strobe keeps the file: its data directory (`%LOCALAPPDATA%\strobe`
/// on Windows, or `STROBE_DIAG_DIR`).
pub fn default_path() -> PathBuf {
    strobe_diag::paths::base_dir().join(FILE_NAME)
}

/// Why the configuration cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// The whole file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// The projectors by key (`panasonic`, `sony`): the key is what the
    /// CLI, the API and the audit log call a unit.
    #[serde(default)]
    pub projectors: BTreeMap<String, Projector>,
    /// The PC's network interfaces.
    #[serde(default)]
    pub network: Network,
    /// The web page.
    #[serde(default)]
    pub api: Api,
    /// Strobe's own switches.
    #[serde(default)]
    pub strobe: StrobeSettings,
}

/// One projector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projector {
    /// What people call it (`Panasonic PT-DZ6700`).
    pub name: String,
    /// `panasonic`, `sony` or empty: which manual steps discovery prints.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vendor: String,
    /// Its address on the projector network.
    pub ip: String,
    /// Always `pjlink`.
    #[serde(default = "pjlink")]
    pub protocol: String,
    /// The PJLink password; empty for none.
    #[serde(default)]
    pub pjlink_password: String,
    /// `ntcontrol` to fall back to Panasonic's protocol when PJLink fails.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fallback: String,
    /// The web interface user NTCONTROL authenticates as.
    #[serde(default = "ntcontrol_user", skip_serializing_if = "is_default_user")]
    pub ntcontrol_user: String,
    /// The web interface password.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ntcontrol_password: String,
    /// Its MAC address, for the DHCP reservation helper; empty for none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mac: String,
    /// PJLink's port when it is not 4352 (a unit behind a port forward).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pjlink_port: Option<u16>,
    /// NTCONTROL's port when it is not 1024.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ntcontrol_port: Option<u16>,
}

fn pjlink() -> String {
    "pjlink".to_string()
}

fn ntcontrol_user() -> String {
    crate::ntcontrol::DEFAULT_USER.to_string()
}

fn is_default_user(user: &String) -> bool {
    user == crate::ntcontrol::DEFAULT_USER
}

impl Projector {
    /// A projector at `ip` with no passwords and no fallback.
    pub fn new(name: &str, ip: &str) -> Self {
        Self {
            name: name.to_string(),
            vendor: String::new(),
            ip: ip.to_string(),
            protocol: pjlink(),
            pjlink_password: String::new(),
            fallback: String::new(),
            ntcontrol_user: ntcontrol_user(),
            ntcontrol_password: String::new(),
            mac: String::new(),
            pjlink_port: None,
            ntcontrol_port: None,
        }
    }

    /// Whether NTCONTROL takes over when PJLink fails.
    pub fn ntcontrol_fallback(&self) -> bool {
        self.fallback == "ntcontrol"
    }
}

/// The PC's network interfaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    /// The Wi-Fi adapter's name, whose address the page binds without
    /// Tailscale.
    #[serde(default = "wifi_alias")]
    pub wifi_alias: String,
    /// The projector Ethernet adapter's name.
    #[serde(default = "ethernet_alias")]
    pub ethernet_alias: String,
    /// The PC's address on the projector network.
    #[serde(default = "ethernet_ip")]
    pub ethernet_ip: String,
    /// The projector network, which the page never binds.
    #[serde(default = "projector_subnet")]
    pub projector_subnet: String,
}

fn wifi_alias() -> String {
    "Wi-Fi".to_string()
}
fn ethernet_alias() -> String {
    "Ethernet".to_string()
}
fn ethernet_ip() -> String {
    "192.168.50.1".to_string()
}
fn projector_subnet() -> String {
    "192.168.50.0/24".to_string()
}

impl Default for Network {
    fn default() -> Self {
        Self {
            wifi_alias: wifi_alias(),
            ethernet_alias: ethernet_alias(),
            ethernet_ip: ethernet_ip(),
            projector_subnet: projector_subnet(),
        }
    }
}

/// Which address the web page binds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bind {
    /// Tailscale's address when it is up, else Wi-Fi's.
    #[default]
    Auto,
    /// Tailscale's address only.
    Tailscale,
    /// Wi-Fi's address only.
    Wifi,
}

impl Bind {
    /// Every mode, in the order Settings offers them.
    pub const ALL: [Self; 3] = [Self::Auto, Self::Tailscale, Self::Wifi];

    /// The mode as the file spells it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Tailscale => "tailscale",
            Self::Wifi => "wifi",
        }
    }
}

/// The web page and API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Api {
    /// Which address to bind. Explicit addresses are not allowed: an
    /// address typed once goes stale with the next lease.
    #[serde(default)]
    pub bind: Bind,
    /// The TCP port.
    #[serde(default = "port")]
    pub port: u16,
    /// The password every request must carry (D516). Read from `token`
    /// in a file from before D516.
    #[serde(default, alias = "token")]
    pub password: String,
}

fn port() -> u16 {
    DEFAULT_PORT
}

impl Default for Api {
    fn default() -> Self {
        Self {
            bind: Bind::Auto,
            port: DEFAULT_PORT,
            password: String::new(),
        }
    }
}

/// Strobe's own switches (Settings > Projectors), saved with the rest so
/// they come back at launch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrobeSettings {
    /// Projector control is on: Strobe polls the units' status.
    #[serde(default)]
    pub enabled: bool,
    /// Strobe serves the web page while it runs.
    #[serde(default)]
    pub serve_web: bool,
}

impl Config {
    /// Reads and checks `path`.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let config = Self::read(path)?;
        if let Some(problem) = config.problems().into_iter().next() {
            return Err(ConfigError(format!("{}: {problem}", path.display())));
        }
        Ok(config)
    }

    /// Reads `path` without checking it: Settings opens a half-filled file
    /// to finish it, and an empty one (no projectors yet) is where it
    /// starts. A missing file is an empty configuration.
    pub fn read_or_default(path: &Path) -> Result<Self, ConfigError> {
        if path.exists() {
            Self::read(path)
        } else {
            Ok(Self::default())
        }
    }

    /// Reads and parses `path`.
    fn read(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            let path = path.display();
            ConfigError(if error.kind() == io::ErrorKind::NotFound {
                format!("config not found: {path}")
            } else if error.kind() == io::ErrorKind::PermissionDenied {
                format!("cannot read {path}: run from an elevated (Administrator) shell")
            } else {
                format!("cannot read {path}: {error}")
            })
        })?;
        serde_json::from_str(&text)
            .map_err(|error| ConfigError(format!("invalid JSON in {}: {error}", path.display())))
    }

    /// Writes the file whole: to a temporary file beside it, then renamed
    /// over it, so a crash mid-write never leaves half a file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, format!("{text}\n"))?;
        std::fs::rename(&temporary, path)
    }

    /// What is wrong with the configuration, for the CLI and Settings.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.projectors.is_empty() {
            problems.push("'projectors' section missing or empty".to_string());
        }
        for (key, projector) in &self.projectors {
            if !valid_key(key) {
                problems.push(format!(
                    "projector key '{key}' must be letters, digits or '_' (it is the API's name \
                     for the unit)"
                ));
            }
            if key == "all" {
                problems.push("'all' cannot be a projector's key".to_string());
            }
            if projector.name.trim().is_empty() {
                problems.push(format!("projector '{key}' missing 'name'"));
            }
            if projector.ip.trim().is_empty() {
                problems.push(format!("projector '{key}' missing 'ip'"));
            }
            if projector.protocol != "pjlink" {
                problems.push(format!(
                    "projector '{key}': protocol must be 'pjlink', not '{}'",
                    projector.protocol
                ));
            }
            if !projector.fallback.is_empty() && !projector.ntcontrol_fallback() {
                problems.push(format!(
                    "projector '{key}': fallback must be 'ntcontrol' or empty"
                ));
            }
            if !projector.mac.is_empty() && parse_mac(&projector.mac).is_none() {
                problems.push(format!(
                    "projector '{key}': mac '{}' is not six hex bytes",
                    projector.mac
                ));
            }
        }
        if Subnet::parse(&self.network.projector_subnet).is_none() {
            problems.push(format!(
                "network.projector_subnet '{}' is not an IPv4 network like 192.168.50.0/24",
                self.network.projector_subnet
            ));
        }
        problems
    }

    /// Why the web page cannot be served with this configuration, if it
    /// cannot.
    pub fn api_problem(&self) -> Option<String> {
        (self.api.password.chars().count() < MIN_PASSWORD)
            .then(|| format!("api.password missing or shorter than {MIN_PASSWORD} characters"))
    }

    /// The projector network.
    pub fn subnet(&self) -> Option<Subnet> {
        Subnet::parse(&self.network.projector_subnet)
    }
}

/// Whether `key` can name a unit in a URL path: ASCII letters, digits and
/// `_`, as the API's routes accept.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A MAC address written `04-5D-4B-96-32-7C`, `04:5d:...` or `045d4b...`.
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let digits: Vec<u8> = text.bytes().filter(|b| !matches!(b, b'-' | b':')).collect();
    if digits.len() != 12 {
        return None;
    }
    let mut mac = [0_u8; 6];
    for (slot, pair) in mac.iter_mut().zip(digits.chunks(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(mac)
}

/// An IPv4 network such as `192.168.50.0/24`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subnet {
    /// The network address (host bits cleared).
    pub network: Ipv4Addr,
    /// The prefix length.
    pub prefix: u8,
}

impl Subnet {
    /// Parses `a.b.c.d/n`; host bits are cleared.
    pub fn parse(text: &str) -> Option<Self> {
        let (address, prefix) = text.trim().split_once('/')?;
        let address: Ipv4Addr = address.parse().ok()?;
        let prefix: u8 = prefix.parse().ok()?;
        if prefix > 32 {
            return None;
        }
        Some(Self {
            network: Ipv4Addr::from(u32::from(address) & mask(prefix)),
            prefix,
        })
    }

    /// Whether `ip` is on this network.
    pub fn contains(self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & mask(self.prefix) == u32::from(self.network)
    }

    /// The host addresses (no network or broadcast address, below /31).
    pub fn hosts(self) -> Vec<Ipv4Addr> {
        let first = u32::from(self.network);
        let size = 1_u64 << (32 - u32::from(self.prefix));
        let (skip_ends, count) = if self.prefix >= 31 {
            (0, size)
        } else {
            (1, size.saturating_sub(2))
        };
        (0..count)
            .filter_map(|offset| u32::try_from(offset + skip_ends).ok())
            .map(|offset| Ipv4Addr::from(first.wrapping_add(offset)))
            .collect()
    }
}

/// The netmask for `prefix` bits.
const fn mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// The repository's example, as the installer starts from it.
    const EXAMPLE: &str = include_str!("../../../tools/projector-control/config.example.json");

    #[test]
    fn the_example_loads_and_round_trips() -> Result<()> {
        let config: Config = serde_json::from_str(EXAMPLE)?;
        assert_eq!(config.projectors.len(), 2, "two projectors");
        let panasonic = config
            .projectors
            .get("panasonic")
            .ok_or_else(|| anyhow::anyhow!("panasonic"))?;
        assert!(panasonic.ntcontrol_fallback(), "Panasonic falls back");
        assert_eq!(panasonic.ntcontrol_user, "admin1", "user");
        assert_eq!(config.api.bind, Bind::Auto, "bind");
        assert_eq!(config.api.port, 8780, "port");
        assert!(config.problems().is_empty(), "{:?}", config.problems());
        let again: Config = serde_json::from_str(&serde_json::to_string(&config)?)?;
        assert_eq!(again, config, "round trip");
        Ok(())
    }

    #[test]
    fn a_python_era_file_with_a_mac_and_no_fallback_loads() -> Result<()> {
        let text = r#"{"projectors": {"sony": {"name": "Sony", "vendor": "sony",
            "ip": "192.168.50.11", "protocol": "pjlink", "pjlink_password": "x",
            "mac": "04-5D-4B-96-32-7C"}},
            "api": {"bind": "auto", "port": 8780, "token": "abcdefghijklmnopqrstuvwxyz0123456789-_ABCDE"}}"#;
        let config: Config = serde_json::from_str(text)?;
        assert!(config.problems().is_empty(), "{:?}", config.problems());
        assert_eq!(
            config.api_problem(),
            None,
            "a token from before D516 is the password"
        );
        assert_eq!(config.api.password.len(), 43, "read from api.token");
        assert!(!config.strobe.enabled, "Strobe's switches default off");
        Ok(())
    }

    #[test]
    fn problems_are_named() -> Result<()> {
        let mut config = Config::default();
        assert!(!config.problems().is_empty(), "empty");
        config
            .projectors
            .insert("a b".into(), Projector::new("", ""));
        let problems = config.problems().join("\n");
        for word in ["key 'a b'", "missing 'name'", "missing 'ip'"] {
            assert!(problems.contains(word), "{word}: {problems}");
        }
        let bad_bind: Result<Config, _> = serde_json::from_str(r#"{"api": {"bind": "0.0.0.0"}}"#);
        assert!(bad_bind.is_err(), "an explicit address is refused");
        assert!(config.api_problem().is_some(), "no password");
        config.api.password = "short".into();
        assert!(config.api_problem().is_some(), "too short");
        config.api.password = "jazz-hands".into();
        assert_eq!(config.api_problem(), None, "a typed password serves");
        Ok(())
    }

    #[test]
    fn macs_and_subnets_parse() -> Result<()> {
        let expected = Some([0x04, 0x5d, 0x4b, 0x96, 0x32, 0x7c]);
        assert_eq!(parse_mac("04-5D-4B-96-32-7C"), expected, "dashes");
        assert_eq!(parse_mac("04:5d:4b:96:32:7c"), expected, "colons");
        assert_eq!(parse_mac("045d4b96327c"), expected, "bare");
        assert_eq!(parse_mac("04-5D-4B"), None, "short");
        let subnet = Subnet::parse("192.168.50.7/24").ok_or_else(|| anyhow::anyhow!("subnet"))?;
        assert_eq!(subnet.network, Ipv4Addr::new(192, 168, 50, 0), "host bits");
        assert!(subnet.contains(Ipv4Addr::new(192, 168, 50, 10)), "inside");
        assert!(!subnet.contains(Ipv4Addr::new(10, 1, 10, 155)), "outside");
        let hosts = subnet.hosts();
        assert_eq!(hosts.len(), 254, "hosts");
        assert_eq!(
            hosts.first(),
            Some(&Ipv4Addr::new(192, 168, 50, 1)),
            "first"
        );
        assert_eq!(
            hosts.last(),
            Some(&Ipv4Addr::new(192, 168, 50, 254)),
            "last"
        );
        assert_eq!(Subnet::parse("1.2.3.4/33"), None, "prefix too long");
        Ok(())
    }

    #[test]
    fn save_then_load_is_the_same() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("strobe-projector-config-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let mut config: Config = serde_json::from_str(EXAMPLE)?;
        config.strobe.enabled = true;
        config.save(&path)?;
        let loaded = Config::load(&path)?;
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(loaded, config, "saved and loaded");
        Ok(())
    }
}
