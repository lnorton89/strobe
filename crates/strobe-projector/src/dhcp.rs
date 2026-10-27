// ============================================================================
// Module:       strobe_projector::dhcp
// Description:  DHCP on the projector network: reading a client's DISCOVER
//               or REQUEST, listening for units stuck asking for a lease,
//               and answering only the reserved projectors.
//
// Dependencies: std::net; crate::{audit, config}
// ============================================================================

//! DHCP, for a unit left in DHCP mode on a LAN with no DHCP server.
//!
//! Such a unit never appears in an address scan; it only broadcasts
//! DISCOVERs. [`sniff`] listens for them on the PC's projector address
//! (UDP 67), sends nothing, and reports each client's MAC, vendor, host
//! name and the address it asks for. [`serve`] is the rescue: a minimal
//! responder that answers **only** MACs reserved in the config (a
//! projector's `mac`) with that projector's address, a /24 mask, and no
//! router or DNS, so nothing on the projector network can take the PC's
//! default route. A static address on the unit is the permanent fix.

use crate::audit::Audit;
use crate::config::{self, Config};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

/// The DHCP magic cookie that starts the options.
pub const MAGIC: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

/// The server port.
pub const SERVER_PORT: u16 = 67;

/// The client port.
pub const CLIENT_PORT: u16 = 68;

/// The lease offered, in seconds.
pub const LEASE_S: u32 = 86_400;

/// Where the options start.
const OPTIONS: usize = 240;

/// Message types (option 53).
pub mod kind {
    /// A client looking for a server.
    pub const DISCOVER: u8 = 1;
    /// The server's offer.
    pub const OFFER: u8 = 2;
    /// A client taking an offer.
    pub const REQUEST: u8 = 3;
    /// The server's acknowledgement.
    pub const ACK: u8 = 5;
}

/// Known projector vendors by the first three bytes of a MAC.
const OUI_VENDORS: [([u8; 3], &str); 7] = [
    ([0x04, 0x5d, 0x4b], "sony"),
    ([0x08, 0x00, 0x46], "sony"),
    ([0x00, 0x1d, 0x0d], "sony"),
    ([0x00, 0x80, 0x9f], "panasonic"),
    ([0x00, 0x80, 0xf0], "panasonic"),
    ([0x00, 0x50, 0xe1], "panasonic"),
    ([0x8c, 0x5a, 0xb0], "panasonic"),
];

/// A client's DISCOVER or REQUEST, as much as is needed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// The client's MAC.
    pub mac: [u8; 6],
    /// The message type (option 53), if sent.
    pub kind: Option<u8>,
    /// The host name (option 12).
    pub hostname: Option<String>,
    /// The vendor class (option 60).
    pub vendor_class: Option<String>,
    /// The address asked for (option 50).
    pub requested_ip: Option<Ipv4Addr>,
}

/// `packet` as a client request, or `None` when it is not one.
pub fn parse(packet: &[u8]) -> Option<Request> {
    if packet.len() < OPTIONS || packet.first() != Some(&1) || packet.get(236..OPTIONS)? != MAGIC {
        return None;
    }
    let mut request = Request {
        mac: packet.get(28..34)?.try_into().ok()?,
        ..Request::default()
    };
    let mut at = OPTIONS;
    while let Some(&code) = packet.get(at) {
        if code == 255 {
            break;
        }
        if code == 0 {
            at += 1;
            continue;
        }
        let Some(&length) = packet.get(at + 1) else {
            break;
        };
        let length = usize::from(length);
        let Some(value) = packet.get(at + 2..at + 2 + length) else {
            break;
        };
        match (code, value) {
            (53, [kind, ..]) => request.kind = Some(*kind),
            (12, _) => request.hostname = Some(String::from_utf8_lossy(value).into_owned()),
            (60, _) => request.vendor_class = Some(String::from_utf8_lossy(value).into_owned()),
            (50, [a, b, c, d]) => request.requested_ip = Some(Ipv4Addr::new(*a, *b, *c, *d)),
            _ => {}
        }
        at += 2 + length;
    }
    Some(request)
}

/// `mac` as `04-5D-4B-96-32-7C`.
pub fn mac_text(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("-")
}

/// The vendor a request's MAC prefix or vendor class names.
pub fn vendor(request: &Request) -> &'static str {
    if let Some(class) = &request.vendor_class {
        let class = class.to_ascii_lowercase();
        if class.contains("sony") {
            return "sony";
        }
        if class.contains("panasonic") {
            return "panasonic";
        }
    }
    let prefix = [request.mac[0], request.mac[1], request.mac[2]];
    OUI_VENDORS
        .iter()
        .find(|(oui, _)| *oui == prefix)
        .map_or("unknown", |(_, name)| name)
}

/// The reply of `reply_kind` to `request` (a raw client packet), offering
/// `address` from `server`: a /24 mask, a lease, no router, no DNS.
pub fn build_reply(reply_kind: u8, request: &[u8], address: Ipv4Addr, server: Ipv4Addr) -> Vec<u8> {
    let field = |range: std::ops::Range<usize>| request.get(range).unwrap_or_default().to_vec();
    let mut packet = Vec::with_capacity(300);
    packet.extend_from_slice(&[2, 1, 6, 0]);
    packet.extend(field(4..8)); // xid
    packet.extend_from_slice(&[0, 0]); // secs
    packet.extend(field(10..12)); // flags
    packet.extend_from_slice(&[0; 4]); // ciaddr
    packet.extend_from_slice(&address.octets()); // yiaddr
    packet.extend_from_slice(&server.octets()); // siaddr
    packet.extend_from_slice(&[0; 4]); // giaddr
    packet.extend(field(28..44)); // chaddr
    packet.extend_from_slice(&[0; 192]); // sname and file
    packet.extend_from_slice(&MAGIC);
    packet.extend_from_slice(&[53, 1, reply_kind]);
    packet.extend_from_slice(&[54, 4]);
    packet.extend_from_slice(&server.octets());
    packet.extend_from_slice(&[51, 4]);
    packet.extend_from_slice(&LEASE_S.to_be_bytes());
    packet.extend_from_slice(&[1, 4, 255, 255, 255, 0]);
    packet.push(255);
    packet
}

/// The reserved projectors: MAC to address, for those with a `mac` on the
/// projector network.
pub fn reservations(config: &Config) -> BTreeMap<[u8; 6], Ipv4Addr> {
    let subnet = config.subnet();
    config
        .projectors
        .values()
        .filter_map(|projector| {
            let mac = config::parse_mac(&projector.mac)?;
            let ip: Ipv4Addr = projector.ip.parse().ok()?;
            subnet
                .is_none_or(|net| net.contains(ip))
                .then_some((mac, ip))
        })
        .collect()
}

/// A client heard asking for a lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    /// The request last heard.
    pub request: Request,
    /// The vendor it looks like.
    pub vendor: &'static str,
    /// How many requests were heard.
    pub count: u32,
}

/// Binds UDP 67 on `address` with broadcast on.
fn bind(address: Ipv4Addr) -> std::io::Result<UdpSocket> {
    let socket = UdpSocket::bind(SocketAddrV4::new(address, SERVER_PORT))?;
    socket.set_broadcast(true)?;
    Ok(socket)
}

/// Listens on `address` for `duration` and reports every DHCP client
/// heard. Sends nothing. An address that cannot be bound (another DHCP
/// server, no permission) reports nobody.
pub fn sniff(address: Ipv4Addr, duration: Duration) -> Vec<Client> {
    let socket = match bind(address) {
        Ok(socket) => socket,
        Err(error) => {
            log::warn!("DHCP listen on {address}:{SERVER_PORT} unavailable: {error}");
            return Vec::new();
        }
    };
    let end = Instant::now() + duration;
    let mut seen: BTreeMap<[u8; 6], Client> = BTreeMap::new();
    let mut buffer = [0_u8; 2048];
    while let Some(left) = end
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
    {
        if socket.set_read_timeout(Some(left)).is_err() {
            break;
        }
        let Ok((length, _)) = socket.recv_from(&mut buffer) else {
            break;
        };
        let Some(request) = buffer.get(..length).and_then(parse) else {
            continue;
        };
        let vendor = vendor(&request);
        let client = seen.entry(request.mac).or_insert_with(|| Client {
            request: request.clone(),
            vendor,
            count: 0,
        });
        client.count += 1;
        client.request = request;
        if client.vendor == "unknown" {
            client.vendor = vendor;
        }
    }
    seen.into_values().collect()
}

/// Answers the reserved projectors from `server` (the PC's projector
/// address) until `duration` passes, or forever with `None`. Returns
/// whether any was acknowledged.
pub fn serve(config: &Config, duration: Option<Duration>, audit: &Audit) -> std::io::Result<bool> {
    /// How long one wait for a packet lasts with no end set.
    const TICK: Duration = Duration::from_secs(5);
    let server: Ipv4Addr = config.network.ethernet_ip.parse().map_err(|_| {
        std::io::Error::other(format!(
            "network.ethernet_ip '{}' is not an IPv4 address",
            config.network.ethernet_ip
        ))
    })?;
    let reserved = reservations(config);
    if reserved.is_empty() {
        return Err(std::io::Error::other(
            "no projector has a 'mac' in the config; nothing to serve",
        ));
    }
    let socket = bind(server)?;
    let listed: Vec<String> = reserved
        .iter()
        .map(|(mac, ip)| format!("{} -> {ip}", mac_text(mac)))
        .collect();
    log::info!(
        "serving reservations on {server}:{SERVER_PORT}: {}",
        listed.join(", ")
    );
    let end = duration.map(|duration| Instant::now() + duration);
    let mut acked = false;
    let mut buffer = [0_u8; 2048];
    loop {
        let wait = match end {
            None => TICK,
            Some(end) => match end.checked_duration_since(Instant::now()) {
                Some(left) if !left.is_zero() => left.max(Duration::from_millis(100)),
                Some(_) | None => break,
            },
        };
        socket.set_read_timeout(Some(wait))?;
        let length = match socket.recv_from(&mut buffer) {
            Ok((length, _)) => length,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        let packet = buffer.get(..length).unwrap_or_default();
        let Some(request) = parse(packet) else {
            continue;
        };
        let Some(&address) = reserved.get(&request.mac) else {
            continue;
        };
        let reply = match request.kind {
            Some(kind::DISCOVER) => kind::OFFER,
            Some(kind::REQUEST) => kind::ACK,
            Some(_) | None => continue,
        };
        socket.send_to(
            &build_reply(reply, packet, address, server),
            SocketAddrV4::new(Ipv4Addr::BROADCAST, CLIENT_PORT),
        )?;
        let mac = mac_text(&request.mac);
        let word = if reply == kind::OFFER { "OFFER" } else { "ACK" };
        log::info!("{word} {mac} -> {address}");
        if reply == kind::ACK {
            acked = true;
            audit.record("dhcp", &mac, "lease", &format!("ACK {address}"));
        }
    }
    Ok(acked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};

    /// A DISCOVER from the Sony's MAC with a host name, a vendor class and
    /// a requested address.
    fn discover() -> Vec<u8> {
        let mut packet = vec![0_u8; OPTIONS];
        packet[0] = 1;
        packet[1] = 1;
        packet[2] = 6;
        packet[4..8].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        packet[28..34].copy_from_slice(&[0x04, 0x5d, 0x4b, 0x96, 0x32, 0x7c]);
        packet[236..240].copy_from_slice(&MAGIC);
        packet.extend_from_slice(&[53, 1, kind::DISCOVER, 0, 12, 3]);
        packet.extend_from_slice(b"VPL");
        packet.extend_from_slice(&[60, 4]);
        packet.extend_from_slice(b"SONY");
        packet.extend_from_slice(&[50, 4, 192, 168, 50, 11, 255]);
        packet
    }

    #[test]
    fn a_discover_is_read_whole() -> Result<()> {
        let request = parse(&discover()).context("a DISCOVER")?;
        assert_eq!(request.kind, Some(kind::DISCOVER), "kind");
        assert_eq!(mac_text(&request.mac), "04-5D-4B-96-32-7C", "mac");
        assert_eq!(request.hostname.as_deref(), Some("VPL"), "host name");
        assert_eq!(request.vendor_class.as_deref(), Some("SONY"), "class");
        assert_eq!(
            request.requested_ip,
            Some(Ipv4Addr::new(192, 168, 50, 11)),
            "ip"
        );
        assert_eq!(vendor(&request), "sony", "vendor");
        Ok(())
    }

    #[test]
    fn garbage_and_truncated_options_are_safe() {
        assert_eq!(parse(&[]), None, "empty");
        assert_eq!(parse(&[2; 300]), None, "a reply is not a request");
        let mut cut = discover();
        cut.truncate(OPTIONS + 4);
        assert!(parse(&cut).is_some(), "a truncated option ends the walk");
    }

    #[test]
    fn an_offer_carries_the_reservation_and_no_router() -> Result<()> {
        let request = discover();
        let reply = build_reply(
            kind::OFFER,
            &request,
            Ipv4Addr::new(192, 168, 50, 11),
            Ipv4Addr::new(192, 168, 50, 1),
        );
        assert_eq!(reply.first(), Some(&2), "BOOTREPLY");
        assert_eq!(reply.get(4..8), request.get(4..8), "same transaction");
        assert_eq!(reply.get(16..20), Some(&[192, 168, 50, 11][..]), "yiaddr");
        assert_eq!(reply.get(28..34), request.get(28..34), "same client");
        assert_eq!(reply.get(236..240), Some(&MAGIC[..]), "magic");
        let options = reply.get(OPTIONS..).context("options")?;
        assert_eq!(options.get(..3), Some(&[53, 1, kind::OFFER][..]), "type");
        assert!(!options.windows(2).any(|w| w == [3, 4]), "no router option");
        assert!(!options.windows(2).any(|w| w == [6, 4]), "no DNS option");
        assert_eq!(options.last(), Some(&255), "end");
        Ok(())
    }

    #[test]
    fn only_projectors_with_a_mac_on_the_subnet_are_reserved() -> Result<()> {
        let text = include_str!("../../../tools/projector-control/config.example.json");
        let mut config: Config = serde_json::from_str(text)?;
        assert!(reservations(&config).is_empty(), "the example has no MACs");
        if let Some(sony) = config.projectors.get_mut("sony") {
            sony.mac = "04-5D-4B-96-32-7C".into();
        }
        if let Some(panasonic) = config.projectors.get_mut("panasonic") {
            panasonic.mac = "00-80-9F-00-00-01".into();
            panasonic.ip = "10.0.0.5".into();
        }
        let reserved = reservations(&config);
        assert_eq!(reserved.len(), 1, "{reserved:?}");
        assert_eq!(
            reserved.get(&[0x04, 0x5d, 0x4b, 0x96, 0x32, 0x7c]),
            Some(&Ipv4Addr::new(192, 168, 50, 11)),
            "the Sony"
        );
        Ok(())
    }
}
