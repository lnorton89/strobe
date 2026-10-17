// ============================================================================
// Module:       strobe_io::midi
// Description:  The MIDI worker thread: finds input devices, opens them,
//               notices when one is unplugged or plugged back in, and
//               forwards parsed messages.
//
// Dependencies: midir (feature `midi`); crate::{hub, message, event, config}
// ============================================================================

//! The MIDI worker (D171).
//!
//! `midir` has no hot-plug notification, so the worker **polls**: once a
//! second it lists the input ports and compares them with the devices it has
//! open.
//!
//! - A port that appeared and is not switched off is opened
//!   ([`Status::DeviceFound`]). If opening fails (on Windows another program
//!   may hold the device) the error goes into the device's entry
//!   ([`DeviceInfo::error`]) and is not retried every second; unplugging and
//!   replugging, or the Refresh button, tries again.
//! - A device that is open and no longer listed is closed and reported
//!   ([`Status::DeviceLost`]). It is opened again when it comes back, with the
//!   same key, so its mappings (which name the key) work again without any
//!   action. A connection also remembers midir's id of the port it was
//!   opened on, and is closed and opened again when its key is listed on
//!   another port: of two devices with one name, unplugging the first gives
//!   the second the plain key.
//! - No MIDI service at all (no ALSA sequencer in a container, no driver) is
//!   one [`Status::MidiUnavailable`], retried every few seconds in case it
//!   appears. Nothing here can take the app down.
//!
//! Each open device's callback runs on midir's own thread. It parses the
//! bytes ([`crate::message::parse`]) and `try_send`s; it never blocks, never
//! allocates beyond the event, and drops what it cannot queue.
//!
//! Without the `midi` feature this module only reports that MIDI is not
//! built in.

use crate::config::MidiSettings;
use crate::hub::{MidiControl, Tx};
use std::sync::mpsc::Receiver;

#[cfg(feature = "midi")]
mod device {
    use super::{MidiControl, MidiSettings, Receiver, Tx};
    use crate::event::{DeviceInfo, Event, InputEvent, Status};
    use crate::message::{device_keys, parse};
    use midir::{MidiInput, MidiInputConnection, MidiInputPort};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::{Duration, Instant};

    /// How often the ports are listed.
    const POLL: Duration = Duration::from_secs(1);

    /// How often a missing MIDI service is looked for again.
    const RETRY: Duration = Duration::from_secs(5);

    struct Present {
        key: String,
        name: String,
        /// midir's identity of the port (ALSA's `client:port`, WinMM's
        /// device interface path, CoreMIDI's unique id).
        id: String,
        port: MidiInputPort,
    }

    struct Worker {
        tx: Tx,
        settings: MidiSettings,
        scanner: Option<MidiInput>,
        /// Each open device by key, with the id of the port it was opened on.
        open: HashMap<String, (String, MidiInputConnection<()>)>,
        /// Devices that would not open, by key: the port tried, and why.
        failed: HashMap<String, (String, String)>,
        published: Option<Vec<DeviceInfo>>,
        unavailable: Option<String>,
        next_retry: Instant,
    }

    pub(super) fn run(commands: &Receiver<MidiControl>, tx: &Tx, settings: MidiSettings) {
        let mut worker = Worker {
            tx: tx.clone(),
            settings,
            scanner: None,
            open: HashMap::new(),
            failed: HashMap::new(),
            published: None,
            unavailable: None,
            next_retry: Instant::now(),
        };
        loop {
            worker.scan();
            match commands.recv_timeout(POLL) {
                Ok(MidiControl::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(MidiControl::Settings(settings)) => worker.settings = settings,
                Ok(MidiControl::Rescan) => worker.failed.clear(),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    impl Worker {
        fn scan(&mut self) {
            if !self.settings.enabled {
                self.close_all();
                self.publish(Vec::new());
                return;
            }
            if self.scanner.is_none() {
                if Instant::now() < self.next_retry {
                    return;
                }
                match MidiInput::new("strobe-scan") {
                    Ok(scanner) => {
                        self.scanner = Some(scanner);
                        self.unavailable = None;
                    }
                    Err(error) => {
                        self.next_retry = Instant::now() + RETRY;
                        let message = format!("no MIDI service: {error}");
                        if self.unavailable.as_deref() != Some(message.as_str()) {
                            self.tx.status(Status::MidiUnavailable(message.clone()));
                            self.unavailable = Some(message);
                        }
                        return;
                    }
                }
            }
            let present = self.list();
            let disabled: HashSet<&str> = self
                .settings
                .disabled_devices
                .iter()
                .map(String::as_str)
                .collect();

            let listed: Vec<(&str, &str)> = present
                .iter()
                .map(|p| (p.key.as_str(), p.id.as_str()))
                .collect();
            let gone = stale(
                self.open
                    .iter()
                    .map(|(key, (id, _))| (key.as_str(), id.as_str())),
                &listed,
            );
            for key in gone {
                self.open.remove(&key);
                log::info!("MIDI device lost: {key}");
                self.tx.status(Status::DeviceLost(key));
            }
            // A failure is for one port: the same key on another port (a
            // replug, a twin unplugged) is tried again.
            self.failed
                .retain(|key, (id, _)| listed.iter().any(|(k, i)| k == key && i == id));

            for device in &present {
                if disabled.contains(device.key.as_str()) {
                    self.open.remove(&device.key);
                    continue;
                }
                if self.open.contains_key(&device.key) || self.failed.contains_key(&device.key) {
                    continue;
                }
                match self.connect(device) {
                    Ok(connection) => {
                        log::info!("MIDI device opened: {} ({})", device.key, device.name);
                        self.open
                            .insert(device.key.clone(), (device.id.clone(), connection));
                        self.tx.status(Status::DeviceFound(device.key.clone()));
                    }
                    Err(error) => {
                        log::warn!("MIDI device {} could not be opened: {error}", device.name);
                        self.failed
                            .insert(device.key.clone(), (device.id.clone(), error));
                    }
                }
            }

            let infos = present
                .iter()
                .map(|p| DeviceInfo {
                    key: p.key.clone(),
                    name: p.name.clone(),
                    connected: self.open.contains_key(&p.key),
                    disabled: disabled.contains(p.key.as_str()),
                    error: self.failed.get(&p.key).map(|(_, why)| why.clone()),
                })
                .collect();
            self.publish(infos);
        }

        /// The ports now, with unique keys.
        fn list(&mut self) -> Vec<Present> {
            let Some(scanner) = self.scanner.as_ref() else {
                return Vec::new();
            };
            let ports = scanner.ports();
            let names: Vec<String> = ports
                .iter()
                .map(|port| scanner.port_name(port).unwrap_or_default())
                .collect();
            let keys = device_keys(names.iter().map(String::as_str));
            ports
                .into_iter()
                .zip(names)
                .zip(keys)
                .map(|((port, name), key)| Present {
                    key,
                    name,
                    id: port.id(),
                    port,
                })
                .collect()
        }

        fn connect(&self, device: &Present) -> Result<MidiInputConnection<()>, String> {
            let input = MidiInput::new("strobe-in").map_err(|e| e.to_string())?;
            let tx = self.tx.clone();
            let key: Arc<str> = Arc::from(device.key.as_str());
            input
                .connect(
                    &device.port,
                    "strobe-input",
                    move |_timestamp, bytes, ()| {
                        if let Some(message) = parse(bytes) {
                            tx.send(Event::Input(InputEvent::Midi {
                                device: Arc::clone(&key),
                                message,
                            }));
                        }
                    },
                    (),
                )
                .map_err(|e| e.to_string())
        }

        fn close_all(&mut self) {
            for (key, _) in self.open.drain() {
                self.tx.status(Status::DeviceLost(key));
            }
            self.failed.clear();
        }

        fn publish(&mut self, infos: Vec<DeviceInfo>) {
            if self.published.as_ref() != Some(&infos) {
                self.published = Some(infos.clone());
                self.tx.status(Status::Devices(infos));
            }
        }
    }

    /// The keys of `open` connections, as (key, port id), that must be
    /// closed because of what is `listed` now: the key is gone, or a
    /// different port has it. A key names a device by its port name
    /// ([`crate::message::device_keys`]), so it moves: of two devices with one
    /// name, the second is `#2` until the first is unplugged and then has
    /// the plain key, and a replug the one-second poll did not see comes back
    /// on another port. Keeping the connection by key alone left the dead
    /// one in place and closed the live one.
    fn stale<'a>(
        open: impl IntoIterator<Item = (&'a str, &'a str)>,
        listed: &[(&str, &str)],
    ) -> Vec<String> {
        open.into_iter()
            .filter(|(key, id)| !listed.iter().any(|(k, i)| k == key && i == id))
            .map(|(key, _)| key.to_string())
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::stale;

        fn sorted(mut keys: Vec<String>) -> Vec<String> {
            keys.sort();
            keys
        }

        #[test]
        fn unplugging_one_of_two_identical_devices_keeps_the_other_working() {
            // Two nanoKONTROL2s on ALSA: the first keeps the plain key, the
            // second is "#2". Unplug the first: the second now has the
            // plain key, on its own port.
            let open = [("nanoKONTROL2", "20:0"), ("nanoKONTROL2 #2", "24:0")];
            let listed = [("nanoKONTROL2", "24:0")];
            assert_eq!(
                sorted(stale(open, &listed)),
                vec!["nanoKONTROL2".to_string(), "nanoKONTROL2 #2".to_string()],
                "the dead first connection and the second's old key both close, \
                 so the device still plugged in is opened again under its new key"
            );
        }

        #[test]
        fn a_device_back_on_another_port_is_reopened() {
            assert_eq!(
                stale([("Launchpad", "20:0")], &[("Launchpad", "28:0")]),
                vec!["Launchpad".to_string()],
                "a replug the one-second poll did not see"
            );
        }

        #[test]
        fn a_device_on_the_same_port_is_left_alone_and_a_vanished_one_closes() {
            assert!(
                stale([("Launchpad", "20:0")], &[("Launchpad", "20:0")]).is_empty(),
                "nothing changed"
            );
            assert_eq!(
                stale([("Launchpad", "20:0")], &[]),
                vec!["Launchpad".to_string()],
                "unplugged"
            );
        }
    }
}

/// Runs the MIDI worker until told to shut down.
#[cfg(feature = "midi")]
pub(crate) fn run(commands: &Receiver<MidiControl>, tx: &Tx, settings: MidiSettings) {
    device::run(commands, tx, settings);
}

/// Without the `midi` feature: say so once, then wait to be stopped.
#[cfg(not(feature = "midi"))]
pub(crate) fn run(commands: &Receiver<MidiControl>, tx: &Tx, _settings: MidiSettings) {
    tx.status(crate::event::Status::MidiUnavailable(
        "this build has no MIDI support (the `midi` feature is off)".to_string(),
    ));
    while let Ok(command) = commands.recv() {
        if let MidiControl::Shutdown = command {
            return;
        }
    }
}
