// ============================================================================
// Module:       strobe_io::feedback
// Description:  OSC feedback: what changed since the last send (scene and
//               meta controls, blackout, the cue), at a capped rate, and a
//               sender thread that encodes and sends it over UDP.
//
// Dependencies: std (net, sync, thread); crate::{config, event, osc};
//               strobe-scene (ControlBank, ControlKind)
// ============================================================================

//! OSC feedback (backlog P-05, D209).
//!
//! A motorised fader or a TouchOSC layout shows what the show is doing only
//! if Strobe tells it. Feedback sends the state on **the same addresses the
//! listener reads**, so one TouchOSC layout both drives and follows:
//!
//! | Address | Argument |
//! |---|---|
//! | `/strobe/control/<name>` (and `/x`, `/y`, `/r`, `/g`, `/b` for pads and colours) | float 0..1 across the range |
//! | `/strobe/meta/<name>` (the same) | float 0..1 |
//! | `/strobe/state/blackout` | int 0 or 1 |
//! | `/strobe/state/cue` | int, the playing cue from 1 (0 for none) |
//!
//! ## Only what changed, no faster than the rate
//!
//! [`Differ::collect`] runs once per scene frame but does its work at most
//! [`RATE_HZ`] times a second, and then only for values that moved more
//! than [`EPSILON`] since they were last sent, at most [`MAX_PER_SEND`] of
//! them (the rest go next time). It sends a control's **target** (what was
//! asked for), not its smoothed output, so a fader is not sent a 30 Hz tail
//! of small steps after every move. A bang has no value to show and is not
//! sent. A new scene sends all of its controls once.
//!
//! ## The frame thread never waits
//!
//! The addresses are built when the scene's controls change, never per
//! frame. [`Sender`] owns a thread and a bounded queue; the frame only does
//! a non-blocking `try_send` of a shared address and a number, and a full
//! queue drops the value and counts it ([`Sender::dropped`]), to be sent
//! again when it next changes.
//!
//! ## No loop back into the listener
//!
//! Feedback to this machine on the listener's own port would make every
//! value arrive again as input. [`target`] refuses that setting with a
//! reason. A controller that echoes what it is sent (TouchOSC does not)
//! settles anyway: the echoed value equals the target, so nothing moves
//! and nothing is sent again.

use crate::config::OscSettings;
use crate::event::Arg;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};
use strobe_scene::control::{Bang, ControlBank, ControlKind};

/// The most sends a second.
pub const RATE_HZ: u32 = 30;

/// The smallest change, in normalised units, worth sending (well under one
/// MIDI step of 1/127).
pub const EPSILON: f32 = 1.0e-3;

/// The most messages in one send; the rest wait for the next.
pub const MAX_PER_SEND: usize = 64;

/// The sender's queue length.
const QUEUE: usize = 512;

/// The suffix for each component of a pad (`xy`) or colour.
const PAD: [&str; 2] = ["x", "y"];
const COLOUR: [&str; 3] = ["r", "g", "b"];

/// One value to send.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A control or meta component, normalised 0..1.
    Float(Arc<str>, f32),
    /// A state value.
    Int(Arc<str>, i32),
}

/// What the show is doing, besides its controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShowState {
    /// The output is blacked out.
    pub blackout: bool,
    /// The playing cue, from 1; `None` outside a playlist.
    pub cue: Option<u32>,
}

/// The addresses and last-sent values of one bank's controls.
#[derive(Debug, Default)]
struct Watched {
    /// The control names the addresses were built for, to notice a new
    /// scene without allocating.
    names: Vec<String>,
    /// Per sent component: control index, component, address, last value
    /// (`NaN` until first sent).
    slots: Vec<(usize, usize, Arc<str>, f32)>,
}

impl Watched {
    /// Rebuilds the slots when `bank`'s controls are not the ones watched.
    fn follow(&mut self, prefix: &str, bank: &ControlBank) {
        let defs = bank.defs();
        let same = defs.len() == self.names.len()
            && defs.iter().zip(&self.names).all(|(d, n)| d.name == *n);
        if same {
            return;
        }
        self.names = defs.iter().map(|d| d.name.clone()).collect();
        self.slots.clear();
        for (index, def) in defs.iter().enumerate() {
            let suffixes: &[&str] = match def.kind {
                ControlKind::Bang(
                    Bang::Plain | Bang::Smooth | Bang::Counter | Bang::CounterSmooth,
                ) => continue,
                ControlKind::Xy(_) => &PAD,
                ControlKind::Color { .. } => &COLOUR,
                ControlKind::Slider(_)
                | ControlKind::Knob(_)
                | ControlKind::Toggle { .. }
                | ControlKind::Dropdown { .. } => &[""],
            };
            for (component, suffix) in suffixes.iter().enumerate().take(def.dims().max(1)) {
                let address = if suffix.is_empty() {
                    format!("{prefix}{}", def.name)
                } else {
                    format!("{prefix}{}/{suffix}", def.name)
                };
                self.slots
                    .push((index, component, Arc::from(address), f32::NAN));
            }
        }
    }

    /// Appends what moved, up to `room` values; marks them sent.
    fn changes(&mut self, bank: &ControlBank, room: usize, out: &mut Vec<Value>) {
        let defs = bank.defs();
        for (index, component, address, last) in &mut self.slots {
            if out.len() >= room {
                return;
            }
            let (Some(def), Some(target)) = (defs.get(*index), bank.target(*index)) else {
                continue;
            };
            let Some(raw) = target.get(*component) else {
                continue;
            };
            let value = def.normalize(*component, *raw).clamp(0.0, 1.0);
            if !value.is_finite() {
                continue;
            }
            if last.is_nan() || (value - *last).abs() > EPSILON {
                *last = value;
                out.push(Value::Float(Arc::clone(address), value));
            }
        }
    }
}

/// Works out what to send (see the module docs).
#[derive(Debug)]
pub struct Differ {
    scene: Watched,
    meta: Watched,
    blackout: Option<bool>,
    cue: Option<Option<u32>>,
    next: Option<Instant>,
    blackout_address: Arc<str>,
    cue_address: Arc<str>,
}

impl Default for Differ {
    fn default() -> Self {
        Self {
            scene: Watched::default(),
            meta: Watched::default(),
            blackout: None,
            cue: None,
            next: None,
            blackout_address: Arc::from("/strobe/state/blackout"),
            cue_address: Arc::from("/strobe/state/cue"),
        }
    }
}

impl Differ {
    /// Forgets what was sent, so everything is sent again (a new target).
    pub fn resend_all(&mut self) {
        *self = Self::default();
    }

    /// Fills `out` (cleared first) with what changed, if a send is due at
    /// `now`. Allocates only when the scene's controls change.
    pub fn collect(
        &mut self,
        now: Instant,
        scene: Option<&ControlBank>,
        meta: Option<&ControlBank>,
        state: ShowState,
        out: &mut Vec<Value>,
    ) {
        out.clear();
        if self.next.is_some_and(|next| now < next) {
            return;
        }
        self.next = Some(now + Duration::from_secs(1) / RATE_HZ);
        if self.blackout != Some(state.blackout) {
            self.blackout = Some(state.blackout);
            out.push(Value::Int(
                Arc::clone(&self.blackout_address),
                i32::from(state.blackout),
            ));
        }
        if self.cue != Some(state.cue) {
            self.cue = Some(state.cue);
            let number = state.cue.and_then(|n| i32::try_from(n).ok()).unwrap_or(0);
            out.push(Value::Int(Arc::clone(&self.cue_address), number));
        }
        if let Some(meta) = meta {
            self.meta.follow("/strobe/meta/", meta);
            self.meta.changes(meta, MAX_PER_SEND, out);
        }
        match scene {
            Some(scene) => {
                self.scene.follow("/strobe/control/", scene);
                self.scene.changes(scene, MAX_PER_SEND, out);
            }
            None => self.scene = Watched::default(),
        }
    }
}

/// Where feedback goes, from the settings: `Ok(None)` when it is off,
/// `Err` (a sentence for the UI) when the setting cannot be used.
pub fn target(settings: &OscSettings) -> Result<Option<SocketAddr>, String> {
    if !settings.feedback {
        return Ok(None);
    }
    let host = settings.feedback_host.trim();
    let address = (host, settings.feedback_port)
        .to_socket_addrs()
        .map_err(|e| format!("feedback host \"{host}\": {e}"))?
        .next()
        .ok_or_else(|| format!("feedback host \"{host}\" has no address"))?;
    let local = |ip: IpAddr| ip.is_loopback() || ip.is_unspecified();
    if settings.enabled && settings.feedback_port == settings.port && local(address.ip()) {
        return Err(format!(
            "feedback to this computer's port {} would loop into the OSC listener; use another port",
            settings.port
        ));
    }
    Ok(Some(address))
}

enum Job {
    Send(Value),
    Retarget(Option<SocketAddr>),
}

/// The thread that encodes and sends feedback (see the module docs).
pub struct Sender {
    queue: SyncSender<Job>,
    dropped: Arc<AtomicU64>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Send(value) => write!(f, "Send({value:?})"),
            Self::Retarget(to) => write!(f, "Retarget({to:?})"),
        }
    }
}

impl std::fmt::Debug for Sender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sender")
            .field("dropped", &self.dropped())
            .finish_non_exhaustive()
    }
}

impl Sender {
    /// Starts the thread, sending nowhere until [`Self::retarget`].
    pub fn start() -> std::io::Result<Self> {
        let (queue, jobs) = sync_channel(QUEUE);
        std::thread::Builder::new()
            .name("strobe-osc-feedback".to_string())
            .spawn(move || run(&jobs))?;
        Ok(Self {
            queue,
            dropped: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Sends from now on to `to`, or nowhere.
    pub fn retarget(&self, to: Option<SocketAddr>) {
        // A retarget must not be lost; the queue is short, so this waits
        // at most for a few hundred sends. It happens on a settings change.
        let _ = self.queue.send(Job::Retarget(to));
    }

    /// Queues `values` without waiting; a full queue drops and counts.
    pub fn send(&self, values: &mut Vec<Value>) {
        for value in values.drain(..) {
            match self.queue.try_send(Job::Send(value)) {
                Ok(()) => {}
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Values dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The address the sending socket binds for feedback to `to`: loopback for
/// a controller on this computer, every interface otherwise. Loopback is the
/// source the system would pick anyway, and a socket on every interface is
/// what makes Windows' firewall ask about the program (D503).
fn source_ip(to: IpAddr) -> IpAddr {
    match to {
        IpAddr::V4(v4) if v4.is_loopback() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_loopback() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    }
}

fn run(jobs: &Receiver<Job>) {
    let mut socket: Option<(UdpSocket, SocketAddr)> = None;
    let mut warned = false;
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Retarget(to) => {
                socket = to.and_then(|to| {
                    match UdpSocket::bind(SocketAddr::new(source_ip(to.ip()), 0)) {
                        Ok(socket) => {
                            log::info!("OSC feedback: sending to {to}");
                            Some((socket, to))
                        }
                        Err(error) => {
                            log::warn!("OSC feedback: no socket: {error}");
                            None
                        }
                    }
                });
                warned = false;
            }
            Job::Send(value) => {
                let Some((socket, to)) = &socket else {
                    continue;
                };
                let packet = match &value {
                    Value::Float(address, x) => crate::osc::encode(address, &[Arg::Float(*x)]),
                    Value::Int(address, n) => crate::osc::encode(address, &[Arg::Int(*n)]),
                };
                if let Some(packet) = packet
                    && let Err(error) = socket.send_to(&packet, to)
                    && !warned
                {
                    log::warn!("OSC feedback to {to}: {error}");
                    warned = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use strobe_scene::control::ControlDef;
    use strobe_scene::json::RawControl;

    fn bank(json: &str) -> Result<ControlBank> {
        let raw: Vec<RawControl> = serde_json::from_str(json)?;
        let defs = raw
            .iter()
            .map(ControlDef::from_raw)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ControlBank::new(defs)?)
    }

    fn scene() -> Result<ControlBank> {
        bank(
            r#"[
                { "NAME": "speed", "TYPE": "slider", "MIN": 0, "MAX": 10, "DEFAULT": 5 },
                { "NAME": "pad", "TYPE": "xy", "DEFAULT": [0.25, 0.75] },
                { "NAME": "tint", "TYPE": "color", "DEF_COLOR": [1, 0, 0.5] },
                { "NAME": "hit", "TYPE": "bang" }
            ]"#,
        )
    }

    fn sent(values: &[Value]) -> Vec<String> {
        values
            .iter()
            .map(|v| match v {
                Value::Float(a, x) => format!("{a} {x:.3}"),
                Value::Int(a, n) => format!("{a} {n}"),
            })
            .collect()
    }

    #[test]
    fn the_first_send_is_everything_then_only_what_moved() -> Result<()> {
        let mut scene = scene()?;
        let mut differ = Differ::default();
        let mut out = Vec::new();
        let start = Instant::now();
        let state = ShowState::default();
        differ.collect(start, Some(&scene), None, state, &mut out);
        assert_eq!(
            sent(&out),
            [
                "/strobe/state/blackout 0",
                "/strobe/state/cue 0",
                "/strobe/control/speed 0.500",
                "/strobe/control/pad/x 0.250",
                "/strobe/control/pad/y 0.750",
                "/strobe/control/tint/r 1.000",
                "/strobe/control/tint/g 0.000",
                "/strobe/control/tint/b 0.500",
            ],
            "every control once, a bang never"
        );

        differ.collect(start, Some(&scene), None, state, &mut out);
        assert!(out.is_empty(), "not due again within the rate: {out:?}");

        let later = start + Duration::from_millis(40);
        differ.collect(later, Some(&scene), None, state, &mut out);
        assert!(out.is_empty(), "nothing moved: {out:?}");

        scene.set(0, None, &[7.5]);
        let later = later + Duration::from_millis(40);
        let blacked = ShowState {
            blackout: true,
            cue: Some(3),
        };
        differ.collect(later, Some(&scene), None, blacked, &mut out);
        assert_eq!(
            sent(&out),
            [
                "/strobe/state/blackout 1",
                "/strobe/state/cue 3",
                "/strobe/control/speed 0.750"
            ],
            "only the changes"
        );
        Ok(())
    }

    #[test]
    fn a_new_scene_is_sent_whole_and_a_send_is_bounded() -> Result<()> {
        let many: Vec<String> = (0..100)
            .map(|i| format!(r#"{{ "NAME": "c{i}", "TYPE": "slider" }}"#))
            .collect();
        let big = bank(&format!("[{}]", many.join(",")))?;
        let mut differ = Differ::default();
        let mut out = Vec::new();
        let mut now = Instant::now();
        differ.collect(now, Some(&big), None, ShowState::default(), &mut out);
        assert_eq!(out.len(), MAX_PER_SEND, "bounded");
        now += Duration::from_millis(40);
        differ.collect(now, Some(&big), None, ShowState::default(), &mut out);
        assert_eq!(out.len(), 100 + 2 - MAX_PER_SEND, "the rest next time");

        let other = scene()?;
        now += Duration::from_millis(40);
        differ.collect(now, Some(&other), None, ShowState::default(), &mut out);
        assert_eq!(out.len(), 6, "a new scene's six components are all sent");
        Ok(())
    }

    #[test]
    fn feedback_into_the_listeners_own_port_is_refused() -> Result<()> {
        let mut settings = OscSettings {
            enabled: true,
            feedback: true,
            feedback_host: "127.0.0.1".to_string(),
            ..OscSettings::default()
        };
        settings.feedback_port = settings.port;
        assert!(target(&settings).is_err(), "a loop");
        settings.feedback_port = settings.port + 1;
        assert!(
            matches!(target(&settings), Ok(Some(_))),
            "another port is fine"
        );
        settings.feedback = false;
        assert_eq!(target(&settings), Ok(None), "off is off");
        Ok(())
    }

    #[test]
    fn the_sender_delivers_over_udp() -> Result<()> {
        let listener = UdpSocket::bind("127.0.0.1:0")?;
        listener.set_read_timeout(Some(Duration::from_secs(5)))?;
        let to = listener.local_addr()?;
        let sender = Sender::start()?;
        sender.retarget(Some(to));
        let mut values = vec![
            Value::Float(Arc::from("/strobe/control/speed"), 0.75),
            Value::Int(Arc::from("/strobe/state/blackout"), 1),
        ];
        sender.send(&mut values);
        assert!(values.is_empty(), "drained");
        let mut got = Vec::new();
        let mut buffer = [0_u8; 1024];
        for _ in 0..2 {
            let (n, _) = listener.recv_from(&mut buffer)?;
            let bytes = buffer.get(..n).ok_or_else(|| anyhow!("short read"))?;
            for message in crate::osc::decode(bytes).map_err(|e| anyhow!("{e}"))? {
                got.push(format!("{} {:?}", message.address, message.args));
            }
        }
        assert_eq!(
            got,
            [
                "/strobe/control/speed [Float(0.75)]",
                "/strobe/state/blackout [Int(1)]"
            ],
            "both arrive, in order"
        );
        assert_eq!(sender.dropped(), 0, "nothing dropped");
        Ok(())
    }

    #[test]
    fn feedback_to_this_computer_is_sent_from_loopback() -> Result<()> {
        for (to, from) in [
            ("127.0.0.1", "127.0.0.1"),
            ("127.0.0.5", "127.0.0.1"),
            ("::1", "::1"),
            ("192.168.1.30", "0.0.0.0"),
            ("fd00::30", "::"),
        ] {
            assert_eq!(source_ip(to.parse()?), from.parse::<IpAddr>()?, "{to}");
        }
        Ok(())
    }
}
