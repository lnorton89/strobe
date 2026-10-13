// ============================================================================
// Module:       strobe_shadertoy::listener
// Description:  Path A: a loopback-only HTTP listener that accepts a shader
//               POSTed by the browser userscript and hands it to a handler.
//
// Dependencies: std::net, std::thread; serde_json; super::import (replies)
// ============================================================================

//! The localhost listener (path A).
//!
//! ```text
//! POST http://127.0.0.1:47811/import
//! X-Strobe-Token: <64 hex>
//! Content-Type: application/json
//!
//! { "shader": { "info": {...}, "renderpass": [...] },
//!   "pageUrl": "https://www.shadertoy.com/view/XsXXDn",
//!   "replace": false }
//! ```
//!
//! ## Security model (D114)
//!
//! A listening socket on a developer's or performer's laptop is an attack
//! surface, and "bound to localhost" does not mean "only my programs can
//! reach it": any web page open in the same browser can send requests to
//! `127.0.0.1`. The defences, in the order a request meets them:
//!
//! 1. **Loopback only.** The socket is bound to `127.0.0.1`, never
//!    `0.0.0.0` or an interface address, so another machine on the network
//!    cannot connect at all. The port is configurable, the address is not.
//! 2. **Host check.** The `Host` header must be `127.0.0.1:<port>` or
//!    `localhost:<port>`. A DNS-rebinding page reaches this socket with its
//!    own hostname in `Host`, and is refused.
//! 3. **Method and path.** Only `POST /import`. Everything else is 404 or
//!    405, and an `OPTIONS` preflight is a 405 with **no CORS headers**, so
//!    a browser never lets a web page's script send the real request.
//! 4. **Token.** `X-Strobe-Token` must equal the per-install secret
//!    ([`Token`]) or the answer is 401 and the body is not read. A custom
//!    header also makes any cross-origin request non-simple, so it needs the
//!    preflight that step 3 refuses. The userscript's `GM_xmlhttpRequest`
//!    is not subject to CORS, which is why no CORS is offered at all.
//! 5. **Size.** `Content-Length` is required and capped at
//!    [`MAX_BODY`] (2 MiB); a larger one is 413 and is not read.
//! 6. **Shape.** The body must be a JSON object with a `shader` object that
//!    has a `renderpass` array, else 400. Nothing touches the disk before
//!    that, and what the handler does with it is the import pipeline's
//!    business (it parses and validates again).
//!
//! Connections are served on short-lived threads, at most
//! [`MAX_CONNECTIONS`] at once, each with read and write timeouts, so a
//! stalled client cannot hold the listener.
//!
//! The listener never imports anything itself. It calls the [`Handler`] it
//! was given, which in the app queues the job on the import worker and
//! waits for the outcome, so the browser sees `imported`, `exists` or the
//! converter's error.

mod token;
mod wire;

pub use token::{TOKEN_FILE, Token};

use crate::import::{ImportError, Imported};
use serde_json::{Value, json};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use wire::{HeadError, drain, read_head, respond};

/// The default port (PLAN section 6.1).
pub const DEFAULT_PORT: u16 = 47811;

/// The largest request body accepted.
pub const MAX_BODY: usize = 2 * 1024 * 1024;

/// The header the token travels in.
pub const TOKEN_HEADER: &str = "x-strobe-token";

/// The most connections served at the same time.
pub const MAX_CONNECTIONS: usize = 4;

/// The route.
const ROUTE: &str = "/import";

/// A shader the userscript sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    /// The shader's JSON text (the page's `info` and `renderpass`).
    pub shader: String,
    /// The shader's page, as the userscript saw it.
    pub page_url: Option<String>,
    /// Replace the scene if it is already in the library.
    pub replace: bool,
}

/// What the handler says happened. [`Reply::from_import`] makes one from the
/// pipeline's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// The scene was written.
    Imported {
        /// Its title.
        title: String,
        /// Its folder.
        folder: String,
        /// Whether it replaced an older copy.
        replaced: bool,
        /// Things converted with a loss.
        warnings: Vec<String>,
    },
    /// The scene is already in the library and `replace` was not set.
    Exists {
        /// Its title.
        title: String,
        /// Its folder.
        folder: String,
    },
    /// The shader could not be converted or written.
    Failed(String),
    /// The app cannot take the job now.
    Busy(String),
}

impl Reply {
    /// The reply for the import pipeline's result.
    pub fn from_import(result: &Result<Imported, ImportError>) -> Self {
        match result {
            Ok(done) => Self::Imported {
                title: done.title.clone(),
                folder: done.folder.clone(),
                replaced: done.replaced,
                warnings: done.warnings.clone(),
            },
            Err(ImportError::Exists { title, folder, .. }) => Self::Exists {
                title: title.clone(),
                folder: folder.clone(),
            },
            Err(error) => Self::Failed(error.to_string()),
        }
    }

    /// The HTTP status and JSON body for this reply.
    fn http(&self) -> (u16, Value) {
        match self {
            Self::Imported {
                title,
                folder,
                replaced,
                warnings,
            } => (
                200,
                json!({
                    "status": "imported",
                    "title": title,
                    "folder": folder,
                    "replaced": replaced,
                    "warnings": warnings,
                }),
            ),
            Self::Exists { title, folder } => (
                409,
                json!({
                    "status": "exists",
                    "title": title,
                    "folder": folder,
                    "error": "already in the library; send again with \"replace\": true to replace it",
                }),
            ),
            Self::Failed(error) => (422, json!({"status": "failed", "error": error})),
            Self::Busy(why) => (503, json!({"status": "busy", "error": why})),
        }
    }
}

/// What serves a [`Submission`]. Called on a connection thread; it may take
/// as long as an import does.
pub type Handler = dyn Fn(Submission) -> Reply + Send + Sync + 'static;

/// How to listen.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// The port on `127.0.0.1`; 0 picks a free one (tests).
    pub port: u16,
    /// The secret the userscript must send.
    pub token: Token,
    /// The body cap.
    pub max_body: usize,
    /// Read and write timeout per connection.
    pub io_timeout: Duration,
}

impl ListenerConfig {
    /// The defaults on `port` with `token`.
    pub fn new(port: u16, token: Token) -> Self {
        Self {
            port,
            token,
            max_body: MAX_BODY,
            io_timeout: Duration::from_secs(10),
        }
    }
}

/// Why the listener could not start.
#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    /// The port could not be bound.
    #[error(
        "cannot listen on 127.0.0.1:{port}: {source}{}",
        if source.kind() == std::io::ErrorKind::AddrInUse {
            " (another Strobe is probably running; pick another port with --listen-port)"
        } else {
            ""
        }
    )]
    Bind {
        /// The port.
        port: u16,
        /// Why.
        source: std::io::Error,
    },
    /// The accept thread could not be started.
    #[error("cannot start the listener thread: {0}")]
    Thread(std::io::Error),
}

/// A running listener. Dropping it stops it.
#[derive(Debug)]
pub struct Listener {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// What every connection thread shares.
struct Shared {
    config: ListenerConfig,
    addr: SocketAddr,
    handler: Arc<Handler>,
    active: AtomicUsize,
}

impl Listener {
    /// Binds `127.0.0.1:<port>` and starts accepting on a new thread.
    pub fn start(config: ListenerConfig, handler: Arc<Handler>) -> Result<Self, ListenerError> {
        let socket = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, config.port)))
            .map_err(|source| ListenerError::Bind {
                port: config.port,
                source,
            })?;
        let addr = socket.local_addr().map_err(ListenerError::Thread)?;
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            config,
            addr,
            handler,
            active: AtomicUsize::new(0),
        });
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("strobe-import-listener".to_string())
                .spawn(move || accept_loop(&socket, &stop, &shared))
                .map_err(ListenerError::Thread)?
        };
        Ok(Self {
            addr,
            stop,
            thread: Some(thread),
        })
    }

    /// The address bound (always `127.0.0.1`).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stops accepting and waits for the accept thread.
    pub fn stop(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.stop.store(true, Ordering::SeqCst);
        // `accept` blocks; a connection to ourselves wakes it to see the flag.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        // Waits for the listener; a panic in it already aborted the process
        // in release builds.
        let _ = thread.join();
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.shut_down();
    }
}

fn accept_loop(socket: &TcpListener, stop: &AtomicBool, shared: &Arc<Shared>) {
    for stream in socket.incoming() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(mut stream) = stream else {
            continue;
        };
        let timeout = Some(shared.config.io_timeout);
        // A connection the timeouts cannot be set on could hold its thread
        // forever on a slow or silent peer: it is not served.
        let timeouts = stream
            .set_read_timeout(timeout)
            .and_then(|()| stream.set_write_timeout(timeout));
        if let Err(error) = timeouts {
            log::warn!("listener: dropped a connection: no timeouts ({error})");
            continue;
        }
        if shared.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            shared.active.fetch_sub(1, Ordering::SeqCst);
            let busy = json!({"status": "busy", "error": "too many connections"});
            // The busy reply is a courtesy to a client being turned away.
            let _ = respond(&mut stream, 503, &busy, None);
            continue;
        }
        let shared = Arc::clone(shared);
        let spawned = std::thread::Builder::new()
            .name("strobe-import-conn".to_string())
            .spawn({
                let shared = Arc::clone(&shared);
                move || {
                    serve(stream, &shared);
                    shared.active.fetch_sub(1, Ordering::SeqCst);
                }
            });
        if spawned.is_err() {
            shared.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// An error response ready to send.
struct Rejection {
    status: u16,
    message: String,
    allow: Option<&'static str>,
    /// Whether the body (if any) was left unread.
    body_unread: bool,
}

impl Rejection {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            allow: None,
            body_unread: true,
        }
    }
}

/// Serves one connection: one request, one response.
fn serve(mut stream: TcpStream, shared: &Shared) {
    let outcome = handle(&mut stream, shared);
    let (status, body, allow, unread) = match outcome {
        Ok(reply) => {
            let (status, body) = reply.http();
            (status, body, None, false)
        }
        Err(rejection) => (
            rejection.status,
            json!({"status": "error", "error": rejection.message}),
            rejection.allow,
            rejection.body_unread,
        ),
    };
    // The browser may have gone; the import's outcome is logged regardless.
    let _ = respond(&mut stream, status, &body, allow);
    if unread {
        drain(&mut stream);
    }
}

/// Reads, authorises and validates one request, and runs the handler.
fn handle(stream: &mut TcpStream, shared: &Shared) -> Result<Reply, Rejection> {
    let config = &shared.config;
    let head = match read_head(stream) {
        Ok(head) => head,
        Err(HeadError::Closed) => return Err(silent()),
        Err(HeadError::TooLarge) => {
            return Err(Rejection::new(400, "the request headers are too large"));
        }
        Err(HeadError::Malformed(why)) => return Err(Rejection::new(400, why)),
        Err(HeadError::Io(error)) => {
            return Err(Rejection::new(
                400,
                format!("could not read the request: {error}"),
            ));
        }
    };

    // 2. Host: refuse a DNS-rebinding page.
    let port = shared.addr.port();
    let host_ok = head.header("host").is_some_and(|host| {
        host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
    });
    if !host_ok {
        return Err(Rejection::new(
            400,
            format!("the Host header must be 127.0.0.1:{port} or localhost:{port}"),
        ));
    }

    // 3. Route and method.
    let path = head.target.split(['?', '#']).next().unwrap_or_default();
    if path != ROUTE {
        return Err(Rejection::new(404, format!("only {ROUTE} exists here")));
    }
    if head.method != "POST" {
        return Err(Rejection {
            allow: Some("POST"),
            ..Rejection::new(405, format!("{ROUTE} accepts POST only"))
        });
    }

    // 4. Token, before any of the body is read.
    let authorised = head
        .header(TOKEN_HEADER)
        .is_some_and(|candidate| config.token.matches(candidate));
    if !authorised {
        return Err(Rejection::new(
            401,
            "missing or wrong X-Strobe-Token (copy it from Strobe's Import panel)",
        ));
    }

    // 5. Size.
    if head.repeated("content-length") || head.header("transfer-encoding").is_some() {
        return Err(Rejection::new(
            400,
            "send exactly one Content-Length and no Transfer-Encoding",
        ));
    }
    let length: usize = head
        .header("content-length")
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| Rejection::new(400, "a Content-Length header is required"))?;
    if length > config.max_body {
        return Err(Rejection::new(
            413,
            format!(
                "the body is {length} bytes; the limit is {}",
                config.max_body
            ),
        ));
    }
    let body = read_body(stream, head.leftover, length).map_err(|why| Rejection::new(400, why))?;

    // 6. Shape.
    let submission = parse_submission(&body).map_err(|why| Rejection {
        body_unread: false,
        ..Rejection::new(400, why)
    })?;
    Ok((shared.handler)(submission))
}

/// A rejection for a connection that sent nothing: still answered, briefly.
fn silent() -> Rejection {
    Rejection::new(400, "empty request")
}

/// The `length` body bytes: those already read, then the rest.
fn read_body(
    stream: &mut impl std::io::Read,
    mut have: Vec<u8>,
    length: usize,
) -> Result<Vec<u8>, String> {
    if have.len() > length {
        return Err("more bytes arrived than Content-Length declared".to_string());
    }
    let mut rest = vec![0_u8; length - have.len()];
    stream
        .read_exact(&mut rest)
        .map_err(|e| format!("the body ended early or timed out: {e}"))?;
    have.extend_from_slice(&rest);
    Ok(have)
}

/// Validates the JSON envelope. The shader inside is checked only for the
/// shape that tells a Shadertoy export from any other object.
fn parse_submission(body: &[u8]) -> Result<Submission, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|e| format!("the body is not valid JSON: {e}"))?;
    let Value::Object(mut object) = value else {
        return Err("the body must be a JSON object { \"shader\": ... }".to_string());
    };
    let shader = match object.remove("shader") {
        // `POST /shadertoy` on the site answers with an array of shaders.
        Some(Value::Array(mut shaders)) if !shaders.is_empty() => shaders.swap_remove(0),
        Some(shader) => shader,
        None => return Err("the body has no \"shader\"".to_string()),
    };
    // The API wraps in {"Shader": ...}; accept it.
    let inner = shader.get("Shader").unwrap_or(&shader);
    if !inner.get("renderpass").is_some_and(Value::is_array) {
        return Err("\"shader\" is not a Shadertoy shader (no \"renderpass\" array)".to_string());
    }
    let page_url = object
        .get("pageUrl")
        .and_then(Value::as_str)
        .map(|url| url.chars().take(300).collect());
    let replace = object
        .get("replace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(Submission {
        shader: inner.to_string(),
        page_url,
        replace,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_envelope_accepts_an_object_an_array_and_the_api_wrapper() -> Result<()> {
        for body in [
            r#"{"shader":{"info":{},"renderpass":[]}}"#,
            r#"{"shader":[{"info":{},"renderpass":[]}]}"#,
            r#"{"shader":{"Shader":{"info":{},"renderpass":[]}}}"#,
        ] {
            let parsed = parse_submission(body.as_bytes()).map_err(anyhow::Error::msg)?;
            assert!(parsed.shader.contains("renderpass"), "{body}");
            assert!(!parsed.replace, "replace defaults to false");
        }
        let full = parse_submission(
            br#"{"shader":{"renderpass":[]},"pageUrl":"https://www.shadertoy.com/view/XsXXDn","replace":true}"#,
        )
        .map_err(anyhow::Error::msg)?;
        assert!(full.replace, "replace");
        assert_eq!(
            full.page_url.as_deref(),
            Some("https://www.shadertoy.com/view/XsXXDn"),
            "page"
        );
        Ok(())
    }

    #[test]
    fn anything_else_is_a_400_with_a_reason() {
        for (body, needle) in [
            ("not json", "not valid JSON"),
            ("[1,2]", "must be a JSON object"),
            ("{}", "no \"shader\""),
            (r#"{"shader":42}"#, "not a Shadertoy shader"),
            (r#"{"shader":{"renderpass":"x"}}"#, "not a Shadertoy shader"),
            (r#"{"shader":[]}"#, "not a Shadertoy shader"),
        ] {
            let error = parse_submission(body.as_bytes()).err().unwrap_or_default();
            assert!(error.contains(needle), "`{body}` -> `{error}`");
        }
    }

    #[test]
    fn replies_map_to_distinct_statuses() {
        let imported = Reply::Imported {
            title: "t".into(),
            folder: "f".into(),
            replaced: false,
            warnings: vec!["w".into()],
        };
        let exists = Reply::Exists {
            title: "t".into(),
            folder: "f".into(),
        };
        assert_eq!(imported.http().0, 200, "imported");
        assert_eq!(imported.http().1["warnings"][0], "w", "warnings carried");
        assert_eq!(exists.http().0, 409, "exists");
        assert_eq!(Reply::Failed("x".into()).http().0, 422, "failed");
        assert_eq!(Reply::Busy("x".into()).http().0, 503, "busy");
    }
}
