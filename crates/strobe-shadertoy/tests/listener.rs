// ============================================================================
// Module:       listener (integration test)
// Description:  The localhost import listener, driven by a real TcpStream
//               client: token, size, method, body shape and loopback binding.
//
// Dependencies: strobe_shadertoy; strobe_gfx::skip; std::net
// ============================================================================

//! The listener over a real socket.
//!
//! Every test starts a listener on port 0 (a free port), speaks raw HTTP to
//! it with `TcpStream`, and checks the status line, the JSON body and
//! whether the handler (the stand-in for the app's import worker) was
//! called. Raw bytes rather than an HTTP client, so a malformed or hostile
//! request can be sent exactly as an attacker would.

use anyhow::{Result, anyhow};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use strobe_shadertoy::import::{ImportOptions, import_json};
use strobe_shadertoy::listener::{
    Listener, ListenerConfig, ListenerError, MAX_BODY, Reply, Submission, Token,
};

const SINGLE: &str = include_str!("fixtures/single_image.json");

/// A running listener plus what its handler saw.
struct Rig {
    listener: Listener,
    token: Token,
    calls: Arc<AtomicUsize>,
    last: Arc<Mutex<Option<Submission>>>,
}

impl Rig {
    /// A listener whose handler answers `reply`.
    fn start(reply: Reply) -> Result<Self> {
        Self::with_handler(move |_| reply.clone())
    }

    fn with_handler(handler: impl Fn(Submission) -> Reply + Send + Sync + 'static) -> Result<Self> {
        let token = Token::generate()?;
        let calls = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let (c, l) = (Arc::clone(&calls), Arc::clone(&last));
        let listener = Listener::start(
            ListenerConfig::new(0, token.clone()),
            Arc::new(move |submission: Submission| {
                c.fetch_add(1, Ordering::SeqCst);
                if let Ok(mut slot) = l.lock() {
                    *slot = Some(submission.clone());
                }
                handler(submission)
            }),
        )?;
        Ok(Self {
            listener,
            token,
            calls,
            last,
        })
    }

    fn addr(&self) -> SocketAddr {
        self.listener.addr()
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// A well-formed `POST /import` with `body` and the real token.
    fn post(&self, body: &str) -> Result<Response> {
        self.post_with_token(body, self.token.expose())
    }

    fn post_with_token(&self, body: &str, token: &str) -> Result<Response> {
        raw(
            self.addr(),
            format!(
                "POST /import HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Strobe-Token: {token}\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                self.addr().port(),
                body.len()
            )
            .as_bytes(),
        )
    }
}

/// A parsed response.
#[derive(Debug)]
struct Response {
    status: u16,
    head: String,
    json: Value,
}

/// Sends `bytes`, reads until the server closes, parses the answer. A write
/// that fails because the server already answered and closed is not an error.
fn raw(addr: SocketAddr, bytes: &[u8]) -> Result<Response> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let _ = stream.write_all(bytes);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    let text = String::from_utf8_lossy(&answer).into_owned();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("no complete response: {text:?}"))?;
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("no status in {head:?}"))?;
    Ok(Response {
        status,
        head: head.to_string(),
        json: serde_json::from_str(body).unwrap_or(Value::Null),
    })
}

fn envelope(shader: &str) -> String {
    format!(r#"{{"shader":{shader},"pageUrl":"https://www.shadertoy.com/view/Tst001"}}"#)
}

fn imported() -> Reply {
    Reply::Imported {
        title: "Test".to_string(),
        folder: "test.synScene".to_string(),
        replaced: false,
        warnings: vec!["a warning".to_string()],
    }
}

#[test]
fn a_good_token_and_a_shader_reach_the_handler_and_the_answer_comes_back() -> Result<()> {
    let rig = Rig::start(imported())?;
    let answer = rig.post(&envelope(SINGLE))?;
    assert_eq!(answer.status, 200, "{answer:?}");
    assert_eq!(answer.json["status"], "imported", "{answer:?}");
    assert_eq!(answer.json["warnings"][0], "a warning", "warnings travel");
    assert_eq!(rig.calls(), 1, "the handler ran once");
    let seen = rig
        .last
        .lock()
        .map_err(|_| anyhow!("poisoned"))?
        .clone()
        .ok_or_else(|| anyhow!("no submission"))?;
    assert!(seen.shader.contains("renderpass"), "the shader arrived");
    assert_eq!(
        seen.page_url.as_deref(),
        Some("https://www.shadertoy.com/view/Tst001"),
        "page url"
    );
    assert!(!seen.replace, "replace defaults to false");
    assert!(
        !answer.head.to_ascii_lowercase().contains("access-control"),
        "no CORS headers are offered: {}",
        answer.head
    );
    Ok(())
}

#[test]
fn the_real_pipeline_writes_the_scene_and_a_repeat_gets_a_409_until_replace() -> Result<()> {
    let library = std::env::temp_dir().join(format!("strobe-listener-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&library);
    let lib = library.clone();
    let rig = Rig::with_handler(move |submission| {
        let result = import_json(
            &submission.shader,
            &ImportOptions {
                replace: submission.replace,
                ..ImportOptions::offline(&lib)
            },
        );
        Reply::from_import(&result)
    })?;

    let first = rig.post(&envelope(SINGLE))?;
    assert_eq!(first.status, 200, "{first:?}");
    let folder = first.json["folder"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        library.join(&folder).join("scene.json").is_file(),
        "scene written to {folder}"
    );

    let again = rig.post(&envelope(SINGLE))?;
    assert_eq!(again.status, 409, "{again:?}");
    assert_eq!(again.json["status"], "exists", "{again:?}");

    let replace = rig.post(&format!(r#"{{"shader":{SINGLE},"replace":true}}"#))?;
    assert_eq!(replace.status, 200, "{replace:?}");
    assert_eq!(replace.json["replaced"], true, "{replace:?}");

    let bad = rig.post(&envelope(r#"{"info":{},"renderpass":[]}"#))?;
    assert_eq!(bad.status, 422, "a shader that cannot convert: {bad:?}");
    assert!(
        bad.json["error"]
            .as_str()
            .is_some_and(|e| e.contains("Image")),
        "the converter's message: {bad:?}"
    );
    std::fs::remove_dir_all(&library)?;
    Ok(())
}

#[test]
fn a_bad_or_missing_token_is_401_and_the_handler_never_runs() -> Result<()> {
    let rig = Rig::start(imported())?;
    let wrong = rig.post_with_token(&envelope(SINGLE), &"0".repeat(64))?;
    assert_eq!(wrong.status, 401, "{wrong:?}");
    let short = rig.post_with_token(&envelope(SINGLE), "abc")?;
    assert_eq!(short.status, 401, "{short:?}");
    let missing = raw(
        rig.addr(),
        format!(
            "POST /import HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 2\r\n\r\n{{}}",
            rig.addr().port()
        )
        .as_bytes(),
    )?;
    assert_eq!(missing.status, 401, "{missing:?}");
    assert!(
        !missing.head.to_ascii_lowercase().contains("access-control"),
        "an unauthorised answer must not grant CORS either"
    );
    assert_eq!(rig.calls(), 0, "no handler call without the token");
    Ok(())
}

#[test]
fn an_oversized_body_is_413_whether_or_not_it_is_sent() -> Result<()> {
    let rig = Rig::start(imported())?;
    let port = rig.addr().port();
    let over = MAX_BODY + 1;

    // Declared only: the server must answer from the header alone.
    let declared = raw(
        rig.addr(),
        format!(
            "POST /import HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Strobe-Token: {}\r\n\
             Content-Length: {over}\r\n\r\n",
            rig.token.expose()
        )
        .as_bytes(),
    )?;
    assert_eq!(declared.status, 413, "{declared:?}");

    // Actually sent: 3 MiB. The answer must still arrive (the server drains).
    let big = format!(
        r#"{{"shader":{{"renderpass":[],"pad":"{}"}}}}"#,
        "a".repeat(3 * 1024 * 1024)
    );
    let sent = rig.post(&big)?;
    assert_eq!(sent.status, 413, "{sent:?}");
    assert!(
        sent.json["error"]
            .as_str()
            .is_some_and(|e| e.contains("limit")),
        "says why: {sent:?}"
    );

    // Exactly at the cap is allowed through to JSON validation.
    let at_cap = " ".repeat(MAX_BODY);
    let ok = rig.post(&at_cap)?;
    assert_eq!(
        ok.status, 400,
        "at the cap is read, then rejected as not JSON: {ok:?}"
    );
    assert_eq!(rig.calls(), 0, "no oversized body reached the handler");
    Ok(())
}

#[test]
fn only_post_import_exists() -> Result<()> {
    let rig = Rig::start(imported())?;
    let port = rig.addr().port();
    let request = |method: &str, path: &str| {
        raw(
            rig.addr(),
            format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 X-Strobe-Token: {}\r\nContent-Length: 0\r\n\r\n",
                rig.token.expose()
            )
            .as_bytes(),
        )
    };
    for method in ["GET", "PUT", "DELETE", "PATCH", "HEAD"] {
        let answer = request(method, "/import")?;
        assert_eq!(answer.status, 405, "{method}: {answer:?}");
        assert!(
            answer.head.contains("Allow: POST"),
            "{method}: {}",
            answer.head
        );
    }
    // A CORS preflight is refused with no CORS grant, so a web page's
    // script can never send the real request.
    let preflight = request("OPTIONS", "/import")?;
    assert_eq!(preflight.status, 405, "{preflight:?}");
    assert!(
        !preflight
            .head
            .to_ascii_lowercase()
            .contains("access-control"),
        "{}",
        preflight.head
    );
    for path in ["/", "/import/", "/shadertoy", "/../import", "/importx"] {
        let answer = request("POST", path)?;
        assert_eq!(answer.status, 404, "{path}: {answer:?}");
    }
    assert_eq!(rig.calls(), 0, "none of these reached the handler");
    Ok(())
}

#[test]
fn wrong_json_is_a_400_with_a_reason() -> Result<()> {
    let rig = Rig::start(imported())?;
    for (body, needle) in [
        ("this is not json", "not valid JSON"),
        ("[1, 2, 3]", "must be a JSON object"),
        ("{}", "no \"shader\""),
        (r#"{"shader": "just a string"}"#, "not a Shadertoy shader"),
        (r#"{"shader": {"info": {}}}"#, "not a Shadertoy shader"),
        ("", "not valid JSON"),
    ] {
        let answer = rig.post(body)?;
        assert_eq!(answer.status, 400, "`{body}`: {answer:?}");
        assert!(
            answer.json["error"]
                .as_str()
                .is_some_and(|e| e.contains(needle)),
            "`{body}` should say `{needle}`: {answer:?}"
        );
    }
    assert_eq!(rig.calls(), 0, "nothing invalid reaches the handler");
    Ok(())
}

#[test]
fn malformed_framing_is_a_400() -> Result<()> {
    let rig = Rig::start(imported())?;
    let port = rig.addr().port();
    let token = rig.token.expose();
    let head = |extra: &str| {
        format!(
            "POST /import HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Strobe-Token: {token}\r\n{extra}\r\n"
        )
    };
    let cases = [
        ("no length", head("")),
        ("chunked", head("Transfer-Encoding: chunked\r\n\r\n0\r\n")),
        (
            "two lengths",
            head("Content-Length: 2\r\nContent-Length: 3\r\n\r\n{}"),
        ),
        ("not a number", head("Content-Length: lots\r\n\r\n")),
        ("negative", head("Content-Length: -1\r\n\r\n")),
        ("garbage", "\u{1}\u{2}\u{3}\r\n\r\n".to_string()),
        ("http/0.9", "GET /import\r\n\r\n".to_string()),
    ];
    for (name, request) in cases {
        let answer = raw(rig.addr(), request.as_bytes())?;
        assert_eq!(answer.status, 400, "{name}: {answer:?}");
    }
    // A body shorter than declared must not hang past the I/O timeout: the
    // client closes early, the server notices.
    let short = raw(
        rig.addr(),
        format!(
            "POST /import HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Strobe-Token: {token}\r\n\
             Content-Length: 100\r\n\r\n{{}}"
        )
        .as_bytes(),
    );
    // Our read_to_end waits for the server; the server waits for 98 more
    // bytes until its timeout. Either a 400 or a closed socket is fine; a
    // hang is not (raw() has a 10 s read timeout).
    assert!(
        short.map_or(true, |r| r.status == 400),
        "a truncated body is a 400 or a close"
    );
    assert_eq!(rig.calls(), 0, "no malformed request reached the handler");
    Ok(())
}

#[test]
fn a_foreign_host_header_is_refused_to_stop_dns_rebinding() -> Result<()> {
    let rig = Rig::start(imported())?;
    let port = rig.addr().port();
    for host in [
        "evil.example",
        &format!("evil.example:{port}"),
        &format!("127.0.0.1:{}", port.wrapping_add(1)),
        "",
    ] {
        let answer = raw(
            rig.addr(),
            format!(
                "POST /import HTTP/1.1\r\nHost: {host}\r\nX-Strobe-Token: {}\r\n\
                 Content-Length: 2\r\n\r\n{{}}",
                rig.token.expose()
            )
            .as_bytes(),
        )?;
        assert_eq!(answer.status, 400, "Host `{host}`: {answer:?}");
    }
    let localhost = raw(
        rig.addr(),
        format!(
            "POST /import HTTP/1.1\r\nHost: localhost:{port}\r\nX-Strobe-Token: {}\r\n\
             Content-Length: 2\r\n\r\n{{}}",
            rig.token.expose()
        )
        .as_bytes(),
    )?;
    assert_eq!(
        localhost.status, 400,
        "localhost passes the Host check and then fails on the body, not the host: {localhost:?}"
    );
    assert!(
        localhost.json["error"]
            .as_str()
            .is_some_and(|e| e.contains("shader")),
        "{localhost:?}"
    );
    Ok(())
}

#[test]
fn it_binds_to_loopback_only() -> Result<()> {
    let rig = Rig::start(imported())?;
    let addr = rig.addr();
    assert!(addr.ip().is_loopback(), "bound to {addr}");
    assert_eq!(
        addr.ip(),
        IpAddr::from([127, 0, 0, 1]),
        "IPv4 loopback exactly"
    );

    // If this machine has another address, nothing must answer on it.
    // Finding that address binds every interface, which on a developer's
    // Windows machine makes the firewall ask about this test binary, so
    // that half runs only with `STROBE_TEST_LAN=1`, as CI sets (D503).
    if std::env::var_os("STROBE_TEST_LAN").is_none_or(|value| value != "1") {
        strobe_gfx::skip::note(
            "the non-loopback half binds every interface; set STROBE_TEST_LAN=1 to run it",
        );
    } else if let Some(other) = non_loopback_address() {
        let refused = TcpStream::connect_timeout(
            &SocketAddr::new(other, addr.port()),
            Duration::from_secs(2),
        );
        assert!(
            refused.is_err(),
            "{other}:{} accepted a connection; the listener must be loopback-only",
            addr.port()
        );
    }
    Ok(())
}

/// This machine's outward-facing address, if it has one: the source address
/// the OS would pick for a route to the internet (no packet is sent).
fn non_loopback_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[test]
fn a_taken_port_is_an_error_that_says_what_to_do_and_stop_frees_it() -> Result<()> {
    let first = Rig::start(imported())?;
    let port = first.addr().port();
    let second = Listener::start(
        ListenerConfig::new(port, first.token.clone()),
        Arc::new(|_| imported()),
    );
    let error = match second {
        Err(error @ ListenerError::Bind { .. }) => error.to_string(),
        other => return Err(anyhow!("expected a bind error, got {other:?}")),
    };
    assert!(error.contains(&port.to_string()), "names the port: {error}");
    assert!(error.contains("--listen-port"), "says what to do: {error}");

    first.listener.stop();
    let reused = Listener::start(
        ListenerConfig::new(port, Token::generate()?),
        Arc::new(|_| imported()),
    )?;
    assert_eq!(reused.addr().port(), port, "the port is free again");
    Ok(())
}

#[test]
fn a_stalled_client_does_not_block_others() -> Result<()> {
    let rig = Rig::start(imported())?;
    // Connect and send nothing; hold the connection open.
    let _stalled = TcpStream::connect(rig.addr())?;
    let _half = {
        let mut half = TcpStream::connect(rig.addr())?;
        half.write_all(b"POST /import HTTP/1.1\r\nHost: 127.0.0.1")?;
        half
    };
    let answer = rig.post(&envelope(SINGLE))?;
    assert_eq!(answer.status, 200, "{answer:?}");
    Ok(())
}
