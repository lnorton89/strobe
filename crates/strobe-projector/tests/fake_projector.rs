// ============================================================================
// Module:       fake_projector (integration test)
// Description:  The power sequence, the fallback and the web API against
//               fake PJLink and NTCONTROL projectors on loopback: the Python
//               tool's tests ported, and the cases they did not cover.
//
// Dependencies: anyhow, serde_json; strobe_projector; std::net, std::thread
// ============================================================================

//! Offline tests against fake projectors (the hardware is not here).
//!
//! [`FakePjLink`] is a PJLink Class 1 unit with a password: it warms for a
//! moment after `POWR 1` and cools after `POWR 0`, can answer the next few
//! power commands with ERR3, can keep its lamp dark for a while after it
//! reports on (the PT-DZ6700's habit), and records any power command that
//! arrives mid-transition, which the power sequence must never send.
//! [`FakeNtControl`] is a Panasonic's NTCONTROL port with its digest.
//!
//! Ported from `tests/test_pjctl.py`: status fields, ERRA, on with two
//! ERR3s and `--wait`, on when already on sends nothing, waiting out a
//! cool-down, never commanding mid-transition, a refused connection.
//! Added: the lamp strike, NTCONTROL's digest and the fallback, the
//! standby status fields, and the web API end to end.

use anyhow::{Context as _, Result, bail, ensure};
use serde_json::Value;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use strobe_projector::audit::Audit;
use strobe_projector::config::{Config, Projector};
use strobe_projector::error::{Error, PjLinkCode};
use strobe_projector::ntcontrol::NtControl;
use strobe_projector::pjlink::PjLink;
use strobe_projector::server::{Server, Shared};
use strobe_projector::unit::{Timing, Unit, Via};
use strobe_projector::wire;

const PASSWORD: &str = "secret";
const RANDOM: &str = "498e4a67";
/// The page's password in these tests (D516).
const PAGE_PASSWORD: &str = "stage-left";

/// How long the fake takes to warm up or cool down.
const TRANSITION: Duration = Duration::from_millis(300);

/// Fast timings: the same sequence, in milliseconds.
const FAST: Timing = Timing {
    retry_budget: Duration::from_secs(10),
    first_backoff: Duration::from_millis(50),
    max_backoff: Duration::from_millis(200),
    poll_interval: Duration::from_millis(50),
    wait_timeout: Duration::from_secs(10),
    status_budget: Duration::from_secs(5),
};

/// The fake unit's state.
#[derive(Debug)]
struct State {
    power: char,
    busy_until: Instant,
    err3_left: u32,
    dark_until: Option<Instant>,
    /// How long the lamp stays dark once the unit reports on.
    dark_after_on: Duration,
    violation: bool,
    power_commands: u32,
}

/// A PJLink Class 1 projector on loopback.
struct FakePjLink {
    port: u16,
    state: Arc<Mutex<State>>,
}

impl FakePjLink {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(Mutex::new(State {
            power: '0',
            busy_until: Instant::now(),
            err3_left: 0,
            dark_until: None,
            dark_after_on: Duration::ZERO,
            violation: false,
            power_commands: 0,
        }));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = Arc::clone(&shared);
                thread::spawn(move || answer_pjlink(stream, &state));
            }
        });
        Ok(Self { port, state })
    }

    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn unit(&self, password: &str) -> Unit {
        Unit::new("fake", &Projector::new("Fake", "127.0.0.1"))
            .with_pjlink(PjLink::new("127.0.0.1", password).with_port(self.port))
            .with_timing(FAST)
    }
}

/// Reads one CR-terminated line.
fn line(reader: &mut BufReader<TcpStream>) -> Option<String> {
    let mut bytes = Vec::new();
    reader.read_until(b'\r', &mut bytes).ok()?;
    if bytes.pop() != Some(b'\r') {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn answer_pjlink(stream: TcpStream, state: &Mutex<State>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    if write!(writer, "PJLINK 1 {RANDOM}\r").is_err() {
        return;
    }
    let Some(request) = line(&mut reader) else {
        return;
    };
    let (digest, command) = request.split_at(request.len().min(32));
    if digest != wire::md5_hex(&format!("{RANDOM}{PASSWORD}")) {
        // A test's assertion catches the missing answer; nothing to do.
        write!(writer, "PJLINK ERRA\r").ok();
        return;
    }
    let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    if s.busy_until <= now && matches!(s.power, '2' | '3') {
        if s.power == '3' {
            // The lamp strikes this long after the unit reports on, however
            // slowly the test machine got here.
            s.dark_until = Some(now + s.dark_after_on);
        }
        s.power = if s.power == '3' { '1' } else { '0' };
    }
    let head = command.get(..6).unwrap_or(command);
    let argument = command.get(7..).unwrap_or_default();
    let reply = match (head, argument) {
        ("%1POWR", "?") => s.power.to_string(),
        ("%1POWR", on_off) => {
            s.power_commands += 1;
            if s.err3_left > 0 {
                s.err3_left -= 1;
                "ERR3".to_string()
            } else if matches!(s.power, '2' | '3') {
                s.violation = true;
                "ERR3".to_string()
            } else {
                s.power = if on_off == "1" { '3' } else { '2' };
                s.busy_until = now + TRANSITION;
                "OK".to_string()
            }
        }
        ("%1LAMP", _) => {
            let dark = s.dark_until.is_some_and(|until| now < until);
            if s.power == '1' && !dark {
                "1234 1"
            } else {
                "1234 0"
            }
            .to_string()
        }
        ("%1ERST", _) => "000010".to_string(),
        ("%1INPT", _) if s.power != '1' => "ERR3".to_string(),
        ("%1INPT", _) => "31".to_string(),
        ("%1NAME", _) => "FAKE".to_string(),
        _ => "ERR1".to_string(),
    };
    drop(s);
    // As above: a missing answer fails the test that waited for it.
    write!(writer, "{head}={reply}\r").ok();
}

/// A Panasonic's NTCONTROL port on loopback, with user `admin1`.
struct FakeNtControl {
    port: u16,
    on: Arc<Mutex<bool>>,
}

impl FakeNtControl {
    fn start(password: &'static str) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let on = Arc::new(Mutex::new(false));
        let shared = Arc::clone(&on);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let on = Arc::clone(&shared);
                thread::spawn(move || answer_ntcontrol(stream, &on, password));
            }
        });
        Ok(Self { port, on })
    }
}

fn answer_ntcontrol(stream: TcpStream, on: &Mutex<bool>, password: &str) {
    const NONCE: &str = "0b2c13f4";
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    if write!(writer, "NTCONTROL 1 {NONCE}\r").is_err() {
        return;
    }
    let Some(request) = line(&mut reader) else {
        return;
    };
    let expected = wire::md5_hex(&format!("admin1:{password}:{NONCE}"));
    let reply = match request.strip_prefix(&expected) {
        None => "ERRA".to_string(),
        Some(command) => {
            let mut on = on.lock().unwrap_or_else(PoisonError::into_inner);
            match command {
                "00QPW" => format!("00{}", if *on { "001" } else { "000" }),
                "00PON" => {
                    *on = true;
                    "00PON".to_string()
                }
                "00POF" => {
                    *on = false;
                    "00POF".to_string()
                }
                _ => "ERR1".to_string(),
            }
        }
    };
    // A missing answer fails the test that waited for it.
    write!(writer, "{reply}\r").ok();
}

/// A loopback port with nothing listening.
fn closed_port() -> Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

#[test]
fn status_reads_every_field() -> Result<()> {
    let fake = FakePjLink::start()?;
    fake.with(|s| s.power = '1');
    let status = fake.unit(PASSWORD).status();
    ensure!(status.ok, "{status:?}");
    assert_eq!(status.power, "on", "power");
    assert_eq!(status.via, Some(Via::PjLink), "via");
    assert_eq!(status.input.as_deref(), Some("DIGITAL 1 (31)"), "input");
    assert_eq!(status.hours.as_deref(), Some("lamp1: 1234 h (on)"), "hours");
    assert_eq!(status.errors.as_deref(), Some("filter=WARNING"), "errors");
    Ok(())
}

#[test]
fn standby_shows_no_input_rather_than_an_error() -> Result<()> {
    let fake = FakePjLink::start()?;
    let status = fake.unit(PASSWORD).status();
    assert_eq!(status.power, "off", "{status:?}");
    assert_eq!(status.input.as_deref(), Some("- (off)"), "ERR3 in standby");
    Ok(())
}

#[test]
fn a_wrong_password_is_erra() -> Result<()> {
    let fake = FakePjLink::start()?;
    match fake.unit("wrong").power_state(None) {
        Err(Error::PjLink {
            code: PjLinkCode::Erra,
            ..
        }) => Ok(()),
        other => bail!("expected ERRA, got {other:?}"),
    }
}

#[test]
fn on_retries_err3_then_waits_until_on() -> Result<()> {
    let fake = FakePjLink::start()?;
    fake.with(|s| s.err3_left = 2);
    let result = fake.unit(PASSWORD).set_power(true, true)?;
    assert!(result.sent, "{result:?}");
    assert_eq!(result.previous, "off", "{result:?}");
    assert_eq!(result.reached.as_deref(), Some("on"), "{result:?}");
    assert!(
        result.states_seen.iter().any(|state| state == "warming"),
        "{result:?}"
    );
    assert!(!fake.with(|s| s.violation), "never mid-transition");
    Ok(())
}

#[test]
fn already_on_sends_nothing() -> Result<()> {
    let fake = FakePjLink::start()?;
    fake.with(|s| s.power = '1');
    let result = fake.unit(PASSWORD).set_power(true, false)?;
    assert!(!result.sent, "{result:?}");
    assert_eq!(result.previous, "on", "{result:?}");
    assert_eq!(fake.with(|s| s.power_commands), 0, "no POWR sent");
    Ok(())
}

#[test]
fn a_cool_down_is_waited_out_before_on() -> Result<()> {
    let fake = FakePjLink::start()?;
    fake.with(|s| s.power = '1');
    fake.unit(PASSWORD).pjlink().set_power(false)?;
    ensure!(fake.with(|s| s.power) == '2', "cooling");
    let result = fake.unit(PASSWORD).set_power(true, true)?;
    assert_eq!(result.previous, "off", "it waited for off: {result:?}");
    assert_eq!(result.reached.as_deref(), Some("on"), "{result:?}");
    assert!(
        !fake.with(|s| s.violation),
        "never commanded mid-transition"
    );
    Ok(())
}

#[test]
fn on_waits_for_the_lamp_to_strike() -> Result<()> {
    let fake = FakePjLink::start()?;
    fake.with(|s| s.dark_after_on = Duration::from_millis(500));
    let result = fake.unit(PASSWORD).set_power(true, true)?;
    assert_eq!(result.reached.as_deref(), Some("on"), "{result:?}");
    assert!(
        result
            .states_seen
            .iter()
            .any(|state| state == "lamp starting"),
        "{result:?}"
    );
    let least = (TRANSITION + Duration::from_millis(500)).as_secs_f64() - 0.1;
    assert!(result.elapsed_s >= least, "waited for the lamp: {result:?}");
    Ok(())
}

#[test]
fn a_refused_connection_is_refused_then_given_up() -> Result<()> {
    let port = closed_port()?;
    let client = PjLink::new("127.0.0.1", "").with_port(port);
    assert!(
        matches!(client.power(), Err(Error::Refused(_))),
        "a refusal is its own kind"
    );
    let unit = Unit::new("gone", &Projector::new("Gone", "127.0.0.1"))
        .with_pjlink(client)
        .with_timing(Timing {
            retry_budget: Duration::from_millis(300),
            ..FAST
        });
    match unit.power_state(None) {
        Err(Error::Transport(message)) => {
            assert!(message.contains("connection refused for"), "{message}");
            Ok(())
        }
        other => bail!("expected a refusal after retries, got {other:?}"),
    }
}

#[test]
fn ntcontrol_takes_over_when_pjlink_is_refused() -> Result<()> {
    let nt_fake = FakeNtControl::start("panasonic")?;
    let nt = NtControl::new("127.0.0.1", "admin1", "panasonic").with_port(nt_fake.port);
    let unit = Unit::new("panasonic", &Projector::new("Panasonic", "127.0.0.1"))
        .with_pjlink(PjLink::new("127.0.0.1", "").with_port(closed_port()?))
        .with_ntcontrol(Some(nt))
        .with_timing(FAST);
    let result = unit.set_power(true, false)?;
    assert!(result.sent, "{result:?}");
    assert_eq!(result.via, Via::NtControl, "{result:?}");
    assert!(
        *nt_fake.on.lock().unwrap_or_else(PoisonError::into_inner),
        "PON reached the unit"
    );
    let status = unit.status();
    assert_eq!(status.power, "on", "{status:?}");
    assert_eq!(status.via, Some(Via::NtControl), "{status:?}");
    assert_eq!(status.input, None, "no PJLink fields over NTCONTROL");
    Ok(())
}

#[test]
fn ntcontrol_with_the_wrong_password_says_so() -> Result<()> {
    let nt_fake = FakeNtControl::start("panasonic")?;
    let nt = NtControl::new("127.0.0.1", "admin1", "nope").with_port(nt_fake.port);
    match nt.power() {
        Err(Error::NtControl { code: "ERRA", .. }) => Ok(()),
        other => bail!("expected ERRA, got {other:?}"),
    }
}

#[test]
fn err3_never_falls_back() -> Result<()> {
    let fake = FakePjLink::start()?;
    fake.with(|s| s.err3_left = 1);
    let nt_fake = FakeNtControl::start("panasonic")?;
    let unit = fake.unit(PASSWORD).with_ntcontrol(Some(
        NtControl::new("127.0.0.1", "admin1", "panasonic").with_port(nt_fake.port),
    ));
    let result = unit.set_power(true, false)?;
    assert_eq!(
        result.via,
        Via::PjLink,
        "ERR3 is retried over PJLink: {result:?}"
    );
    assert!(
        !*nt_fake.on.lock().unwrap_or_else(PoisonError::into_inner),
        "NTCONTROL untouched"
    );
    Ok(())
}

/// Sends a raw HTTP request to `address` and returns the status and body.
fn http(address: SocketAddr, request: &str) -> Result<(u16, String)> {
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    write!(stream, "{request}\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response
        .get(9..12)
        .context("status")?
        .parse()
        .context("status code")?;
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    Ok((status, body))
}

#[test]
fn the_web_api_switches_and_reports() -> Result<()> {
    let fake = FakePjLink::start()?;
    let mut config = Config::default();
    config.api.password = PAGE_PASSWORD.into();
    let mut projector = Projector::new("Fake", "127.0.0.1");
    projector.pjlink_password = PASSWORD.into();
    projector.pjlink_port = Some(fake.port);
    config.projectors.insert("fake".into(), projector);
    let mut unreachable = Projector::new("Gone", "127.0.0.1");
    unreachable.pjlink_port = Some(closed_port()?);
    config.projectors.insert("gone".into(), unreachable);
    let shared = Arc::new(Shared {
        config: Arc::new(RwLock::new(config)),
        audit: Audit::none(),
        timing: Timing {
            retry_budget: Duration::from_millis(300),
            status_budget: Duration::from_millis(300),
            ..FAST
        },
    });
    let server = Server::start(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), shared)?;
    let address = server.local_addr();
    let bearer = format!("Authorization: Bearer {PAGE_PASSWORD}");

    let (status, _) = http(address, "POST /api/power/fake/on HTTP/1.1")?;
    assert_eq!(status, 401, "no password");

    let (status, body) = http(
        address,
        &format!("POST /api/power/fake/on HTTP/1.1\r\n{bearer}"),
    )?;
    assert_eq!(status, 200, "{body}");
    let value: Value = serde_json::from_str(&body)?;
    assert_eq!(value["fake"]["ok"], true, "{value}");
    assert_eq!(value["fake"]["sent"], true, "{value}");
    assert_eq!(value["fake"]["via"], "pjlink", "{value}");

    let (status, body) = http(
        address,
        &format!("POST /api/power/all/on HTTP/1.1\r\n{bearer}"),
    )?;
    assert_eq!(status, 502, "one unit failed: {body}");
    let value: Value = serde_json::from_str(&body)?;
    assert_eq!(value["gone"]["ok"], false, "{value}");
    assert!(value["gone"]["error"].is_string(), "{value}");

    thread::sleep(TRANSITION);
    let (status, body) = http(
        address,
        &format!("GET /api/status/fake HTTP/1.1\r\n{bearer}"),
    )?;
    assert_eq!(status, 200, "{body}");
    let value: Value = serde_json::from_str(&body)?;
    assert_eq!(value["fake"]["power"], "on", "{value}");
    assert_eq!(value["fake"]["errors"], "filter=WARNING", "{value}");
    drop(server);
    Ok(())
}
