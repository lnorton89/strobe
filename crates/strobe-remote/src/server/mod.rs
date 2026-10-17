// ============================================================================
// Module:       strobe_remote::server
// Description:  The opt-in LAN listener: binds a chosen private address,
//               serves the controller page and a WebSocket carrying the v1
//               protocol on threads of its own, and stops at once.
//
// Dependencies: std::net, std::thread, std::sync; crate::{bridge, clients,
//               pairing}; submodules http, ws, page, net, session
// ============================================================================

//! The LAN remote's listener (L-02, L-04, D250 to D253).
//!
//! Nothing here runs until the app calls [`Server::start`], and nothing in
//! it touches the renderer, the playlist or the UI. It speaks to the app
//! only through the [`bridge`](crate::bridge): commands go in on a bounded
//! queue and come back answered, and the state comes out as a revisioned
//! snapshot.
//!
//! ## Threads
//!
//! One accept thread, plus one thread per connection, at most
//! [`Limits::max_connections`] of them. A connection thread blocks only on
//! its own socket, with a read timeout of [`Limits::tick`], so it wakes
//! every tick to push a changed snapshot, check its timers and notice the
//! listener stopping or its client being revoked. The app's frame never
//! waits on any of them.
//!
//! ## What a request meets, in order
//!
//! 1. **The peer.** A connection from a public address is closed unread,
//!    whatever was bound: a port forward cannot expose the show.
//! 2. **The count.** Past [`Limits::max_connections`] the answer is 503.
//! 3. **The head**, whole within [`Limits::handshake_timeout`], at most
//!    8 KiB, `GET` only, no body.
//! 4. **The `Host`**: this listener's own address and port, as an IP
//!    literal ([`net::host_matches`]). A host name is refused (421): that
//!    is what defeats DNS rebinding.
//! 5. **The route.** `/` is the page; `/ws` is the WebSocket; anything else
//!    is 404.
//! 6. **The `Origin`**, for `/ws`: exactly `http://` and the `Host` just
//!    checked. Another site's page cannot open a socket here (cross-site
//!    WebSocket hijacking), token or no token. A missing `Origin` is
//!    refused too: the only client is the page.
//! 7. **The protocol**, in [`session`]: `hello`, then `authenticate` or
//!    `pair`, then commands, each decoded by [`crate::decode_client`] and
//!    rate-limited, with idle and authentication timeouts.
//!
//! ## Bounded both ways
//!
//! Inbound, a frame larger than [`Limits::max_message`] is refused from its
//! header, and a connection holds at most one frame. Outbound there is no
//! queue at all: a connection sends the latest snapshot when its revision
//! moved (so a hundred changes between two ticks are one message) and the
//! replies to the one message it is handling, each written with
//! [`Limits::write_timeout`]. A client that does not read is dropped when
//! that runs out, not buffered for.
//!
//! ## Stopping
//!
//! [`Server::stop`] (and dropping the server) sets the stop flag, shuts
//! every connection's socket down at once, so phones see the remote go
//! immediately, then joins the accept thread, which polls a non-blocking
//! listener every tick and so needs no wake-up connection (one could reach
//! nothing once the address has gone), and frees the port. Connection
//! threads finish on their own within a tick. Stopping
//! twice is harmless.

mod http;
pub mod net;
mod page;
mod session;
pub(crate) mod ws;

use crate::bridge::Remote;
use crate::clients::Clients;
use crate::pairing::{Pairing, Request};
use crate::trust::{Limiter, TrustSecret};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The default port. One above the Shadertoy listener's 47811.
pub const DEFAULT_PORT: u16 = 47812;

/// The limits a listener enforces. [`Limits::default`] is what the app
/// uses; tests shorten the timers.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Connections served at once (page loads and sockets together).
    pub max_connections: usize,
    /// The largest client message, in bytes.
    pub max_message: usize,
    /// Messages a client may send per second, sustained.
    pub rate_per_second: f64,
    /// Messages a client may send in a burst.
    pub burst: f64,
    /// How often a connection wakes to push state and check its timers.
    pub tick: Duration,
    /// How long a request head may take to arrive.
    pub handshake_timeout: Duration,
    /// How long a connection may stay unpaired and unauthenticated.
    pub auth_timeout: Duration,
    /// How long a connection may send nothing at all.
    pub idle_timeout: Duration,
    /// How long the connection holding the desktop lease may send nothing
    /// before the same paired desktop, connecting again, takes the lease
    /// back from it (D407). The native controller pings every 5 s, so this
    /// is two missed pings and a margin.
    pub dead_lease: Duration,
    /// How long one write may take before the client is dropped as slow.
    pub write_timeout: Duration,
    /// How long a command waits for the app's answer.
    pub command_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 8,
            max_message: crate::MAX_MESSAGE_BYTES,
            rate_per_second: 10.0,
            burst: 20.0,
            tick: Duration::from_millis(100),
            handshake_timeout: Duration::from_secs(5),
            auth_timeout: Duration::from_secs(120),
            idle_timeout: Duration::from_secs(30),
            dead_lease: Duration::from_secs(12),
            write_timeout: Duration::from_secs(2),
            command_timeout: Duration::from_secs(1),
        }
    }
}

/// How to listen.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The address and port. Port 0 picks a free one (tests).
    pub bind: SocketAddr,
    /// What to enforce.
    pub limits: Limits,
    /// The public name of an HTTPS reverse proxy on this computer that
    /// forwards to the listener (L-07, D258), such as
    /// `strobe.example.com` or `strobe.example.com:8443`. A request whose
    /// `Host` is exactly this is accepted only from a loopback peer (the
    /// proxy), and its socket only with the `Origin` `https://` and this
    /// name. `None`, the default, accepts IP literals only.
    pub proxied: Option<String>,
    /// The venue trust file's secret (D288), if the host was started with
    /// one. With it, and the full-desktop switch on, a controller that
    /// proves it knows the secret is paired for full desktop control
    /// without an approval. `None`, the default, means every pairing
    /// needs the operator.
    pub trust: Option<TrustSecret>,
}

impl ServerConfig {
    /// The default limits on `ip:port`, no proxy.
    pub fn new(ip: IpAddr, port: u16) -> Self {
        Self {
            bind: SocketAddr::new(ip, port),
            limits: Limits::default(),
            proxied: None,
            trust: None,
        }
    }
}

/// Why the listener did not start. Each message says what to do about it.
#[derive(Debug)]
pub enum StartError {
    /// The address is on the public internet.
    PublicAddress(IpAddr),
    /// The socket could not be bound.
    Bind {
        /// What was asked for.
        addr: SocketAddr,
        /// Why not.
        source: std::io::Error,
    },
    /// The accept thread could not be started.
    Thread(std::io::Error),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PublicAddress(ip) => write!(
                f,
                "refusing to listen on {ip}: it is a public internet address, and the remote is \
                 for a trusted local network only. Choose this computer's LAN address \
                 (192.168.x.x, 10.x.x.x or 172.16-31.x.x)"
            ),
            Self::Bind { addr, source } => {
                let kind = source.kind();
                let advice = if kind == std::io::ErrorKind::AddrInUse {
                    " (another program, or another Strobe, has that port: choose another port \
                     in Settings or with --remote-port)"
                } else if kind == std::io::ErrorKind::AddrNotAvailable {
                    " (that is not an address of this computer: choose one of its own, or leave \
                     the address empty to use its LAN address)"
                } else if kind == std::io::ErrorKind::PermissionDenied {
                    " (the system refused the port: use one above 1024, or check that a \
                     firewall or security tool is not blocking Strobe)"
                } else {
                    ""
                };
                write!(f, "cannot listen on {addr}: {source}{advice}")
            }
            Self::Thread(error) => write!(f, "cannot start the remote's listener thread: {error}"),
        }
    }
}

impl std::error::Error for StartError {}

/// An open connection, as the desktop lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionInfo {
    /// Where it comes from.
    pub peer: SocketAddr,
    /// The paired client it authenticated as, if it has.
    pub client_id: Option<String>,
}

impl ConnectionInfo {
    /// Whether this is a paired client that has authenticated. A socket
    /// that has only said hello, is waiting for approval or was refused is
    /// open but is nobody's connection.
    pub fn is_authenticated(&self) -> bool {
        self.client_id.is_some()
    }
}

/// A connection the listener tracks.
#[derive(Debug)]
struct Conn {
    id: u64,
    peer: SocketAddr,
    /// A handle to the same socket, to shut it down from outside.
    stream: TcpStream,
    client_id: Option<String>,
    /// Set by a revoke: the connection closes on its next tick.
    revoked: bool,
    /// When a message last arrived on it, for [`Shared::reclaim_dead_lease`].
    heard: Instant,
}

/// What the accept thread, every connection and the server share.
#[derive(Debug)]
struct Shared {
    limits: Limits,
    addr: SocketAddr,
    proxied: Option<String>,
    remote: Remote,
    clients: Clients,
    pairing: Mutex<Pairing>,
    trust: Option<TrustState>,
    stop: AtomicBool,
    conns: Mutex<Vec<Conn>>,
    next_conn: AtomicU64,
    active: AtomicUsize,
}

/// The venue trust file's secret and the failed-proof limiter (D288).
#[derive(Debug)]
struct TrustState {
    secret: TrustSecret,
    limiter: Mutex<Limiter>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn pairing(&self) -> MutexGuard<'_, Pairing> {
        lock(&self.pairing)
    }

    /// Records that connection `conn` is now client `client_id`.
    fn set_client(&self, conn: u64, client_id: &str) {
        if let Some(entry) = lock(&self.conns).iter_mut().find(|c| c.id == conn) {
            entry.client_id = Some(client_id.to_string());
        }
    }

    /// A trusted controller proved itself again (D288): its earlier
    /// connections, `ids` being their client ids, are closed and give up
    /// the desktop lease at once, so the new one can take it. Without this
    /// a Wi-Fi drop would leave the dead connection holding the lease until
    /// its idle timeout.
    fn supersede(&self, ids: &[String]) {
        for conn in lock(&self.conns).iter_mut() {
            if conn
                .client_id
                .as_deref()
                .is_some_and(|id| ids.iter().any(|old| old == id))
            {
                conn.revoked = true;
                self.remote.desktop.release(conn.id);
            }
        }
    }

    /// Notes that a message arrived on connection `conn`.
    fn heard(&self, conn: u64) {
        if let Some(entry) = lock(&self.conns).iter_mut().find(|c| c.id == conn) {
            entry.heard = Instant::now();
        }
    }

    /// A paired desktop asks for the lease that another of its own
    /// connections holds (D407). If that connection has been silent for
    /// [`Limits::dead_lease`] it is gone (a Wi-Fi drop the host has not yet
    /// noticed): it is closed and gives up the lease, so `conn` can take it.
    /// A live connection, or another client's, keeps it. Returns whether a
    /// lease was freed.
    fn reclaim_dead_lease(&self, conn: u64, client_id: &str) -> bool {
        let now = Instant::now();
        let mut freed = false;
        for other in lock(&self.conns).iter_mut() {
            if other.id != conn
                && other.client_id.as_deref() == Some(client_id)
                && self.remote.desktop.owns(other.id)
                && now.saturating_duration_since(other.heard) >= self.limits.dead_lease
            {
                other.revoked = true;
                self.remote.desktop.release(other.id);
                freed = true;
            }
        }
        freed
    }

    /// Whether connection `conn`'s client was revoked.
    fn revoked(&self, conn: u64) -> bool {
        lock(&self.conns).iter().any(|c| c.id == conn && c.revoked)
    }

    fn forget(&self, conn: u64) {
        lock(&self.conns).retain(|c| c.id != conn);
    }
}

/// A running listener. Dropping it stops it.
#[derive(Debug)]
pub struct Server {
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    warning: Option<String>,
}

impl Server {
    /// Binds `config.bind` and starts serving on threads of its own.
    /// Commands go to `remote`; pairing and authentication use `clients`.
    pub fn start(
        config: ServerConfig,
        remote: Remote,
        clients: Clients,
    ) -> Result<Self, StartError> {
        let ip = config.bind.ip();
        let warning = match net::scope(ip) {
            net::Scope::Public => return Err(StartError::PublicAddress(ip)),
            net::Scope::Unspecified => Some(
                "Listening on every network interface, including any public one. Prefer this \
                 computer's LAN address."
                    .to_string(),
            ),
            net::Scope::Loopback if config.proxied.is_some() => None,
            net::Scope::Loopback => Some(
                "Listening on this computer only: a phone cannot reach it. Choose the LAN \
                 address to use a phone."
                    .to_string(),
            ),
            net::Scope::Private | net::Scope::LinkLocal => None,
        };
        let socket = TcpListener::bind(config.bind).map_err(|source| StartError::Bind {
            addr: config.bind,
            source,
        })?;
        let addr = socket.local_addr().map_err(StartError::Thread)?;
        socket
            .set_nonblocking(true)
            .map_err(|source| StartError::Bind {
                addr: config.bind,
                source,
            })?;
        let shared = Arc::new(Shared {
            limits: config.limits,
            addr,
            proxied: config.proxied.clone(),
            remote,
            clients,
            pairing: Mutex::new(Pairing::default()),
            trust: config.trust.map(|secret| TrustState {
                secret,
                limiter: Mutex::new(Limiter::default()),
            }),
            stop: AtomicBool::new(false),
            conns: Mutex::new(Vec::new()),
            next_conn: AtomicU64::new(1),
            active: AtomicUsize::new(0),
        });
        let accept = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("strobe-remote-accept".to_string())
                .spawn(move || accept_loop(&socket, &shared))
                .map_err(StartError::Thread)?
        };
        log::info!("remote: listening on {addr}");
        if let Some(warning) = &warning {
            log::warn!("remote: {warning}");
        }
        Ok(Self {
            shared,
            accept: Some(accept),
            warning,
        })
    }

    /// Whether the host was started with a trust file (D288). Trusted
    /// pairing also needs the full-desktop switch on.
    pub fn trust_configured(&self) -> bool {
        self.shared.trust.is_some()
    }

    /// The address bound.
    pub fn addr(&self) -> SocketAddr {
        self.shared.addr
    }

    /// The address a phone should open (for every interface, the LAN
    /// address). It carries no secret.
    pub fn url(&self) -> String {
        net::url(net::display_ip(self.shared.addr), self.shared.addr.port())
    }

    /// A warning about the bind, for the desktop to show.
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// Opens pairing with a fresh code and returns it for the desktop to
    /// show, as `123 456`.
    pub fn open_pairing(&self) -> std::io::Result<String> {
        let code = self.shared.pairing().open(Instant::now())?;
        log::info!("remote: pairing opened");
        Ok(code)
    }

    /// Closes pairing: the code stops working.
    pub fn close_pairing(&self) {
        self.shared.pairing().close();
    }

    /// The open code and how long it has left.
    pub fn pairing_code(&self) -> Option<(String, Duration)> {
        self.shared.pairing().code(Instant::now())
    }

    /// Phones that gave the right code and wait for Approve or Deny.
    pub fn pairing_requests(&self) -> Vec<Request> {
        self.shared.pairing().waiting(Instant::now())
    }

    /// Approves or denies request `id`. Returns whether it was waiting.
    pub fn decide(&self, id: u64, approve: bool) -> bool {
        let decided = self.shared.pairing().decide(id, approve, Instant::now());
        if decided {
            log::info!(
                "remote: pairing request {id} {}",
                if approve { "approved" } else { "denied" }
            );
        }
        decided
    }

    /// The connections open now.
    pub fn connections(&self) -> Vec<ConnectionInfo> {
        lock(&self.shared.conns)
            .iter()
            .map(|c| ConnectionInfo {
                peer: c.peer,
                client_id: c.client_id.clone(),
            })
            .collect()
    }

    /// The paired clients this listener authenticates against.
    pub fn clients(&self) -> &Clients {
        &self.shared.clients
    }

    /// Revokes client `id` and closes its connection. Returns whether it
    /// was paired.
    pub fn revoke(&self, id: &str) -> bool {
        let known = self.shared.clients.revoke(id);
        for conn in lock(&self.shared.conns).iter_mut() {
            if conn.client_id.as_deref() == Some(id) {
                conn.revoked = true;
            }
        }
        log::info!("remote: client {id} revoked");
        known
    }

    /// Revokes every client paired for full desktop access and closes
    /// their connections (D275): the host switched that access off.
    /// Returns how many there were.
    pub fn revoke_desktop(&self) -> usize {
        let ids = self.shared.clients.revoke_desktop();
        for conn in lock(&self.shared.conns).iter_mut() {
            if conn
                .client_id
                .as_deref()
                .is_some_and(|id| ids.iter().any(|revoked| revoked == id))
            {
                conn.revoked = true;
            }
        }
        if !ids.is_empty() {
            log::info!("remote: {} desktop clients revoked", ids.len());
        }
        ids.len()
    }

    /// Revokes every client and closes their connections.
    pub fn revoke_all(&self) -> usize {
        let count = self.shared.clients.revoke_all();
        for conn in lock(&self.shared.conns).iter_mut() {
            if conn.client_id.is_some() {
                conn.revoked = true;
            }
        }
        log::info!("remote: all {count} clients revoked");
        count
    }

    /// Stops listening and drops every client at once.
    pub fn stop(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        let Some(accept) = self.accept.take() else {
            return;
        };
        self.shared.stop.store(true, Ordering::SeqCst);
        for conn in lock(&self.shared.conns).iter() {
            // The connection is refused either way; a peer that already left
            // needs no shutdown.
            let _ = conn.stream.shutdown(Shutdown::Both);
        }
        // The accept loop polls, so it sees the flag within a tick. Waits for
        // it; a panic in it already aborted the process in release builds.
        let _ = accept.join();
        self.shared.pairing().close();
        log::info!("remote: stopped listening on {}", self.shared.addr);
    }

    /// Connection threads still finishing (tests watch this reach 0).
    pub fn active(&self) -> usize {
        self.shared.active.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// Accepts until the stop flag is set. The listener is non-blocking and
/// polled every tick, so the flag alone ends the loop: no wake-up
/// connection is needed, which could reach nothing once the bound address
/// has gone (a DHCP renewal, a Wi-Fi rejoin) or when `::` is IPv6-only,
/// and then left Stop, and quitting, joining this thread for ever. A
/// failing `accept` (out of file handles) waits a tick too, rather than
/// spinning.
fn accept_loop(socket: &TcpListener, shared: &Arc<Shared>) {
    while !shared.stopping() {
        let mut stream = match socket.accept() {
            Ok((stream, _)) => stream,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::WouldBlock {
                    log::debug!("remote: accept failed: {error}");
                }
                std::thread::sleep(shared.limits.tick);
                continue;
            }
        };
        // Windows and macOS hand out sockets as non-blocking as their
        // listener; a session needs blocking reads with a timeout.
        if stream.set_nonblocking(false).is_err() {
            continue;
        }
        let Ok(peer) = stream.peer_addr() else {
            continue;
        };
        if net::scope(peer.ip()) == net::Scope::Public {
            log::warn!("remote: refused a connection from public address {peer}");
            // The connection is refused either way; a peer that already left
            // needs no shutdown.
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        let limits = &shared.limits;
        // A connection the timeouts cannot be set on could hold its thread
        // forever on a slow or silent peer: it is not served.
        let timeouts = stream
            .set_read_timeout(Some(limits.tick))
            .and_then(|()| stream.set_write_timeout(Some(limits.write_timeout)));
        if let Err(error) = timeouts {
            log::warn!("remote: dropped a connection from {peer}: no timeouts ({error})");
            continue;
        }
        // Nagle only delays small frames; a socket that refuses this works.
        let _ = stream.set_nodelay(true);
        if shared.active.fetch_add(1, Ordering::SeqCst) >= limits.max_connections {
            shared.active.fetch_sub(1, Ordering::SeqCst);
            // The busy reply is a courtesy: without a shorter timeout it
            // waits the normal one.
            let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
            // The busy reply is a courtesy to a client being turned away.
            let _ = http::respond(
                &mut stream,
                &http::Response::text(503, "Too many remote connections are open."),
            );
            continue;
        }
        let id = shared.next_conn.fetch_add(1, Ordering::SeqCst);
        let Ok(handle) = stream.try_clone() else {
            shared.active.fetch_sub(1, Ordering::SeqCst);
            continue;
        };
        lock(&shared.conns).push(Conn {
            id,
            peer,
            stream: handle,
            client_id: None,
            revoked: false,
            heard: Instant::now(),
        });
        let spawned = std::thread::Builder::new()
            .name("strobe-remote-conn".to_string())
            .spawn({
                let shared = Arc::clone(shared);
                move || {
                    session::serve(stream, &shared, id, peer);
                    shared.forget(id);
                    shared.active.fetch_sub(1, Ordering::SeqCst);
                }
            });
        if spawned.is_err() {
            shared.forget(id);
            shared.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_accept_thread_ends_on_the_stop_flag_without_a_wake_up_connection() -> Result<()> {
        // Stopping used to wake a blocked `accept` by connecting to the
        // bound address. That connection reaches nothing when the address
        // has gone (a DHCP renewal or a Wi-Fi rejoin mid-show) or when `::`
        // is IPv6-only (Windows' default) and the wake-up goes to
        // 127.0.0.1, and then joining the thread hung Stop, and quitting.
        let (remote, _app) = crate::bridge::bridge(4);
        let config = ServerConfig {
            limits: Limits {
                tick: Duration::from_millis(20),
                ..Limits::default()
            },
            ..ServerConfig::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0)
        };
        let server = Server::start(config, remote, Clients::in_memory())?;
        server.shared.stop.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(2);
        let finished = || server.accept.as_ref().is_some_and(JoinHandle::is_finished);
        while !finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            finished(),
            "the accept thread must notice the stop flag by itself"
        );
        Ok(())
    }
}
