// ============================================================================
// Module:       strobe_io::midi_out
// Description:  The MIDI feedback sender: a thread that owns the output
//               ports, opens one per device key on first use, and sends
//               what the frame queued without the frame ever waiting.
//
// Dependencies: midir (feature `midi`); crate::{message, midi_feedback}
// ============================================================================

//! Sending MIDI feedback (backlog P-05, D257).
//!
//! The frame calls [`Sender::send`], which only `try_send`s into a bounded
//! queue ([`QUEUE`]); a full queue drops the message and counts it
//! ([`Sender::dropped`]). [`crate::midi_feedback::MidiDiffer`] sends a value
//! again when it next changes, so a dropped one is never stuck.
//!
//! The thread keys output ports exactly as the input worker keys input
//! ports ([`crate::message::device_keys`]), so a mapping's device names the
//! controller's output too: most controllers show the same name both ways.
//! A port is opened the first time something is sent to its key, the port
//! list is read again at most every [`RESCAN`], and a port whose send fails
//! (unplugged) is closed and found again later. A message for a key with
//! no output port is dropped quietly: plenty of controllers have none.
//!
//! Without the `midi` feature the sender starts and drops everything.

use crate::midi_feedback::MidiOut;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

/// Messages that can wait for the thread.
const QUEUE: usize = 256;

/// The least time between two readings of the output port list.
const RESCAN: Duration = Duration::from_secs(2);

enum Job {
    Send(MidiOut),
    Stop,
}

/// The sending thread and its queue.
#[derive(Debug)]
pub struct Sender {
    jobs: SyncSender<Job>,
    dropped: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Send(out) => write!(f, "Send({out:?})"),
            Self::Stop => f.write_str("Stop"),
        }
    }
}

impl Sender {
    /// Starts the thread.
    ///
    /// # Errors
    ///
    /// When the OS will not start a thread.
    pub fn start() -> std::io::Result<Self> {
        let (jobs, queue) = sync_channel(QUEUE);
        let thread = std::thread::Builder::new()
            .name("strobe-midi-out".to_string())
            .spawn(move || run(&queue))?;
        Ok(Self {
            jobs,
            dropped: Arc::new(AtomicU64::new(0)),
            thread: Some(thread),
        })
    }

    /// Queues every message in `out` (which it empties) without waiting.
    pub fn send(&self, out: &mut Vec<MidiOut>) {
        for message in out.drain(..) {
            match self.jobs.try_send(Job::Send(message)) {
                Ok(()) => {}
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Messages dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // A full queue still ends: the thread exits when the channel
        // disconnects, which dropping `jobs` below does.
        let _ = self.jobs.try_send(Job::Stop);
        if let Some(thread) = self.thread.take() {
            let (jobs, _) = sync_channel(0);
            drop(std::mem::replace(&mut self.jobs, jobs));
            // Waits for the sender; a panic in it already aborted the process
            // in release builds.
            let _ = thread.join();
        }
    }
}

#[cfg(feature = "midi")]
fn run(queue: &Receiver<Job>) {
    use crate::message::device_keys;
    use midir::{MidiOutput, MidiOutputConnection};
    use std::collections::HashMap;
    use std::time::Instant;

    let mut open: HashMap<String, MidiOutputConnection> = HashMap::new();
    let mut last_scan: Option<Instant> = None;
    let mut known: Vec<String> = Vec::new();
    while let Ok(job) = queue.recv() {
        let out = match job {
            Job::Send(out) => out,
            Job::Stop => return,
        };
        let key = out.device.as_ref();
        if !open.contains_key(key) {
            let due = last_scan.is_none_or(|at| at.elapsed() >= RESCAN);
            if !due && !known.iter().any(|k| k == key) {
                continue;
            }
            if due {
                last_scan = Some(Instant::now());
            }
            let Ok(scanner) = MidiOutput::new("strobe-out-scan") else {
                continue;
            };
            let ports = scanner.ports();
            let names: Vec<String> = ports
                .iter()
                .map(|port| scanner.port_name(port).unwrap_or_default())
                .collect();
            known = device_keys(names.iter().map(String::as_str));
            let Some(port) = known
                .iter()
                .position(|k| k == key)
                .and_then(|i| ports.get(i))
            else {
                continue;
            };
            match scanner.connect(port, "strobe-feedback") {
                Ok(connection) => {
                    log::info!("MIDI feedback: sending to {key}");
                    open.insert(key.to_string(), connection);
                }
                Err(error) => {
                    log::warn!("MIDI feedback: {key} would not open: {error}");
                    // Not tried again before the next scan.
                    known.retain(|k| k != key);
                    continue;
                }
            }
        }
        if let Some(connection) = open.get_mut(key)
            && let Err(error) = connection.send(&out.bytes)
        {
            log::warn!("MIDI feedback: {key} stopped taking messages ({error}); closed");
            open.remove(key);
        }
    }
}

#[cfg(not(feature = "midi"))]
fn run(queue: &Receiver<Job>) {
    while let Ok(job) = queue.recv() {
        if let Job::Stop = job {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_device_with_no_output_is_harmless_and_dropping_the_sender_ends_the_thread() -> Result<()> {
        let sender = Sender::start()?;
        let mut out: Vec<MidiOut> = (0..10)
            .map(|i| MidiOut {
                device: Arc::from("no such device"),
                bytes: [0xB0, i, 0],
            })
            .collect();
        sender.send(&mut out);
        assert!(out.is_empty(), "the batch was taken");
        assert_eq!(sender.dropped(), 0, "ten fit in the queue");
        // Nothing can be opened for that key, so nothing is waited on.
        drop(sender);
        Ok(())
    }
}
