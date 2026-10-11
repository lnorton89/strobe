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
//!   action.
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
    use crate::message::{device_key, parse};
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
        port: MidiInputPort,
    }

    struct Worker {
        tx: Tx,
        settings: MidiSettings,
        scanner: Option<MidiInput>,
        open: HashMap<String, MidiInputConnection<()>>,
        failed: HashMap<String, String>,
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

            let gone: Vec<String> = self
                .open
                .keys()
                .filter(|key| !present.iter().any(|p| &p.key == *key))
                .cloned()
                .collect();
            for key in gone {
                self.open.remove(&key);
                log::info!("MIDI device lost: {key}");
                self.tx.status(Status::DeviceLost(key));
            }
            self.failed
                .retain(|key, _| present.iter().any(|p| &p.key == key));

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
                        self.open.insert(device.key.clone(), connection);
                        self.tx.status(Status::DeviceFound(device.key.clone()));
                    }
                    Err(error) => {
                        log::warn!("MIDI device {} could not be opened: {error}", device.name);
                        self.failed.insert(device.key.clone(), error);
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
                    error: self.failed.get(&p.key).cloned(),
                })
                .collect();
            self.publish(infos);
        }

        /// The ports now, with unique keys.
        fn list(&mut self) -> Vec<Present> {
            let Some(scanner) = self.scanner.as_ref() else {
                return Vec::new();
            };
            let mut seen: HashMap<String, usize> = HashMap::new();
            let mut present = Vec::new();
            for port in scanner.ports() {
                let name = scanner.port_name(&port).unwrap_or_default();
                let base = device_key(&name);
                let count = seen.entry(base.clone()).or_insert(0);
                *count += 1;
                let key = if *count == 1 {
                    base
                } else {
                    format!("{base} #{count}")
                };
                present.push(Present { key, name, port });
            }
            present
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
