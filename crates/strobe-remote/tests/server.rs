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
//! need no network. The one that needs every interface (`0.0.0.0`) runs
//! only with `STROBE_TEST_LAN=1`, which CI sets: on a developer's Windows
//! machine that bind makes the firewall ask about each new test binary
//! (D503). The WebSocket client below is deliberately tiny: it
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
use strobe_remote::trust::TrustSecret;

// ---------------------------------------------------------------- LAN opt-in

/// Whether tests may bind beyond loopback: `STROBE_TEST_LAN=1` (D503).
/// Otherwise the caller skips, and the skip is said the way
/// `strobe_gfx::skip::note` says one (stderr, and a line in the file
/// `STROBE_SKIP_LOG` names), which this crate cannot call: it builds on
/// macOS, where strobe-gfx does not yet.
fn lan_tests() -> bool {
    if std::env::var_os("STROBE_TEST_LAN").is_some_and(|value| value == "1") {
        return true;
    }
    let test = std::thread::current()
        .name()
        .unwrap_or("unnamed test")
        .to_string();
    let reason = "binds every interface; set STROBE_TEST_LAN=1 to run it";
    eprintln!("skipping {test}: {reason}");
    if let Some(path) = std::env::var_os("STROBE_SKIP_LOG") {
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(format!("{test}\t{reason}\n").as_bytes()));
        if let Err(error) = written {
            eprintln!("could not record the skip in {}: {error}", path.display());
        }
    }
    false
}

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
    rig_on(limits, clients, None, None)
}

fn rig_on(
    limits: Limits,
    clients: Clients,
    proxied: Option<&str>,
    trust: Option<TrustSecret>,
) -> Result<Rig> {
    rig_at(Ipv4Addr::LOCALHOST, limits, clients, proxied, trust)
}

fn rig_at(
    ip: Ipv4Addr,
    limits: Limits,
    clients: Clients,
    proxied: Option<&str>,
    trust: Option<TrustSecret>,
) -> Result<Rig> {
    capture_logs();
    let (remote, app_end) = bridge::bridge(4);
    let desktop = app_end.desktop.clone();
    let server = Server::start(
        ServerConfig {
            bind: SocketAddr::new(IpAddr::V4(ip), 0),
            limits,
            proxied: proxied.map(str::to_string),
            trust,
            advertise: None,
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
fn desktop_input_reaches_the_app_only_from_the_lease_holder() -> Result<()> {
    use strobe_remote::desktop::Input;
    let rig = rig()?;
    rig.desktop.enable(true);
    // Before hello, and before pairing: no lease.
    let mut early = open(&rig)?;
    early.send(&json!({"type":"desktop_open"}))?;
    early.send(&json!({"type":"desktop_input","events":[{"kind":"text","text":"x"}]}))?;
    let (_, _, mut owner) = pair_as(&rig, true)?;
    owner.send(&json!({"type":"desktop_open"}))?;
    eventually("the owner's lease", || rig.desktop.active())?;
    // A paired phone, and a second paired desktop that never got the
    // lease, are each dropped for sending input.
    let (_, _, mut phone) = pair(&rig)?;
    phone.send(&json!({"type":"desktop_input","events":[{"kind":"text","text":"phone"}]}))?;
    assert_eq!(phone.closed()?, 1008, "a phone's input closes it");
    let (_, _, mut other) = pair_as(&rig, true)?;
    other.send(&json!({"type":"desktop_input","events":[{"kind":"text","text":"other"}]}))?;
    assert_eq!(other.closed()?, 1008, "a non-holder's input closes it");
    owner.send(&json!({"type":"desktop_input","events":[{"kind":"text","text":"owner"}]}))?;
    let mut events = Vec::new();
    eventually("the owner's input", || {
        rig.desktop.drain(&mut events);
        !events.is_empty()
    })?;
    assert_eq!(
        events,
        vec![Input::Text {
            text: "owner".into()
        }],
        "only the lease holder's input reached the app"
    );
    assert!(rig.desktop.active(), "the owner kept the lease throughout");
    drop(early);
    Ok(())
}

#[test]
fn the_lease_holder_gets_the_controls_once_and_its_edits_reach_the_app() -> Result<()> {
    use strobe_remote::desktop::{ControlEdit, SyncedBank};
    let rig = rig()?;
    rig.desktop.enable(true);
    let (_, _, mut owner) = pair_as(&rig, true)?;
    owner.send(&json!({"type":"desktop_open"}))?;
    eventually("the owner's lease", || rig.desktop.active())?;
    // A paired phone cannot ask for the controls or send edits.
    let (_, _, mut phone) = pair(&rig)?;
    phone.send(&json!({"type":"desktop_controls_open"}))?;
    assert_eq!(
        phone.until("error")?["code"],
        "unavailable",
        "a phone is refused the controls"
    );
    phone.send(&json!({"type":"desktop_control","edits":[{"bank":7,"index":0,"values":[1.0]}]}))?;
    assert_eq!(phone.closed()?, 1008, "a phone's edits close it");
    owner.send(&json!({"type":"desktop_controls_open"}))?;
    eventually("the owner's wish", || rig.desktop.controls_wanted())?;
    let bank = |values: [f32; 3]| SyncedBank {
        id: 7,
        title: "Acid Mirror".into(),
        defs: Some(vec![json!({"NAME":"speed","TYPE":"slider"})]),
        values: vec![values],
    };
    rig.desktop.publish_controls(vec![bank([0.5; 3])]);
    let first = owner.until("desktop_controls")?;
    assert_eq!(
        first["banks"][0]["defs"][0]["NAME"], "speed",
        "the first send carries the definitions: {first}"
    );
    rig.desktop.publish_controls(vec![bank([0.75; 3])]);
    let second = owner.until("desktop_controls")?;
    assert!(
        second["banks"][0].get("defs").is_none() && second["banks"][0]["values"][0][0] == 0.75,
        "after that, values alone: {second}"
    );
    owner
        .send(&json!({"type":"desktop_control","edits":[{"bank":7,"index":0,"values":[0.25]}]}))?;
    let mut edits = Vec::new();
    eventually("the owner's edit", || {
        rig.desktop.drain_controls(&mut edits);
        !edits.is_empty()
    })?;
    assert_eq!(
        edits,
        vec![ControlEdit {
            bank: 7,
            index: 0,
            values: vec![0.25]
        }],
        "the edit reached the app as sent"
    );
    // Where the next picture has its rows comes just before the picture.
    rig.desktop.publish_with_rows(
        [64, 48],
        "/9j/".into(),
        vec![strobe_remote::desktop::ControlRow {
            bank: 7,
            index: 0,
            rect: [0.1, 0.2, 0.5, 0.1],
            touch: true,
        }],
    );
    let layout = owner.until("desktop_layout")?;
    let frame = owner.json()?;
    assert!(
        frame["type"] == "desktop_frame" && frame["revision"] == layout["revision"],
        "the rows, then their frame under the same revision: {layout} {frame}"
    );
    // The lease holder may send far more than a phone: a drag's edits at
    // 20 a second for two seconds, beside its pings, keep the connection.
    for step in 0..40 {
        owner.send(&json!({"type":"desktop_control","edits":[{"bank":7,"index":0,"values":[step as f32 / 40.0]}]}))?;
        std::thread::sleep(Duration::from_millis(50));
    }
    owner.send(&json!({"type":"ping"}))?;
    owner.until("pong")?;
    assert!(
        rig.desktop.active(),
        "a drag's edits did not cost the lease"
    );
    Ok(())
}

#[test]
fn desktop_input_is_bounded_and_validated_before_the_app_sees_it() -> Result<()> {
    let rig = rig()?;
    rig.desktop.enable(true);
    let (_, _, mut owner) = pair_as(&rig, true)?;
    owner.send(&json!({"type":"desktop_open"}))?;
    eventually("lease", || rig.desktop.active())?;
    let refused = [
        json!({"type":"desktop_input","events":[{"kind":"move","x":1.5,"y":0.5}]}),
        json!({"type":"desktop_input","events":[{"kind":"move","x":-0.1,"y":0.5}]}),
        json!({"type":"desktop_input","events":[{"kind":"button","x":0.5,"y":0.5,"button":7,
            "down":true,"modifiers":{"alt":false,"ctrl":false,"shift":false,"mac_cmd":false}}]}),
        json!({"type":"desktop_input","events":[{"kind":"key","key":"x".repeat(25),"down":true,
            "modifiers":{"alt":false,"ctrl":false,"shift":false,"mac_cmd":false}}]}),
        json!({"type":"desktop_input","events":[{"kind":"scroll","x":5000.0,"y":0.0,
            "modifiers":{"alt":false,"ctrl":false,"shift":false,"mac_cmd":false}}]}),
        json!({"type":"desktop_input","events":vec![json!({"kind":"reset"}); 33]}),
        json!({"type":"desktop_input","events":[]}),
        json!({"type":"desktop_input","events":[{"kind":"text","text":"a\u{0}b"}]}),
        json!({"type":"desktop_input","events":[{"kind":"move","x":0.5,"y":0.5,"extra":1}]}),
    ];
    for message in &refused {
        owner.send(message)?;
        assert_eq!(
            owner.until("error")?["code"],
            "invalid_request",
            "refused before the app: {message}"
        );
    }
    let mut events = Vec::new();
    rig.desktop.drain(&mut events);
    assert!(events.is_empty(), "nothing reached the app: {events:?}");
    // Over the 4 KiB message limit: refused from the frame header, and the
    // connection with it, which releases the lease.
    owner.send(
        &json!({"type":"desktop_input","events":[{"kind":"paste","text":"x".repeat(5000)}]}),
    )?;
    assert_eq!(owner.closed()?, 1009, "an oversized message closes");
    eventually("the lease to go with it", || !rig.desktop.active())?;
    rig.desktop.drain(&mut events);
    assert_eq!(
        events,
        vec![strobe_remote::desktop::Input::Reset {}],
        "only the release of held input reached the app"
    );
    Ok(())
}

#[test]
fn a_phone_asks_with_a_name_and_the_host_approves_it_with_no_code() -> Result<()> {
    let rig = rig()?;
    let mut phone = open(&rig)?;
    phone.hello()?;
    phone.send(&json!({"type":"request_pair","name":"Ana's iPad"}))?;
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
        request.label == "Ana's iPad, iPhone at 127.0.0.1",
        "the name it gave, its kind and its real address: {}",
        request.label
    );
    ensure!(rig.server.decide(request.id, true), "approved");
    let paired = phone.until("paired")?;
    let token = paired["token"].as_str().context("token")?.to_string();
    phone.until("authenticated")?;
    let state = phone.until("state")?;
    ensure!(state["current"] == "Aurora", "the show's state: {state}");
    phone.send(&json!({"type":"command","request_id":"r1","command":"blackout_on"}))?;
    let result = phone.until("command_result")?;
    ensure!(result["accepted"] == true, "a phone command runs: {result}");
    // A phone paired this way has no desktop scope.
    phone.send(&json!({"type":"desktop_input","events":[{"kind":"text","text":"x"}]}))?;
    assert_eq!(
        phone.closed()?,
        1008,
        "desktop input from a phone closes it"
    );
    // It keeps its pairing like any phone: the token authenticates later.
    let (_, _rotated) = authenticate(&rig, &token)?;
    // The Paired devices list tells it apart by what it asked under (D500).
    let listed = rig.clients.list();
    let [client] = listed.as_slice() else {
        anyhow::bail!("one paired client: {listed:?}");
    };
    ensure!(
        client.name.as_deref() == Some("Ana's iPad")
            && client.platform.as_deref() == Some("iPhone")
            && client.address == Some(IpAddr::from([127, 0, 0, 1]))
            && client.access == strobe_remote::clients::Access::Phone
            && client.label == "Ana's iPad, iPhone at 127.0.0.1",
        "the name it gave, its kind and address, as approved: {client:?}"
    );
    ensure!(
        !format!("{listed:?}").contains(&token),
        "no token in what the desktop lists"
    );
    // A name the protocol does not allow is refused before the operator.
    let mut bad = open(&rig)?;
    bad.hello()?;
    bad.send(&json!({"type":"request_pair","name":"<script>"}))?;
    assert_eq!(
        bad.until("error")?["code"],
        "invalid_request",
        "a name outside the plain set"
    );
    assert!(
        rig.server.pairing_requests().is_empty(),
        "nothing put to the operator"
    );
    Ok(())
}

#[test]
fn a_desktop_asks_without_a_code_and_the_host_approves_or_denies() -> Result<()> {
    let rig = rig()?;
    let ask = json!({"type":"request_desktop","name":"stage-laptop"});
    // With full desktop control off, nothing reaches the operator.
    let mut off = open(&rig)?;
    off.hello()?;
    off.send(&ask)?;
    assert_eq!(off.until("error")?["code"], "unavailable", "switch off");
    assert!(
        rig.server.pairing_requests().is_empty(),
        "no request put to the operator"
    );
    drop(off);
    rig.desktop.enable(true);
    let mut asking = open(&rig)?;
    asking.hello()?;
    asking.send(&ask)?;
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
        request.label == "Full desktop: stage-laptop at 127.0.0.1",
        "named by the controller and its real address: {}",
        request.label
    );
    // The same address asking again while it waits is refused.
    let mut again = open(&rig)?;
    again.hello()?;
    again.send(&ask)?;
    assert_eq!(
        again.until("error")?["code"],
        "pairing_rejected",
        "one waiting request per address"
    );
    drop(again);
    ensure!(rig.server.decide(request.id, true), "approved");
    asking.until("paired")?;
    asking.until("authenticated")?;
    asking.send(&json!({"type":"desktop_open"}))?;
    eventually("an approved desktop to get the lease", || {
        rig.desktop.active()
    })?;
    drop(asking);
    eventually("the lease to go with the connection", || {
        !rig.desktop.active()
    })?;
    // Denied: told so, and the address waits before it may ask again.
    let mut denied = open(&rig)?;
    denied.hello()?;
    denied.send(&ask)?;
    eventually("the second request", || {
        !rig.server.pairing_requests().is_empty()
    })?;
    let second = rig
        .server
        .pairing_requests()
        .into_iter()
        .next()
        .context("a second request")?;
    ensure!(rig.server.decide(second.id, false), "denied");
    let error = denied.until("error")?;
    assert_eq!(error["code"], "pairing_rejected", "told no: {error}");
    denied.send(&ask)?;
    assert_eq!(
        denied.until("error")?["code"],
        "pairing_rejected",
        "and refused at once while it cools down"
    );
    assert!(
        rig.server.pairing_requests().is_empty(),
        "the refused ask put nothing in front of the operator"
    );
    Ok(())
}

#[test]
fn switching_desktop_access_off_ends_desktop_pairings_and_spares_phones() -> Result<()> {
    let rig = rig()?;
    // Asked while the switch is off, a desktop pairing is refused without
    // spending the code.
    let code = rig.server.open_pairing()?;
    let mut early = open(&rig)?;
    early.hello()?;
    early.send(&json!({"type":"pair_desktop","code":code}))?;
    assert_eq!(early.until("error")?["code"], "unavailable", "switch off");
    assert!(
        rig.server.pairing_requests().is_empty(),
        "no request put to the operator"
    );
    rig.desktop.enable(true);
    early.send(&json!({"type":"pair_desktop","code":code}))?;
    eventually("the same code to still work", || {
        !rig.server.pairing_requests().is_empty()
    })?;
    drop(early);
    eventually("the abandoned request to go", || {
        rig.server.pairing_requests().is_empty()
    })?;

    let (phone_id, phone_token, mut phone) = pair(&rig)?;
    let (desktop_id, desktop_token, mut desktop) = pair_as(&rig, true)?;
    desktop.send(&json!({"type":"desktop_open"}))?;
    eventually("lease", || rig.desktop.active())?;
    // What Lan::set_desktop(false) does.
    rig.desktop.enable(false);
    assert_eq!(rig.server.revoke_desktop(), 1, "the one desktop pairing");
    assert_eq!(desktop.closed()?, 1008, "the desktop is dropped at once");
    assert!(!rig.desktop.active(), "lease released");
    assert!(
        !rig.clients.contains(&desktop_id) && rig.clients.contains(&phone_id),
        "desktop pairing revoked, phone kept"
    );
    rig.desktop.enable(true);
    let mut retry = open(&rig)?;
    retry.hello()?;
    retry.send(&json!({"type":"authenticate","token":desktop_token}))?;
    assert_eq!(
        retry.until("error")?["code"],
        "unauthorized",
        "the desktop token is dead even with the switch back on"
    );
    phone.send(&json!({"type":"command","request_id":"still-here","command":"blackout_on"}))?;
    assert_eq!(
        phone.until("command_result")?["accepted"],
        true,
        "the phone was not touched"
    );
    authenticate(&rig, &phone_token).context("the phone's token still works")?;
    Ok(())
}

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
fn a_code_paired_desktop_reconnecting_takes_back_its_own_dead_lease_only() -> Result<()> {
    // A Wi-Fi drop leaves the old connection open at the host, holding the
    // lease, until its idle timeout (D407).
    let dead_lease = Duration::from_millis(300);
    let rig = rig_with(
        Limits {
            dead_lease,
            ..limits()
        },
        Clients::in_memory(),
    )?;
    rig.desktop.enable(true);
    let (_, token, mut old) = pair_as(&rig, true)?;
    old.send(&json!({"type":"desktop_open"}))?;
    eventually("lease", || rig.desktop.active())?;
    // The same desktop again at once, its old connection still talking:
    // the lease is not taken from a live connection, even by its own token.
    old.send(&json!({"type":"ping"}))?;
    let (token, mut fresh) = authenticate(&rig, &token)?;
    fresh.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        fresh.until("error")?["code"],
        "unavailable",
        "a live lease stays where it is"
    );
    // Another paired desktop never takes it, however quiet the holder is.
    let (_, _, mut other) = pair_as(&rig, true)?;
    std::thread::sleep(dead_lease * 2);
    other.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        other.until("error")?["code"],
        "unavailable",
        "one desktop at a time"
    );
    // The old connection has been silent past the dead-lease time: the
    // same paired desktop takes the lease back.
    let (_, mut back) = authenticate(&rig, &token)?;
    back.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(old.closed()?, 1008, "the dead connection was closed");
    rig.desktop.publish([16, 16], "after-reconnect".into());
    assert_eq!(
        back.until("desktop_frame")?["jpeg"],
        "after-reconnect",
        "the reconnected desktop has the lease"
    );
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
    let proxied = rig_on(
        limits(),
        Clients::in_memory(),
        Some("strobe.example.com"),
        None,
    )?;
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
    // The refused socket is open but is nobody's connection: the desktop's
    // "connected" count and the title bar must not include it.
    assert!(
        !rig.server.connections().is_empty()
            && rig
                .server
                .connections()
                .iter()
                .all(|c| !c.is_authenticated()),
        "a refused socket is open and not authenticated: {:?}",
        rig.server.connections()
    );

    let (client_id, token, mut ws) = pair(&rig)?;
    assert_eq!(
        rig.server
            .connections()
            .iter()
            .filter(|c| c.is_authenticated())
            .count(),
        1,
        "only the paired client counts, not the refused socket beside it"
    );
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
fn a_session_ends_when_its_client_leaves_the_list_even_unmarked() -> Result<()> {
    // `Server::revoke` closes the connections already marked with the
    // client's id. A revoke (or a revoke all) that lands between a token's
    // `authenticate`, or an approval, and the connection being marked finds
    // nothing to close, and the phone went on running commands with a dead
    // token for as long as it stayed connected. This removes the record
    // behind the listener's back, which is the state that race leaves.
    let rig = rig()?;
    let (client_id, _token, mut ws) = pair(&rig)?;
    ensure!(rig.clients.revoke(&client_id), "it was paired");
    assert_eq!(
        ws.closed()?,
        1008,
        "a session whose client is no longer paired is closed as revoked"
    );
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
            trust: None,
            advertise: None,
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
fn a_command_the_phone_was_told_is_busy_is_not_run_when_the_app_catches_up() -> Result<()> {
    // The venue PC's frame is held up longer than the command timeout (a
    // scene compiling). The phone is told "busy, try again" and the operator
    // presses Next again. Before D460 the first Next stayed queued and ran
    // as well once the frame came round: two scenes skipped, or a Hold
    // pressed twice undone.
    capture_logs();
    let (remote, app_end) = bridge::bridge(4);
    let clients = Clients::in_memory();
    let (_, token) = clients.pair("test")?;
    let server = Server::start(
        ServerConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            limits: Limits {
                command_timeout: Duration::from_millis(50),
                ..limits()
            },
            proxied: None,
            trust: None,
            advertise: None,
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
    // The app is stalled: nothing drains while the phone waits.
    ws.send(&json!({"type": "command", "request_id": "r1", "command": "next_scene"}))?;
    let first = ws.until("command_result")?;
    assert_eq!(first["accepted"], false, "told it was not done: {first}");
    // The operator tries again, and this time the app's frame comes round
    // while the phone waits.
    let app = std::thread::spawn(move || {
        let mut ran = Vec::new();
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            while let Some(incoming) = app_end.next() {
                ran.push(incoming.request_id.clone());
                incoming.answer(Answer::Accepted);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        ran
    });
    std::thread::sleep(Duration::from_millis(20));
    ws.send(&json!({"type": "command", "request_id": "r2", "command": "next_scene"}))?;
    let second = ws.until("command_result")?;
    assert_eq!(second["accepted"], true, "the retry ran: {second}");
    let ran = app.join().map_err(|_| anyhow!("the app thread panicked"))?;
    assert_eq!(
        ran,
        vec!["r2".to_string()],
        "only the command the phone was told ran; the busy one was withdrawn"
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
            trust: None,
            advertise: None,
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
            trust: None,
            advertise: None,
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
            trust: None,
            advertise: None,
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
            trust: None,
            advertise: None,
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

// ------------------------------------------------------- venue mode (D288)

/// A listener that trusts `secret`, with full desktop access switched on as
/// `--allow-desktop-control` does.
fn trusted_rig(secret: &TrustSecret) -> Result<Rig> {
    let rig = rig_on(limits(), Clients::in_memory(), None, Some(secret.clone()))?;
    rig.desktop.enable(true);
    Ok(rig)
}

fn new_secret() -> Result<TrustSecret> {
    Ok(TrustSecret::generate()?)
}

/// Starts a trusted pairing as `controller` and returns the socket and the
/// nonce the host sent.
fn trust_start(rig: &Rig, controller: &str) -> Result<(Ws, String)> {
    let mut ws = open(rig)?;
    ws.hello()?;
    ws.send(&json!({"type":"trust_start","controller_id":controller}))?;
    let challenge = ws.json()?;
    ensure!(challenge["type"] == "trust_challenge", "{challenge}");
    let nonce = challenge["nonce"].as_str().context("nonce")?.to_string();
    Ok((ws, nonce))
}

/// The whole trusted pairing; returns the socket and the client id.
fn trust_pair(rig: &Rig, secret: &TrustSecret, controller: &str) -> Result<(Ws, String)> {
    let (mut ws, nonce) = trust_start(rig, controller)?;
    let response = secret
        .respond(&nonce, &rig.host(), controller)
        .context("a response")?;
    ws.send(&json!({"type":"trust_proof","response":response}))?;
    let authenticated = ws.until("authenticated")?;
    let client_id = authenticated["client_id"]
        .as_str()
        .context("client id")?
        .to_string();
    ws.until("state")?;
    Ok((ws, client_id))
}

#[test]
fn a_controller_that_proves_the_trust_file_pairs_with_no_approval() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    let (mut ws, client_id) = trust_pair(&rig, &secret, "t480-1a2b3c4d")?;
    assert!(
        rig.server.pairing_requests().is_empty(),
        "nothing was put to the operator"
    );
    let listed = rig.clients.list();
    let entry = listed
        .iter()
        .find(|c| c.id == client_id)
        .context("the pairing is listed")?;
    assert!(
        entry
            .label
            .starts_with("Full desktop: trusted t480-1a2b3c4d, "),
        "labelled trusted: {}",
        entry.label
    );
    // The pairing carries the desktop scope: it can lease and operate.
    ws.send(&json!({"type":"desktop_open"}))?;
    eventually("the lease", || rig.desktop.active())?;
    rig.desktop.publish([16, 16], "jpeg-fixture".into());
    assert_eq!(ws.until("desktop_frame")?["jpeg"], "jpeg-fixture", "frames");
    ws.send(&json!({"type":"desktop_input","events":[{"kind":"text","text":"x"}]}))?;
    let mut events = Vec::new();
    eventually("input", || {
        rig.desktop.drain(&mut events);
        !events.is_empty()
    })?;
    // It is a connection's pairing, not a token: ending the connection ends
    // it, and no token was ever sent.
    drop(ws);
    eventually("the listing to go with the connection", || {
        rig.clients.list().iter().all(|c| c.id != client_id)
    })?;
    eventually("the lease to go", || !rig.desktop.active())?;
    Ok(())
}

#[test]
fn neither_the_secret_nor_its_proof_is_on_the_wire_or_in_the_log() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    let text = secret.to_file_text();
    let hex = text
        .lines()
        .find_map(|l| l.strip_prefix("secret "))
        .context("secret line")?
        .to_string();
    let (mut ws, nonce) = trust_start(&rig, "wire-check")?;
    let proof = secret
        .respond(&nonce, &rig.host(), "wire-check")
        .context("proof")?;
    ws.send(&json!({"type":"trust_proof","response":proof}))?;
    // Every message the host sent is searched for the secret.
    let mut seen = nonce.clone();
    for _ in 0..2 {
        if let Frame::Text(t) = ws.recv()? {
            seen.push_str(&t);
        }
    }
    assert!(
        !seen.contains(&hex),
        "the secret is not sent to the controller"
    );
    assert_ne!(proof, hex, "and the proof is not the secret");
    let mut bad = open(&rig)?;
    bad.hello()?;
    bad.send(&json!({"type":"trust_start","controller_id":"wrong"}))?;
    bad.json()?;
    bad.send(&json!({"type":"trust_proof","response":"0".repeat(64)}))?;
    bad.json()?;
    let log = logged();
    assert!(log.contains("trusted pairing"), "it did log: {log:.200}");
    assert!(!log.contains(&hex), "the secret never reaches the log");
    assert!(!log.contains(&proof), "nor does a proof");
    Ok(())
}

#[test]
fn a_wrong_secret_a_wrong_controller_or_a_wrong_host_is_refused() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    // Another file's secret.
    let (mut ws, nonce) = trust_start(&rig, "intruder")?;
    let response = new_secret()?
        .respond(&nonce, &rig.host(), "intruder")
        .context("response")?;
    ws.send(&json!({"type":"trust_proof","response":response}))?;
    assert_eq!(
        ws.until("error")?["code"],
        "pairing_rejected",
        "wrong secret"
    );
    // The right secret, answering for a controller id it did not give.
    let (mut ws, nonce) = trust_start(&rig, "honest")?;
    let response = secret
        .respond(&nonce, &rig.host(), "someone-else")
        .context("response")?;
    ws.send(&json!({"type":"trust_proof","response":response}))?;
    assert_eq!(ws.until("error")?["code"], "pairing_rejected", "wrong id");
    // The right secret, bound to another host address.
    let (mut ws, nonce) = trust_start(&rig, "honest")?;
    let response = secret
        .respond(&nonce, "192.168.1.9:47812", "honest")
        .context("response")?;
    ws.send(&json!({"type":"trust_proof","response":response}))?;
    assert_eq!(ws.until("error")?["code"], "pairing_rejected", "wrong host");
    // A proof with no challenge outstanding.
    let mut cold = open(&rig)?;
    cold.hello()?;
    cold.send(&json!({"type":"trust_proof","response":"a".repeat(64)}))?;
    assert_eq!(
        cold.until("error")?["code"],
        "invalid_request",
        "no challenge"
    );
    assert!(rig.clients.is_empty(), "nobody was paired");
    assert!(!rig.desktop.active(), "and nobody holds the lease");
    Ok(())
}

#[test]
fn on_every_interface_a_proof_binds_to_the_address_the_connection_reached() -> Result<()> {
    // Bound to every interface, the listener takes any IP literal as the
    // `Host`. A relay at another address (10.9.8.7 here) that forwards a
    // controller's handshake would present its own address, which the
    // controller's proof is bound to; that must not pair.
    if !lan_tests() {
        return Ok(());
    }
    let secret = new_secret()?;
    let rig = rig_at(
        Ipv4Addr::UNSPECIFIED,
        limits(),
        Clients::in_memory(),
        None,
        Some(secret.clone()),
    )?;
    rig.desktop.enable(true);
    let port = rig.addr().port();
    let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    for relayed in [format!("10.9.8.7:{port}"), format!("[::1]:{port}")] {
        let mut ws = open_as(target, &relayed, Some(&format!("http://{relayed}")))?;
        ws.hello()?;
        ws.send(&json!({"type":"trust_start","controller_id":"laptop"}))?;
        let answer = ws.json()?;
        assert_eq!(
            answer["code"], "invalid_request",
            "no challenge under another address's Host {relayed}: {answer}"
        );
    }
    // The address it reached, by literal and as localhost, still works.
    for host in [format!("127.0.0.1:{port}"), format!("localhost:{port}")] {
        let mut ws = open_as(target, &host, Some(&format!("http://{host}")))?;
        ws.hello()?;
        ws.send(&json!({"type":"trust_start","controller_id":"laptop"}))?;
        let challenge = ws.json()?;
        let nonce = challenge["nonce"].as_str().context("a challenge")?;
        let response = secret
            .respond(nonce, &host, "laptop")
            .context("a response")?;
        ws.send(&json!({"type":"trust_proof","response":response}))?;
        ws.until("authenticated")
            .with_context(|| format!("paired at {host}"))?;
    }
    Ok(())
}

#[test]
fn a_replayed_response_is_refused_because_each_nonce_answers_once() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    let (mut first, nonce) = trust_start(&rig, "laptop")?;
    let response = secret
        .respond(&nonce, &rig.host(), "laptop")
        .context("response")?;
    first.send(&json!({"type":"trust_proof","response":response}))?;
    first.until("authenticated")?;
    // The sniffed response, on a new connection with a new nonce.
    let (mut replay, new_nonce) = trust_start(&rig, "laptop")?;
    assert_ne!(nonce, new_nonce, "the host never repeats a nonce");
    replay.send(&json!({"type":"trust_proof","response":response}))?;
    assert_eq!(
        replay.until("error")?["code"],
        "pairing_rejected",
        "a replay fails"
    );
    // The same connection, answered twice: the second is out of place.
    first.send(&json!({"type":"trust_proof","response":response}))?;
    assert_eq!(
        first.until("error")?["code"],
        "invalid_request",
        "a connection past that step"
    );
    // A wrong answer spends its challenge too.
    let (mut once, nonce) = trust_start(&rig, "other")?;
    once.send(&json!({"type":"trust_proof","response":"0".repeat(64)}))?;
    once.until("error")?;
    let good = secret
        .respond(&nonce, &rig.host(), "other")
        .context("response")?;
    once.send(&json!({"type":"trust_proof","response":good}))?;
    assert_eq!(
        once.until("error")?["code"],
        "invalid_request",
        "the spent nonce cannot be answered after a failed try"
    );
    Ok(())
}

#[test]
fn a_proof_for_a_nonce_the_host_did_not_issue_is_refused() -> Result<()> {
    // The clock side of expiry is a unit test (the challenge lifetime is
    // not configurable); this is the socket path for a foreign nonce.
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    let (mut ws, _) = trust_start(&rig, "slow")?;
    let response = secret
        .respond(&"ab".repeat(32), &rig.host(), "slow")
        .context("response")?;
    ws.send(&json!({"type":"trust_proof","response":response}))?;
    assert_eq!(
        ws.until("error")?["code"],
        "pairing_rejected",
        "not its nonce"
    );
    Ok(())
}

#[test]
fn trusted_pairing_is_refused_without_a_trust_file_or_with_desktop_control_off() -> Result<()> {
    // No trust file: even the right-looking exchange is unavailable.
    let plain = rig()?;
    plain.desktop.enable(true);
    let mut ws = open(&plain)?;
    ws.hello()?;
    ws.send(&json!({"type":"trust_start","controller_id":"laptop"}))?;
    let refused = ws.until("error")?;
    assert_eq!(refused["code"], "unavailable", "no trust file");
    // A trust file but the switch off.
    let secret = new_secret()?;
    let rig = rig_on(limits(), Clients::in_memory(), None, Some(secret.clone()))?;
    let mut off = open(&rig)?;
    off.hello()?;
    off.send(&json!({"type":"trust_start","controller_id":"laptop"}))?;
    let also = off.until("error")?;
    assert_eq!(also["code"], "unavailable", "desktop control off");
    assert_eq!(
        also["message"], refused["message"],
        "the two cases cannot be told apart from outside"
    );
    assert!(
        rig.clients.is_empty() && !rig.desktop.active(),
        "nothing granted"
    );
    Ok(())
}

#[test]
fn the_kill_switch_ends_a_trusted_session_and_stops_new_ones() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    let (mut ws, client_id) = trust_pair(&rig, &secret, "laptop")?;
    ws.send(&json!({"type":"desktop_open"}))?;
    eventually("lease", || rig.desktop.active())?;
    // What Lan::set_desktop(false) does.
    rig.desktop.enable(false);
    assert_eq!(rig.server.revoke_desktop(), 1, "the trusted pairing counts");
    assert_eq!(ws.closed()?, 1008, "dropped at once");
    assert!(!rig.clients.contains(&client_id), "and gone from the list");
    let mut again = open(&rig)?;
    again.hello()?;
    again.send(&json!({"type":"trust_start","controller_id":"laptop"}))?;
    assert_eq!(
        again.until("error")?["code"],
        "unavailable",
        "the file opens nothing while the switch is off"
    );
    rig.desktop.enable(true);
    trust_pair(&rig, &secret, "laptop").context("back on, the file works again")?;
    Ok(())
}

#[test]
fn the_operator_can_revoke_a_trusted_controller_for_the_run() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    let (mut ws, client_id) = trust_pair(&rig, &secret, "laptop")?;
    ws.send(&json!({"type":"desktop_open"}))?;
    eventually("lease", || rig.desktop.active())?;
    assert!(rig.server.revoke(&client_id), "revoked");
    assert_eq!(ws.closed()?, 1008, "dropped");
    eventually("lease released", || !rig.desktop.active())?;
    let mut back = open(&rig)?;
    back.hello()?;
    back.send(&json!({"type":"trust_start","controller_id":"laptop"}))?;
    assert_eq!(
        back.until("error")?["code"],
        "unauthorized",
        "the same controller does not prove its way back in"
    );
    // Another controller id is not revoked by that...
    let (other, _) = trust_pair(&rig, &secret, "second")?;
    // ...but Revoke all closes trusted pairing for the run.
    assert_eq!(rig.server.revoke_all(), 1, "the second is listed");
    drop(other);
    let mut closed = open(&rig)?;
    closed.hello()?;
    closed.send(&json!({"type":"trust_start","controller_id":"third"}))?;
    assert_eq!(
        closed.until("error")?["code"],
        "unauthorized",
        "revoke all shuts the trust file out until the host restarts"
    );
    Ok(())
}

#[test]
fn a_controller_that_reconnects_takes_back_its_own_lease() -> Result<()> {
    // A Wi-Fi drop leaves the old connection open at the host for a while.
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    let (mut old, old_id) = trust_pair(&rig, &secret, "laptop")?;
    old.send(&json!({"type":"desktop_open"}))?;
    eventually("lease", || rig.desktop.active())?;
    let (mut fresh, fresh_id) = trust_pair(&rig, &secret, "laptop")?;
    assert_ne!(old_id, fresh_id, "a new pairing");
    fresh.send(&json!({"type":"desktop_open"}))?;
    eventually("the new lease", || rig.desktop.active())?;
    rig.desktop.publish([16, 16], "after-reconnect".into());
    assert_eq!(
        fresh.until("desktop_frame")?["jpeg"],
        "after-reconnect",
        "the new connection has the lease"
    );
    assert_eq!(old.closed()?, 1008, "the old one was closed");
    assert!(!rig.clients.contains(&old_id), "and replaced in the list");
    // A different trusted controller does not take the lease from it.
    let (mut other, _) = trust_pair(&rig, &secret, "second")?;
    other.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        other.until("error")?["code"],
        "unavailable",
        "one controller at a time"
    );
    Ok(())
}

#[test]
fn failed_proofs_back_off_per_address_and_then_recover() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    // Three free failures, then a pause of two seconds.
    for _ in 0..4 {
        let (mut ws, _) = trust_start(&rig, "guesser")?;
        ws.send(&json!({"type":"trust_proof","response":"0".repeat(64)}))?;
        ws.until("error")?;
    }
    let mut paused = open(&rig)?;
    paused.hello()?;
    paused.send(&json!({"type":"trust_start","controller_id":"laptop"}))?;
    let busy = paused.until("error")?;
    assert_eq!(busy["code"], "busy", "paused: {busy}");
    assert_eq!(paused.closed()?, 1008, "and the connection is closed");
    assert!(logged().contains("paused"), "the pause is logged");
    std::thread::sleep(Duration::from_millis(2300));
    trust_pair(&rig, &secret, "laptop").context("admitted once the pause is over")?;
    Ok(())
}

#[test]
fn phone_pairing_is_unaffected_by_a_trust_file() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    // The code and the operator's approval still pair a phone, and the
    // phone still has no desktop scope.
    let (phone_id, phone_token, mut phone) = pair(&rig)?;
    phone.send(&json!({"type":"desktop_open"}))?;
    assert_eq!(
        phone.until("error")?["code"],
        "unavailable",
        "a phone is not upgraded by the trust file"
    );
    phone.send(&json!({"type":"command","request_id":"r1","command":"blackout_on"}))?;
    assert_eq!(
        phone.until("command_result")?["accepted"],
        true,
        "commands run"
    );
    authenticate(&rig, &phone_token).context("tokens still rotate")?;
    // A full-desktop pairing by code and approval still needs the click.
    let (_, _, _approved) = pair_as(&rig, true)?;
    assert!(rig.clients.contains(&phone_id), "the phone stays paired");
    // The trust messages from a connection past pairing are out of place.
    phone.send(&json!({"type":"trust_start","controller_id":"x"}))?;
    assert_eq!(phone.until("error")?["code"], "invalid_request", "not now");
    Ok(())
}

#[test]
fn version_one_traffic_is_byte_for_byte_what_it_was() -> Result<()> {
    let secret = new_secret()?;
    let rig = trusted_rig(&secret)?;
    let mut ws = open(&rig)?;
    ws.send(&json!({"type":"hello","protocol_version":1}))?;
    assert_eq!(
        ws.recv()?,
        Frame::Text(r#"{"type":"hello","protocol_version":1,"paired":false}"#.into()),
        "the D236 hello"
    );
    ws.send(&json!({"type":"ping"}))?;
    assert_eq!(
        ws.recv()?,
        Frame::Text(r#"{"type":"pong"}"#.into()),
        "the D236 pong"
    );
    ws.send(&json!({"type":"command","request_id":"r1","command":"tap_tempo"}))?;
    assert_eq!(
        ws.recv()?,
        Frame::Text(
            r#"{"type":"error","code":"unauthorized","message":"pair this device first"}"#.into()
        ),
        "the D236 error shape"
    );
    Ok(())
}
