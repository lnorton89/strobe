// ============================================================================
// Module:       strobe_remote::client
// Description:  Native desktop WebSocket transport with strict handshake,
//               bounded frames, random masking and finite socket timeouts.
//
// Dependencies: std::net, std::io; crate::{server::ws, protocol types}
// ============================================================================

//! Blocking transport for a controller worker; never call it on the UI thread.
//! Connects to an explicit private IP and port, without DNS or redirects.
//! Pairing and bearer tokens travel in message bodies, never URLs or logs.

use crate::server::ws::{self, Message, Reader};
use crate::{ClientMessage, ServerMessage};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// The `Host` header [`Connection::connect`] sends for `address`: the
/// address and port exactly as dialled. Trusted pairing (D288) binds its
/// proof to this string, so a controller must use this function rather than
/// rebuild it.
pub fn host_header(address: SocketAddr) -> String {
    address.to_string()
}

/// One native controller connection. No automatic reconnection or replay:
/// reconnect requires a new pairing and no old click is retried.
pub struct Connection {
    stream: TcpStream,
    reader: Reader,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Connection {
    /// Connect with a five-second handshake deadline, checking the server's
    /// upgrade status, tokens and computed accept key.
    pub fn connect(address: SocketAddr) -> io::Result<Self> {
        if matches!(
            crate::server::net::scope(address.ip()),
            crate::server::net::Scope::Public | crate::server::net::Scope::Unspecified
        ) || address.port() == 0
        {
            return Err(invalid("Use a private IP address and a nonzero port"));
        }
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_millis(50)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| invalid("Cannot generate a WebSocket key"))?;
        let key = ws::base64(&random);
        let host = host_header(address);
        let request = format!(
            "GET /ws HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\nUser-Agent: Strobe Desktop\r\n\r\n"
        );
        stream.write_all(request.as_bytes())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() >= 8192 || Instant::now() >= deadline {
                return Err(invalid("Desktop handshake exceeded its limit"));
            }
            let mut byte = [0_u8; 1];
            match stream.read(&mut byte) {
                Ok(0) => return Err(invalid("Host closed during handshake")),
                Ok(_) => head.push(byte[0]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        let head = String::from_utf8(head).map_err(|_| invalid("Invalid handshake text"))?;
        let expected = ws::accept_key(&key);
        let header = |name: &str| {
            head.lines()
                .skip(1)
                .filter_map(|line| line.split_once(':'))
                .find(|(field, _)| field.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.trim())
        };
        let token = |name: &str, wanted: &str| {
            header(name).is_some_and(|value| {
                value
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case(wanted))
            })
        };
        if !head.starts_with("HTTP/1.1 101 ")
            || header("sec-websocket-accept") != Some(expected.as_str())
            || !token("upgrade", "websocket")
            || !token("connection", "upgrade")
        {
            return Err(invalid("Host refused the desktop WebSocket upgrade"));
        }
        Ok(Self {
            stream,
            reader: Reader::server(crate::desktop::MAX_FRAME),
        })
    }

    /// A cloned socket used only to interrupt this worker during shutdown.
    pub fn interrupter(&self) -> io::Result<TcpStream> {
        self.stream.try_clone()
    }

    /// Send one bounded typed message with a fresh random mask.
    pub fn send(&mut self, message: &ClientMessage) -> io::Result<()> {
        let text =
            serde_json::to_string(message).map_err(|_| invalid("Cannot encode desktop message"))?;
        if crate::decode_client(&text).is_err() {
            return Err(invalid("Invalid desktop message"));
        }
        self.write_masked(1, text.as_bytes())
    }

    fn write_masked(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let mut mask = [0_u8; 4];
        getrandom::fill(&mut mask).map_err(|_| invalid("Cannot generate a frame mask"))?;
        let mut frame = vec![0x80 | opcode];
        match payload.len() {
            len @ 0..=125 => frame.push(0x80 | len as u8),
            len @ 126..=65535 => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(len as u16).to_be_bytes());
            }
            len => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(len as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .zip(mask.iter().cycle())
                .map(|(byte, key)| byte ^ key),
        );
        self.stream.write_all(&frame)
    }

    /// Receive a complete message or `None` after a short read timeout.
    pub fn receive(&mut self) -> io::Result<Option<ServerMessage>> {
        match self
            .reader
            .read(&mut self.stream)
            .map_err(|_| invalid("Desktop connection closed or sent an invalid frame"))?
        {
            Some(Message::Text(text)) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|_| invalid("Host sent an unknown message")),
            Some(Message::Ping(payload)) => {
                self.write_masked(0xA, &payload)?;
                Ok(None)
            }
            Some(Message::Close(_)) => Err(invalid("Host closed desktop control")),
            Some(Message::Pong) | None => Ok(None),
        }
    }
}

/// Standard base64 for capture workers.
pub fn encode_image(bytes: &[u8]) -> String {
    ws::base64(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};

    #[test]
    fn the_host_header_sent_is_exactly_the_address_dialled() -> Result<()> {
        // The trusted proof (D288/D294) is bound to this string, so what goes
        // on the wire must be what `host_header` says.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let reader = std::thread::spawn(move || -> io::Result<String> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") && head.len() < 4096 {
                if stream.read(&mut byte)? == 0 {
                    break;
                }
                head.push(byte[0]);
            }
            Ok(String::from_utf8_lossy(&head).into_owned())
        });
        // The listener never answers, so connecting ends in an error; the
        // request is what matters.
        let _ = Connection::connect(address);
        let head = reader
            .join()
            .map_err(|_| anyhow::anyhow!("reader panicked"))??;
        let host = head
            .lines()
            .find_map(|line| line.strip_prefix("Host: "))
            .context("a Host header")?;
        assert_eq!(host, host_header(address), "as dialled");
        assert_eq!(host, address.to_string(), "ip:port, nothing added");
        Ok(())
    }

    /// How a scripted host answers the handshake.
    #[derive(Clone, Copy)]
    enum Answer {
        /// 101 with the right accept key, then `frames`.
        Upgrade,
        /// 101 with an accept key for some other key.
        WrongKey,
        /// A plain 403.
        Refuse,
        /// Closes the socket after reading the request.
        Hangup,
    }

    /// A host on loopback that reads one handshake, answers it as told,
    /// writes `frames` (unmasked, as a server does) and then returns what
    /// the client sent after the handshake.
    fn host(
        answer: Answer,
        frames: Vec<u8>,
    ) -> Result<(SocketAddr, std::thread::JoinHandle<io::Result<Vec<u8>>>)> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let thread = std::thread::spawn(move || -> io::Result<Vec<u8>> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                if stream.read(&mut byte)? == 0 {
                    break;
                }
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let key = head
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap_or_default()
                .to_owned();
            let reply = match answer {
                Answer::Upgrade => Some(ws::accept_key(&key)),
                Answer::WrongKey => Some(ws::accept_key("dGhlIHNhbXBsZSBub25jZQ==")),
                Answer::Refuse => {
                    stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")?;
                    None
                }
                Answer::Hangup => return Ok(Vec::new()),
            };
            let Some(accept) = reply else {
                return Ok(Vec::new());
            };
            stream.write_all(
                format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
                     Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                )
                .as_bytes(),
            )?;
            stream.write_all(&frames)?;
            // What the client answers (a pong), until it hangs up.
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut sent = Vec::new();
            let _ = stream.read_to_end(&mut sent);
            Ok(sent)
        });
        Ok((address, thread))
    }

    /// One unmasked server frame with a short payload.
    fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x80 | opcode, u8::try_from(payload.len()).unwrap_or(0)];
        frame.extend_from_slice(payload);
        frame
    }

    /// Receives until a message, an error, or two seconds of nothing.
    fn next(connection: &mut Connection) -> io::Result<Option<ServerMessage>> {
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if let Some(message) = connection.receive()? {
                return Ok(Some(message));
            }
        }
        Ok(None)
    }

    #[test]
    fn a_handshake_the_host_did_not_complete_properly_is_refused() -> Result<()> {
        for (answer, what) in [
            (Answer::WrongKey, "an accept key for another key"),
            (Answer::Refuse, "a 403 instead of 101"),
            (Answer::Hangup, "a host that hangs up"),
        ] {
            let (address, thread) = host(answer, Vec::new())?;
            let outcome = Connection::connect(address);
            assert!(outcome.is_err(), "{what} must not connect");
            let _ = thread.join();
        }
        Ok(())
    }

    #[test]
    fn only_a_private_address_with_a_port_is_dialled() -> Result<()> {
        for address in ["8.8.8.8:80", "0.0.0.0:80", "127.0.0.1:0"] {
            let address: SocketAddr = address.parse()?;
            let refused = Connection::connect(address).err();
            assert_eq!(
                refused.as_ref().map(io::Error::kind),
                Some(io::ErrorKind::InvalidData),
                "{address} is refused before any packet: {refused:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_ping_is_answered_and_a_message_and_a_close_are_read() -> Result<()> {
        let message = serde_json::to_string(&ServerMessage::error(crate::ErrorCode::Busy, "wait"))?;
        let mut frames = frame(0x9, b"hi");
        frames.extend(frame(0x1, message.as_bytes()));
        frames.extend(frame(0x8, &1000_u16.to_be_bytes()));
        let (address, thread) = host(Answer::Upgrade, frames)?;
        let mut connection = Connection::connect(address)?;
        let got = next(&mut connection)?;
        assert_eq!(
            got,
            Some(ServerMessage::error(crate::ErrorCode::Busy, "wait")),
            "the message after the ping"
        );
        let closed = next(&mut connection);
        assert!(closed.is_err(), "a close ends the connection: {closed:?}");
        drop(connection);
        let sent = thread
            .join()
            .map_err(|_| anyhow::anyhow!("host panicked"))??;
        // The pong: FIN + opcode 0xA, masked, two bytes, unmasked "hi".
        assert!(sent.len() >= 8, "the client answered the ping: {sent:?}");
        assert_eq!(sent.first(), Some(&0x8A), "a pong");
        assert_eq!(sent.get(1), Some(&0x82), "masked, two bytes");
        let mask: Vec<u8> = sent.get(2..6).context("a mask")?.to_vec();
        let payload: Vec<u8> = sent
            .get(6..8)
            .context("a payload")?
            .iter()
            .zip(mask.iter())
            .map(|(byte, key)| byte ^ key)
            .collect();
        assert_eq!(payload, b"hi", "the ping's payload echoed");
        Ok(())
    }

    #[test]
    fn a_message_the_client_does_not_know_ends_the_connection() -> Result<()> {
        let frames = frame(0x1, br#"{"type":"from_the_future"}"#);
        let (address, thread) = host(Answer::Upgrade, frames)?;
        let mut connection = Connection::connect(address)?;
        let outcome = next(&mut connection);
        assert!(
            outcome.is_err(),
            "an unknown message is an error: {outcome:?}"
        );
        drop(connection);
        let _ = thread.join();
        Ok(())
    }
}
