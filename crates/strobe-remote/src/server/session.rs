// ============================================================================
// Module:       strobe_remote::server::session
// Description:  One connection: the HTTP checks (Host, route, Origin), the
//               page, the WebSocket upgrade, then the v1 protocol with its
//               pairing, authentication, rate limit and timers.
//
// Dependencies: crate (protocol, bridge, clients, pairing); super::{http, ws,
//               page, net, Shared}
// ============================================================================

//! A connection's life (L-02, L-03, L-04).
//!
//! The protocol, once the socket is up:
//!
//! ```text
//! client: hello {protocol_version}
//! server: hello {protocol_version, paired: false}
//! client: authenticate {token}          | client: pair {code}
//! server: authenticated {client_id}     |   (operator approves on the desktop)
//!         paired {client_id, token}     | server: paired {client_id, token}
//!           (the rotated token)         |         authenticated {client_id}
//!         state {...}                   |         state {...}
//! client: command {request_id, command}  -> server: command_result {...}
//! client: ping                           -> server: pong
//!                                           server: state {...} whenever it changes
//! ```
//!
//! Venue mode (D288) adds a third way past `Unauthenticated`, when the host
//! was started with a trust file and full desktop access is on:
//!
//! ```text
//! client: trust_start {controller_id}  -> server: trust_challenge {nonce}
//! client: trust_proof {response}       -> server: authenticated {client_id}
//!                                                 state {...}
//! ```
//!
//! The proof is an HMAC over the nonce, this connection's `Host`, and the
//! controller id ([`crate::trust`]); no token is issued, and the pairing
//! lasts as long as this connection.
//!
//! A `paired` message after `authenticated` is a token rotation (D251): the
//! client replaces the token it keeps.
//!
//! Every client message goes through [`crate::decode_client`] and costs one
//! token of a per-connection bucket ([`super::Limits::rate_per_second`],
//! [`super::Limits::burst`]); an empty bucket closes the connection with
//! 1008. So do five failed pairing or authentication attempts on one
//! connection. A connection that is not authenticated within
//! [`super::Limits::auth_timeout`], or sends nothing for
//! [`super::Limits::idle_timeout`], is closed.

use super::http::{self, Head, HeadError, Response};
use super::ws::{self, Message, Reader, close};
use super::{Shared, net, page};
use crate::bridge::Answer;
use crate::clients::{Device, TrustedRefusal};
use crate::pairing::{Decision, Refusal};
use crate::trust::{Challenge, Verdict};
use crate::{ClientMessage, ErrorCode, PROTOCOL_VERSION, ServerMessage, decode_client, negotiate};
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::sync::LazyLock;
use std::time::Instant;

/// Failed pairing or authentication attempts one connection may make.
const MAX_FAILURES: u32 = 5;

/// Serves one connection to the end.
pub(super) fn serve(mut stream: TcpStream, shared: &Shared, conn: u64, peer: SocketAddr) {
    let deadline = Instant::now() + shared.limits.handshake_timeout;
    let head = match http::read_head(&mut stream, deadline) {
        Ok(head) => head,
        Err(HeadError::Closed) => return,
        Err(HeadError::TooLarge) => {
            // The connection ends after this reply either way; a peer that
            // left needs none.
            let _ = http::respond(
                &mut stream,
                &Response::text(431, "The request is too large."),
            );
            return;
        }
        Err(HeadError::TooSlow) => return,
        Err(HeadError::Malformed(why)) => {
            // The connection ends either way; the reply is a courtesy.
            let _ = http::respond(&mut stream, &Response::text(400, why));
            return;
        }
        Err(HeadError::Io(error)) => {
            log::debug!("remote: reading a request from {peer} failed: {error}");
            return;
        }
    };
    if let Err(refusal) = route(&mut stream, shared, &head, peer) {
        // The connection ends either way; the reply is a courtesy.
        let _ = http::respond(&mut stream, &refusal);
        return;
    }
    if head.path() == "/ws" {
        let platform = platform(&head);
        let label = format!("{platform} at {}", peer.ip());
        // The `Host` just checked: what the controller dialled, and so what
        // a trusted pairing's proof is bound to (D288). It is used for that
        // only if it names the address this connection reached (or the
        // operator's proxy, from this computer): a listener on every
        // interface takes any literal, and a relay elsewhere could then
        // forward a controller's handshake here under its own address.
        let header = head.header("host").unwrap_or_default();
        let proxied = shared
            .proxied
            .as_deref()
            .is_some_and(|name| header.eq_ignore_ascii_case(name) && peer.ip().is_loopback());
        let here = proxied
            || stream
                .local_addr()
                .is_ok_and(|local| net::host_is_local(header, local));
        let host = here.then(|| header.to_ascii_lowercase());
        let leftover = head.leftover;
        Session::new(
            stream,
            shared,
            conn,
            peer,
            (platform, label),
            host,
            leftover,
        )
        .run();
    }
}

/// Checks the request and serves the page, or answers the upgrade. A
/// refusal is returned to be written; `Ok` with `/ws` means the socket is
/// now a WebSocket.
fn route(
    stream: &mut TcpStream,
    shared: &Shared,
    head: &Head,
    peer: SocketAddr,
) -> Result<(), Response<'static>> {
    let header = head.header("host");
    // Through the operator's own HTTPS proxy (D258): its exact name, from
    // this computer only, so no other machine can claim it.
    let proxied = match (header, shared.proxied.as_deref()) {
        (Some(host), Some(name)) => host.eq_ignore_ascii_case(name) && peer.ip().is_loopback(),
        (Some(_) | None, None) | (None, Some(_)) => false,
    };
    let Some(host) = header.filter(|host| proxied || net::host_matches(host, shared.addr)) else {
        return Err(Response::text(
            421,
            "Open the remote at the address Strobe shows (an IP address, not a name).",
        ));
    };
    let scheme = if proxied { "https" } else { "http" };
    if head.header("content-length").is_some_and(|l| l != "0") || head.has("transfer-encoding") {
        return Err(Response::text(400, "Requests here have no body."));
    }
    match head.path() {
        "/" | "/index.html" => {
            let head_only = head.method == "HEAD";
            if head.method != "GET" && !head_only {
                return Err(Response::text(405, "GET only."));
            }
            let headers = page::headers(host, proxied);
            // The connection ends after this reply either way; a peer that
            // left needs none.
            let _ = http::respond(
                stream,
                &Response {
                    status: 200,
                    content_type: "text/html; charset=utf-8",
                    headers: &headers,
                    body: page::HTML.as_bytes(),
                    head_only,
                },
            );
            Ok(())
        }
        "/ws" => upgrade(stream, head, &format!("{scheme}://{host}")),
        path => {
            // The web app's manifest, worker and icons (D517): the same
            // Host check as the page, GET only, nothing secret in any.
            let Some((content_type, body, extra)) = page::asset(path) else {
                return Err(Response::text(404, "Not here."));
            };
            if head.method != "GET" {
                return Err(Response::text(405, "GET only."));
            }
            let headers: Vec<String> = extra.iter().map(|line| (*line).to_string()).collect();
            // As for the page: the connection ends after this reply either
            // way, and a peer that left needs none.
            let _ = http::respond(
                stream,
                &Response {
                    status: 200,
                    content_type,
                    headers: &headers,
                    body,
                    head_only: false,
                },
            );
            Ok(())
        }
    }
}

/// Validates a WebSocket upgrade and answers it with 101.
fn upgrade(stream: &mut TcpStream, head: &Head, own: &str) -> Result<(), Response<'static>> {
    if head.method != "GET" {
        return Err(Response::text(405, "GET only."));
    }
    let origin_ok = head
        .header("origin")
        .is_some_and(|origin| origin.eq_ignore_ascii_case(own));
    if !origin_ok {
        log::warn!("remote: refused a WebSocket from another origin");
        return Err(Response::text(
            403,
            "Only Strobe's own remote page may connect here.",
        ));
    }
    if !head.lists("connection", "upgrade") || !head.lists("upgrade", "websocket") {
        return Err(Response::text(
            426,
            "This address takes WebSocket upgrades.",
        ));
    }
    if head.header("sec-websocket-version") != Some("13") {
        return Err(Response {
            headers: VERSION_HEADER.as_slice(),
            ..Response::text(426, "WebSocket version 13 only.")
        });
    }
    let Some(key) = head
        .header("sec-websocket-key")
        .filter(|k| ws::valid_key(k))
    else {
        return Err(Response::text(
            400,
            "A valid Sec-WebSocket-Key is required.",
        ));
    };
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        ws::accept_key(key)
    );
    use io::Write as _;
    stream
        .write_all(response.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|_| Response::text(400, "The upgrade could not be written."))
}

/// The header a 426 for a wrong version carries.
static VERSION_HEADER: LazyLock<Vec<String>> =
    LazyLock::new(|| vec!["Sec-WebSocket-Version: 13".to_string()]);

/// What kind of device a connection is, from its User-Agent: one word,
/// never the whole string. The desktop names a phone by it and its address.
fn platform(head: &Head) -> &'static str {
    let agent = head.header("user-agent").unwrap_or_default();
    [
        ("Strobe Desktop", "Strobe desktop"),
        ("iPhone", "iPhone"),
        ("iPad", "iPad"),
        ("Android", "Android"),
        ("Macintosh", "Mac"),
        ("Windows", "Windows"),
        ("CrOS", "Chromebook"),
        ("Linux", "Linux"),
    ]
    .iter()
    .find(|(needle, _)| agent.contains(needle))
    .map_or("Device", |(_, kind)| kind)
}

/// After a close frame: stops sending, then reads and discards what the
/// client still sends for a moment. Closing with unread data makes the
/// system reset the connection, and a reset can destroy the close frame
/// (with its reason, "revoked" say) before the phone reads it.
fn linger(stream: &mut TcpStream) {
    /// Read at most this much.
    const BYTES: usize = 64 * 1024;
    /// For at most this long.
    const TIME: std::time::Duration = std::time::Duration::from_millis(250);
    use io::Read as _;
    // Lingering is best effort; a closed socket needs no half-close.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let until = Instant::now() + TIME;
    let mut sink = [0_u8; 4096];
    let mut total = 0;
    while total < BYTES && Instant::now() < until {
        match stream.read(&mut sink) {
            Ok(0) => break,
            Ok(read) => total += read,
            Err(error) if http::timed_out(&error) => {}
            Err(_) => break,
        }
    }
}

/// Where the protocol stands on this connection.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Waiting for `hello`.
    Hello,
    /// Said hello; must pair or authenticate.
    Unauthenticated,
    /// Gave the right code; the operator has not answered.
    Approval(u64),
    /// A paired client.
    Authenticated(String),
}

/// A token bucket.
#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Why the session ends.
enum End {
    /// Send this close frame first.
    Close(u16, &'static str),
    /// The socket is gone; just stop.
    Gone,
}

struct Session<'a> {
    stream: TcpStream,
    shared: &'a Shared,
    conn: u64,
    peer: SocketAddr,
    /// Its kind, from the User-Agent ([`platform`]).
    platform: &'static str,
    /// `PLATFORM at ADDRESS`.
    label: String,
    /// The name it asked under (empty for a code) and the label the
    /// operator was shown: what a pairing approved for it keeps (D500).
    asked: (String, String),
    reader: Reader,
    phase: Phase,
    bucket: Bucket,
    failures: u32,
    heard: Instant,
    since_auth_wait: Instant,
    sent_revision: Option<u64>,
    desktop_revision: Option<u64>,
    clipboard_revision: Option<u64>,
    /// The controls publication last sent, and the banks whose definitions
    /// went with it or before (D513): later sends leave those out.
    controls_revision: Option<u64>,
    banks_sent: Vec<u64>,
    /// This connection asked for the controls (D513), so it is also sent
    /// where each picture has their rows (D515).
    controls_asked: bool,
    desktop_open: bool,
    desktop_pairing: bool,
    desktop_authorized: bool,
    /// The `Host` this connection was opened for, if it names the address
    /// the connection reached; `None` refuses trusted pairing (D288).
    host: Option<String>,
    /// The trust challenge waiting for its proof (D288). Taken, not
    /// cloned, when answered: a nonce answers once.
    trust_challenge: Option<Challenge>,
    /// This connection's trusted pairing, removed when it ends.
    trusted_client: Option<String>,
}

/// What a controller is told when trusted pairing is not on. One text for
/// "no trust file" and "full desktop access is off", so a scan learns
/// nothing about how the host was started.
const TRUST_UNAVAILABLE: &str = "Trusted pairing is not available on this host";

impl<'a> Session<'a> {
    fn new(
        stream: TcpStream,
        shared: &'a Shared,
        conn: u64,
        peer: SocketAddr,
        (platform, label): (&'static str, String),
        host: Option<String>,
        leftover: Vec<u8>,
    ) -> Self {
        let now = Instant::now();
        Self {
            stream,
            shared,
            conn,
            peer,
            platform,
            label,
            asked: (String::new(), String::new()),
            reader: Reader::new(shared.limits.max_message, leftover),
            phase: Phase::Hello,
            bucket: Bucket {
                tokens: shared.limits.burst,
                last: now,
            },
            failures: 0,
            heard: now,
            since_auth_wait: now,
            sent_revision: None,
            desktop_revision: None,
            clipboard_revision: None,
            controls_revision: None,
            banks_sent: Vec::new(),
            controls_asked: false,
            desktop_open: false,
            desktop_pairing: false,
            desktop_authorized: false,
            host,
            trust_challenge: None,
            trusted_client: None,
        }
    }

    fn run(mut self) {
        let end = loop {
            if let Err(end) = self.step() {
                break end;
            }
        };
        self.shared.remote.desktop.release(self.conn);
        if let Some(id) = self.trusted_client.take() {
            self.shared.clients.forget_trusted(&id);
        } else if let Phase::Authenticated(id) = &self.phase {
            // Last seen is when it left, not when it came (D500).
            self.shared.clients.seen(id);
        }
        if let Phase::Approval(id) = self.phase {
            self.shared.pairing().forget(id);
        }
        if let End::Close(code, reason) = end
            && ws::write_close(&mut self.stream, code, reason).is_ok()
        {
            linger(&mut self.stream);
        }
        // Closing: a socket already closed by its peer needs no shutdown.
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    /// One read (or tick), then the timers and the state push.
    fn step(&mut self) -> Result<(), End> {
        if self.shared.stopping() {
            return Err(End::Close(close::GOING_AWAY, "the remote was switched off"));
        }
        if self.shared.revoked(self.conn) {
            return Err(End::Close(close::POLICY, "revoked on the desktop"));
        }
        match self.reader.read(&mut self.stream) {
            Ok(Some(message)) => {
                self.heard = Instant::now();
                self.shared.heard(self.conn);
                self.spend()?;
                self.message(message)?;
            }
            Ok(None) => {}
            Err(error) => {
                if let ws::WsError::Io(io) = &error {
                    log::debug!("remote: reading from {} failed: {io}", self.peer);
                }
                return Err(match error.close_code() {
                    Some(code) => {
                        log::info!("remote: {} closed: {}", self.peer, error.reason());
                        End::Close(code, error.reason())
                    }
                    None => End::Gone,
                });
            }
        }
        self.timers()?;
        self.push_state()
    }

    /// Takes one token from the bucket, or ends a flooding connection.
    fn spend(&mut self) -> Result<(), End> {
        let limits = &self.shared.limits;
        // The desktop holding the lease sends its edits as they happen
        // (D513); anyone else keeps the phone's budget.
        let (rate, burst) = if self.desktop_open {
            (limits.desktop_rate_per_second, limits.desktop_burst)
        } else {
            (limits.rate_per_second, limits.burst)
        };
        let now = Instant::now();
        let elapsed = now
            .saturating_duration_since(self.bucket.last)
            .as_secs_f64();
        self.bucket.last = now;
        self.bucket.tokens = (self.bucket.tokens + elapsed * rate).min(burst);
        if self.bucket.tokens < 1.0 {
            log::warn!("remote: {} sent too many messages; dropped", self.peer);
            // If the error cannot be sent, the read loop ends this connection
            // on its next turn.
            let _ = self.send(&ServerMessage::error(ErrorCode::Busy, "too many messages"));
            return Err(End::Close(close::POLICY, "too many messages"));
        }
        self.bucket.tokens -= 1.0;
        Ok(())
    }

    fn timers(&mut self) -> Result<(), End> {
        let limits = &self.shared.limits;
        let now = Instant::now();
        if now.saturating_duration_since(self.heard) >= limits.idle_timeout {
            return Err(End::Close(close::GOING_AWAY, "idle"));
        }
        match self.phase.clone() {
            Phase::Hello | Phase::Unauthenticated => {
                if now.saturating_duration_since(self.since_auth_wait) >= limits.auth_timeout {
                    return Err(End::Close(close::POLICY, "pair or authenticate first"));
                }
            }
            Phase::Approval(id) => {
                let decision = self.shared.pairing().decision(id, now);
                match decision {
                    Decision::Waiting => {}
                    Decision::Approved => {
                        self.shared.pairing().forget(id);
                        self.approved()?;
                    }
                    Decision::Denied | Decision::Expired => {
                        self.shared.pairing().forget(id);
                        self.phase = Phase::Unauthenticated;
                        self.since_auth_wait = now;
                        let why = if decision == Decision::Denied {
                            "declined on the desktop"
                        } else {
                            "not approved in time; ask again"
                        };
                        log::info!("remote: pairing for {} {why}", self.peer);
                        self.send(&ServerMessage::error(ErrorCode::PairingRejected, why))?;
                    }
                }
            }
            Phase::Authenticated(id) => {
                // The trusted pairing belongs to the switch it was proved
                // under: with it off the session ends, whatever raced.
                if self.trusted_client.is_some() && !self.shared.remote.desktop.enabled() {
                    return Err(End::Close(close::POLICY, "desktop control disabled"));
                }
                // Every session belongs to its entry in the list. A revoke,
                // a revoke all or a trusted reconnect that replaced it can
                // land between the authentication (a token, an approval or
                // a proof) and this connection being marked with its client
                // id, which would leave the session running with nothing on
                // the list to revoke.
                if !self.shared.clients.contains(&id) {
                    return Err(End::Close(close::POLICY, "revoked on the desktop"));
                }
            }
        }
        Ok(())
    }

    /// A request with no code (D468, D469), labelled `label`, from a
    /// device that called itself `name`: in front of the operator, or
    /// refused with why.
    fn ask(&mut self, label: String, name: &str) -> Result<(), End> {
        self.asked = (name.to_owned(), label.clone());
        let attempt = self
            .shared
            .pairing()
            .request(label, self.peer.ip(), Instant::now());
        match attempt {
            Ok(id) => {
                log::info!(
                    "remote: {} ({name}) asked to pair{}; waiting for approval",
                    self.peer,
                    if self.desktop_pairing {
                        " for desktop control"
                    } else {
                        ""
                    }
                );
                self.phase = Phase::Approval(id);
                Ok(())
            }
            Err(refusal) => {
                let why = match refusal {
                    Refusal::Full => "too many devices are waiting for approval",
                    Refusal::Pending => "this device is already waiting for approval",
                    Refusal::Cooling => "the desktop said no a moment ago; try again later",
                    Refusal::NotOpen | Refusal::Wrong(_) | Refusal::Burned => "not now; try again",
                };
                log::info!("remote: refused a request from {}: {refusal:?}", self.peer);
                self.failed(ErrorCode::PairingRejected, why)
            }
        }
    }

    /// Trusted pairing, step one (D288): hand out a challenge.
    fn trust_start(&mut self, controller_id: String) -> Result<(), End> {
        let Some(trust) = self
            .shared
            .trust
            .as_ref()
            .filter(|_| self.shared.remote.desktop.enabled())
        else {
            log::info!(
                "remote: {} asked for trusted pairing; the host has no trust file or full \
                 desktop access is off",
                self.peer
            );
            return self.send(&ServerMessage::error(
                ErrorCode::Unavailable,
                TRUST_UNAVAILABLE,
            ));
        };
        let Some(host) = self.host.clone() else {
            log::warn!(
                "remote: {} asked for trusted pairing under a Host that is not the address it \
                 reached",
                self.peer
            );
            return self.send(&ServerMessage::error(
                ErrorCode::InvalidRequest,
                "for trusted pairing, open the host at its own IP address and port",
            ));
        };
        let now = Instant::now();
        let admitted = super::lock(&trust.limiter).admit(self.peer.ip(), now);
        if let Err(wait) = admitted {
            log::warn!(
                "remote: trusted pairing from {} paused after failed proofs ({} s left)",
                self.peer,
                wait.as_secs() + 1
            );
            // The close that follows ends the connection either way.
            let _ = self.send(&ServerMessage::error(
                ErrorCode::Busy,
                &format!(
                    "too many failed attempts; try again in {} s",
                    wait.as_secs() + 1
                ),
            ));
            return Err(End::Close(close::POLICY, "too many failed attempts"));
        }
        if self.shared.clients.trust_blocked(&controller_id) {
            log::warn!(
                "remote: refused trusted controller {controller_id} from {}: revoked on the host",
                self.peer
            );
            return self.send(&ServerMessage::error(
                ErrorCode::Unauthorized,
                "this controller was revoked on the host",
            ));
        }
        match Challenge::issue(&host, &controller_id, now) {
            Ok(challenge) => {
                let nonce = challenge.nonce_hex();
                self.trust_challenge = Some(challenge);
                self.send(&ServerMessage::TrustChallenge { nonce })
            }
            Err(error) => {
                log::warn!("remote: no random bytes for a trust challenge: {error}");
                self.send(&ServerMessage::error(
                    ErrorCode::Unavailable,
                    TRUST_UNAVAILABLE,
                ))
            }
        }
    }

    /// Trusted pairing, step two (D288): judge the proof and, if it holds,
    /// pair for full desktop control with no approval.
    fn trust_proof(&mut self, response: &str) -> Result<(), End> {
        let Some(challenge) = self.trust_challenge.take() else {
            return self.failed(
                ErrorCode::InvalidRequest,
                "no trust challenge is waiting; start again",
            );
        };
        let Some(trust) = self
            .shared
            .trust
            .as_ref()
            .filter(|_| self.shared.remote.desktop.enabled())
        else {
            return self.send(&ServerMessage::error(
                ErrorCode::Unavailable,
                TRUST_UNAVAILABLE,
            ));
        };
        let now = Instant::now();
        let controller_id = challenge.controller_id().to_owned();
        let verdict = challenge.verify(&trust.secret, response, now);
        let why = match verdict {
            Verdict::Accepted => None,
            Verdict::Expired => Some("the challenge expired; try again"),
            Verdict::Wrong => Some("that trust file does not match this host"),
        };
        if let Some(why) = why {
            let recorded = super::lock(&trust.limiter).failure(self.peer.ip(), now);
            log::warn!(
                "remote: trusted pairing for {controller_id} from {} refused ({verdict:?}); \
                 failure {} from this address{}{}",
                self.peer,
                recorded.failures,
                recorded
                    .peer_pause
                    .map_or(String::new(), |p| format!(", paused for {} s", p.as_secs())),
                recorded.global_pause.map_or(String::new(), |p| format!(
                    ", and every address paused for {} s",
                    p.as_secs()
                )),
            );
            return self.failed(ErrorCode::PairingRejected, why);
        }
        super::lock(&trust.limiter).success(self.peer.ip());
        let label = format!(
            "{}{controller_id}, {}",
            crate::desktop::TRUSTED_LABEL_PREFIX,
            self.label
        );
        let device = self.device(&controller_id);
        match self
            .shared
            .clients
            .pair_trusted(&label, &device, &controller_id)
        {
            Ok(pairing) => {
                // The switch may have gone off while the proof was judged.
                if !self.shared.remote.desktop.enabled() {
                    self.shared.clients.forget_trusted(&pairing.client_id);
                    return self.send(&ServerMessage::error(
                        ErrorCode::Unavailable,
                        TRUST_UNAVAILABLE,
                    ));
                }
                self.shared.supersede(&pairing.superseded);
                log::info!(
                    "remote: {} paired as trusted controller {controller_id} (client {}) \
                     with the trust file; no approval was asked",
                    self.peer,
                    pairing.client_id
                );
                self.trusted_client = Some(pairing.client_id.clone());
                self.desktop_authorized = true;
                self.authenticated(pairing.client_id)
            }
            Err(TrustedRefusal::Revoked) => {
                log::warn!(
                    "remote: refused trusted controller {controller_id} from {}: revoked on the host",
                    self.peer
                );
                self.send(&ServerMessage::error(
                    ErrorCode::Unauthorized,
                    "this controller was revoked on the host",
                ))
            }
            Err(TrustedRefusal::Full) => self.send(&ServerMessage::error(
                ErrorCode::Unavailable,
                "too many remotes are paired; revoke one on the host",
            )),
            Err(TrustedRefusal::NoRandom(error)) => {
                log::warn!("remote: no random bytes for a trusted pairing: {error}");
                self.send(&ServerMessage::error(
                    ErrorCode::Unavailable,
                    TRUST_UNAVAILABLE,
                ))
            }
        }
    }

    /// The operator approved: issue the client's token.
    fn approved(&mut self) -> Result<(), End> {
        if self.desktop_pairing && !self.shared.remote.desktop.enabled() {
            self.phase = Phase::Unauthenticated;
            self.since_auth_wait = Instant::now();
            return self.send(&ServerMessage::error(
                ErrorCode::Unavailable,
                "Full desktop access was disabled before approval; open pairing again",
            ));
        }
        // What the operator approved, the name included (D500).
        let (name, label) = self.asked.clone();
        let device = self.device(&name);
        match self
            .shared
            .clients
            .pair_device(&label, &device, self.desktop_pairing)
        {
            Ok((client_id, token)) => {
                log::info!("remote: {} paired as client {client_id}", self.peer);
                self.send(&ServerMessage::Paired {
                    client_id: client_id.clone(),
                    token: token.expose().to_string(),
                })?;
                self.desktop_authorized = self.desktop_pairing;
                self.authenticated(client_id)
            }
            Err(error) => {
                log::warn!("remote: could not pair {}: {error}", self.peer);
                self.phase = Phase::Unauthenticated;
                self.since_auth_wait = Instant::now();
                self.send(&ServerMessage::error(
                    ErrorCode::PairingRejected,
                    &format!("the desktop could not save the pairing: {error}"),
                ))
            }
        }
    }

    /// This connection's device, under `name` (D500).
    fn device(&self, name: &str) -> Device {
        Device {
            name: name.to_owned(),
            platform: self.platform.to_owned(),
            address: Some(self.peer.ip()),
        }
    }

    /// Enters the authenticated phase and sends the whole state.
    fn authenticated(&mut self, client_id: String) -> Result<(), End> {
        self.shared.set_client(self.conn, &client_id);
        self.send(&ServerMessage::Authenticated {
            client_id: client_id.clone(),
        })?;
        self.phase = Phase::Authenticated(client_id);
        self.sent_revision = None;
        self.push_state()
    }

    /// Sends the snapshot if its revision moved since the last one sent.
    fn push_state(&mut self) -> Result<(), End> {
        if !matches!(self.phase, Phase::Authenticated(_)) {
            return Ok(());
        }
        if self.desktop_open {
            if !self.shared.remote.desktop.owns(self.conn) {
                return Err(End::Close(close::POLICY, "desktop control disabled"));
            }
            if let Some((revision, frame)) = self.shared.remote.desktop.frame()
                && self.desktop_revision != Some(revision)
            {
                // The rows go first, so the frame finds them waiting (D515).
                if self.controls_asked
                    && let Some((at, layout)) = self.shared.remote.desktop.layout()
                    && at == revision
                {
                    self.send(&layout)?;
                }
                self.send(&frame)?;
                self.desktop_revision = Some(revision);
            }
            if let Some((revision, message)) = self.shared.remote.desktop.clipboard()
                && self.clipboard_revision != Some(revision)
            {
                self.send(&message)?;
                self.clipboard_revision = Some(revision);
            }
            self.push_controls()?;
        }
        let (revision, snapshot) = self.shared.remote.snapshot();
        if self.sent_revision == Some(revision) {
            return Ok(());
        }
        self.send(&snapshot.to_message(revision))?;
        self.sent_revision = Some(revision);
        Ok(())
    }

    /// Sends the host's controls if a newer publication is out (D513). A
    /// bank's definitions go once per connection, the first time its id is
    /// sent; after that only its values do.
    fn push_controls(&mut self) -> Result<(), End> {
        let Some((revision, banks)) = self.shared.remote.desktop.controls() else {
            return Ok(());
        };
        if self.controls_revision == Some(revision) {
            return Ok(());
        }
        let banks: Vec<_> = banks
            .iter()
            .map(|bank| crate::desktop::SyncedBank {
                defs: bank
                    .defs
                    .clone()
                    .filter(|_| !self.banks_sent.contains(&bank.id)),
                ..bank.clone()
            })
            .collect();
        self.banks_sent = banks.iter().map(|bank| bank.id).collect();
        self.send(&ServerMessage::DesktopControls { revision, banks })?;
        self.controls_revision = Some(revision);
        Ok(())
    }

    /// Writes one message; a write that fails or times out drops the client.
    fn send(&mut self, message: &ServerMessage) -> Result<(), End> {
        let text = serde_json::to_string(message).map_err(|_| End::Gone)?;
        ws::write_text(&mut self.stream, &text).map_err(|error| {
            if http::timed_out(&error) {
                log::warn!("remote: {} is not reading; dropped", self.peer);
            }
            End::Gone
        })
    }

    /// A failed pairing or authentication attempt; too many end it.
    fn failed(&mut self, code: ErrorCode, why: &str) -> Result<(), End> {
        self.failures += 1;
        self.send(&ServerMessage::error(code, why))?;
        if self.failures >= MAX_FAILURES {
            log::warn!("remote: {} failed {MAX_FAILURES} times; dropped", self.peer);
            return Err(End::Close(close::POLICY, "too many failed attempts"));
        }
        Ok(())
    }

    fn message(&mut self, message: Message) -> Result<(), End> {
        match message {
            Message::Text(text) => self.text(&text),
            Message::Ping(payload) => {
                ws::write_pong(&mut self.stream, &payload).map_err(|_| End::Gone)
            }
            Message::Pong => Ok(()),
            Message::Close(code) => {
                log::debug!("remote: {} closed the connection ({code:?})", self.peer);
                Err(End::Close(close::NORMAL, "bye"))
            }
        }
    }

    fn text(&mut self, text: &str) -> Result<(), End> {
        let message = match decode_client(text) {
            Ok(message) => message,
            Err(code) => {
                let fatal = self.phase == Phase::Hello;
                self.send(&ServerMessage::error(
                    code,
                    "that message was not understood",
                ))?;
                return if fatal || code == ErrorCode::VersionMismatch {
                    Err(End::Close(close::POLICY, "say hello first"))
                } else {
                    Ok(())
                };
            }
        };
        match (self.phase.clone(), message) {
            (Phase::Unauthenticated, ClientMessage::TrustStart { controller_id }) => {
                self.trust_start(controller_id)
            }
            (Phase::Unauthenticated, ClientMessage::TrustProof { response }) => {
                self.trust_proof(&response)
            }
            (Phase::Authenticated(client_id), ClientMessage::DesktopOpen {}) => {
                let desktop = &self.shared.remote.desktop;
                // Our own earlier connection, gone silent, gives its lease
                // back (D407); nobody else's ever does.
                let leased = self.desktop_authorized
                    && (desktop.acquire(self.conn)
                        || (self.shared.reclaim_dead_lease(self.conn, &client_id) && {
                            log::info!(
                                "remote: client {client_id} reconnected from {}; its earlier \
                                 connection had gone silent and gave up the desktop lease (D407)",
                                self.peer
                            );
                            desktop.acquire(self.conn)
                        }));
                if leased {
                    self.desktop_open = true;
                    self.desktop_revision = None;
                    self.push_state()
                } else {
                    self.send(&ServerMessage::error(ErrorCode::Unavailable, "Enable full desktop control on the host; only one desktop can control it at a time"))
                }
            }
            (Phase::Authenticated(_), ClientMessage::DesktopInput { events }) => {
                if self.desktop_open && self.shared.remote.desktop.submit(self.conn, events) {
                    Ok(())
                } else {
                    Err(End::Close(close::POLICY, "desktop input refused"))
                }
            }
            (Phase::Authenticated(_), ClientMessage::DesktopControlsOpen {}) => {
                if self.desktop_open && self.shared.remote.desktop.want_controls(self.conn) {
                    self.controls_asked = true;
                    self.controls_revision = None;
                    self.banks_sent.clear();
                    self.push_state()
                } else {
                    self.send(&ServerMessage::error(
                        ErrorCode::Unavailable,
                        "only the desktop holding control gets the controls",
                    ))
                }
            }
            (Phase::Authenticated(_), ClientMessage::DesktopControl { edits }) => {
                if self.desktop_open && self.shared.remote.desktop.submit_controls(self.conn, edits)
                {
                    Ok(())
                } else {
                    Err(End::Close(close::POLICY, "control edits refused"))
                }
            }
            (
                Phase::Unauthenticated | Phase::Approval(_),
                ClientMessage::DesktopOpen {}
                | ClientMessage::DesktopInput { .. }
                | ClientMessage::DesktopControlsOpen {}
                | ClientMessage::DesktopControl { .. },
            ) => self.send(&ServerMessage::error(
                ErrorCode::Unauthorized,
                "pair this desktop first",
            )),
            (_, ClientMessage::Ping {}) => self.send(&ServerMessage::Pong {}),
            (Phase::Hello, ClientMessage::Hello { protocol_version }) => {
                match negotiate(protocol_version) {
                    Ok(_) => {
                        self.phase = Phase::Unauthenticated;
                        self.send(&ServerMessage::Hello {
                            protocol_version: PROTOCOL_VERSION,
                            paired: false,
                        })
                    }
                    Err(code) => {
                        // If the error cannot be sent, the read loop ends
                        // this connection on its next turn.
                        let _ = self.send(&ServerMessage::error(
                            code,
                            &format!("this Strobe speaks protocol version {PROTOCOL_VERSION}"),
                        ));
                        Err(End::Close(close::POLICY, "version mismatch"))
                    }
                }
            }
            (Phase::Hello, _) => {
                // If the error cannot be sent, the read loop ends this
                // connection on its next turn.
                let _ = self.send(&ServerMessage::error(
                    ErrorCode::InvalidRequest,
                    "say hello first",
                ));
                Err(End::Close(close::POLICY, "say hello first"))
            }
            (Phase::Unauthenticated, ClientMessage::Authenticate { token }) => {
                match self
                    .shared
                    .clients
                    .authenticate_from(&token, self.peer.ip())
                {
                    Some(auth) => {
                        log::info!(
                            "remote: client {} connected from {}",
                            auth.client_id,
                            self.peer
                        );
                        let client_id = auth.client_id.clone();
                        self.desktop_authorized = auth.desktop;
                        self.shared.set_client(self.conn, &client_id);
                        self.send(&ServerMessage::Authenticated {
                            client_id: client_id.clone(),
                        })?;
                        if let Some(next) = auth.rotated {
                            self.send(&ServerMessage::Paired {
                                client_id: client_id.clone(),
                                token: next.expose().to_string(),
                            })?;
                        }
                        self.phase = Phase::Authenticated(client_id);
                        self.sent_revision = None;
                        self.push_state()
                    }
                    None => {
                        log::info!("remote: {} sent a token that is not valid", self.peer);
                        self.failed(ErrorCode::Unauthorized, "not paired, or revoked")
                    }
                }
            }
            (
                Phase::Unauthenticated,
                request @ (ClientMessage::Pair { .. } | ClientMessage::PairDesktop { .. }),
            ) => {
                let (code, desktop) = match request {
                    ClientMessage::Pair { code } => (code, false),
                    ClientMessage::PairDesktop { code } => (code, true),
                    ClientMessage::Hello { .. }
                    | ClientMessage::Authenticate { .. }
                    | ClientMessage::Command { .. }
                    | ClientMessage::Ping {}
                    | ClientMessage::DesktopOpen {}
                    | ClientMessage::DesktopInput { .. }
                    | ClientMessage::DesktopControlsOpen {}
                    | ClientMessage::DesktopControl { .. }
                    | ClientMessage::TrustStart { .. }
                    | ClientMessage::TrustProof { .. }
                    | ClientMessage::RequestDesktop { .. }
                    | ClientMessage::RequestPair { .. } => return Err(End::Gone),
                };
                if desktop && !self.shared.remote.desktop.enabled() {
                    return self.send(&ServerMessage::error(
                        ErrorCode::Unavailable,
                        "Enable full desktop control on the host before pairing this desktop",
                    ));
                }
                self.desktop_pairing = desktop;
                let label = if desktop {
                    format!("{}{}", crate::desktop::LABEL_PREFIX, self.label)
                } else {
                    self.label.clone()
                };
                self.asked = (String::new(), label.clone());
                let attempt = self.shared.pairing().try_code(&code, label, Instant::now());
                match attempt {
                    Ok(id) => {
                        log::info!(
                            "remote: {} gave the pairing code; waiting for approval",
                            self.peer
                        );
                        self.phase = Phase::Approval(id);
                        Ok(())
                    }
                    Err(refusal) => {
                        let why = match refusal {
                            Refusal::NotOpen => {
                                "pairing is not open on the desktop, or the code expired"
                            }
                            Refusal::Wrong(_) => "wrong code",
                            Refusal::Burned => "too many wrong codes; open pairing again",
                            Refusal::Full => "too many devices are waiting for approval",
                            Refusal::Pending | Refusal::Cooling => "not now; try again",
                        };
                        log::info!(
                            "remote: refused a pairing code from {}: {refusal:?}",
                            self.peer
                        );
                        self.failed(ErrorCode::PairingRejected, why)
                    }
                }
            }
            (Phase::Unauthenticated, ClientMessage::RequestPair { name }) => {
                self.desktop_pairing = false;
                let label = format!("{name}, {}", self.label);
                self.ask(label, &name)
            }
            (Phase::Unauthenticated, ClientMessage::RequestDesktop { name }) => {
                if !self.shared.remote.desktop.enabled() {
                    return self.send(&ServerMessage::error(
                        ErrorCode::Unavailable,
                        "Full desktop control is off on the host",
                    ));
                }
                self.desktop_pairing = true;
                let label = format!(
                    "{}{name} at {}",
                    crate::desktop::LABEL_PREFIX,
                    self.peer.ip()
                );
                self.ask(label, &name)
            }
            (Phase::Unauthenticated | Phase::Approval(_), ClientMessage::Command { .. }) => self
                .send(&ServerMessage::error(
                    ErrorCode::Unauthorized,
                    "pair this device first",
                )),
            (
                Phase::Authenticated(_),
                ClientMessage::Command {
                    request_id,
                    command,
                },
            ) => {
                let reply = match self.shared.remote.submit(&request_id, command) {
                    Ok(pending) => pending.wait(self.shared.limits.command_timeout),
                    Err(code) => Err(code),
                };
                let result = match reply {
                    Ok(Answer::Accepted) => (true, None),
                    Ok(Answer::Rejected(reason)) => (false, Some(reason)),
                    Err(ErrorCode::Busy) => (false, Some("Strobe is busy; try again".to_string())),
                    Err(ErrorCode::Unavailable) => {
                        // If the error cannot be sent, the read loop ends
                        // this connection on its next turn.
                        let _ = self.send(&ServerMessage::error(
                            ErrorCode::Unavailable,
                            "the remote was switched off",
                        ));
                        return Err(End::Close(close::GOING_AWAY, "the remote was switched off"));
                    }
                    Err(other) => (false, Some(format!("{other:?}"))),
                };
                self.send(&ServerMessage::CommandResult {
                    request_id,
                    accepted: result.0,
                    reason: result.1,
                })?;
                self.push_state()
            }
            (
                Phase::Unauthenticated | Phase::Approval(_) | Phase::Authenticated(_),
                ClientMessage::Hello { .. },
            )
            | (
                Phase::Approval(_) | Phase::Authenticated(_),
                ClientMessage::Pair { .. }
                | ClientMessage::PairDesktop { .. }
                | ClientMessage::Authenticate { .. }
                | ClientMessage::TrustStart { .. }
                | ClientMessage::TrustProof { .. }
                | ClientMessage::RequestDesktop { .. }
                | ClientMessage::RequestPair { .. },
            ) => self.send(&ServerMessage::error(
                ErrorCode::InvalidRequest,
                "not now: this connection is past that step",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(agent: &str) -> Head {
        Head {
            method: "GET".into(),
            target: "/ws".into(),
            headers: vec![("user-agent".into(), agent.into())],
            leftover: Vec::new(),
        }
    }

    #[test]
    fn a_device_is_named_by_one_word_of_its_user_agent() {
        assert_eq!(
            platform(&head(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Secret/1.0"
            )),
            "iPhone",
            "an iPhone, and nothing else of the string"
        );
        assert_eq!(
            platform(&head("Mozilla/5.0 (Linux; Android 14; Pixel 8)")),
            "Android",
            "Android before Linux"
        );
        assert_eq!(platform(&head("")), "Device", "unknown");
    }
}
