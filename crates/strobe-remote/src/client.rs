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
}
