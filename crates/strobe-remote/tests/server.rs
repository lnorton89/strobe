// ============================================================================
// Module:       server (integration test)
// Description:  The LAN listener end to end over real sockets: the page,
//               Host and Origin checks, pairing, tokens, revocation, restart,
//               stop, and the adversarial cases (oversized, malformed, flood,
//               slow client, too many connections, idle), plus a log capture
//               that checks no secret is ever written to the log.
//
// Dependencies: strobe_remote; serde_json; log; std::net (a minimal
//               WebSocket client written here)
// ============================================================================

//! The listener, tested the way a phone and an attacker would use it.
//!
//! Every test binds `127.0.0.1` on a free port, so they run in parallel and
//! need no network. The WebSocket client below is deliberately tiny: it
//! masks what it sends, reads unmasked frames, and can send anything at all,
//! which is the point of the adversarial tests.

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use strobe_remote::bridge::{self, Answer, AppEnd, Snapshot};
use strobe_remote::clients::Clients;
use strobe_remote::server::{Limits, Server, ServerConfig, StartError};

// ------------------------------------------------------------------ logging

/// Every log line any test wrote, for the redaction test.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        let line = format!("{} {}", record.level(), record.args());
        LOG.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(line);
    }
    fn flush(&self) {}
}

static CAPTURE: Capture = Capture;

fn capture_logs() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        if log::set_logger(&CAPTURE).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
        }
    });
}

fn logged() -> String {
    LOG.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .join("\n")
}

// ------------------------------------------------------------------ fixtures

/// Short timers so the timeout tests finish quickly.
fn limits() -> Limits {
    Limits {
        tick: Duration::from_millis(10),
        command_timeout: Duration::from_millis(500),
        ..Limits::default()
    }
}

/// A listener on a free loopback port, a fake app that answers every
/// command and publishes a snapshot, and the paired-client list.
struct Rig {
    desktop: strobe_remote::desktop::Desktop,
    server: Server,
    clients: Clients,
    _app: FakeApp,
}

/// The fake app's thread, stopped when dropped.
struct FakeApp {
    thread: Option<JoinHandle<AppEnd>>,
    stop: Arc<AtomicBool>,
}

impl Drop for FakeApp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn rig_with(limits: Limits, clients: Clients) -> Result<Rig> {
    rig_on(limits, clients, None)
}

fn rig_on(limits: Limits, clients: Clients, proxied: Option<&str>) -> Result<Rig> {
    capture_logs();
    let (remote, app_end) = bridge::bridge(4);
    let desktop = app_end.desktop.clone();
    let server = Server::start(
        ServerConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            limits,
            proxied: proxied.map(str::to_string),
        },
        remote,
        clients.clone(),
    )
    .map_err(|e| anyhow!("{e}"))?;
    let app_stop = Arc::new(AtomicBool::new(false));
    let app = {
        let stop = Arc::clone(&app_stop);
        std::thread::spawn(move || {
            let snapshot = Snapshot {
                current: Some("Aurora".into()),
                ..Snapshot::default()
            };
            while !stop.load(Ordering::SeqCst) {
                app_end.publish(&snapshot);
                while let Some(incoming) = app_end.next() {
                    incoming.answer(Answer::Accepted);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            app_end
        })
    };
    Ok(Rig {
        desktop,
        server,
        clients,
        _app: FakeApp {
            thread: Some(app),
            stop: app_stop,
        },
    })
}

fn rig() -> Result<Rig> {
    rig_with(limits(), Clients::in_memory())
}

impl Rig {
    fn addr(&self) -> SocketAddr {
        self.server.addr()
    }

    fn host(&self) -> String {
        self.addr().to_string()
    }
}

/// Polls `check` for up to five seconds.
fn eventually(what: &str, mut check: impl FnMut() -> bool) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if check() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    bail!("timed out waiting for {what}")
}

// ------------------------------------------------------------- HTTP client

/// Sends a raw request and returns the whole response.
fn http(addr: SocketAddr, request: &str) -> Result<String> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(request.as_bytes())?;
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    Ok(String::from_utf8_lossy(&response).into_owned())
}

fn status(response: &str) -> u16 {
    response
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

// -------------------------------------------------------- WebSocket client

/// What the server sent.
#[derive(Debug, PartialEq)]
enum Frame {
    Text(String),
    Close(u16),
    Pong,
    /// The socket closed with no close frame.
    Eof,
}

struct Ws {
    stream: TcpStream,
    buffer: Vec<u8>,
}

/// The handshake a browser sends, with `host` and `origin` as given.
fn upgrade_request(host: &str, origin: Option<&str>) -> String {
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    format!(
        "GET /ws HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         User-Agent: Mozilla/5.0 (iPhone; CPU iPhone OS 17_0)\r\n{origin}\r\n"
    )
}

/// Opens a WebSocket the way the page does.
fn open(rig: &Rig) -> Result<Ws> {
    let host = rig.host();
    open_as(rig.addr(), &host, Some(&format!("http://{host}")))
}

fn open_as(addr: SocketAddr, host: &str, origin: Option<&str>) -> Result<Ws> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(upgrade_request(host, origin).as_bytes())?;
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let read = stream.read(&mut byte)?;
        ensure!(
            read == 1,
            "the handshake ended early: {:?}",
            String::from_utf8_lossy(&head)
        );
        head.extend_from_slice(&byte);
    }
    let head = String::from_utf8(head)?;
    ensure!(head.starts_with("HTTP/1.1 101 "), "not upgraded: {head}");
    ensure!(
        head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"),
        "the RFC's accept key: {head}"
    );
    Ok(Ws {
        stream,
        buffer: Vec::new(),
    })
}

/// A masked client frame.
fn frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mask = [0x11_u8, 0x22, 0x33, 0x44];
    let mut out = vec![(if fin { 0x80 } else { 0 }) | opcode];
    match payload.len() {
        len @ 0..=125 => out.push(0x80 | len as u8),
        len @ 126..=0xffff => {
            out.push(0x80 | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            out.push(0x80 | 127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().zip(mask.iter().cycle()).map(|(b, k)| b ^ k));
    out
}

impl Ws {
    fn raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes)?;
        Ok(())
    }

    fn send(&mut self, value: &Value) -> Result<()> {
        self.raw(&frame(true, 0x1, value.to_string().as_bytes()))
    }

    fn fill(&mut self, need: usize) -> Result<bool> {
        while self.buffer.len() < need {
            let mut chunk = [0_u8; 4096];
            let read = match self.stream.read(&mut chunk) {
                Ok(read) => read,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    0
                }
                Err(error) => return Err(error.into()),
            };
            if read == 0 {
                return Ok(false);
            }
            self.buffer
                .extend_from_slice(chunk.get(..read).unwrap_or_default());
        }
        Ok(true)
    }

    /// The next frame the server sent.
    fn recv(&mut self) -> Result<Frame> {
        if !self.fill(2)? {
            return Ok(Frame::Eof);
        }
        let b0 = self.buffer.first().copied().unwrap_or_default();
        let b1 = self.buffer.get(1).copied().unwrap_or_default();
        ensure!(b1 & 0x80 == 0, "server frames are never masked");
        let (len, at) = match b1 & 0x7f {
            126 => {
                ensure!(self.fill(4)?, "eof in a length");
                let bytes = [self.buffer[2], self.buffer[3]];
                (usize::from(u16::from_be_bytes(bytes)), 4)
            }
            127 => {
                ensure!(self.fill(10)?, "eof in a length");
                let mut bytes = [0_u8; 8];
                bytes.copy_from_slice(&self.buffer[2..10]);
                (usize::try_from(u64::from_be_bytes(bytes))?, 10)
            }
            short => (usize::from(short), 2),
        };
        if !self.fill(at + len)? {
            // The server dropped the socket mid-frame (a reset after a flood).
            return Ok(Frame::Eof);
        }
        let payload: Vec<u8> = self.buffer.drain(..at + len).skip(at).collect();
        Ok(match b0 & 0x0f {
            0x1 => Frame::Text(String::from_utf8(payload)?),
            0x8 => Frame::Close(
                payload
                    .get(..2)
                    .map_or(0, |b| u16::from_be_bytes([b[0], b[1]])),
            ),
            0xA => Frame::Pong,
            other => bail!("unexpected opcode {other}"),
        })
    }

    /// The next text message, as JSON.
    fn json(&mut self) -> Result<Value> {
        match self.recv()? {
            Frame::Text(text) => Ok(serde_json::from_str(&text)?),
            other @ (Frame::Close(_) | Frame::Pong | Frame::Eof) => {
                bail!("expected a message, got {other:?}")
            }
        }
    }

    /// Reads messages until one of type `kind`; fails on a close.
    fn until(&mut self, kind: &str) -> Result<Value> {
        for _ in 0..50 {
            let message = self.json()?;
            if message["type"] == kind {
                return Ok(message);
            }
        }
        bail!("no {kind} message")
    }

    /// Reads until the server closes; the close code (0 for none).
    fn closed(&mut self) -> Result<u16> {
        for _ in 0..200 {
            match self.recv()? {
                Frame::Close(code) => return Ok(code),
                Frame::Eof => return Ok(0),
                Frame::Text(_) | Frame::Pong => {}
            }
        }
        bail!("the server never closed")
    }

    fn hello(&mut self) -> Result<()> {
        self.send(&json!({"type": "hello", "protocol_version": 1}))?;
        let hello = self.json()?;
        ensure!(
            hello == json!({"type":"hello","protocol_version":1,"paired":false}),
            "{hello}"
        );
        Ok(())
    }
}

/// Pairs a new phone through the whole flow; returns its id and token.
fn pair(rig: &Rig) -> Result<(String, String, Ws)> {
    pair_as(rig, false)
}

fn pair_as(rig: &Rig, desktop: bool) -> Result<(String, String, Ws)> {
    let mut ws = open(rig)?;
    ws.hello()?;
    let code = rig.server.open_pairing()?;
    ws.send(&json!({"type": if desktop { "pair_desktop" } else { "pair" }, "code": code}))?;
    eventually("the request to reach the desktop", || {
        !rig.server.pairing_requests().is_empty()
    })?;
    let request = rig
        .server
        .pairing_requests()
        .into_iter()
        .next()
        .context("a request")?;
    ensure!(
        request.label.starts_with(if desktop {
            "Full desktop: iPhone at 127.0.0.1"
        } else {
            "iPhone at 127.0.0.1"
        }),
        "{}",
        request.label
    );
    ensure!(rig.server.decide(request.id, true), "approved");
    let paired = ws.until("paired")?;
    let client_id = paired["client_id"].as_str().context("id")?.to_string();
    let token = paired["token"].as_str().context("token")?.to_string();
    let authenticated = ws.until("authenticated")?;
    ensure!(
        authenticated["client_id"] == client_id.as_str(),
        "{authenticated}"
    );
    let state = ws.until("state")?;
    ensure!(state["current"] == "Aurora", "the app's state: {state}");
    Ok((client_id, token, ws))
}

/// Authenticates with `token`; returns the rotated token.
fn authenticate(rig: &Rig, token: &str) -> Result<(String, Ws)> {
    let mut ws = open(rig)?;
    ws.hello()?;
    ws.send(&json!({"type": "authenticate", "token": token}))?;
    let authenticated = ws.json()?;
    ensure!(authenticated["type"] == "authenticated", "{authenticated}");
    let rotated = ws.json()?;
    ensure!(rotated["type"] == "paired", "a rotation: {rotated}");
    let next = rotated["token"].as_str().context("token")?.to_string();
    ensure!(next != token, "a new token");
    ws.until("state")?;
    Ok((next, ws))
}

// --------------------------------------------------------------------- tests

#[test]
fn desktop_control_requires_pairing_and_opt_in_and_is_exclusive() -> Result<()> {
    let rig = rig()?;
    let mut anonymous = open(&rig)?;
    anonymous.hello()?;
    anonymous.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        anonymous.until("error")?["code"],
        "unauthorized",
        "paired first"
    );
    let (_, phone_token, mut phone) = pair(&rig)?;
    phone.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        phone.until("error")?["code"],
        "unavailable",
        "host opt-in required"
    );
    rig.desktop.enable(true);
    phone.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        phone.until("error")?["code"],
        "unavailable",
        "an existing phone token is never upgraded"
    );
    let (_, mut phone_reconnected) = authenticate(&rig, &phone_token)?;
    phone_reconnected.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        phone_reconnected.until("error")?["code"],
        "unavailable",
        "phone scope survives token rotation"
    );
    let (id, _, mut first) = pair_as(&rig, true)?;
    first.send(&json!({"type":"desktop_open"}))?;
    eventually("desktop lease", || rig.desktop.active())?;
    rig.desktop.publish([16, 16], "jpeg-fixture".into());
    assert_eq!(
        first.until("desktop_frame")?["jpeg"],
        "jpeg-fixture",
        "owner receives frames"
    );
    let (_, _, mut second) = pair_as(&rig, true)?;
    second.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        second.until("error")?["code"],
        "unavailable",
        "cannot steal lease"
    );
    first.send(
        &json!({"type":"desktop_input", "events":[{"kind":"text","text":"edited shader"}]}),
    )?;
    let mut events = Vec::new();
    eventually("input on app bridge", || {
        rig.desktop.drain(&mut events);
        !events.is_empty()
    })?;
    assert_eq!(
        events,
        vec![strobe_remote::desktop::Input::Text {
            text: "edited shader".into()
        }],
        "host receives UI text"
    );
    rig.desktop.copy_text("selected shader");
    assert_eq!(
        first.until("desktop_clipboard")?["text"],
        "selected shader",
        "only explicit UI clipboard output"
    );
    assert!(rig.server.revoke(&id), "revoke controller");
    eventually("revocation releases input", || !rig.desktop.active())?;
    assert!(first.closed()? == 1008, "revoked connection closes");
    events.clear();
    rig.desktop.drain(&mut events);
    assert_eq!(
        events,
        vec![strobe_remote::desktop::Input::Reset {}],
        "release held host keys and buttons"
    );
    second.send(&json!({"type":"desktop_open"}))?;
    eventually("next owner", || rig.desktop.active())?;
    rig.desktop.enable(false);
    assert_eq!(second.closed()?, 1008, "kill switch closes desktop");
    Ok(())
}

#[test]
fn native_desktop_transport_pairs_and_reads_large_frames_without_affecting_phones() -> Result<()> {
    use strobe_remote::{ClientMessage, ServerMessage};
    let rig = rig()?;
    rig.desktop.enable(true);
    let mut native = strobe_remote::client::Connection::connect(rig.addr())?;
    native.send(&ClientMessage::Hello {
        protocol_version: 1,
    })?;
    let until = Instant::now() + Duration::from_secs(5);
    let mut hello = false;
    while !hello && Instant::now() < until {
        hello = matches!(native.receive()?, Some(ServerMessage::Hello { .. }));
    }
    assert!(hello, "native handshake and hello");
    native.send(&ClientMessage::PairDesktop {
        code: rig.server.open_pairing()?,
    })?;
    eventually("native approval request", || {
        !rig.server.pairing_requests().is_empty()
    })?;
    let request = rig
        .server
        .pairing_requests()
        .into_iter()
        .next()
        .context("native pairing request")?;
    assert!(rig.server.decide(request.id, true), "approved locally");
    let mut authenticated = false;
    let until = Instant::now() + Duration::from_secs(5);
    while !authenticated && Instant::now() < until {
        authenticated = matches!(native.receive()?, Some(ServerMessage::Authenticated { .. }));
    }
    assert!(authenticated, "paired and authenticated");
    native.send(&ClientMessage::DesktopOpen {})?;
    eventually("native exclusive lease", || rig.desktop.active())?;
    let jpeg = "A".repeat(100_000);
    rig.desktop.publish([640, 480], jpeg.clone());
    let mut image = None;
    let until = Instant::now() + Duration::from_secs(5);
    while image.is_none() && Instant::now() < until {
        if let Some(ServerMessage::DesktopFrame { jpeg, .. }) = native.receive()? {
            image = Some(jpeg);
        }
    }
    assert_eq!(
        image.as_deref(),
        Some(jpeg.as_str()),
        "large unmasked frame parsed across reads"
    );
    let (_, _, mut phone) = pair(&rig)?;
    phone.send(
        &json!({"type":"command", "request_id":"phone-still-works", "command":"blackout_on"}),
    )?;
    assert_eq!(
        phone.until("command_result")?["accepted"],
        true,
        "phone protocol preserved"
    );
    rig.server.stop();
    eventually("server stop releases desktop lease", || {
        !rig.desktop.active()
    })?;
    Ok(())
}

#[test]
fn the_page_is_served_same_origin_with_a_strict_policy() -> Result<()> {
    let rig = rig()?;
    let host = rig.host();
    let response = http(
        rig.addr(),
        &format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    )?;
    assert_eq!(status(&response), 200, "{response:.200}");
    assert!(response.contains("Content-Type: text/html"), "html");
    assert!(
        response.contains("Content-Security-Policy: default-src 'none'"),
        "a CSP"
    );
    assert!(response.contains(&format!("ws://{host}")), "its own socket");
    assert!(response.contains("X-Frame-Options: DENY"), "not framable");
    assert!(
        !response.to_ascii_lowercase().contains("access-control-"),
        "no CORS"
    );
    assert!(
        response.contains("<title>Strobe remote</title>"),
        "the page"
    );

    let missing = http(
        rig.addr(),
        &format!("GET /admin HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    )?;
    assert_eq!(status(&missing), 404, "nothing else is served");
    let post = http(
        rig.addr(),
        &format!("POST / HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\n\r\n"),
    )?;
    assert_eq!(status(&post), 405, "GET only");
    Ok(())
}

#[test]
fn an_unexpected_host_is_refused_to_stop_dns_rebinding() -> Result<()> {
    let rig = rig()?;
    let port = rig.addr().port();
    for host in [
        format!("evil.example:{port}"),
        format!("127.0.0.2:{port}"),
        "127.0.0.1:1".to_string(),
        String::new(),
    ] {
        let response = http(
            rig.addr(),
            &format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n"),
        )?;
        assert_eq!(status(&response), 421, "Host {host:?}: {response:.120}");
    }
    let none = http(rig.addr(), "GET / HTTP/1.1\r\n\r\n")?;
    assert_eq!(status(&none), 421, "no Host at all");
    let mut socket = TcpStream::connect(rig.addr())?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.write_all(
        upgrade_request(&format!("evil.example:{port}"), Some("http://evil.example")).as_bytes(),
    )?;
    let mut answer = String::new();
    let _ = socket.read_to_string(&mut answer);
    assert_eq!(status(&answer), 421, "nor a socket");
    Ok(())
}

#[test]
fn an_https_proxy_on_this_computer_is_accepted_by_its_name_only() -> Result<()> {
    // The proxy's name, from loopback (the test connects from 127.0.0.1,
    // as a proxy on this computer does): the page is served with a wss:
    // policy and the socket opens for the https origin only (D258).
    let proxied = rig_on(limits(), Clients::in_memory(), Some("strobe.example.com"))?;
    let page = http(
        proxied.addr(),
        "GET / HTTP/1.1\r\nHost: strobe.example.com\r\n\r\n",
    )?;
    assert_eq!(status(&page), 200, "served through the proxy: {page:.160}");
    assert!(
        page.contains("connect-src 'self' wss://strobe.example.com"),
        "the policy allows the secure socket: {page:.400}"
    );
    let _socket = open_as(
        proxied.addr(),
        "STROBE.example.com",
        Some("https://strobe.example.com"),
    )?;
    for (host, origin) in [
        ("strobe.example.com", "http://strobe.example.com"),
        ("strobe.example.com", "https://evil.example"),
    ] {
        let mut socket = TcpStream::connect(proxied.addr())?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
        socket.write_all(upgrade_request(host, Some(origin)).as_bytes())?;
        let mut answer = String::new();
        let _ = socket.read_to_string(&mut answer);
        assert_eq!(status(&answer), 403, "{origin} through the proxy name");
    }
    let other = http(
        proxied.addr(),
        "GET / HTTP/1.1\r\nHost: other.example.com\r\n\r\n",
    )?;
    assert_eq!(status(&other), 421, "any other name is still refused");
    // Without the setting the same name is refused, proxy or not.
    let plain = rig()?;
    let refused = http(
        plain.addr(),
        "GET / HTTP/1.1\r\nHost: strobe.example.com\r\n\r\n",
    )?;
    assert_eq!(status(&refused), 421, "no proxy configured");
    Ok(())
}

#[test]
fn another_origin_cannot_open_the_socket() -> Result<()> {
    let rig = rig()?;
    let host = rig.host();
    let port = rig.addr().port();
    for origin in [
        Some("http://evil.example".to_string()),
        Some(format!("https://{host}")),
        Some(format!("http://127.0.0.1:{}", port.wrapping_add(1))),
        Some("null".to_string()),
        None,
    ] {
        let response = http(rig.addr(), &upgrade_request(&host, origin.as_deref()))?;
        assert_eq!(status(&response), 403, "Origin {origin:?}: {response:.120}");
    }
    let mut bad_version = upgrade_request(&host, Some(&format!("http://{host}")));
    bad_version = bad_version.replace("Sec-WebSocket-Version: 13", "Sec-WebSocket-Version: 8");
    let response = http(rig.addr(), &bad_version)?;
    assert_eq!(status(&response), 426, "version 13 only");
    assert!(
        response.contains("Sec-WebSocket-Version: 13"),
        "and it says so"
    );
    Ok(())
}

#[test]
fn pairing_needs_the_code_and_the_desktop_and_then_commands_run() -> Result<()> {
    let rig = rig()?;
    let mut early = open(&rig)?;
    early.hello()?;
    early.send(&json!({"type": "pair", "code": "123456"}))?;
    let refused = early.json()?;
    assert_eq!(
        refused["code"], "pairing_rejected",
        "pairing is not open: {refused}"
    );
    early.send(&json!({"type": "command", "request_id": "r1", "command": "blackout_on"}))?;
    let unauthorized = early.json()?;
    assert_eq!(
        unauthorized["code"], "unauthorized",
        "no commands before pairing"
    );

    let (client_id, token, mut ws) = pair(&rig)?;
    assert_eq!(token.len(), 64, "a 256-bit token");
    assert!(rig.clients.contains(&client_id), "listed on the desktop");
    assert!(rig.server.pairing_code().is_none(), "the code was used up");

    ws.send(&json!({"type": "command", "request_id": "r1", "command": "next_scene"}))?;
    let result = ws.until("command_result")?;
    assert_eq!(
        result,
        json!({"type":"command_result","request_id":"r1","accepted":true,"reason":null}),
        "the app ran it and said so"
    );
    ws.send(&json!({"type": "ping"}))?;
    assert_eq!(ws.until("pong")?, json!({"type": "pong"}), "ping");
    assert!(
        rig.server
            .connections()
            .iter()
            .any(|c| c.client_id.as_deref() == Some(client_id.as_str())),
        "the desktop sees it connected"
    );
    Ok(())
}

#[test]
fn a_denied_request_pairs_nothing_and_a_replayed_code_fails() -> Result<()> {
    let rig = rig()?;
    let mut ws = open(&rig)?;
    ws.hello()?;
    let code = rig.server.open_pairing()?;
    ws.send(&json!({"type": "pair", "code": code}))?;
    eventually("the request", || !rig.server.pairing_requests().is_empty())?;
    let id = rig
        .server
        .pairing_requests()
        .first()
        .map(|r| r.id)
        .context("id")?;
    assert!(rig.server.decide(id, false), "denied");
    let denied = ws.json()?;
    assert_eq!(denied["code"], "pairing_rejected", "{denied}");
    assert!(rig.clients.is_empty(), "nobody paired");

    let mut replay = open(&rig)?;
    replay.hello()?;
    replay.send(&json!({"type": "pair", "code": code}))?;
    let refused = replay.json()?;
    assert_eq!(
        refused["code"], "pairing_rejected",
        "a used code is dead: {refused}"
    );
    Ok(())
}

#[test]
fn guessing_codes_burns_the_code_and_drops_the_guesser() -> Result<()> {
    let rig = rig()?;
    let code = rig.server.open_pairing()?;
    let wrong = if code.starts_with('9') {
        "111111"
    } else {
        "999999"
    };
    let mut ws = open(&rig)?;
    ws.hello()?;
    for attempt in 1..=5 {
        ws.send(&json!({"type": "pair", "code": wrong}))?;
        let refused = ws.json()?;
        assert_eq!(refused["code"], "pairing_rejected", "attempt {attempt}");
    }
    assert_eq!(ws.closed()?, 1008, "five failures end the connection");
    assert!(
        rig.server.pairing_code().is_none(),
        "and the code is burned"
    );
    Ok(())
}

#[test]
fn tokens_rotate_and_a_revoked_client_is_dropped_and_refused() -> Result<()> {
    let rig = rig()?;
    let (client_id, first, mut paired_ws) = pair(&rig)?;
    let (second, mut ws) = authenticate(&rig, &first)?;
    let (third, ws3) = authenticate(&rig, &second)?;
    drop(ws3);
    let mut stale = open(&rig)?;
    stale.hello()?;
    stale.send(&json!({"type": "authenticate", "token": first}))?;
    let refused = stale.json()?;
    assert_eq!(
        refused["code"], "unauthorized",
        "a replaced token is dead: {refused}"
    );

    assert!(rig.server.revoke(&client_id), "revoked");
    assert_eq!(ws.closed()?, 1008, "its open connection is dropped");
    assert_eq!(paired_ws.closed()?, 1008, "every one of them");
    let mut after = open(&rig)?;
    after.hello()?;
    after.send(&json!({"type": "authenticate", "token": third}))?;
    let refused = after.json()?;
    assert_eq!(refused["code"], "unauthorized", "and its token is refused");
    Ok(())
}

#[test]
fn paired_clients_survive_a_restart_and_revoke_all_clears_them() -> Result<()> {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("strobe-remote-restart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (clients, problem) = Clients::load(&dir);
    assert!(problem.is_none(), "{problem:?}");
    let token = {
        let rig = rig_with(limits(), clients)?;
        let (_, token, _) = pair(&rig)?;
        rig.server.open_pairing()?;
        token
    };
    let (reloaded, problem) = Clients::load(&dir);
    assert!(problem.is_none(), "{problem:?}");
    let rig = rig_with(limits(), reloaded)?;
    assert!(
        rig.server.pairing_code().is_none(),
        "a pairing code does not survive"
    );
    let (next, mut ws) = authenticate(&rig, &token)?;
    assert_eq!(rig.server.revoke_all(), 1, "revoke all");
    assert_eq!(ws.closed()?, 1008, "drops it");
    let mut again = open(&rig)?;
    again.hello()?;
    again.send(&json!({"type": "authenticate", "token": next}))?;
    assert_eq!(
        again.json()?["code"],
        "unauthorized",
        "refused after revoke all"
    );
    let (after, _) = Clients::load(&dir);
    assert!(after.is_empty(), "and that is saved");
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn oversized_and_malformed_messages_are_refused_without_harm() -> Result<()> {
    let rig = rig()?;
    // Malformed JSON and unknown types are answered, and the connection lives.
    let mut ws = open(&rig)?;
    ws.hello()?;
    for bad in [
        "not json",
        r#"{"type":"exec","cmd":"rm -rf /"}"#,
        r#"{"type":"command","request_id":"r 1","command":"next_scene"}"#,
        r#"{"type":"command","request_id":"r1","command":"load_scene"}"#,
        r#"{"type":"pair","code":"1","extra":true}"#,
    ] {
        ws.raw(&frame(true, 0x1, bad.as_bytes()))?;
        let answer = ws.json()?;
        assert_eq!(answer["code"], "invalid_request", "{bad}: {answer}");
    }
    ws.send(&json!({"type": "ping"}))?;
    assert_eq!(ws.json()?["type"], "pong", "still alive");

    // Before hello, anything else ends the connection.
    let mut rude = open(&rig)?;
    rude.send(&json!({"type": "ping"}))?;
    assert_eq!(rude.json()?["type"], "pong", "a ping is fine");
    rude.send(&json!({"type": "pair", "code": "123456"}))?;
    assert_eq!(rude.json()?["code"], "invalid_request", "hello first");
    assert_eq!(rude.closed()?, 1008, "closed");

    // Frame-level abuse closes with the matching code.
    let mut gigantic = vec![0x81, 0x80 | 127];
    gigantic.extend_from_slice(&(1_u64 << 40).to_be_bytes());
    gigantic.extend_from_slice(&[1, 2, 3, 4]);
    let mut unmasked = frame(true, 0x1, b"{}");
    unmasked[1] &= 0x7f;
    let oversized = frame(true, 0x1, &vec![b' '; strobe_remote::MAX_MESSAGE_BYTES + 1]);
    for (name, bytes, code) in [
        ("a 1 TiB header", gigantic, 1009),
        ("one byte over the limit", oversized, 1009),
        ("unmasked", unmasked, 1002),
        ("binary", frame(true, 0x2, b"\x00\x01"), 1003),
        ("not UTF-8", frame(true, 0x1, &[0xc3, 0x28]), 1007),
    ] {
        let mut ws = open(&rig)?;
        ws.hello()?;
        ws.raw(&bytes)?;
        assert_eq!(ws.closed()?, code, "{name}");
    }

    // A newer client is told why.
    let mut newer = open(&rig)?;
    newer.send(&json!({"type": "hello", "protocol_version": 2, "features": []}))?;
    assert_eq!(newer.json()?["code"], "version_mismatch", "a newer client");
    assert_eq!(newer.closed()?, 1008, "and closed");

    drop(ws);
    drop(rude);
    drop(newer);
    eventually("every connection thread to finish", || {
        rig.server.connections().is_empty()
    })?;
    Ok(())
}

#[test]
fn a_flood_is_cut_off() -> Result<()> {
    let rig = rig()?;
    let mut ws = open(&rig)?;
    ws.hello()?;
    let ping = frame(true, 0x1, br#"{"type":"ping"}"#);
    let mut burst = Vec::new();
    for _ in 0..200 {
        burst.extend_from_slice(&ping);
    }
    let _ = ws.raw(&burst);
    let mut busy = false;
    let code = loop {
        match ws.recv()? {
            Frame::Text(text) if text.contains("\"busy\"") => busy = true,
            Frame::Text(_) | Frame::Pong => {}
            Frame::Close(code) => break code,
            Frame::Eof => break 0,
        }
    };
    assert!(busy, "told it is sending too much");
    assert_eq!(code, 1008, "then dropped");
    Ok(())
}

#[test]
fn a_full_command_queue_is_busy_not_a_backlog() -> Result<()> {
    // An app that never drains: the bridge holds four, then says busy.
    capture_logs();
    let (remote, app_end) = bridge::bridge(2);
    let clients = Clients::in_memory();
    let (_, token) = clients.pair("test")?;
    let server = Server::start(
        ServerConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            limits: Limits {
                command_timeout: Duration::from_millis(20),
                ..limits()
            },
            proxied: None,
        },
        remote,
        clients,
    )
    .map_err(|e| anyhow!("{e}"))?;
    let host = server.addr().to_string();
    let mut ws = open_as(server.addr(), &host, Some(&format!("http://{host}")))?;
    ws.hello()?;
    ws.send(&json!({"type": "authenticate", "token": token.expose()}))?;
    ws.until("state")?;
    let mut refused = 0;
    for i in 0..6 {
        ws.send(
            &json!({"type": "command", "request_id": format!("r{i}"), "command": "tap_tempo"}),
        )?;
        let result = ws.until("command_result")?;
        if result["accepted"] == false {
            refused += 1;
            assert!(
                result["reason"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("busy"),
                "{result}"
            );
        }
    }
    assert_eq!(
        refused, 6,
        "nothing was run, and nothing waits beyond the bridge"
    );
    let mut queued = 0;
    while app_end.next().is_some() {
        queued += 1;
    }
    assert!(queued <= 2, "the queue held at most its capacity: {queued}");
    drop(app_end);
    ws.send(&json!({"type": "command", "request_id": "gone", "command": "tap_tempo"}))?;
    let unavailable = ws.json()?;
    assert_eq!(unavailable["code"], "unavailable", "{unavailable}");
    assert_eq!(
        ws.closed()?,
        1001,
        "an app that went away closes the socket"
    );
    Ok(())
}

#[test]
fn a_client_that_stops_reading_is_dropped_not_buffered() -> Result<()> {
    let (remote, app_end) = bridge::bridge(2);
    let clients = Clients::in_memory();
    let (_, token) = clients.pair("slow")?;
    capture_logs();
    let server = Server::start(
        ServerConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            limits: Limits {
                tick: Duration::from_millis(1),
                write_timeout: Duration::from_millis(200),
                idle_timeout: Duration::from_secs(60),
                ..limits()
            },
            proxied: None,
        },
        remote,
        clients,
    )
    .map_err(|e| anyhow!("{e}"))?;
    let host = server.addr().to_string();
    let mut ws = open_as(server.addr(), &host, Some(&format!("http://{host}")))?;
    ws.hello()?;
    ws.send(&json!({"type": "authenticate", "token": token.expose()}))?;
    ws.until("state")?;
    // Now never read again, while the state changes as fast as it can.
    let big = "x".repeat(256 * 1024);
    let started = Instant::now();
    let mut n = 0_u64;
    while !server.connections().is_empty() {
        ensure!(
            started.elapsed() < Duration::from_secs(20),
            "the slow client was never dropped"
        );
        n += 1;
        app_end.publish(&Snapshot {
            current: Some(format!("{n}{big}")),
            ..Snapshot::default()
        });
        std::thread::sleep(Duration::from_millis(1));
    }
    eventually("its thread to end", || server.active() == 0)?;
    drop(ws);
    assert!(
        logged().contains("is not reading; dropped"),
        "the log says why"
    );
    Ok(())
}

#[test]
fn connections_beyond_the_limit_are_turned_away() -> Result<()> {
    let rig = rig_with(
        Limits {
            max_connections: 2,
            ..limits()
        },
        Clients::in_memory(),
    )?;
    let _a = open(&rig)?;
    let _b = open(&rig)?;
    let response = http(
        rig.addr(),
        &format!("GET / HTTP/1.1\r\nHost: {}\r\n\r\n", rig.host()),
    )?;
    assert_eq!(status(&response), 503, "{response:.80}");
    Ok(())
}

#[test]
fn idle_unpaired_and_trickling_connections_time_out() -> Result<()> {
    let rig = rig_with(
        Limits {
            idle_timeout: Duration::from_millis(300),
            auth_timeout: Duration::from_millis(600),
            handshake_timeout: Duration::from_millis(300),
            ..limits()
        },
        Clients::in_memory(),
    )?;
    let mut idle = open(&rig)?;
    idle.hello()?;
    assert_eq!(idle.closed()?, 1001, "silent for too long");

    let mut unpaired = open(&rig)?;
    unpaired.hello()?;
    let until = Instant::now() + Duration::from_millis(900);
    while Instant::now() < until {
        let _ = unpaired.send(&json!({"type": "ping"}));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(unpaired.closed()?, 1008, "pinging is not pairing");

    let mut trickle = TcpStream::connect(rig.addr())?;
    trickle.set_read_timeout(Some(Duration::from_secs(5)))?;
    trickle.write_all(b"GET / HTTP/1.1\r\nHo")?;
    let started = Instant::now();
    let mut rest = Vec::new();
    let _ = trickle.read_to_end(&mut rest);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "a half-sent head is cut off: {:?}",
        started.elapsed()
    );
    Ok(())
}

#[test]
fn stopping_drops_every_client_at_once_and_frees_the_port() -> Result<()> {
    let rig = rig()?;
    let (_, _, mut ws) = pair(&rig)?;
    let port = rig.addr().port();
    let Rig {
        server,
        clients,
        _app,
        ..
    } = rig;
    let started = Instant::now();
    server.stop();
    let code = ws.closed()?;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the phone hears it at once: {:?}",
        started.elapsed()
    );
    assert!(code == 0 || code == 1001, "closed: {code}");
    assert!(
        TcpStream::connect_timeout(
            &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            Duration::from_millis(500)
        )
        .is_err(),
        "nothing listens any more"
    );
    let (remote, _app) = bridge::bridge(1);
    let again = Server::start(
        ServerConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            limits: limits(),
            proxied: None,
        },
        remote,
        clients,
    )
    .map_err(|e| anyhow!("{e}"))?;
    assert_eq!(again.addr().port(), port, "the same port binds again");
    drop(again);
    Ok(())
}

#[test]
fn a_public_or_taken_address_is_refused_with_advice() -> Result<()> {
    capture_logs();
    let (remote, _app) = bridge::bridge(1);
    let public = Server::start(
        ServerConfig::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 0),
        remote.clone(),
        Clients::in_memory(),
    );
    match public {
        Err(error @ StartError::PublicAddress(_)) => {
            assert!(error.to_string().contains("LAN address"), "{error}");
        }
        Err(other) => bail!("expected a refusal, got {other}"),
        Ok(_) => bail!("a public address was bound"),
    }
    let first = Server::start(
        ServerConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            limits: limits(),
            proxied: None,
        },
        remote.clone(),
        Clients::in_memory(),
    )
    .map_err(|e| anyhow!("{e}"))?;
    assert!(
        first.warning().is_some(),
        "loopback warns that a phone cannot reach it"
    );
    let second = Server::start(
        ServerConfig {
            bind: first.addr(),
            limits: limits(),
            proxied: None,
        },
        remote,
        Clients::in_memory(),
    );
    match second {
        Err(error @ StartError::Bind { .. }) => {
            assert!(error.to_string().contains("another port"), "{error}");
        }
        Err(other) => bail!("expected a bind error, got {other}"),
        Ok(_) => bail!("the same port bound twice"),
    }
    Ok(())
}

#[test]
fn no_secret_ever_reaches_the_log() -> Result<()> {
    let rig = rig()?;
    let code = rig.server.open_pairing()?;
    let mut wrong = open(&rig)?;
    wrong.hello()?;
    let guess = if code.starts_with('1') {
        "222 222"
    } else {
        "111 111"
    };
    wrong.send(&json!({"type": "pair", "code": guess}))?;
    wrong.json()?;
    rig.server.close_pairing();
    let (_, token, _ws) = pair(&rig)?;
    let (rotated, _ws2) = authenticate(&rig, &token)?;
    let mut bad = open(&rig)?;
    bad.hello()?;
    let fake = "f".repeat(64);
    bad.send(&json!({"type": "authenticate", "token": fake}))?;
    bad.json()?;
    let log = logged();
    assert!(log.contains("remote:"), "the remote did log: {log:.200}");
    for secret in [
        code.clone(),
        code.replace(' ', ""),
        guess.replace(' ', ""),
        token,
        rotated,
        fake,
    ] {
        assert!(!log.contains(&secret), "the log holds a secret");
    }
    Ok(())
}
