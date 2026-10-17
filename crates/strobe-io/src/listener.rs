// ============================================================================
// Module:       strobe_io::listener
// Description:  The OSC worker thread: a UDP socket, decoded datagrams, and
//               a listener that can be moved or stopped without a restart.
//
// Dependencies: std::net; crate::{hub, osc, event}
// ============================================================================

//! The OSC listener thread (D171, D172).
//!
//! One thread for the life of the hub. It waits for [`OscControl`] commands
//! when it has no socket, and otherwise sits in `recv_from` with a 50 ms
//! timeout so a command (rebind, stop, shutdown) is noticed promptly without
//! the app ever joining on a blocked read.
//!
//! The read buffer is allocated once, at the largest UDP payload. A datagram
//! is decoded by [`crate::osc::decode`] (which is bounded, see there); one
//! that does not decode is dropped silently, because anything on the network
//! can send garbage and logging each one would be a way to fill a disk.
//!
//! A socket error other than a timeout or a reset (the network going down,
//! an interface disappearing) closes the socket and is reported. The
//! listener then binds the same address again after 1, 2, 4 and 8 seconds and
//! every 15 after that ([`retry_delay`]), for as long as it is wanted: a
//! laptop that lost its Wi-Fi mid-show starts listening again when the
//! network returns, without the operator finding the MAP pane. A failure
//! that repeats is reported once, not once per try, and the wait starts over
//! after the socket has worked. A command (switch off, move, shutdown)
//! interrupts the wait at once (D374). The socket itself is behind
//! [`Transport`] so the tests can script a network that fails.

use crate::event::{Event, InputEvent, Status};
use crate::hub::{OscControl, Tx};
use crate::osc;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

/// How long a read waits before the thread looks for a command.
const WAKE: Duration = Duration::from_millis(50);

/// The largest UDP payload (65535 minus the IP and UDP headers).
const MAX_DATAGRAM: usize = 65_507;

/// Where the listener gets its sockets. The real one is UDP; the tests'
/// scripts errors a network only gives at the wrong moment.
pub(crate) trait Transport {
    /// What a bound listener reads from.
    type Socket: Datagrams;

    /// Binds `addr`, or says in words an operator can read why not.
    fn bind(&mut self, addr: SocketAddr) -> Result<Self::Socket, String>;
}

/// One bound listener.
pub(crate) trait Datagrams {
    /// Reads one datagram into `buffer` and returns its length. A read that
    /// waited [`WAKE`] for nothing is a `WouldBlock` or `TimedOut` error.
    fn receive(&mut self, buffer: &mut [u8]) -> std::io::Result<usize>;

    /// The address actually bound (the port may have been 0).
    fn local(&self) -> Option<SocketAddr>;
}

/// The real thing.
struct Udp;

impl Transport for Udp {
    type Socket = UdpSocket;

    fn bind(&mut self, addr: SocketAddr) -> Result<UdpSocket, String> {
        let socket = UdpSocket::bind(addr).map_err(|error| {
            format!(
                "cannot listen on {addr}: {error} (is another program using port {}?)",
                addr.port()
            )
        })?;
        socket
            .set_read_timeout(Some(WAKE))
            .map_err(|error| format!("cannot set the OSC read timeout: {error}"))?;
        Ok(socket)
    }
}

impl Datagrams for UdpSocket {
    fn receive(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.recv_from(buffer).map(|(length, _from)| length)
    }

    fn local(&self) -> Option<SocketAddr> {
        self.local_addr().ok()
    }
}

/// Runs the OSC worker until told to shut down.
pub(crate) fn run(commands: &Receiver<OscControl>, tx: &Tx) {
    run_with(&mut Udp, retry_delay, commands, tx);
}

/// How long to wait before the `attempt`th try (from 0) at getting a socket
/// back after it failed: 1, 2, 4 and 8 seconds, then every 15.
fn retry_delay(attempt: u32) -> Duration {
    const STEADY: Duration = Duration::from_secs(15);
    // 2^attempt seconds; past 3 it is over STEADY anyway, and capping the
    // shift keeps a very long outage from overflowing it.
    Duration::from_secs(1_u64 << attempt.min(4)).min(STEADY)
}

/// What the listener knows about an outage: the socket failed, and it is
/// trying to get it back.
#[derive(Default)]
struct Outage {
    /// Failures since the socket last read anything, so the next wait.
    failures: u32,
    /// When to try again; `None` while there is nothing to retry.
    retry_at: Option<Instant>,
    /// The last failure that was reported, so a failure that repeats is said
    /// once.
    reported: Option<String>,
}

impl Outage {
    /// The socket (or its bind) failed with `why`: say so unless it is what
    /// was last said, and set the time of the next try.
    fn failed(&mut self, why: String, delay: &impl Fn(u32) -> Duration, tx: &Tx) {
        self.retry_at = Some(Instant::now() + delay(self.failures));
        self.failures = self.failures.saturating_add(1);
        if self.reported.as_deref() != Some(why.as_str()) {
            tx.status(Status::OscError(why.clone()));
            self.reported = Some(why);
        }
    }

    /// The socket read without error (a quiet wait counts): the outage, if
    /// there was one, is over, and the next one starts its backoff afresh.
    fn recovered(&mut self) {
        self.failures = 0;
        self.reported = None;
    }

    /// Nothing is wanted back: forget it all.
    fn clear(&mut self) {
        self.recovered();
        self.retry_at = None;
    }
}

fn run_with<N: Transport>(
    net: &mut N,
    delay: impl Fn(u32) -> Duration,
    commands: &Receiver<OscControl>,
    tx: &Tx,
) {
    let mut socket: Option<N::Socket> = None;
    // Where the listener should be listening, when the socket is lost.
    let mut wanted: Option<SocketAddr> = None;
    let mut outage = Outage::default();
    let mut buffer = vec![0_u8; MAX_DATAGRAM];
    loop {
        // Commands first. With no socket there is nothing else to wait for,
        // so block on them, until the time to try for a socket again.
        let command = if socket.is_some() {
            match commands.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else if let Some(at) = outage.retry_at {
            match commands.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        match command {
            Some(OscControl::Shutdown) => return,
            Some(OscControl::Bind(addr)) => {
                // Close the old socket first so a move to the same port can
                // bind. Whatever outage there was belongs to the old request.
                socket = None;
                outage.clear();
                wanted = addr;
                match addr {
                    Some(addr) => socket = bind(net, addr, tx),
                    None => tx.status(Status::OscStopped),
                }
                continue;
            }
            None => {}
        }
        let Some(open) = socket.as_mut() else {
            // The wait for a retry ran out: try for the socket again.
            if let (Some(addr), Some(at)) = (wanted, outage.retry_at)
                && Instant::now() >= at
            {
                match net.bind(addr) {
                    Ok(bound) => {
                        tx.status(Status::OscListening(bound.local().unwrap_or(addr)));
                        outage.retry_at = None;
                        socket = Some(bound);
                    }
                    Err(why) => outage.failed(why, &delay, tx),
                }
            }
            continue;
        };
        match open.receive(&mut buffer) {
            Ok(length) => {
                outage.recovered();
                let Some(datagram) = buffer.get(..length) else {
                    continue;
                };
                if let Ok(messages) = osc::decode(datagram) {
                    for message in messages {
                        tx.send(Event::Input(InputEvent::Osc(message)));
                    }
                }
            }
            Err(error) => {
                let kind = error.kind();
                if matches!(kind, ErrorKind::WouldBlock | ErrorKind::TimedOut) {
                    // A quiet wait: the socket works.
                    outage.recovered();
                } else if !matches!(kind, ErrorKind::Interrupted | ErrorKind::ConnectionReset) {
                    // Anything else (the network going down, an interface
                    // disappearing) loses the socket. Close it and keep
                    // trying to get one back, with a growing wait, until it
                    // is switched off (D374).
                    //
                    // Not these two: Windows reports an earlier ICMP "port
                    // unreachable" as a reset on the next read, and a signal
                    // can cut a read short. The socket is fine.
                    socket = None;
                    outage.failed(format!("the OSC socket failed: {error}"), &delay, tx);
                }
            }
        }
    }
}

fn bind<N: Transport>(net: &mut N, addr: SocketAddr, tx: &Tx) -> Option<N::Socket> {
    match net.bind(addr) {
        Ok(socket) => {
            tx.status(Status::OscListening(socket.local().unwrap_or(addr)));
            Some(socket)
        }
        Err(why) => {
            tx.status(Status::OscError(why));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use std::collections::VecDeque;
    use std::io;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::thread::JoinHandle;
    use std::time::Instant;

    /// What a scripted socket does on its next read.
    enum Read {
        Datagram(&'static str),
        Fail(io::ErrorKind, &'static str),
    }

    /// What the next `bind` does.
    enum Bind {
        Refuse(&'static str),
        Open(Vec<Read>),
    }

    /// The network, as a script: each bind takes the next entry, and once
    /// they run out the network stays down.
    struct Script {
        binds: VecDeque<Bind>,
        attempts: Arc<Mutex<Vec<(SocketAddr, Instant)>>>,
    }

    struct Scripted {
        reads: VecDeque<Read>,
        addr: SocketAddr,
    }

    impl Transport for Script {
        type Socket = Scripted;

        fn bind(&mut self, addr: SocketAddr) -> Result<Scripted, String> {
            self.attempts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((addr, Instant::now()));
            match self.binds.pop_front() {
                Some(Bind::Open(reads)) => Ok(Scripted {
                    reads: reads.into(),
                    addr,
                }),
                Some(Bind::Refuse(why)) => Err(why.to_string()),
                None => Err("the network is still down".to_string()),
            }
        }
    }

    impl Datagrams for Scripted {
        fn receive(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            match self.reads.pop_front() {
                Some(Read::Datagram(address)) => {
                    let bytes = osc::encode(address, &[])
                        .ok_or_else(|| io::Error::other("cannot encode the test message"))?;
                    let target = buffer
                        .get_mut(..bytes.len())
                        .ok_or_else(|| io::Error::other("buffer too small"))?;
                    target.copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Some(Read::Fail(kind, why)) => Err(io::Error::new(kind, why)),
                None => {
                    // Like the real socket's read timeout.
                    std::thread::sleep(Duration::from_millis(2));
                    Err(io::ErrorKind::WouldBlock.into())
                }
            }
        }

        fn local(&self) -> Option<SocketAddr> {
            Some(self.addr)
        }
    }

    /// A listener thread on a scripted network.
    struct Rig {
        control: Sender<OscControl>,
        events: Receiver<Event>,
        attempts: Arc<Mutex<Vec<(SocketAddr, Instant)>>>,
        delays: Arc<Mutex<Vec<u32>>>,
        done: Receiver<()>,
        thread: Option<JoinHandle<()>>,
    }

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    impl Rig {
        fn start(binds: Vec<Bind>, delay: fn(u32) -> Duration) -> Result<Self> {
            let (control, commands) = channel();
            let (tx, events) = Tx::detached(256);
            let attempts = Arc::new(Mutex::new(Vec::new()));
            let delays = Arc::new(Mutex::new(Vec::new()));
            let mut script = Script {
                binds: binds.into(),
                attempts: Arc::clone(&attempts),
            };
            let (finished, done) = channel();
            let recorded = Arc::clone(&delays);
            let thread = std::thread::Builder::new()
                .name("test-osc".into())
                .spawn(move || {
                    let record = |attempt: u32| {
                        recorded
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(attempt);
                        delay(attempt)
                    };
                    run_with(&mut script, record, &commands, &tx);
                    let _ = finished.send(());
                })?;
            Ok(Self {
                control,
                events,
                attempts,
                delays,
                done,
                thread: Some(thread),
            })
        }

        fn bind_attempts(&self) -> Vec<(SocketAddr, Instant)> {
            self.attempts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        fn delays(&self) -> Vec<u32> {
            self.delays
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        /// Events until `stop` accepts one (it is included) or five seconds
        /// pass.
        fn until(&self, mut stop: impl FnMut(&Event) -> bool) -> Vec<Event> {
            let limit = Instant::now() + Duration::from_secs(5);
            let mut seen = Vec::new();
            while let Some(left) = limit.checked_duration_since(Instant::now()) {
                let Ok(event) = self.events.recv_timeout(left) else {
                    break;
                };
                let last = stop(&event);
                seen.push(event);
                if last {
                    break;
                }
            }
            seen
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = self.control.send(OscControl::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn is_error(event: &Event) -> bool {
        matches!(event, Event::Status(Status::OscError(_)))
    }

    fn is_message(event: &Event, address: &str) -> bool {
        matches!(event, Event::Input(InputEvent::Osc(m)) if m.address == address)
    }

    fn quick(attempt: u32) -> Duration {
        Duration::from_millis(20 * (u64::from(attempt) + 1))
    }

    fn slow(_attempt: u32) -> Duration {
        Duration::from_secs(60)
    }

    fn down() -> Read {
        Read::Fail(io::ErrorKind::Other, "the network is down")
    }

    #[test]
    fn the_retry_schedule_backs_off_and_then_settles() {
        let seconds: Vec<u64> = (0..8).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(seconds, vec![1, 2, 4, 8, 15, 15, 15, 15], "1, 2, 4, 8, 15");
        assert_eq!(
            retry_delay(u32::MAX),
            Duration::from_secs(15),
            "a long outage does not overflow the schedule"
        );
    }

    #[test]
    fn a_socket_error_is_retried_with_backoff_and_each_failure_is_reported_once() -> Result<()> {
        let rig = Rig::start(
            vec![
                Bind::Open(vec![Read::Datagram("/a"), down()]),
                Bind::Refuse("no route to the network"),
                Bind::Refuse("no route to the network"),
                Bind::Open(vec![Read::Datagram("/b")]),
            ],
            quick,
        )?;
        rig.control.send(OscControl::Bind(Some(addr(9000))))?;
        let seen = rig.until(|e| is_message(e, "/b"));
        assert!(
            seen.last().is_some_and(|e| is_message(e, "/b")),
            "the listener never came back after the socket failed: {seen:?}"
        );
        let statuses: Vec<&Event> = seen
            .iter()
            .filter(|e| matches!(e, Event::Status(_)))
            .collect();
        assert_eq!(
            statuses,
            vec![
                &Event::Status(Status::OscListening(addr(9000))),
                &Event::Status(Status::OscError(
                    "the OSC socket failed: the network is down".to_string()
                )),
                &Event::Status(Status::OscError("no route to the network".to_string())),
                &Event::Status(Status::OscListening(addr(9000))),
            ],
            "two failures, the second refused twice but said once"
        );
        let attempts = rig.bind_attempts();
        assert_eq!(
            attempts.len(),
            4,
            "one bind, then three tries: {attempts:?}"
        );
        for (n, pair) in attempts.windows(2).enumerate() {
            let [earlier, later] = pair else {
                return Err(anyhow!("windows(2) gave {pair:?}"));
            };
            let waited = later.1.duration_since(earlier.1);
            let n = u32::try_from(n)?;
            assert!(
                waited >= quick(n),
                "try {n} came after {waited:?}, before the {:?} backoff",
                quick(n)
            );
        }
        assert_eq!(rig.delays(), vec![0, 1, 2], "the schedule advances per try");
        Ok(())
    }

    #[test]
    fn the_backoff_starts_over_once_the_socket_has_worked() -> Result<()> {
        let rig = Rig::start(
            vec![
                // Fails, comes back, fails again before it ever reads: the
                // outage goes on, so the wait grows.
                Bind::Open(vec![down()]),
                Bind::Open(vec![down()]),
                // Comes back and works (a timed-out read is a working one),
                // then fails: a new outage, so the wait starts over.
                Bind::Open(vec![Read::Fail(io::ErrorKind::WouldBlock, "idle"), down()]),
                Bind::Open(vec![Read::Datagram("/end")]),
            ],
            quick,
        )?;
        rig.control.send(OscControl::Bind(Some(addr(9000))))?;
        let seen = rig.until(|e| is_message(e, "/end"));
        assert!(
            seen.last().is_some_and(|e| is_message(e, "/end")),
            "never settled: {seen:?}"
        );
        assert_eq!(
            rig.delays(),
            vec![0, 1, 0],
            "growing while it keeps failing, and back to the start after a good read"
        );
        Ok(())
    }

    #[test]
    fn switching_osc_off_while_it_waits_to_retry_stops_it_for_good() -> Result<()> {
        let rig = Rig::start(vec![Bind::Open(vec![down()])], quick)?;
        rig.control.send(OscControl::Bind(Some(addr(9000))))?;
        let limit = Instant::now() + Duration::from_secs(5);
        while rig.bind_attempts().len() < 3 && Instant::now() < limit {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            rig.bind_attempts().len() >= 3,
            "it never tried again: {:?}",
            rig.bind_attempts()
        );
        rig.control.send(OscControl::Bind(None))?;
        let seen = rig.until(|e| matches!(e, Event::Status(Status::OscStopped)));
        assert!(
            seen.last()
                .is_some_and(|e| matches!(e, Event::Status(Status::OscStopped))),
            "no stop report: {seen:?}"
        );
        let tried = rig.bind_attempts().len();
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(
            rig.bind_attempts().len(),
            tried,
            "it kept trying after it was switched off"
        );
        Ok(())
    }

    #[test]
    fn a_new_bind_command_during_the_wait_is_acted_on_at_once() -> Result<()> {
        let rig = Rig::start(vec![Bind::Open(vec![down()]), Bind::Open(Vec::new())], slow)?;
        rig.control.send(OscControl::Bind(Some(addr(9000))))?;
        let seen = rig.until(is_error);
        assert!(seen.last().is_some_and(is_error), "no failure: {seen:?}");
        rig.control.send(OscControl::Bind(Some(addr(9001))))?;
        let seen = rig.until(|e| matches!(e, Event::Status(Status::OscListening(_))));
        assert_eq!(
            seen.last(),
            Some(&Event::Status(Status::OscListening(addr(9001)))),
            "rebinding to the new address did not wait out the old backoff: {seen:?}"
        );
        Ok(())
    }

    #[test]
    fn shutdown_does_not_wait_for_a_long_backoff() -> Result<()> {
        let mut rig = Rig::start(vec![Bind::Open(vec![down()])], slow)?;
        rig.control.send(OscControl::Bind(Some(addr(9000))))?;
        let seen = rig.until(is_error);
        assert!(seen.last().is_some_and(is_error), "no failure: {seen:?}");
        rig.control.send(OscControl::Shutdown)?;
        rig.done
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| anyhow!("the listener was still sleeping off its backoff"))?;
        if let Some(thread) = rig.thread.take() {
            let _ = thread.join();
        }
        Ok(())
    }
}
