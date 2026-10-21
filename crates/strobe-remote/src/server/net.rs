// ============================================================================
// Module:       strobe_remote::server::net
// Description:  Which network an address is on (loopback, private LAN,
//               link-local, every interface, public), finding this machine's
//               LAN address, and reading a Host header.
//
// Dependencies: std::net
// ============================================================================

//! Addresses (L-02, L-04, D252).
//!
//! The listener binds one address the operator chose, or this machine's LAN
//! address when they chose none. [`scope`] sorts addresses into the
//! networks the remote cares about: a public address is refused outright
//! (the remote is for a trusted LAN, never the internet), every interface
//! (`0.0.0.0`) is allowed with a warning, and a peer that connects from a
//! public address is turned away whatever was bound, so a port forward on
//! the router does not expose the show.
//!
//! [`lan_address`] asks the operating system which local address it would
//! use to reach a documentation address (`192.0.2.1`, TEST-NET-1) over UDP.
//! A UDP "connect" sends nothing; it only picks a route. That is the
//! address a phone on the same Wi-Fi reaches, without a crate to list
//! interfaces.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

/// What kind of network an address belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// This machine only.
    Loopback,
    /// A private LAN: RFC 1918, carrier-grade NAT (RFC 6598, which VPN
    /// meshes use too) or IPv6 unique-local.
    Private,
    /// Link-local (`169.254/16`, `fe80::/10`).
    LinkLocal,
    /// `0.0.0.0` or `::`: every interface, including any public one.
    Unspecified,
    /// Anything else: reachable from the internet.
    Public,
}

/// The network `ip` is on.
pub fn scope(ip: IpAddr) -> Scope {
    match ip {
        IpAddr::V4(v4) => scope_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => scope_v4(v4),
            None => scope_v6(v6),
        },
    }
}

fn scope_v4(ip: Ipv4Addr) -> Scope {
    let [a, b, ..] = ip.octets();
    if ip.is_unspecified() {
        Scope::Unspecified
    } else if ip.is_loopback() {
        Scope::Loopback
    } else if ip.is_private() || (a == 100 && (64..128).contains(&b)) {
        Scope::Private
    } else if ip.is_link_local() {
        Scope::LinkLocal
    } else {
        Scope::Public
    }
}

fn scope_v6(ip: Ipv6Addr) -> Scope {
    let first = ip.segments()[0];
    if ip.is_unspecified() {
        Scope::Unspecified
    } else if ip.is_loopback() {
        Scope::Loopback
    } else if first & 0xfe00 == 0xfc00 {
        Scope::Private
    } else if first & 0xffc0 == 0xfe80 {
        Scope::LinkLocal
    } else {
        Scope::Public
    }
}

/// This machine's address on its LAN, if it has one that is private.
pub fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    matches!(scope(ip), Scope::Private | Scope::LinkLocal).then_some(ip)
}

/// Whether `ip` is still one of this machine's addresses: a socket can be
/// bound to it. An address DHCP took back, or the self-assigned
/// `169.254.x.x` one Windows drops when DHCP answers, cannot be (the
/// system says the address is not available). Any other failure is taken
/// as "still here", so a refusal for some other reason never moves a
/// listener that works (D461).
pub fn is_local_address(ip: IpAddr) -> bool {
    match UdpSocket::bind(SocketAddr::new(ip, 0)) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::AddrNotAvailable,
    }
}

/// Whether a `Host` header names the listener at `bound`, by address:
/// an IP literal (or `localhost` for a loopback bind) and the bound port.
/// A host *name* is refused. That is what stops DNS rebinding: a page on
/// `evil.example` that re-points its name at this machine still sends
/// `Host: evil.example`.
pub fn host_matches(host: &str, bound: SocketAddr) -> bool {
    let Some((name, port)) = split_host(host) else {
        return false;
    };
    if port != bound.port() {
        return false;
    }
    let bound_ip = bound.ip();
    if name.eq_ignore_ascii_case("localhost") {
        return bound_ip.is_loopback() || bound_ip.is_unspecified();
    }
    let Ok(ip) = name.parse::<IpAddr>() else {
        return false;
    };
    if bound_ip.is_unspecified() {
        true
    } else {
        ip == bound_ip
    }
}

/// Whether a `Host` header names `local`, the address this one connection
/// actually arrived at: the same IP (a v4-mapped spelling counts as the v4
/// address) and port, or `localhost` when it arrived on loopback.
///
/// [`host_matches`] is about the bind, and a listener on every interface
/// takes any literal there. A trusted pairing's proof is bound to the
/// `Host` (D288), so it needs this stricter check: otherwise a relay at
/// another address could forward a controller's handshake to this host
/// under the relay's own `Host`, and the proof would hold.
pub fn host_is_local(host: &str, local: SocketAddr) -> bool {
    let Some((name, port)) = split_host(host) else {
        return false;
    };
    if port != local.port() {
        return false;
    }
    let local_ip = local.ip().to_canonical();
    if name.eq_ignore_ascii_case("localhost") {
        return local_ip.is_loopback();
    }
    name.parse::<IpAddr>()
        .is_ok_and(|ip| ip.to_canonical() == local_ip)
}

/// `host:port` or `[v6]:port` into its parts. A missing port is HTTP's 80.
fn split_host(host: &str) -> Option<(&str, u16)> {
    if host.is_empty()
        || host
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b == b'@' || b == b'/')
    {
        return None;
    }
    if let Some(rest) = host.strip_prefix('[') {
        let (name, after) = rest.split_once(']')?;
        let port = match after {
            "" => 80,
            other => other.strip_prefix(':')?.parse().ok()?,
        };
        return Some((name, port));
    }
    match host.rsplit_once(':') {
        Some((name, port)) => Some((name, port.parse().ok()?)),
        None => Some((host, 80)),
    }
}

/// The address to show for `bound`: itself, or for every interface the LAN
/// address (loopback if there is none).
pub fn display_ip(bound: SocketAddr) -> IpAddr {
    if bound.ip().is_unspecified() {
        lan_address().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
    } else {
        bound.ip()
    }
}

/// `http://ip:port/`, brackets and all.
pub fn url(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(v4) => format!("http://{v4}:{port}/"),
        IpAddr::V6(v6) => format!("http://[{v6}]:{port}/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn ip(text: &str) -> Result<IpAddr> {
        Ok(text.parse()?)
    }

    #[test]
    fn addresses_are_sorted_by_network() -> Result<()> {
        for (text, expected) in [
            ("127.0.0.1", Scope::Loopback),
            ("::1", Scope::Loopback),
            ("10.1.2.3", Scope::Private),
            ("172.16.0.1", Scope::Private),
            ("172.31.255.255", Scope::Private),
            ("192.168.1.20", Scope::Private),
            ("100.64.0.1", Scope::Private),
            ("fd12:3456::1", Scope::Private),
            ("::ffff:192.168.1.2", Scope::Private),
            ("169.254.10.10", Scope::LinkLocal),
            ("fe80::1", Scope::LinkLocal),
            ("0.0.0.0", Scope::Unspecified),
            ("::", Scope::Unspecified),
            ("8.8.8.8", Scope::Public),
            ("172.32.0.1", Scope::Public),
            ("100.128.0.1", Scope::Public),
            ("2001:db8::1", Scope::Public),
            ("::ffff:8.8.8.8", Scope::Public),
        ] {
            assert_eq!(scope(ip(text)?), expected, "{text}");
        }
        Ok(())
    }

    #[test]
    fn the_host_must_be_the_bound_address_and_port() -> Result<()> {
        let lan: SocketAddr = "192.168.1.20:47812".parse()?;
        assert!(host_matches("192.168.1.20:47812", lan), "exact");
        assert!(!host_matches("192.168.1.21:47812", lan), "another address");
        assert!(!host_matches("192.168.1.20:80", lan), "another port");
        assert!(!host_matches("192.168.1.20", lan), "no port is port 80");
        assert!(
            !host_matches("evil.example:47812", lan),
            "a name (DNS rebinding)"
        );
        assert!(
            !host_matches("localhost:47812", lan),
            "localhost is not this bind"
        );
        assert!(!host_matches("", lan), "empty");
        assert!(!host_matches("user@192.168.1.20:47812", lan), "userinfo");
        assert!(!host_matches("192.168.1.20:47812:1", lan), "garbage");

        let any: SocketAddr = "0.0.0.0:47812".parse()?;
        assert!(
            host_matches("10.0.0.5:47812", any),
            "every interface takes any literal"
        );
        assert!(host_matches("[fe80::1]:47812", any), "v6 literal");
        assert!(!host_matches("strobe.local:47812", any), "never a name");

        let loopback: SocketAddr = "127.0.0.1:47812".parse()?;
        assert!(
            host_matches("localhost:47812", loopback),
            "localhost on loopback"
        );
        assert!(host_matches("LOCALHOST:47812", loopback), "any case");
        Ok(())
    }

    #[test]
    fn a_host_is_local_only_at_the_address_the_connection_arrived_at() -> Result<()> {
        let local: SocketAddr = "192.168.1.20:47812".parse()?;
        assert!(host_is_local("192.168.1.20:47812", local), "exact");
        assert!(
            !host_is_local("10.0.0.5:47812", local),
            "another literal, which a bind on every interface would take"
        );
        assert!(!host_is_local("192.168.1.20:47813", local), "another port");
        assert!(
            !host_is_local("localhost:47812", local),
            "localhost, from off the machine"
        );
        assert!(!host_is_local("strobe.local:47812", local), "a name");
        let mapped: SocketAddr = "[::ffff:192.168.1.20]:47812".parse()?;
        assert!(
            host_is_local("192.168.1.20:47812", mapped),
            "a dual-stack socket's v4-mapped local address"
        );
        let loopback: SocketAddr = "127.0.0.1:47812".parse()?;
        assert!(host_is_local("LOCALHOST:47812", loopback), "localhost");
        let v6: SocketAddr = "[fe80::1]:47812".parse()?;
        assert!(host_is_local("[fe80::1]:47812", v6), "v6");
        Ok(())
    }

    #[test]
    fn an_address_this_machine_does_not_have_is_told_apart() -> Result<()> {
        assert!(
            is_local_address(ip("127.0.0.1")?),
            "loopback is always this machine's"
        );
        assert!(
            !is_local_address(ip("192.0.2.1")?),
            "a documentation address (TEST-NET-1) is never assigned"
        );
        Ok(())
    }

    #[test]
    fn urls_bracket_v6() -> Result<()> {
        assert_eq!(url(ip("10.0.0.2")?, 1), "http://10.0.0.2:1/", "v4");
        assert_eq!(url(ip("fe80::1")?, 2), "http://[fe80::1]:2/", "v6");
        Ok(())
    }
}
