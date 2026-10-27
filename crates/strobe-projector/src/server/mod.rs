// ============================================================================
// Module:       strobe_projector::server
// Description:  The web page and JSON API: token check, the routes (units,
//               status, power), the login form and the token-to-cookie
//               exchange, and the listener that serves them on one address.
//
// Dependencies: serde_json, log; std::net, std::thread;
//               crate::{audit, config, unit}; self::http
// ============================================================================

//! The page a phone opens to switch the projectors (D502).
//!
//! Every route but `/favicon.ico` needs the token from `api.token`, as
//! `Authorization: Bearer <token>`, as the `pj_token` cookie the page sets,
//! or (for `GET` only) as `?token=`. Opening `/?token=...` once swaps the
//! query for an `HttpOnly`, `SameSite=Strict` cookie and redirects to `/`,
//! so the token leaves the address bar. Tokens are compared in constant
//! time. A request without one gets the login form (a page) or
//! `{"error": "unauthorized"}` (the API), and a line in the audit log.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | `/` | the page |
//! | GET | `/api/units` | the configured projectors |
//! | GET | `/api/status`, `/api/status/{all\|key}` | status |
//! | POST | `/api/power/{all\|key}/{on\|off}` | 200, or 502 if any unit failed |
//!
//! The browser asks for `/favicon.ico` before anyone has logged in; it is
//! answered with 204 and no audit line (on the venue PC it was the only
//! noise in the Python tool's log).
//!
//! The listener accepts on its own thread and answers each connection on
//! another, at most [`MAX_CONNECTIONS`] at once: a power command can hold
//! its connection for the 90 s retry budget.

pub mod http;

use crate::audit::Audit;
use crate::config::{self, Config};
use crate::unit::{self, Timing, Unit};
use http::{Head, HeadError, Reply};
use serde_json::{Map, Value, json};
use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The cookie the page keeps the token in.
pub const COOKIE: &str = "pj_token";

/// Connections answered at once; more are closed unanswered.
pub const MAX_CONNECTIONS: usize = 32;

/// How long a client has to send its request head.
const HEAD_DEADLINE: Duration = Duration::from_secs(10);

/// The page.
const PAGE: &str = include_str!("ui.html");

/// The form a browser without the token gets.
const LOGIN: &str = "<!doctype html><meta name=viewport content='width=device-width'>\
<title>Projector control</title>\
<body style='background:#111;color:#ddd;font:16px system-ui;padding:2em'>\
<h2>Projector control</h2><form method=get action='/'>\
<input name=token type=password placeholder='API token' autofocus \
style='padding:.6em;background:#222;color:#eee;border:1px solid #444'> \
<button style='padding:.6em'>Unlock</button></form>";

/// What every connection needs: the configuration (which Strobe's
/// Settings may change while the server runs), the audit log, and the
/// power sequence's timings.
#[derive(Debug)]
pub struct Shared {
    /// The configuration, read afresh by each request.
    pub config: Arc<RwLock<Config>>,
    /// The command log.
    pub audit: Audit,
    /// The power sequence's timings.
    pub timing: Timing,
}

impl Shared {
    /// A copy of the configuration as it is now.
    pub fn config(&self) -> Config {
        self.config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The units `target` names, with this server's timings.
    fn units(&self, config: &Config, target: &str) -> Result<Vec<Unit>, config::ConfigError> {
        Ok(unit::units_for(config, target)?
            .into_iter()
            .map(|unit| unit.with_timing(self.timing))
            .collect())
    }
}

/// Whether `a` and `b` are equal, taking as long whichever byte differs.
fn same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0_u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The token the request presents: the bearer header, then (for a `GET`)
/// the query, then the cookie.
fn presented(head: &Head, allow_query: bool) -> Option<String> {
    if let Some(bearer) = head
        .header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
    {
        return Some(bearer.trim().to_string());
    }
    if allow_query && let Some(token) = head.query("token") {
        return Some(token);
    }
    head.cookie(COOKIE)
}

/// Whether the request carries the configured token.
fn authorized(head: &Head, token: &str, allow_query: bool) -> bool {
    !token.is_empty()
        && presented(head, allow_query)
            .is_some_and(|given| !given.is_empty() && same(given.as_bytes(), token.as_bytes()))
}

/// The unit key and action in `/api/power/{key}/{on|off}`.
fn power_route(path: &str) -> Option<(&str, bool)> {
    let rest = path.strip_prefix("/api/power/")?;
    let (target, action) = rest.split_once('/')?;
    let on = match action {
        "on" => true,
        "off" => false,
        _ => return None,
    };
    config::valid_key(target).then_some((target, on))
}

/// The target in `/api/status` or `/api/status/{key}`.
fn status_route(path: &str) -> Option<&str> {
    if path == "/api/status" {
        return Some("all");
    }
    let target = path.strip_prefix("/api/status/")?;
    config::valid_key(target).then_some(target)
}

/// The reply to `head` from `peer`. Power and status block while the
/// projectors answer.
pub fn handle(head: &Head, peer: IpAddr, shared: &Shared) -> Reply {
    let source = peer.to_string();
    let path = head.path();
    let config = shared.config();
    match head.method.as_str() {
        "GET" => {
            if path == "/favicon.ico" {
                return Reply::empty(204);
            }
            if !authorized(head, &config.api.token, true) {
                return reject(shared, &source, head, !path.starts_with("/api"));
            }
            if path == "/" {
                if let Some(token) = head.query("token") {
                    // Only reached with the right token, which is ours, so
                    // it carries nothing that could break the header.
                    return Reply::empty(303)
                        .with_header("Location: /".to_string())
                        .with_header(format!(
                            "Set-Cookie: {COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; \
                             Max-Age=31536000"
                        ));
                }
                return Reply::html(200, PAGE);
            }
            if path == "/api/units" {
                let units: Map<String, Value> = config
                    .projectors
                    .iter()
                    .map(|(key, projector)| {
                        (
                            key.clone(),
                            json!({"name": projector.name, "ip": projector.ip}),
                        )
                    })
                    .collect();
                return Reply::json(200, &Value::Object(units));
            }
            match status_route(path) {
                Some(target) => status(shared, &config, &source, target),
                None => Reply::json(404, &json!({"error": "not found"})),
            }
        }
        "POST" => {
            if !authorized(head, &config.api.token, false) {
                return reject(shared, &source, head, false);
            }
            match power_route(path) {
                Some((target, on)) => power(shared, &config, &source, target, on),
                None => Reply::json(404, &json!({"error": "not found"})),
            }
        }
        _ => Reply::json(405, &json!({"error": "method not allowed"})),
    }
}

/// The login form or the API's 401, and an audit line.
fn reject(shared: &Shared, source: &str, head: &Head, page: bool) -> Reply {
    shared.audit.record(
        source,
        "-",
        &format!("{} {}", head.method, head.path()),
        "DENIED no/invalid token",
    );
    if page {
        Reply::html(401, LOGIN).with_header("WWW-Authenticate: Bearer".to_string())
    } else {
        Reply::json(401, &json!({"error": "unauthorized"}))
    }
}

fn status(shared: &Shared, config: &Config, source: &str, target: &str) -> Reply {
    let units = match shared.units(config, target) {
        Ok(units) => units,
        Err(error) => return Reply::json(404, &json!({"error": error.to_string()})),
    };
    let results = unit::run_on(units, |unit| Ok(unit.status()));
    let mut out = Map::new();
    let mut all_ok = true;
    for (unit, result) in results {
        let value = match result {
            Ok(status) => {
                all_ok &= status.ok;
                serde_json::to_value(status).unwrap_or(Value::Null)
            }
            Err(error) => {
                all_ok = false;
                json!({"ok": false, "error": error.to_string()})
            }
        };
        out.insert(unit.key().to_string(), value);
    }
    shared.audit.record(
        source,
        target,
        "status",
        if all_ok { "OK" } else { "PARTIAL" },
    );
    Reply::json(200, &Value::Object(out))
}

fn power(shared: &Shared, config: &Config, source: &str, target: &str, on: bool) -> Reply {
    let units = match shared.units(config, target) {
        Ok(units) => units,
        Err(error) => return Reply::json(404, &json!({"error": error.to_string()})),
    };
    let action = if on { "on" } else { "off" };
    let results = unit::run_on(units, |unit| unit.set_power(on, false));
    let mut out = Map::new();
    let mut all_ok = true;
    for (unit, result) in results {
        let value = match result {
            Ok(result) => {
                let mut value = serde_json::to_value(&result).unwrap_or(Value::Null);
                shared
                    .audit
                    .record(source, unit.key(), action, &format!("OK {value}"));
                if let Value::Object(fields) = &mut value {
                    fields.insert("ok".to_string(), Value::Bool(true));
                }
                value
            }
            Err(error) => {
                all_ok = false;
                shared
                    .audit
                    .record(source, unit.key(), action, &format!("FAIL {error}"));
                json!({"ok": false, "error": error.to_string()})
            }
        };
        out.insert(unit.key().to_string(), value);
    }
    Reply::json(if all_ok { 200 } else { 502 }, &Value::Object(out))
}

/// Holds one slot of [`MAX_CONNECTIONS`] while a connection is answered.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Answers one connection.
fn serve_connection(mut stream: TcpStream, peer: SocketAddr, shared: &Shared) {
    // An accepted socket can inherit the listener's non-blocking mode on
    // Windows; a failure here only makes the reads below fail fast.
    if stream.set_nonblocking(false).is_err()
        || stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .is_err()
    {
        return;
    }
    let reply = match http::read_head(&mut stream, Instant::now() + HEAD_DEADLINE) {
        Ok(head) => {
            let reply = handle(&head, peer.ip(), shared);
            log::info!(
                "{} \"{} {}\" {}",
                peer.ip(),
                head.method,
                head.path(),
                reply.status
            );
            reply
        }
        Err(HeadError::Closed | HeadError::Io(_)) => return,
        Err(HeadError::TooSlow) => return,
        Err(HeadError::TooLarge) => Reply::json(431, &json!({"error": "request too large"})),
        Err(HeadError::Malformed(why)) => Reply::json(400, &json!({"error": why})),
    };
    if let Err(error) = http::respond(&mut stream, &reply) {
        log::info!("{}: reply not sent: {error}", peer.ip());
    }
}

/// A running listener. Dropping it (or [`Server::stop`]) stops accepting;
/// a request already being answered finishes.
#[derive(Debug)]
pub struct Server {
    local: SocketAddr,
    stop: Arc<AtomicBool>,
    accepting: Option<JoinHandle<()>>,
}

impl Server {
    /// Binds `address` and starts answering.
    pub fn start(address: SocketAddr, shared: Arc<Shared>) -> io::Result<Self> {
        /// How often the accept loop checks for a stop.
        const TICK: Duration = Duration::from_millis(50);
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let local = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let active = Arc::new(AtomicUsize::new(0));
        let accepting = thread::Builder::new()
            .name("projector-http".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    let (stream, peer) = match listener.accept() {
                        Ok(accepted) => accepted,
                        Err(error) => {
                            if error.kind() != io::ErrorKind::WouldBlock {
                                log::warn!("projector page: accept failed: {error}");
                            }
                            thread::sleep(TICK);
                            continue;
                        }
                    };
                    if active.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                        active.fetch_sub(1, Ordering::AcqRel);
                        log::warn!("projector page: busy, closing {peer}");
                        continue;
                    }
                    let slot = Slot(Arc::clone(&active));
                    let shared = Arc::clone(&shared);
                    let spawned = thread::Builder::new()
                        .name("projector-request".into())
                        .spawn(move || {
                            let _slot = slot;
                            serve_connection(stream, peer, &shared);
                        });
                    if let Err(error) = spawned {
                        log::warn!("projector page: no thread for {peer}: {error}");
                    }
                }
            })?;
        Ok(Self {
            local,
            stop,
            accepting: Some(accepting),
        })
    }

    /// The address it listens on.
    pub const fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Stops accepting and waits for the accept loop to end.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(accepting) = self.accepting.take() {
            // The loop only ends by seeing the flag; a join error would
            // mean it panicked, and the release build aborts first.
            accepting.join().ok();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use std::net::Ipv4Addr;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn shared() -> Shared {
        let mut config = Config::default();
        config.api.token = TOKEN.into();
        config
            .projectors
            .insert("sony".into(), config::Projector::new("Sony", "192.0.2.1"));
        Shared {
            config: Arc::new(RwLock::new(config)),
            audit: Audit::none(),
            timing: Timing::default(),
        }
    }

    fn request(text: &str) -> Result<Reply> {
        let head = http::read_head(
            &mut format!("{text}\r\n\r\n").as_bytes(),
            Instant::now() + Duration::from_secs(5),
        )
        .map_err(|error| anyhow!("{error:?}"))?;
        Ok(handle(&head, IpAddr::V4(Ipv4Addr::LOCALHOST), &shared()))
    }

    #[test]
    fn no_token_gets_the_form_or_401() -> Result<()> {
        let page = request("GET / HTTP/1.1")?;
        assert_eq!(page.status, 401, "page");
        assert_eq!(page.content_type, "text/html; charset=utf-8", "form");
        let api = request("GET /api/units HTTP/1.1")?;
        assert_eq!(api.status, 401, "api");
        assert_eq!(api.content_type, "application/json", "json");
        let wrong = request("GET /api/units HTTP/1.1\r\nAuthorization: Bearer nope")?;
        assert_eq!(wrong.status, 401, "wrong token");
        let favicon = request("GET /favicon.ico HTTP/1.1")?;
        assert_eq!(favicon.status, 204, "favicon needs no token");
        Ok(())
    }

    #[test]
    fn a_query_token_becomes_a_cookie() -> Result<()> {
        let reply = request(&format!("GET /?token={TOKEN} HTTP/1.1"))?;
        assert_eq!(reply.status, 303, "redirect");
        let cookie = reply
            .headers
            .iter()
            .find(|line| line.starts_with("Set-Cookie:"))
            .ok_or_else(|| anyhow!("no cookie"))?;
        assert!(cookie.contains(&format!("{COOKIE}={TOKEN}")), "{cookie}");
        assert!(
            cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
            "{cookie}"
        );
        let page = request(&format!("GET / HTTP/1.1\r\nCookie: {COOKIE}={TOKEN}"))?;
        assert_eq!(page.status, 200, "cookie opens the page");
        assert_eq!(page.body, PAGE.as_bytes(), "the page");
        Ok(())
    }

    #[test]
    fn a_post_ignores_a_query_token() -> Result<()> {
        let reply = request(&format!("POST /api/power/sony/on?token={TOKEN} HTTP/1.1"))?;
        assert_eq!(reply.status, 401, "query tokens are for GET only");
        Ok(())
    }

    #[test]
    fn units_and_unknown_routes() -> Result<()> {
        let bearer = format!("Authorization: Bearer {TOKEN}");
        let units = request(&format!("GET /api/units HTTP/1.1\r\n{bearer}"))?;
        assert_eq!(units.status, 200, "units");
        let value: Value = serde_json::from_slice(&units.body)?;
        assert_eq!(value["sony"]["ip"], "192.0.2.1", "{value}");
        let unknown = request(&format!("GET /api/status/nobody HTTP/1.1\r\n{bearer}"))?;
        assert_eq!(unknown.status, 404, "unknown unit");
        let missing = request(&format!("GET /nowhere HTTP/1.1\r\n{bearer}"))?;
        assert_eq!(missing.status, 404, "unknown path");
        let bad_action = request(&format!("POST /api/power/sony/toggle HTTP/1.1\r\n{bearer}"))?;
        assert_eq!(bad_action.status, 404, "bad action");
        let method = request(&format!("DELETE / HTTP/1.1\r\n{bearer}"))?;
        assert_eq!(method.status, 405, "method");
        Ok(())
    }

    #[test]
    fn routes_parse() {
        assert_eq!(
            power_route("/api/power/all/on"),
            Some(("all", true)),
            "all on"
        );
        assert_eq!(
            power_route("/api/power/sony/off"),
            Some(("sony", false)),
            "off"
        );
        assert_eq!(power_route("/api/power/a b/on"), None, "bad key");
        assert_eq!(status_route("/api/status"), Some("all"), "bare");
        assert_eq!(status_route("/api/status/sony"), Some("sony"), "one");
        assert!(
            same(b"abc", b"abc") && !same(b"abc", b"abd") && !same(b"a", b"ab"),
            "same"
        );
    }
}
