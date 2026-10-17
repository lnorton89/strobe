// ============================================================================
// Module:       strobe_io::hub
// Description:  The worker threads (MIDI and OSC) behind one handle, and the
//               bounded queues they deliver into: one for events in order,
//               one for the latest value of each fader.
//
// Dependencies: std (threads, mpsc, net); crate::{event, config, message,
//               midi, osc}
// ============================================================================

//! The I/O hub (D171).
//!
//! Two worker threads, one per transport, own every blocking call: the MIDI
//! worker enumerates and opens devices, the OSC worker sits in `recv_from`.
//! They never touch app state. They hand the app typed [`Event`]s through
//! two **bounded** queues, and a full one drops the newest message and
//! counts it, because a stalled frame loop must never push back into a
//! driver callback.
//!
//! - **Continuous MIDI values** (a CC, a pitch bend) wait in a table of at
//!   most [`LATEST`] entries holding the **newest value per source** (device,
//!   channel, controller): a fader sweep at 1 kHz, or a controller stuck
//!   sending, while the frame loop is stalled replaces one entry instead of
//!   piling up. A CC that crosses the middle of its range (64) is kept as a
//!   new entry rather than folded into the last one, so a CC button's press
//!   and release (127 then 0) both arrive even if both land within one frame
//!   (D421).
//! - **Everything else** (notes, program changes, OSC, status reports) keeps
//!   its order in a channel of [`QUEUE`] events. Faders no longer share it,
//!   so a flood of them can never fill it and drop a note off.
//!
//! The app calls [`Hub::drain`] once per scene frame; it takes the ordered
//! events first, then the latest values, at most the number it is told to in
//! all, and never blocks.
//!
//! The app commands the workers through small channels
//! ([`Hub::set_osc`], [`Hub::set_midi`], [`Hub::rescan_midi`]); those calls
//! return at once and the worker acts on its next wake-up (the OSC socket
//! wakes every 50 ms, the MIDI worker immediately).
//!
//! Dropping the hub stops both workers and waits for them, which takes at
//! most one OSC wake-up.

use crate::config::{MidiSettings, OscSettings};
use crate::event::{Event, InputEvent, Status};
use crate::message::MidiMessage;
use crate::{listener, midi};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// How many ordered events the queue holds before it starts dropping.
pub const QUEUE: usize = 2048;

/// How many continuous MIDI values (each the newest for its source, or for
/// one half of a CC's range) wait at once before new ones are dropped.
pub const LATEST: usize = 1024;

/// A source of continuous MIDI values on one device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Continuous {
    /// A CC: its channel and controller.
    Control(u8, u8),
    /// The pitch wheel on a channel.
    Bend(u8),
}

impl Continuous {
    /// The source `message` is a value of, and whether the value is in the
    /// top half of a CC's range; `None` for a discrete message, which is
    /// never folded into another.
    fn of(message: &MidiMessage) -> Option<(Self, bool)> {
        match *message {
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => Some((Self::Control(channel, controller), value >= 64)),
            MidiMessage::PitchBend { channel, .. } => Some((Self::Bend(channel), false)),
            MidiMessage::NoteOn { .. }
            | MidiMessage::NoteOff { .. }
            | MidiMessage::ProgramChange { .. } => None,
        }
    }
}

/// One continuous value waiting for the app.
#[derive(Debug)]
struct Pending {
    device: Arc<str>,
    source: Continuous,
    /// Which half of a CC's range the value is in.
    high: bool,
    message: MidiMessage,
}

/// The continuous values waiting for the app, oldest source first.
#[derive(Debug)]
struct Latest {
    pending: Vec<Pending>,
}

impl Latest {
    /// An empty table. It never grows past [`LATEST`], so offering a value
    /// never allocates.
    fn new() -> Self {
        Self {
            pending: Vec::with_capacity(LATEST),
        }
    }

    /// Takes `message`, a value of `source` on `device`. It replaces the
    /// newest waiting value of the same source when that one is in the same
    /// half of the range, and joins the end of the table otherwise. `false`
    /// when the table is full and the value was dropped.
    fn offer(
        &mut self,
        device: &Arc<str>,
        source: Continuous,
        high: bool,
        message: MidiMessage,
    ) -> bool {
        if let Some(newest) = self
            .pending
            .iter_mut()
            .rev()
            .find(|p| p.source == source && *p.device == **device)
            && newest.high == high
        {
            newest.message = message;
            return true;
        }
        if self.pending.len() >= LATEST {
            return false;
        }
        self.pending.push(Pending {
            device: Arc::clone(device),
            source,
            high,
            message,
        });
        true
    }
}

/// The workers' end of the queues.
#[derive(Clone)]
pub(crate) struct Tx {
    queue: SyncSender<Event>,
    latest: Arc<Mutex<Latest>>,
    dropped: Arc<AtomicU64>,
}

impl Tx {
    /// Queues an event, or counts it as dropped. A continuous MIDI value
    /// goes to the table of latest values, whose lock [`Hub::drain`] holds
    /// only to move at most [`LATEST`] small entries; nothing else waits.
    pub(crate) fn send(&self, event: Event) {
        if let Event::Input(InputEvent::Midi { device, message }) = &event
            && let Some((source, high)) = Continuous::of(message)
        {
            let kept = self
                .latest
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .offer(device, source, high, *message);
            if !kept {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        if self.queue.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Queues a status report.
    pub(crate) fn status(&self, status: Status) {
        self.send(Event::Status(status));
    }

    /// A queue of `capacity` and its receiving end, with no hub behind it,
    /// for a worker's tests. Continuous MIDI values go to a table nothing
    /// drains; the OSC worker sends none.
    #[cfg(test)]
    pub(crate) fn detached(capacity: usize) -> (Self, Receiver<Event>) {
        let (queue, events) = sync_channel(capacity);
        let tx = Self {
            queue,
            latest: Arc::new(Mutex::new(Latest::new())),
            dropped: Arc::new(AtomicU64::new(0)),
        };
        (tx, events)
    }
}

/// A command for the OSC worker.
pub(crate) enum OscControl {
    /// Listen here (replacing any listener), or stop.
    Bind(Option<std::net::SocketAddr>),
    /// Exit.
    Shutdown,
}

/// A command for the MIDI worker.
pub(crate) enum MidiControl {
    /// New settings.
    #[cfg_attr(
        not(feature = "midi"),
        expect(
            dead_code,
            reason = "only the device worker reads the settings; the build without `midi` just waits to be stopped"
        )
    )]
    Settings(MidiSettings),
    /// Look at the ports again now.
    Rescan,
    /// Exit.
    Shutdown,
}

/// The handle on both workers.
pub struct Hub {
    events: Receiver<Event>,
    latest: Arc<Mutex<Latest>>,
    osc: Sender<OscControl>,
    midi: Sender<MidiControl>,
    dropped: Arc<AtomicU64>,
    reported_dropped: u64,
    threads: Vec<JoinHandle<()>>,
}

impl Hub {
    /// Starts the workers. OSC listens only if `osc.enabled`; MIDI opens
    /// devices only if `midi.enabled`. A worker that cannot start is
    /// reported as a status and the hub carries on without it.
    pub fn start(midi_settings: &MidiSettings, osc_settings: &OscSettings) -> Self {
        let (queue, events) = sync_channel(QUEUE);
        let latest = Arc::new(Mutex::new(Latest::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let tx = Tx {
            queue,
            latest: Arc::clone(&latest),
            dropped: Arc::clone(&dropped),
        };
        let (osc_ctl, osc_rx) = channel();
        let (midi_ctl, midi_rx) = channel();
        let mut threads = Vec::new();

        let osc_tx = tx.clone();
        match std::thread::Builder::new()
            .name("strobe-osc".into())
            .spawn(move || listener::run(&osc_rx, &osc_tx))
        {
            Ok(handle) => threads.push(handle),
            Err(error) => tx.status(Status::OscError(format!(
                "could not start the OSC worker: {error}"
            ))),
        }
        let midi_tx = tx.clone();
        let initial = midi_settings.clone();
        match std::thread::Builder::new()
            .name("strobe-midi".into())
            .spawn(move || midi::run(&midi_rx, &midi_tx, initial))
        {
            Ok(handle) => threads.push(handle),
            Err(error) => tx.status(Status::MidiUnavailable(format!(
                "could not start the MIDI worker: {error}"
            ))),
        }
        let hub = Self {
            events,
            latest,
            osc: osc_ctl,
            midi: midi_ctl,
            dropped,
            reported_dropped: 0,
            threads,
        };
        hub.set_osc(osc_settings);
        hub
    }

    /// Applies new OSC settings: starts, moves or stops the listener.
    pub fn set_osc(&self, settings: &OscSettings) {
        let addr = settings.enabled.then(|| settings.bind_addr());
        // A closed channel means the OSC worker has stopped (shutdown); there
        // is nothing to apply.
        let _ = self.osc.send(OscControl::Bind(addr));
    }

    /// Applies new MIDI settings.
    pub fn set_midi(&self, settings: &MidiSettings) {
        // A closed channel means the MIDI worker has stopped (shutdown);
        // there is nothing to apply.
        let _ = self.midi.send(MidiControl::Settings(settings.clone()));
    }

    /// Asks the MIDI worker to look at the ports now (a "Refresh" button).
    pub fn rescan_midi(&self) {
        // A closed channel means the MIDI worker has stopped (shutdown);
        // there is nothing to rescan.
        let _ = self.midi.send(MidiControl::Rescan);
    }

    /// Moves up to `max` queued events into `out` without blocking: the
    /// ordered ones oldest first, then the latest continuous values, oldest
    /// source first. When messages were dropped since the last call, a
    /// [`Status::Dropped`] with the new total is appended.
    pub fn drain(&mut self, max: usize, out: &mut Vec<Event>) {
        let mut taken = 0;
        while taken < max {
            match self.events.try_recv() {
                Ok(event) => out.push(event),
                Err(_) => break,
            }
            taken += 1;
        }
        let room = max - taken;
        if room > 0 {
            let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
            let count = room.min(latest.pending.len());
            out.extend(latest.pending.drain(..count).map(|pending| {
                Event::Input(InputEvent::Midi {
                    device: pending.device,
                    message: pending.message,
                })
            }));
        }
        let total = self.dropped.load(Ordering::Relaxed);
        if total != self.reported_dropped {
            self.reported_dropped = total;
            out.push(Event::Status(Status::Dropped(total)));
        }
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        // A worker that has already stopped needs no shutdown message.
        let _ = self.osc.send(OscControl::Shutdown);
        // A worker that has already stopped needs no shutdown message.
        let _ = self.midi.send(MidiControl::Shutdown);
        for handle in self.threads.drain(..) {
            // Waits for the workers; a panic in one already aborted the
            // process in release builds.
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Arg, InputEvent};
    use anyhow::{Result, anyhow};
    use std::net::{SocketAddr, UdpSocket};
    use std::time::{Duration, Instant};

    fn quiet_midi() -> MidiSettings {
        MidiSettings {
            enabled: false,
            disabled_devices: Vec::new(),
            feedback: false,
        }
    }

    /// Drains until `wanted` accepts an event or two seconds pass.
    fn wait_for(hub: &mut Hub, mut wanted: impl FnMut(&Event) -> bool) -> Option<Event> {
        let limit = Instant::now() + Duration::from_secs(3);
        let mut buffer = Vec::new();
        while Instant::now() < limit {
            hub.drain(64, &mut buffer);
            if let Some(found) = buffer.drain(..).find(|e| wanted(e)) {
                return Some(found);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    fn listening(hub: &mut Hub) -> Result<SocketAddr> {
        match wait_for(hub, |e| matches!(e, Event::Status(Status::OscListening(_)))) {
            Some(Event::Status(Status::OscListening(addr))) => Ok(addr),
            other => Err(anyhow!("never started listening: {other:?}")),
        }
    }

    #[test]
    fn udp_messages_arrive_as_typed_events_end_to_end() -> Result<()> {
        let osc = OscSettings {
            enabled: true,
            port: 0,
            ..OscSettings::default()
        };
        let mut hub = Hub::start(&quiet_midi(), &osc);
        let addr = listening(&mut hub)?;
        assert!(addr.ip().is_loopback(), "bound to loopback: {addr}");
        let client = UdpSocket::bind("127.0.0.1:0")?;
        let bytes = crate::osc::encode("/strobe/control/speed", &[Arg::Float(0.75)])
            .ok_or_else(|| anyhow!("encode"))?;
        client.send_to(&bytes, addr)?;
        let got = wait_for(&mut hub, |e| matches!(e, Event::Input(_)))
            .ok_or_else(|| anyhow!("no message arrived"))?;
        let Event::Input(InputEvent::Osc(message)) = got else {
            return Err(anyhow!("unexpected {got:?}"));
        };
        assert_eq!(message.address, "/strobe/control/speed", "address");
        assert_eq!(message.args, vec![Arg::Float(0.75)], "argument");
        Ok(())
    }

    #[test]
    fn garbage_datagrams_are_ignored_and_the_listener_carries_on() -> Result<()> {
        let osc = OscSettings {
            enabled: true,
            port: 0,
            ..OscSettings::default()
        };
        let mut hub = Hub::start(&quiet_midi(), &osc);
        let addr = listening(&mut hub)?;
        let client = UdpSocket::bind("127.0.0.1:0")?;
        client.send_to(b"\xff\xfe not osc", addr)?;
        client.send_to(&[], addr)?;
        let bytes = crate::osc::encode("/after", &[]).ok_or_else(|| anyhow!("encode"))?;
        client.send_to(&bytes, addr)?;
        let got = wait_for(&mut hub, |e| matches!(e, Event::Input(_)))
            .ok_or_else(|| anyhow!("the good message after the bad ones never arrived"))?;
        assert!(
            matches!(&got, Event::Input(InputEvent::Osc(m)) if m.address == "/after"),
            "{got:?}"
        );
        Ok(())
    }

    #[test]
    fn switching_osc_off_stops_the_listener_and_on_again_rebinds() -> Result<()> {
        let mut settings = OscSettings {
            enabled: true,
            port: 0,
            ..OscSettings::default()
        };
        let mut hub = Hub::start(&quiet_midi(), &settings);
        listening(&mut hub)?;
        settings.enabled = false;
        hub.set_osc(&settings);
        wait_for(&mut hub, |e| matches!(e, Event::Status(Status::OscStopped)))
            .ok_or_else(|| anyhow!("no stop report"))?;
        settings.enabled = true;
        hub.set_osc(&settings);
        listening(&mut hub)?;
        Ok(())
    }

    #[test]
    fn a_port_that_is_taken_is_reported_and_not_a_crash() -> Result<()> {
        let squatter = UdpSocket::bind("127.0.0.1:0")?;
        let taken = squatter.local_addr()?.port();
        let osc = OscSettings {
            enabled: true,
            port: taken,
            ..OscSettings::default()
        };
        let mut hub = Hub::start(&quiet_midi(), &osc);
        let got = wait_for(&mut hub, |e| {
            matches!(e, Event::Status(Status::OscError(_)))
        })
        .ok_or_else(|| anyhow!("no error report"))?;
        if let Event::Status(Status::OscError(message)) = got {
            assert!(
                message.contains(&taken.to_string()),
                "names the port: {message}"
            );
        }
        Ok(())
    }

    /// A hub with no workers behind it whose ordered queue holds
    /// `capacity`, and the workers' end of it.
    fn rig(capacity: usize) -> (Tx, Hub) {
        let (queue, events) = sync_channel(capacity);
        let latest = Arc::new(Mutex::new(Latest::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let tx = Tx {
            queue,
            latest: Arc::clone(&latest),
            dropped: Arc::clone(&dropped),
        };
        // The workers' ends are dropped: the hub's shutdown messages then
        // go nowhere, which it allows.
        let (osc, _) = channel();
        let (midi, _) = channel();
        let hub = Hub {
            events,
            latest,
            osc,
            midi,
            dropped,
            reported_dropped: 0,
            threads: Vec::new(),
        };
        (tx, hub)
    }

    /// Sends `message` from `device` as the MIDI callback does.
    fn midi(tx: &Tx, device: &Arc<str>, message: MidiMessage) {
        tx.send(Event::Input(InputEvent::Midi {
            device: Arc::clone(device),
            message,
        }));
    }

    fn cc(channel: u8, controller: u8, value: u8) -> MidiMessage {
        MidiMessage::ControlChange {
            channel,
            controller,
            value,
        }
    }

    /// The MIDI messages in `events`, with their devices, in order.
    fn messages(events: &[Event]) -> Vec<(String, MidiMessage)> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Input(InputEvent::Midi { device, message }) => {
                    Some((device.to_string(), *message))
                }
                Event::Input(InputEvent::Osc(_)) | Event::Status(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_full_queue_drops_and_counts_instead_of_blocking() -> Result<()> {
        let (tx, mut hub) = rig(2);
        let started = Instant::now();
        for _ in 0..10 {
            tx.status(Status::OscStopped);
        }
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "never blocked"
        );
        let mut out = Vec::new();
        hub.drain(100, &mut out);
        assert_eq!(
            out.len(),
            3,
            "the two that fit, then the drop report: {out:?}"
        );
        assert_eq!(
            out.last(),
            Some(&Event::Status(Status::Dropped(8))),
            "eight were dropped"
        );
        out.clear();
        hub.drain(100, &mut out);
        assert!(out.is_empty(), "reported once");
        Ok(())
    }

    #[test]
    fn drain_takes_no_more_than_it_is_asked_for() -> Result<()> {
        let (tx, mut hub) = rig(16);
        let device: Arc<str> = Arc::from("faders");
        for _ in 0..10 {
            tx.status(Status::OscStopped);
        }
        for controller in 0..4 {
            midi(&tx, &device, cc(0, controller, 10));
        }
        let mut out = Vec::new();
        hub.drain(4, &mut out);
        assert_eq!(out.len(), 4, "four");
        hub.drain(8, &mut out);
        assert_eq!(out.len(), 12, "the six ordered ones, then two faders");
        hub.drain(100, &mut out);
        assert_eq!(out.len(), 14, "the last two faders on the next call");
        Ok(())
    }

    #[test]
    fn a_fader_flood_keeps_the_latest_value_and_never_drops_a_note() -> Result<()> {
        // A stalled frame loop (a shader compile, a window drag on Windows)
        // while a fader sweeps at 1 kHz and a wheel is held moving, with a
        // pad played through it: 100,000 continuous messages and 200 notes,
        // nothing drained. Before D421 they shared one queue of 2048, which
        // the fader filled in two seconds; every note after that, note offs
        // included, was dropped.
        let (tx, mut hub) = rig(QUEUE);
        let device: Arc<str> = Arc::from("nanoKONTROL2");
        let mut notes = Vec::new();
        for i in 0..100_000_u32 {
            let value = 64 + u8::try_from(i % 64)?;
            midi(&tx, &device, cc(0, 7, value));
            let bend = u16::try_from(i % 16_384)?;
            midi(
                &tx,
                &device,
                MidiMessage::PitchBend {
                    channel: 1,
                    value: bend,
                },
            );
            if i % 1000 == 0 {
                let note = u8::try_from(i / 1000)?;
                let on = MidiMessage::NoteOn {
                    channel: 9,
                    note,
                    velocity: 100,
                };
                let off = MidiMessage::NoteOff { channel: 9, note };
                midi(&tx, &device, on);
                midi(&tx, &device, off);
                notes.extend([on, off]);
            }
        }
        let mut out = Vec::new();
        hub.drain(usize::MAX, &mut out);
        assert!(
            !out.iter()
                .any(|event| matches!(event, Event::Status(Status::Dropped(_)))),
            "nothing was dropped"
        );
        let got = messages(&out);
        let played: Vec<MidiMessage> = got
            .iter()
            .map(|(_, message)| *message)
            .filter(|message| Continuous::of(message).is_none())
            .collect();
        assert_eq!(played, notes, "every note on and off, in order");
        let continuous: Vec<MidiMessage> = got
            .iter()
            .map(|(_, message)| *message)
            .filter(|message| Continuous::of(message).is_some())
            .collect();
        assert_eq!(
            continuous,
            vec![
                cc(0, 7, 64 + u8::try_from(99_999 % 64)?),
                MidiMessage::PitchBend {
                    channel: 1,
                    value: u16::try_from(99_999 % 16_384)?,
                },
            ],
            "one entry per source, holding its latest value"
        );
        assert_eq!(out.len(), 202, "the queue held 202 events, not 200,200");
        Ok(())
    }

    #[test]
    fn a_cc_button_press_and_release_in_one_frame_both_arrive() -> Result<()> {
        // A nanoKONTROL2 button is a CC: 127 down, 0 up. Pressed twice
        // within one stalled frame, the presses must survive coalescing; a
        // fader moving within one half of its range still folds.
        let (tx, mut hub) = rig(QUEUE);
        let device: Arc<str> = Arc::from("nanoKONTROL2");
        for value in [127, 0, 127, 0] {
            midi(&tx, &device, cc(0, 41, value));
        }
        for value in [10, 20, 30] {
            midi(&tx, &device, cc(0, 0, value));
        }
        let mut out = Vec::new();
        hub.drain(usize::MAX, &mut out);
        let values: Vec<MidiMessage> = messages(&out).into_iter().map(|(_, m)| m).collect();
        assert_eq!(
            values,
            vec![
                cc(0, 41, 127),
                cc(0, 41, 0),
                cc(0, 41, 127),
                cc(0, 41, 0),
                cc(0, 0, 30),
            ],
            "each press and release, and the fader's latest value"
        );
        Ok(())
    }

    #[test]
    fn the_same_controller_on_two_devices_or_channels_is_two_sources() -> Result<()> {
        let (tx, mut hub) = rig(QUEUE);
        let left: Arc<str> = Arc::from("deck A");
        let right: Arc<str> = Arc::from("deck B");
        for value in [1, 2, 3] {
            midi(&tx, &left, cc(0, 7, value));
            midi(&tx, &right, cc(0, 7, value + 10));
            midi(&tx, &left, cc(1, 7, value + 20));
        }
        let mut out = Vec::new();
        hub.drain(usize::MAX, &mut out);
        assert_eq!(
            messages(&out),
            vec![
                ("deck A".to_string(), cc(0, 7, 3)),
                ("deck B".to_string(), cc(0, 7, 13)),
                ("deck A".to_string(), cc(1, 7, 23)),
            ],
            "latest per (device, channel, controller)"
        );
        Ok(())
    }

    #[test]
    fn the_table_of_latest_values_is_bounded_and_counts_what_it_drops() -> Result<()> {
        // A controller sending every CC on every channel (more sources than
        // the table holds) cannot grow it, and still cannot crowd out a note.
        let (tx, mut hub) = rig(QUEUE);
        let device: Arc<str> = Arc::from("flood");
        let mut sent = 0_u64;
        for channel in 0..16 {
            for controller in 0..120 {
                midi(&tx, &device, cc(channel, controller, 5));
                sent += 1;
            }
        }
        let off = MidiMessage::NoteOff {
            channel: 0,
            note: 60,
        };
        midi(&tx, &device, off);
        let mut out = Vec::new();
        hub.drain(usize::MAX, &mut out);
        let dropped = sent - LATEST as u64;
        assert_eq!(
            out.last(),
            Some(&Event::Status(Status::Dropped(dropped))),
            "the sources past the table's size are counted"
        );
        let got = messages(&out);
        assert_eq!(got.len(), LATEST + 1, "the table and the note");
        assert_eq!(
            got.first().map(|(_, m)| *m),
            Some(off),
            "the note off was kept, and comes first"
        );
        Ok(())
    }

    #[test]
    fn the_midi_worker_reports_unavailable_or_an_empty_list_without_hardware() -> Result<()> {
        let mut hub = Hub::start(&MidiSettings::default(), &OscSettings::default());
        let got = wait_for(&mut hub, |e| {
            matches!(
                e,
                Event::Status(Status::MidiUnavailable(_) | Status::Devices(_))
            )
        });
        assert!(
            got.is_some(),
            "a machine with no MIDI says so rather than saying nothing"
        );
        Ok(())
    }
}
