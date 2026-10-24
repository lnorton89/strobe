// ============================================================================
// Module:       strobe_remote::discovery
// Description:  Finding Strobes on the LAN: a host with the remote on answers
//               a broadcast query with its name, port and whether it takes
//               desktop controllers; a controller lists who answered.
//
// Dependencies: serde, serde_json; std::net (UDP); crate::{secret, desktop,
//               server::net}
// ============================================================================

//! LAN discovery (D468).
//!
//! A controller should not need to know the host's address. While the
//! remote listens, the host runs a [`Responder`] on UDP [`PORT`]; a
//! controller runs a [`Scanner`] while its Remote page is open, which
//! broadcasts a `discover` query every [`QUERY_EVERY`] and keeps whoever
//! answered `here` within [`FORGET_AFTER`].
//!
//! ## Query and answer, not announcements
//!
//! The controller asks and the host answers to the port the question came
//! from. An answer to a datagram a program sent is let in by the system's
//! firewall as part of that exchange, so the controller needs no inbound
//! rule; only the host, which already accepts connections, listens.
//!
//! ## What an answer is trusted for
//!
//! Nothing but a suggestion. The address used is the one the answer came
//! from, never one it names; the name is cut to [`MAX_NAME`] characters of
//! a plain set and shown as a label. Connecting still needs the host's
//! approval (or a trust file), exactly as an address typed by hand does. A
//! responder answers only private, link-local or loopback addresses (D252)
//! and at most [`REPLIES_PER_SECOND`] times a second, so it is neither a
//! public service nor much of an amplifier: an answer is about three times
//! the size of the question.

use crate::desktop::Desktop;
use crate::server::net;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The UDP port hosts answer on: the remote's default TCP port plus one.
pub const PORT: u16 = 47813;

/// How often a scanner asks.
pub const QUERY_EVERY: Duration = Duration::from_secs(2);

/// How long a host stays listed after its last answer: three missed
/// queries.
pub const FORGET_AFTER: Duration = Duration::from_secs(6);

/// The longest name an answer may carry, in bytes.
pub const MAX_NAME: usize = 63;

/// Answers a responder sends in any one second, to anyone.
pub const REPLIES_PER_SECOND: u32 = 20;

/// The version both datagrams carry.
const VERSION: u16 = 1;

/// The largest datagram read; both shapes are well under it.
const MAX_DATAGRAM: usize = 512;

/// How long a socket read waits, so a stop is seen within it.
const TICK: Duration = Duration::from_millis(200);

/// The two datagrams.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "strobe", rename_all = "snake_case", deny_unknown_fields)]
enum Datagram {
    /// "Is a Strobe there?"
    Discover {
        /// The asker's version.
        v: u16,
    },
    /// "Here: this name, this port."
    Here {
        /// The answerer's version.
        v: u16,
        /// Its process's [`instance_id`], so a scanner can leave out its
        /// own Strobe.
        id: String,
        /// Its computer's name, for the list.
        name: String,
        /// The remote's TCP port.
        port: u16,
        /// Whether full desktop control is on, so a desktop can connect.
        desktop: bool,
    },
}

/// This process's id in answers: 16 hex digits drawn once.
pub fn instance_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        crate::secret::random::<8>().map_or_else(|_| "0".to_string(), |b| crate::secret::hex(&b))
    })
}

/// `name` made fit for an answer: letters, digits, `.`, `_`, `-` and
/// spaces kept, anything else a `-`, cut to [`MAX_NAME`] bytes; `Strobe`
/// when nothing is left.
pub fn display_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars() {
        if out.len() >= MAX_NAME {
            break;
        }
        out.push(
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ' ') {
                ch
            } else {
                '-'
            },
        );
    }
    if out.trim().is_empty() {
        "Strobe".to_string()
    } else {
        out.trim().to_string()
    }
}

/// Whether a received name is one [`display_name`] could have made.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b' '))
}

/// Whether a responder answers a question from `ip`.
fn answers(ip: IpAddr) -> bool {
    match net::scope(ip) {
        net::Scope::Loopback | net::Scope::Private | net::Scope::LinkLocal => true,
        net::Scope::Public | net::Scope::Unspecified => false,
    }
}

/// Whether a socket error is a read timeout.
fn timed_out(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// What a host says about itself.
#[derive(Clone, Debug)]
pub struct Advert {
    /// The id it answers with ([`instance_id`] in the app).
    pub id: String,
    /// Its name, already through [`display_name`].
    pub name: String,
    /// The remote's TCP port.
    pub port: u16,
}

/// The host side: answers `discover` while it runs. Dropping it stops it.
#[derive(Debug)]
pub struct Responder {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    addr: SocketAddr,
}

impl Responder {
    /// Binds `bind` (in the app every interface on [`PORT`], so broadcasts
    /// reach it) and answers on a thread of its own. `desktop` says, at
    /// each answer, whether full desktop control is on.
    pub fn start(bind: SocketAddr, advert: Advert, desktop: Desktop) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
        socket.set_read_timeout(Some(TICK))?;
        let addr = socket.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("strobe-remote-discovery".to_string())
                .spawn(move || respond(&socket, &advert, &desktop, &stop))?
        };
        log::info!("remote: answering discovery on {addr}");
        Ok(Self {
            stop,
            thread: Some(thread),
            addr,
        })
    }

    /// The address bound (tests use port 0).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // It reads with a short timeout, so it sees the flag within a
            // tick; a panic in it already aborted the process in release.
            let _ = thread.join();
        }
    }
}

/// The responder's loop.
fn respond(socket: &UdpSocket, advert: &Advert, desktop: &Desktop, stop: &AtomicBool) {
    let mut buffer = [0_u8; MAX_DATAGRAM];
    let mut window = Instant::now();
    let mut sent = 0;
    while !stop.load(Ordering::SeqCst) {
        let (read, from) = match socket.recv_from(&mut buffer) {
            Ok(got) => got,
            Err(error) if timed_out(&error) => continue,
            Err(error) => {
                // Windows reports an earlier answer's unreachable port as
                // an error on the next read; it ends nothing.
                log::debug!("remote: discovery read failed: {error}");
                std::thread::sleep(TICK);
                continue;
            }
        };
        if !answers(from.ip()) {
            continue;
        }
        let Some(Datagram::Discover { .. }) = buffer
            .get(..read)
            .and_then(|bytes| serde_json::from_slice::<Datagram>(bytes).ok())
        else {
            continue;
        };
        if window.elapsed() >= Duration::from_secs(1) {
            window = Instant::now();
            sent = 0;
        }
        if sent >= REPLIES_PER_SECOND {
            continue;
        }
        sent += 1;
        let here = Datagram::Here {
            v: VERSION,
            id: advert.id.clone(),
            name: advert.name.clone(),
            port: advert.port,
            desktop: desktop.enabled(),
        };
        if let Ok(bytes) = serde_json::to_vec(&here)
            && let Err(error) = socket.send_to(&bytes, from)
        {
            log::debug!("remote: discovery answer to {from} failed: {error}");
        }
    }
}

/// A Strobe that answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Its instance id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Where its remote listens: the answer's source address and the port
    /// it named.
    pub addr: SocketAddr,
    /// Whether it takes desktop controllers now.
    pub desktop: bool,
    /// When it last answered.
    pub seen: Instant,
}

/// The controller side: asks every [`QUERY_EVERY`] while it runs and keeps
/// the answers. Dropping it stops it without waiting for its thread, which
/// sees the flag within a tick: it is dropped on the frame loop's thread.
#[derive(Debug)]
pub struct Scanner {
    found: Arc<Mutex<Vec<Found>>>,
    stop: Arc<AtomicBool>,
}

impl Scanner {
    /// Starts asking at `targets` (broadcast addresses, see
    /// [`broadcast_targets`]), on `port`, leaving out answers with the id
    /// `ignore` (this process's own Strobe).
    pub fn start(targets: Vec<SocketAddr>, ignore: Option<String>) -> std::io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        socket.set_read_timeout(Some(TICK))?;
        let found = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let found = Arc::clone(&found);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("strobe-remote-scan".to_string())
                .spawn(move || scan(&socket, &targets, ignore.as_deref(), &found, &stop))?;
        }
        Ok(Self { found, stop })
    }

    /// The Strobes heard from within [`FORGET_AFTER`], by name.
    pub fn found(&self) -> Vec<Found> {
        let now = Instant::now();
        let mut found: Vec<Found> = self
            .found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|f| now.saturating_duration_since(f.seen) < FORGET_AFTER)
            .cloned()
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name).then(a.addr.cmp(&b.addr)));
        found
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Where a scanner asks: the limited broadcast and, for each of `lan`, its
/// /24's directed broadcast (Windows sends the limited one on one interface
/// only), each on `port`.
pub fn broadcast_targets(lan: &[Ipv4Addr], port: u16) -> Vec<SocketAddr> {
    let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, port))];
    for ip in lan {
        let [a, b, c, _] = ip.octets();
        let directed = SocketAddr::from((Ipv4Addr::new(a, b, c, 255), port));
        if !targets.contains(&directed) {
            targets.push(directed);
        }
    }
    targets
}

/// The scanner's loop.
fn scan(
    socket: &UdpSocket,
    targets: &[SocketAddr],
    ignore: Option<&str>,
    found: &Mutex<Vec<Found>>,
    stop: &AtomicBool,
) {
    let query = serde_json::to_vec(&Datagram::Discover { v: VERSION }).unwrap_or_default();
    let mut buffer = [0_u8; MAX_DATAGRAM];
    let mut asked: Option<Instant> = None;
    while !stop.load(Ordering::SeqCst) {
        if asked.is_none_or(|at| at.elapsed() >= QUERY_EVERY) {
            asked = Some(Instant::now());
            for target in targets {
                if let Err(error) = socket.send_to(&query, target) {
                    log::debug!("remote: discovery query to {target} failed: {error}");
                }
            }
            let now = Instant::now();
            found
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|f| now.saturating_duration_since(f.seen) < FORGET_AFTER);
        }
        let (read, from) = match socket.recv_from(&mut buffer) {
            Ok(got) => got,
            Err(error) if timed_out(&error) => continue,
            Err(error) => {
                log::debug!("remote: discovery answer read failed: {error}");
                std::thread::sleep(TICK);
                continue;
            }
        };
        let Some(Datagram::Here {
            id,
            name,
            port,
            desktop,
            ..
        }) = buffer
            .get(..read)
            .and_then(|bytes| serde_json::from_slice::<Datagram>(bytes).ok())
        else {
            continue;
        };
        let foreign = ignore != Some(id.as_str());
        if !foreign || port == 0 || !valid_name(&name) || id.is_empty() || id.len() > MAX_NAME {
            continue;
        }
        let entry = Found {
            id,
            name,
            addr: SocketAddr::new(from.ip(), port),
            desktop,
            seen: Instant::now(),
        };
        let mut found = found.lock().unwrap_or_else(PoisonError::into_inner);
        match found.iter_mut().find(|f| f.id == entry.id) {
            Some(known) => *known = entry,
            None => found.push(entry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_datagrams_have_a_fixed_shape() -> Result<()> {
        let query = serde_json::to_string(&Datagram::Discover { v: 1 })?;
        assert_eq!(query, r#"{"strobe":"discover","v":1}"#, "the query");
        let here =
            r#"{"strobe":"here","v":1,"id":"ab","name":"STAGE-PC","port":47812,"desktop":true}"#;
        let parsed: Datagram = serde_json::from_str(here)?;
        assert_eq!(serde_json::to_string(&parsed)?, here, "the answer");
        assert!(
            serde_json::from_str::<Datagram>(r#"{"strobe":"discover","v":1,"x":1}"#).is_err(),
            "an unknown field is refused"
        );
        Ok(())
    }

    #[test]
    fn names_are_cut_to_a_plain_set() {
        assert_eq!(display_name("STAGE-PC"), "STAGE-PC", "kept");
        assert_eq!(display_name("Lü's laptop"), "L--s laptop", "replaced");
        assert_eq!(display_name("  "), "Strobe", "empty");
        assert!(display_name(&"x".repeat(200)).len() <= MAX_NAME, "cut");
        assert!(valid_name("STAGE-PC 2"), "a name it made");
        assert!(!valid_name("<script>"), "not one it made");
    }

    #[test]
    fn only_local_askers_are_answered() {
        assert!(answers(IpAddr::from([192, 168, 1, 5])), "private");
        assert!(answers(IpAddr::from([127, 0, 0, 1])), "loopback");
        assert!(answers(IpAddr::from([169, 254, 3, 4])), "link-local");
        assert!(!answers(IpAddr::from([8, 8, 8, 8])), "public");
    }

    #[test]
    fn targets_are_the_broadcast_and_each_lans_directed_broadcast() {
        let targets = broadcast_targets(
            &[
                Ipv4Addr::new(192, 168, 1, 20),
                Ipv4Addr::new(192, 168, 1, 30),
            ],
            PORT,
        );
        assert_eq!(
            targets,
            vec![
                SocketAddr::from(([255, 255, 255, 255], PORT)),
                SocketAddr::from(([192, 168, 1, 255], PORT)),
            ],
            "one directed broadcast per /24"
        );
    }

    #[test]
    fn a_scanner_finds_a_responder_and_leaves_out_its_own_strobe() -> Result<()> {
        let desktop = Desktop::default();
        desktop.enable(true);
        let responder = Responder::start(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            Advert {
                id: "host-under-test".into(),
                name: "STAGE-PC".into(),
                port: 47812,
            },
            desktop,
        )?;
        let at = responder.addr();
        let scanner = Scanner::start(vec![at], None)?;
        let ignoring = Scanner::start(vec![at], Some("host-under-test".into()))?;
        let until = Instant::now() + Duration::from_secs(5);
        let mut found = Vec::new();
        while found.is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
            found = scanner.found();
        }
        let first = found
            .first()
            .ok_or_else(|| anyhow::anyhow!("nothing found in 5 s"))?;
        assert_eq!(first.name, "STAGE-PC", "its name");
        assert_eq!(
            first.addr,
            SocketAddr::from(([127, 0, 0, 1], 47812)),
            "where it answered from, on the port it named"
        );
        assert!(first.desktop, "desktop control on");
        assert!(
            ignoring.found().is_empty(),
            "a scanner leaves out the id it was told to"
        );
        Ok(())
    }
}
