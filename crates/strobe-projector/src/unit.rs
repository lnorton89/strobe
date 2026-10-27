// ============================================================================
// Module:       strobe_projector::unit
// Description:  One projector: PJLink with the NTCONTROL fallback, the retry
//               policy, the safe power sequence, waiting for a state, and
//               the status a person reads.
//
// Dependencies: serde, log; crate::{config, error, format, ntcontrol, pjlink}
// ============================================================================

//! The safe power sequence (D502, carried over from the Python tool).
//!
//! - **Never a power command mid-transition.** [`Unit::set_power`] reads
//!   the state first and waits while it is warming (3) or cooling (2), then
//!   reads it again right before sending: a lamp unit commanded while it
//!   cools can refuse every command until it is power-cycled.
//! - **Nothing sent when nothing would change.** Already on is not
//!   switched on again.
//! - **Retries.** PJLink's ERR3 (unavailable time) and a refused connection
//!   (a unit rebooting its network) are retried with a backoff of 1 s
//!   doubling to 10 s, for up to [`Timing::retry_budget`] (90 s).
//! - **Fallback.** A unit configured with `"fallback": "ntcontrol"` (the
//!   Panasonic) tries NTCONTROL when PJLink is unusable on it: a wrong
//!   password, an unsupported command, a refused or silent port. ERR3 is an
//!   answer, not a broken PJLink, so it never falls back.
//! - **Waiting.** `--wait` polls every [`Timing::poll_interval`] until the
//!   state is reached, for up to [`Timing::wait_timeout`] (300 s: the
//!   PT-DZ6700 cools for about 172 s). The PT-DZ6700 reports on the moment
//!   it accepts the command, about 21 s before its lamps strike, so "on"
//!   over PJLink also waits for a lit flag in `LAMP ?`.
//!
//! One command at a time per unit within a process: every [`Unit`] for the
//! same key shares a lock, whether the CLI, Strobe's page or a web request
//! made it.

use crate::config::{Config, ConfigError, Projector};
use crate::error::{Error, PjLinkCode};
use crate::format;
use crate::ntcontrol::NtControl;
use crate::pjlink::PjLink;
use serde::Serialize;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The timings of the power sequence. [`Timing::default`] is the real
/// one; tests shrink it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// How long ERR3 and refused connections are retried, and a transition
    /// waited out, before giving up.
    pub retry_budget: Duration,
    /// The first retry's delay; it doubles each time.
    pub first_backoff: Duration,
    /// The longest retry delay.
    pub max_backoff: Duration,
    /// How often the state is polled while waiting.
    pub poll_interval: Duration,
    /// How long `--wait` waits for the state.
    pub wait_timeout: Duration,
    /// How long one status read may retry.
    pub status_budget: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            retry_budget: Duration::from_secs(90),
            first_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(10),
            poll_interval: Duration::from_secs(3),
            wait_timeout: Duration::from_secs(300),
            status_budget: Duration::from_secs(20),
        }
    }
}

/// A unit's power state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Power {
    /// Standby (`0`).
    Off,
    /// On (`1`).
    On,
    /// Cooling down (`2`).
    Cooling,
    /// Warming up (`3`).
    Warming,
    /// An answer outside the specification, kept as sent.
    Other(String),
}

impl Power {
    /// The state PJLink's code means.
    pub fn from_code(code: &str) -> Self {
        match code {
            "0" => Self::Off,
            "1" => Self::On,
            "2" => Self::Cooling,
            "3" => Self::Warming,
            other => Self::Other(other.to_string()),
        }
    }

    /// The state in a word (`on`, `cooling`), or the raw answer.
    pub fn name(&self) -> &str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Cooling => "cooling",
            Self::Warming => "warming",
            Self::Other(code) => code,
        }
    }

    /// Whether the unit is between states and takes no power command.
    pub const fn transitional(&self) -> bool {
        match self {
            Self::Cooling | Self::Warming => true,
            Self::Off | Self::On | Self::Other(_) => false,
        }
    }
}

/// Which protocol the last call went over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    /// PJLink.
    PjLink,
    /// Panasonic NTCONTROL.
    NtControl,
}

impl Via {
    /// The protocol's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::PjLink => "pjlink",
            Self::NtControl => "ntcontrol",
        }
    }
}

/// What a power command did.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PowerResult {
    /// The unit's key.
    pub unit: String,
    /// Whether a power command was sent (false: it was already there).
    pub sent: bool,
    /// The state before, in a word.
    pub previous: String,
    /// The protocol used.
    pub via: Via,
    /// The state reached, when waiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reached: Option<String>,
    /// Every state seen while waiting, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub states_seen: Vec<String>,
    /// Seconds from start to finish, to a tenth.
    pub elapsed_s: f64,
}

/// What a person reads about a unit. A field that could not be read says
/// why in its text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The unit's key.
    pub unit: String,
    /// Its name.
    pub name: String,
    /// Its address.
    pub ip: String,
    /// The power state in a word, or `error: ...`.
    pub power: String,
    /// The protocol used, when the power state was read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<Via>,
    /// The input, over PJLink.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Lamp or laser hours, over PJLink.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours: Option<String>,
    /// The error status, over PJLink.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<String>,
    /// Whether the power state was read.
    pub ok: bool,
}

/// A call the fallback can make over either protocol.
#[derive(Clone, Copy, Debug)]
enum Call {
    /// Read the power state.
    Power,
    /// Switch power.
    SetPower(bool),
}

/// The per-key locks every [`Unit`] in this process shares.
fn lock_for(key: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(locks.entry(key.to_string()).or_default())
}

/// One projector.
#[derive(Debug)]
pub struct Unit {
    key: String,
    name: String,
    ip: String,
    pjlink: PjLink,
    nt: Option<NtControl>,
    lock: Arc<Mutex<()>>,
    via: Cell<Via>,
    timing: Timing,
}

impl Unit {
    /// The unit `key` as configured.
    pub fn new(key: &str, projector: &Projector) -> Self {
        let nt = projector.ntcontrol_fallback().then(|| {
            NtControl::new(
                &projector.ip,
                &projector.ntcontrol_user,
                &projector.ntcontrol_password,
            )
            .with_port(projector.ntcontrol_port.unwrap_or(crate::ntcontrol::PORT))
        });
        Self {
            key: key.to_string(),
            name: projector.name.clone(),
            ip: projector.ip.clone(),
            pjlink: PjLink::new(&projector.ip, &projector.pjlink_password)
                .with_port(projector.pjlink_port.unwrap_or(crate::pjlink::PORT)),
            nt,
            lock: lock_for(key),
            via: Cell::new(Via::PjLink),
            timing: Timing::default(),
        }
    }

    /// The same unit with other timings (tests).
    #[must_use]
    pub const fn with_timing(mut self, timing: Timing) -> Self {
        self.timing = timing;
        self
    }

    /// The same unit with another PJLink client (a fake's port).
    #[must_use]
    pub fn with_pjlink(mut self, pjlink: PjLink) -> Self {
        self.pjlink = pjlink;
        self
    }

    /// The same unit with another NTCONTROL client, or none.
    #[must_use]
    pub fn with_ntcontrol(mut self, nt: Option<NtControl>) -> Self {
        self.nt = nt;
        self
    }

    /// The unit's key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its address.
    pub fn ip(&self) -> &str {
        &self.ip
    }

    /// The PJLink client, for raw commands.
    pub const fn pjlink(&self) -> &PjLink {
        &self.pjlink
    }

    /// The protocol the last call went over.
    pub fn via(&self) -> Via {
        self.via.get()
    }

    /// Runs `call` over PJLink, or over NTCONTROL when PJLink is unusable
    /// and the unit has the fallback.
    fn call(&self, call: Call) -> Result<String, Error> {
        self.via.set(Via::PjLink);
        let over_pjlink = match call {
            Call::Power => self.pjlink.power(),
            Call::SetPower(on) => self.pjlink.set_power(on).map(|()| String::new()),
        };
        let error = match over_pjlink {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let Some(nt) = self.nt.as_ref().filter(|_| error.pjlink_unusable()) else {
            return Err(error);
        };
        log::warn!(
            "{}: PJLink failed ({error}); falling back to NTCONTROL",
            self.key
        );
        self.via.set(Via::NtControl);
        match call {
            Call::Power => nt.power(),
            Call::SetPower(on) => nt.set_power(on).map(|()| String::new()),
        }
    }

    /// Runs `attempt` until it succeeds, retrying ERR3 and refused
    /// connections with backoff while `deadline` allows.
    fn with_retry<T>(
        &self,
        what: &str,
        deadline: Instant,
        mut attempt: impl FnMut() -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut delay = self.timing.first_backoff;
        loop {
            match attempt() {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let out_of_time = Instant::now() + delay > deadline;
                    match &error {
                        Error::PjLink {
                            code: PjLinkCode::Err3,
                            ..
                        } if !out_of_time => {
                            log::info!(
                                "{}: {what} -> ERR3, retrying in {:.1}s",
                                self.key,
                                delay.as_secs_f64()
                            );
                        }
                        Error::Refused(_) => {
                            if out_of_time {
                                return Err(Error::Transport(format!(
                                    "{}: connection refused for {:.0}s",
                                    self.ip,
                                    self.timing.retry_budget.as_secs_f64()
                                )));
                            }
                            log::info!(
                                "{}: {what} -> connection refused, retrying in {:.1}s",
                                self.key,
                                delay.as_secs_f64()
                            );
                        }
                        Error::PjLink { .. }
                        | Error::Transport(_)
                        | Error::NtControl { .. }
                        | Error::Failed(_) => return Err(error),
                    }
                }
            }
            thread::sleep(delay);
            delay = (delay * 2).min(self.timing.max_backoff);
        }
    }

    /// The power state, retrying until `deadline` (default: the retry
    /// budget from now).
    pub fn power_state(&self, deadline: Option<Instant>) -> Result<Power, Error> {
        let deadline = deadline.unwrap_or_else(|| Instant::now() + self.timing.retry_budget);
        self.with_retry("POWR ?", deadline, || self.call(Call::Power))
            .map(|code| Power::from_code(&code))
    }

    /// Switches the unit on or off safely (see the module docs). With
    /// `wait`, polls until the state is reached.
    pub fn set_power(&self, on: bool, wait: bool) -> Result<PowerResult, Error> {
        let target = if on { Power::On } else { Power::Off };
        let started = Instant::now();
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let deadline = started + self.timing.retry_budget;
        let mut state = self.power_state(Some(deadline))?;
        while state.transitional() {
            if Instant::now() > deadline {
                return Err(Error::Failed(format!(
                    "still {} after {:.0}s; not sending power command",
                    state.name(),
                    self.timing.retry_budget.as_secs_f64()
                )));
            }
            log::info!(
                "{}: {}, waiting before sending power command",
                self.key,
                state.name()
            );
            thread::sleep(self.timing.poll_interval);
            state = self.power_state(Some(deadline))?;
        }
        let mut sent = false;
        if state != target {
            self.with_retry("POWR set", deadline, || {
                // Re-check right before sending: never command a unit that
                // started a transition since the first read.
                let now = Power::from_code(&self.call(Call::Power)?);
                if now.transitional() {
                    return Err(Error::PjLink {
                        code: PjLinkCode::Err3,
                        command: "%1POWR (pre-check)".to_string(),
                    });
                }
                if now != target {
                    self.call(Call::SetPower(on))?;
                }
                Ok(())
            })?;
            sent = true;
        }
        let mut result = PowerResult {
            unit: self.key.clone(),
            sent,
            previous: state.name().to_string(),
            via: self.via(),
            reached: None,
            states_seen: Vec::new(),
            elapsed_s: 0.0,
        };
        if wait {
            let seen = self.wait_for(&target, started + self.timing.wait_timeout)?;
            result.reached = Some(target.name().to_string());
            result.states_seen = seen;
        }
        result.elapsed_s = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        Ok(result)
    }

    /// Polls until the unit is in `target` (and, for on over PJLink, a lamp
    /// is lit), returning the states seen.
    fn wait_for(&self, target: &Power, deadline: Instant) -> Result<Vec<String>, Error> {
        /// Each read may retry at least this long, even near the deadline.
        const READ_FLOOR: Duration = Duration::from_secs(15);
        /// The pseudo-state shown while the PT-DZ6700's lamps strike.
        const LAMP_STARTING: &str = "lamp starting";
        let mut seen: Vec<String> = Vec::new();
        loop {
            let read_deadline = deadline.max(Instant::now() + READ_FLOOR);
            let state = self.power_state(Some(read_deadline))?;
            let name = state.name().to_string();
            if seen.last() != Some(&name) {
                seen.push(name);
            }
            if state == *target {
                let lamp_pending =
                    *target == Power::On && self.via() == Via::PjLink && !self.lamp_lit();
                if !lamp_pending {
                    return Ok(seen);
                }
                if !seen.iter().any(|state| state == LAMP_STARTING) {
                    seen.push(LAMP_STARTING.to_string());
                }
            }
            if Instant::now() > deadline {
                return Err(Error::Failed(format!(
                    "did not reach {} in time; states seen: {}",
                    target.name(),
                    seen.join(", ")
                )));
            }
            thread::sleep(self.timing.poll_interval);
        }
    }

    /// Whether `LAMP ?` reports a lit lamp. A unit that does not support it
    /// (ERR1) is trusted on its power state; ERR3 means still starting.
    fn lamp_lit(&self) -> bool {
        match self.pjlink.lamp() {
            Ok(value) => {
                let flags: Vec<&str> = value.split_whitespace().skip(1).step_by(2).collect();
                flags.is_empty() || flags.contains(&"1")
            }
            Err(error) => match error.pjlink_code() {
                Some(PjLinkCode::Err3) | None => false,
                Some(PjLinkCode::Erra | PjLinkCode::Err1 | PjLinkCode::Err2 | PjLinkCode::Err4) => {
                    true
                }
            },
        }
    }

    /// Power, input, hours and errors. Never fails: a field that cannot be
    /// read says why.
    pub fn status(&self) -> Status {
        let mut status = Status {
            unit: self.key.clone(),
            name: self.name.clone(),
            ip: self.ip.clone(),
            ..Status::default()
        };
        match self.power_state(Some(Instant::now() + self.timing.status_budget)) {
            Ok(power) => {
                status.power = power.name().to_string();
                status.via = Some(self.via());
            }
            Err(error) => {
                status.power = format!("error: {error}");
                status.ok = false;
                return status;
            }
        }
        if status.via == Some(Via::PjLink) {
            let power = status.power.clone();
            status.input = Some(field(&power, self.pjlink.input(), format::input));
            status.hours = Some(field(&power, self.pjlink.lamp(), format::lamp));
            status.errors = Some(field(&power, self.pjlink.erst(), format::erst));
        }
        status.ok = true;
        status
    }
}

/// A status field: the formatted value, `n/a` when unsupported, `- (off)`
/// for ERR3 while the unit is not on (no input is reported in standby),
/// or the error.
fn field(power: &str, read: Result<String, Error>, format: fn(&str) -> String) -> String {
    match read {
        Ok(value) => format(&value),
        Err(error) => match error.pjlink_code() {
            Some(PjLinkCode::Err1) => "n/a (not supported)".to_string(),
            Some(PjLinkCode::Err3) if power != "on" => format!("- ({power})"),
            Some(PjLinkCode::Erra | PjLinkCode::Err2 | PjLinkCode::Err3 | PjLinkCode::Err4)
            | None => format!("error: {error}"),
        },
    }
}

/// The units `target` names: `all`, or one key.
pub fn units_for(config: &Config, target: &str) -> Result<Vec<Unit>, ConfigError> {
    if target == "all" {
        return Ok(config
            .projectors
            .iter()
            .map(|(key, projector)| Unit::new(key, projector))
            .collect());
    }
    match config.projectors.get(target) {
        Some(projector) => Ok(vec![Unit::new(target, projector)]),
        None => {
            let known: Vec<&str> = config.projectors.keys().map(String::as_str).collect();
            Err(ConfigError(format!(
                "unknown target '{target}'. Known: all, {}",
                known.join(", ")
            )))
        }
    }
}

/// Runs `job` on every unit at once, one thread each, and returns each
/// unit with its result, in the order given.
pub fn run_on<T: Send>(
    units: Vec<Unit>,
    job: impl Fn(&Unit) -> Result<T, Error> + Sync,
) -> Vec<(Unit, Result<T, Error>)> {
    thread::scope(|scope| {
        let handles: Vec<_> = units
            .into_iter()
            .map(|unit| {
                let job = &job;
                scope.spawn(move || {
                    let result = job(&unit);
                    (unit, result)
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .collect()
    })
}
