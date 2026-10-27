// ============================================================================
// Module:       strobe_projector::wire
// Description:  What both projector protocols share: a TCP connection with
//               a connect and a read timeout, a CR-terminated line, and the
//               MD5 hex digest that authenticates a command.
//
// Dependencies: std::net, md-5; crate::error
// ============================================================================

//! The line-oriented TCP both protocols use.
//!
//! PJLink and NTCONTROL both greet on connect, take one command ending in
//! CR, answer with one line ending in CR and close. A projector in standby
//! can take seconds to answer, a unit that is off the network never does,
//! so the connect waits at most [`CONNECT_TIMEOUT`] and each read at most
//! [`IO_TIMEOUT`]. A line longer than [`MAX_LINE`] is not a projector.

use crate::error::Error;
use md5::{Digest as _, Md5};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// How long a TCP connect may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);

/// How long one read may wait for the projector.
pub const IO_TIMEOUT: Duration = Duration::from_secs(6);

/// The longest line a projector sends, in bytes.
const MAX_LINE: usize = 512;

/// Connects to `host:port`, mapping a refusal to [`Error::Refused`] (which
/// the retry policy retries) and anything else to [`Error::Transport`].
pub fn connect(host: &str, port: u16) -> Result<TcpStream, Error> {
    let where_ = format!("{host}:{port}");
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| Error::Transport(format!("{where_} {error}")))?;
    let mut last = Error::Transport(format!("{where_} has no address"));
    for address in addresses {
        match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(IO_TIMEOUT))
                    .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
                    .map_err(|error| Error::Transport(format!("{where_} {error}")))?;
                return Ok(stream);
            }
            Err(error) => {
                let kind = error.kind();
                last = if kind == io::ErrorKind::ConnectionRefused {
                    Error::Refused(where_.clone())
                } else if matches!(kind, io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) {
                    Error::Transport(format!("{where_} connect timeout (no response)"))
                } else {
                    Error::Transport(format!("{where_} {error}"))
                };
            }
        }
    }
    Err(last)
}

/// Reads one line ending in CR (not included). A stray LF either side is
/// dropped, so a unit that ends lines with CR LF reads the same.
pub fn read_line(stream: &mut impl Read) -> Result<String, Error> {
    let mut line = Vec::with_capacity(64);
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => {
                if line.is_empty() {
                    return Err(Error::Transport("connection closed by projector".into()));
                }
                break;
            }
            Ok(_) => {
                if byte == *b"\r" {
                    break;
                }
                line.extend_from_slice(&byte);
                if line.len() > MAX_LINE {
                    return Err(Error::Transport("response too long".into()));
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(Error::Transport(format!(
                    "timeout waiting for response (got {:?})",
                    String::from_utf8_lossy(&line)
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(Error::Transport(format!("read failed: {error}"))),
        }
    }
    Ok(String::from_utf8_lossy(&line)
        .trim_matches('\n')
        .to_string())
}

/// Sends `line` followed by CR.
pub fn send_line(stream: &mut impl Write, line: &str) -> Result<(), Error> {
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\r');
    stream
        .write_all(&bytes)
        .and_then(|()| stream.flush())
        .map_err(|error| Error::Transport(format!("send failed: {error}")))
}

/// Lowercase hex of the MD5 of `text`.
pub fn md5_hex(text: &str) -> String {
    hex(&Md5::digest(text.as_bytes()))
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// A greeting of the form `<WORD> 0` (no authentication) or
/// `<WORD> 1 <random>` (authentication with this random), as PJLink and
/// NTCONTROL both send.
#[derive(Debug, PartialEq, Eq)]
pub enum Greeting {
    /// Commands go without a digest.
    Open,
    /// Commands carry a digest made with this random.
    Challenge(String),
}

/// Parses `line` as a `word` greeting, or `None` when it is something
/// else. A challenge without its random is not a greeting.
pub fn greeting(line: &str, word: &str) -> Option<Greeting> {
    let rest = line.strip_prefix(word)?.strip_prefix(' ')?;
    let mut parts = rest.split(' ');
    let class = parts.next()?;
    let random = parts.next();
    if parts.next().is_some() {
        return None;
    }
    match (class, random) {
        ("0", None) => Some(Greeting::Open),
        ("1", Some(random)) if !random.is_empty() && !random.contains(char::is_whitespace) => {
            Some(Greeting::Challenge(random.to_string()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn md5_matches_the_rfc_1321_vectors() {
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e", "empty");
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72", "abc");
        // The PJLink specification's own example: random 498e4a67,
        // password JBMIAProjectorLink.
        assert_eq!(
            md5_hex("498e4a67JBMIAProjectorLink"),
            "5d8409bc1c3fa39749434aa3a5c38682",
            "PJLink example"
        );
    }

    #[test]
    fn a_line_ends_at_cr_and_loses_stray_lf() -> Result<()> {
        let mut input: &[u8] = b"PJLINK 0\r\n%1POWR=1\r";
        assert_eq!(read_line(&mut input)?, "PJLINK 0", "first");
        assert_eq!(read_line(&mut input)?, "%1POWR=1", "second, LF dropped");
        assert!(read_line(&mut input).is_err(), "closed with nothing");
        let mut unterminated: &[u8] = b"%1POWR=0";
        assert_eq!(read_line(&mut unterminated)?, "%1POWR=0", "EOF ends it");
        let long = vec![b'x'; MAX_LINE + 2];
        assert!(read_line(&mut long.as_slice()).is_err(), "too long");
        Ok(())
    }

    #[test]
    fn greetings_parse_strictly() {
        assert_eq!(greeting("PJLINK 0", "PJLINK"), Some(Greeting::Open), "open");
        assert_eq!(
            greeting("PJLINK 1 498e4a67", "PJLINK"),
            Some(Greeting::Challenge("498e4a67".into())),
            "challenge"
        );
        for bad in [
            "PJLINK 1",
            "PJLINK 0 extra",
            "PJLINK 2",
            "PJLINK ERRA",
            "NTCONTROL 0",
            "PJLINK 1 a b",
        ] {
            assert_eq!(greeting(bad, "PJLINK"), None, "{bad}");
        }
        assert_eq!(
            greeting("NTCONTROL 1 0b2c", "NTCONTROL"),
            Some(Greeting::Challenge("0b2c".into())),
            "NTCONTROL"
        );
    }
}
