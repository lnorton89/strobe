// ============================================================================
// Module:       strobe_io::hub
// Description:  The worker threads (MIDI and OSC) behind one handle, and the
//               bounded queue they deliver into.
//
// Dependencies: std (threads, mpsc, net); crate::{event, config, midi, osc}
// ============================================================================

//! The I/O hub (D171).
//!
//! Two worker threads, one per transport, own every blocking call: the MIDI
//! worker enumerates and opens devices, the OSC worker sits in `recv_from`.
//! They never touch app state. They hand the app typed [`Event`]s through
//! one **bounded** queue ([`QUEUE`] events) with `try_send`: a full queue
//! drops the newest message and counts it, because a stalled frame loop must
//! never push back into a driver callback, and a flood of fader messages is
//! exactly the thing that should be thinned. The app calls [`Hub::drain`] once
//! per scene frame; it takes at most the number it is told to and never
//! blocks.
//!
//! The app commands the workers through small channels
//! ([`Hub::set_osc`], [`Hub::set_midi`], [`Hub::rescan_midi`]); those calls
//! return at once and the worker acts on its next wake-up (the OSC socket
//! wakes every 50 ms, the MIDI worker immediately).
//!
//! Dropping the hub stops both workers and waits for them, which takes at
//! most one OSC wake-up.

use crate::config::{MidiSettings, OscSettings};
use crate::event::{Event, Status};
use crate::{listener, midi};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::thread::JoinHandle;

/// How many events the queue holds before it starts dropping.
pub const QUEUE: usize = 2048;

/// The workers' end of the queue.
#[derive(Clone)]
pub(crate) struct Tx {
    queue: SyncSender<Event>,
    dropped: Arc<AtomicU64>,
}

impl Tx {
    /// Queues an event, or counts it as dropped. Never blocks.
    pub(crate) fn send(&self, event: Event) {
        if self.queue.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Queues a status report.
    pub(crate) fn status(&self, status: Status) {
        self.send(Event::Status(status));
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
        let dropped = Arc::new(AtomicU64::new(0));
        let tx = Tx {
            queue,
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

    /// Moves up to `max` queued events into `out`, oldest first, without
    /// blocking. When messages were dropped since the last call, a
    /// [`Status::Dropped`] with the new total is appended.
    pub fn drain(&mut self, max: usize, out: &mut Vec<Event>) {
        for _ in 0..max {
            match self.events.try_recv() {
                Ok(event) => out.push(event),
                Err(_) => break,
            }
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

    #[test]
    fn a_full_queue_drops_and_counts_instead_of_blocking() -> Result<()> {
        let (queue, events) = sync_channel(2);
        let dropped = Arc::new(AtomicU64::new(0));
        let tx = Tx {
            queue,
            dropped: Arc::clone(&dropped),
        };
        let (osc, _osc_rx) = channel();
        let (midi, _midi_rx) = channel();
        let mut hub = Hub {
            events,
            osc,
            midi,
            dropped,
            reported_dropped: 0,
            threads: Vec::new(),
        };
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
        let (queue, events) = sync_channel(16);
        let dropped = Arc::new(AtomicU64::new(0));
        let tx = Tx {
            queue,
            dropped: Arc::clone(&dropped),
        };
        let (osc, _a) = channel();
        let (midi, _b) = channel();
        let mut hub = Hub {
            events,
            osc,
            midi,
            dropped,
            reported_dropped: 0,
            threads: Vec::new(),
        };
        for _ in 0..10 {
            tx.status(Status::OscStopped);
        }
        let mut out = Vec::new();
        hub.drain(4, &mut out);
        assert_eq!(out.len(), 4, "four");
        hub.drain(100, &mut out);
        assert_eq!(out.len(), 10, "the rest on the next call");
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
