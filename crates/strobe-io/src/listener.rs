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
//! can send garbage and logging each one would be a way to fill a disk. A
//! socket error other than a timeout ends the listener with a status the
//! operator can see, and a re-bind (toggling OSC off and on) starts it again.

use crate::event::{Event, InputEvent, Status};
use crate::hub::{OscControl, Tx};
use crate::osc;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

/// How long a read waits before the thread looks for a command.
const WAKE: Duration = Duration::from_millis(50);

/// The largest UDP payload (65535 minus the IP and UDP headers).
const MAX_DATAGRAM: usize = 65_507;

/// Runs the OSC worker until told to shut down.
pub(crate) fn run(commands: &Receiver<OscControl>, tx: &Tx) {
    let mut socket: Option<UdpSocket> = None;
    let mut buffer = vec![0_u8; MAX_DATAGRAM];
    loop {
        // Commands first. With no socket there is nothing else to wait for,
        // so block on them.
        let command = if socket.is_some() {
            match commands.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
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
                // bind.
                socket = None;
                match addr {
                    Some(addr) => socket = bind(addr, tx),
                    None => tx.status(Status::OscStopped),
                }
                continue;
            }
            None => {}
        }
        let Some(open) = socket.as_ref() else {
            continue;
        };
        match open.recv_from(&mut buffer) {
            Ok((length, _from)) => {
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
                // Windows reports an earlier ICMP "port unreachable" as a
                // reset on the next read; like a timeout, it is not a reason
                // to stop.
                let benign = matches!(
                    error.kind(),
                    ErrorKind::WouldBlock
                        | ErrorKind::TimedOut
                        | ErrorKind::Interrupted
                        | ErrorKind::ConnectionReset
                );
                if !benign {
                    tx.status(Status::OscError(format!("the OSC socket failed: {error}")));
                    socket = None;
                }
            }
        }
    }
}

fn bind(addr: SocketAddr, tx: &Tx) -> Option<UdpSocket> {
    let socket = match UdpSocket::bind(addr) {
        Ok(socket) => socket,
        Err(error) => {
            tx.status(Status::OscError(format!(
                "cannot listen on {addr}: {error} (is another program using port {}?)",
                addr.port()
            )));
            return None;
        }
    };
    if let Err(error) = socket.set_read_timeout(Some(WAKE)) {
        tx.status(Status::OscError(format!(
            "cannot set the OSC read timeout: {error}"
        )));
        return None;
    }
    match socket.local_addr() {
        Ok(local) => tx.status(Status::OscListening(local)),
        Err(_) => tx.status(Status::OscListening(addr)),
    }
    Some(socket)
}
