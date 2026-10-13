// ============================================================================
// Module:       strobe_remote::server::ws
// Description:  The WebSocket (RFC 6455) the remote needs: the handshake's
//               accept key, a bounded frame reader for masked client frames,
//               and unmasked server frames.
//
// Dependencies: sha1 (the handshake); std::io
// ============================================================================

//! WebSocket framing (D250).
//!
//! The remote's messages are a few dozen bytes of JSON each way, so this is
//! a strict subset of RFC 6455 rather than a general library:
//!
//! - **Bounded before it is read.** A frame's header states its length; a
//!   length over the reader's limit is refused (close 1009) before one byte
//!   of the payload is read. The reader's buffer never holds more than one
//!   largest frame, and a fragmented message is assembled up to the same
//!   limit.
//! - **Strict.** Client frames must be masked, reserved bits clear, control
//!   frames short and unfragmented, text valid UTF-8, a continuation must
//!   continue something. Anything else closes with 1002 or 1007. Binary
//!   messages are not part of the protocol (1003).
//! - **Non-blocking-friendly.** [`Reader::read`] does at most one socket
//!   read. A read timeout is "nothing yet", and a partial frame stays in the
//!   buffer for the next call, so the connection thread can wake every tick
//!   to push state and check its timers.
//!
//! Server frames are never masked and never fragmented.

use sha1::{Digest as _, Sha1};
use std::io::{self, Read, Write};

/// RFC 6455's fixed GUID for the accept key.
const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The longest frame header: 2 bytes, an 8-byte length, a 4-byte mask.
const MAX_HEADER: usize = 14;

/// The longest control-frame payload.
const MAX_CONTROL: usize = 125;

/// Close codes the listener sends.
pub(super) mod close {
    /// Normal closure.
    pub(crate) const NORMAL: u16 = 1000;
    /// The server is going away (stopped, idle).
    pub(crate) const GOING_AWAY: u16 = 1001;
    /// A protocol error.
    pub(crate) const PROTOCOL: u16 = 1002;
    /// A data type the protocol does not take (binary).
    pub(crate) const UNSUPPORTED: u16 = 1003;
    /// Text that is not UTF-8.
    pub(crate) const NOT_UTF8: u16 = 1007;
    /// A policy violation: a flood, too many wrong codes, revoked.
    pub(crate) const POLICY: u16 = 1008;
    /// A message over the size limit.
    pub(crate) const TOO_BIG: u16 = 1009;
}

/// Standard base64 of `bytes`, with padding.
pub(super) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18_u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Whether `key` is a `Sec-WebSocket-Key`: base64 of 16 bytes.
pub(super) fn valid_key(key: &str) -> bool {
    key.len() == 24
        && key.ends_with("==")
        && key
            .bytes()
            .take(22)
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

/// The `Sec-WebSocket-Accept` for a client's key.
pub(super) fn accept_key(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(GUID);
    base64(&hasher.finalize())
}

/// A whole message from the client.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Message {
    /// A text message.
    Text(String),
    /// A WebSocket ping, with its payload to echo.
    Ping(Vec<u8>),
    /// A WebSocket pong.
    Pong,
    /// The client is closing, with its code if it gave one.
    Close(Option<u16>),
}

/// Why the connection must end.
#[derive(Debug)]
pub(super) enum WsError {
    /// The client broke the protocol.
    Protocol(&'static str),
    /// A message over the limit.
    TooBig,
    /// Text that is not UTF-8.
    NotUtf8,
    /// A binary message.
    Unsupported,
    /// The connection closed without a close frame.
    Closed,
    /// The read failed.
    Io(io::Error),
}

impl WsError {
    /// The close frame's reason.
    pub(super) fn reason(&self) -> &'static str {
        match self {
            Self::Protocol(why) => why,
            Self::TooBig => "message too large",
            Self::NotUtf8 => "text is not UTF-8",
            Self::Unsupported => "binary messages are not part of the protocol",
            Self::Closed => "closed",
            Self::Io(_) => "read failed",
        }
    }

    /// The close code to send, when one should be sent.
    pub(super) fn close_code(&self) -> Option<u16> {
        match self {
            Self::Protocol(_) => Some(close::PROTOCOL),
            Self::TooBig => Some(close::TOO_BIG),
            Self::NotUtf8 => Some(close::NOT_UTF8),
            Self::Unsupported => Some(close::UNSUPPORTED),
            Self::Closed | Self::Io(_) => None,
        }
    }
}

/// Reads client frames into messages, holding at most one frame.
#[derive(Debug)]
pub(super) struct Reader {
    buffer: Vec<u8>,
    max_message: usize,
    /// A fragmented text message being assembled.
    partial: Option<Vec<u8>>,
}

/// One parsed frame.
struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

impl Reader {
    /// A reader for messages of at most `max_message` bytes, starting with
    /// bytes that arrived with the handshake.
    pub(super) fn new(max_message: usize, leftover: Vec<u8>) -> Self {
        Self {
            buffer: leftover,
            max_message,
            partial: None,
        }
    }

    /// The most bytes the buffer may hold: one largest frame.
    fn capacity(&self) -> usize {
        MAX_HEADER + self.max_message.max(MAX_CONTROL)
    }

    /// The next whole message: from what is buffered if there is one, else
    /// after one read from `stream`. `Ok(None)` is "nothing complete yet"
    /// (including a read timeout).
    pub(super) fn read(&mut self, stream: &mut impl Read) -> Result<Option<Message>, WsError> {
        if let Some(message) = self.next_message()? {
            return Ok(Some(message));
        }
        let room = self.capacity().saturating_sub(self.buffer.len());
        if room == 0 {
            // Cannot happen: a frame that fits is parsed, and one that does
            // not is refused from its header. Fail closed all the same.
            return Err(WsError::TooBig);
        }
        let mut chunk = [0_u8; 2048];
        let want = room.min(chunk.len());
        let slot = chunk.get_mut(..want).unwrap_or_default();
        match stream.read(slot) {
            Ok(0) => Err(WsError::Closed),
            Ok(read) => {
                self.buffer
                    .extend_from_slice(chunk.get(..read).unwrap_or_default());
                self.next_message()
            }
            Err(error) if super::http::timed_out(&error) => Ok(None),
            Err(error) => Err(WsError::Io(error)),
        }
    }

    /// Takes whole frames off the buffer until one finishes a message.
    fn next_message(&mut self) -> Result<Option<Message>, WsError> {
        while let Some(frame) = self.next_frame()? {
            if let Some(message) = self.assemble(frame)? {
                return Ok(Some(message));
            }
        }
        Ok(None)
    }

    /// Parses one frame off the front of the buffer, if it is all there.
    fn next_frame(&mut self) -> Result<Option<Frame>, WsError> {
        let (Some(&b0), Some(&b1)) = (self.buffer.first(), self.buffer.get(1)) else {
            return Ok(None);
        };
        let fin = b0 & 0x80 != 0;
        let opcode = b0 & 0x0f;
        if b0 & 0x70 != 0 {
            return Err(WsError::Protocol("reserved bits set"));
        }
        if b1 & 0x80 == 0 {
            return Err(WsError::Protocol("client frames must be masked"));
        }
        let (length, at) = match b1 & 0x7f {
            126 => match (self.buffer.get(2), self.buffer.get(3)) {
                (Some(&a), Some(&b)) => (u64::from(u16::from_be_bytes([a, b])), 4),
                _ => return Ok(None),
            },
            127 => {
                let Some(bytes) = self.buffer.get(2..10) else {
                    return Ok(None);
                };
                let mut be = [0_u8; 8];
                be.copy_from_slice(bytes);
                let length = u64::from_be_bytes(be);
                if length >> 63 != 0 {
                    return Err(WsError::Protocol("a length with its top bit set"));
                }
                (length, 10)
            }
            short => (u64::from(short), 2),
        };
        let control = opcode & 0x08 != 0;
        if control && (!fin || length > MAX_CONTROL as u64) {
            return Err(WsError::Protocol("a control frame must be short and whole"));
        }
        let limit = if control {
            MAX_CONTROL
        } else {
            self.max_message
        };
        let length = match usize::try_from(length) {
            Ok(length) if length <= limit => length,
            Ok(_) | Err(_) => return Err(WsError::TooBig),
        };
        let total = at + 4 + length;
        if self.buffer.len() < total {
            return Ok(None);
        }
        let mut mask = [0_u8; 4];
        mask.copy_from_slice(self.buffer.get(at..at + 4).unwrap_or(&[0; 4]));
        let payload: Vec<u8> = self
            .buffer
            .get(at + 4..total)
            .unwrap_or_default()
            .iter()
            .zip(mask.iter().cycle())
            .map(|(byte, key)| byte ^ key)
            .collect();
        self.buffer.drain(..total);
        Ok(Some(Frame {
            fin,
            opcode,
            payload,
        }))
    }

    /// Turns a frame into a message, or keeps it as part of one.
    fn assemble(&mut self, frame: Frame) -> Result<Option<Message>, WsError> {
        match frame.opcode {
            0x0 => {
                let Some(partial) = self.partial.as_mut() else {
                    return Err(WsError::Protocol("a continuation of nothing"));
                };
                if partial.len() + frame.payload.len() > self.max_message {
                    return Err(WsError::TooBig);
                }
                partial.extend_from_slice(&frame.payload);
                if frame.fin {
                    let whole = self.partial.take().unwrap_or_default();
                    return text(whole).map(Some);
                }
                Ok(None)
            }
            0x1 => {
                if self.partial.is_some() {
                    return Err(WsError::Protocol("a new message inside a fragmented one"));
                }
                if frame.fin {
                    text(frame.payload).map(Some)
                } else {
                    self.partial = Some(frame.payload);
                    Ok(None)
                }
            }
            0x2 => Err(WsError::Unsupported),
            0x8 => match frame.payload.as_slice() {
                [] => Ok(Some(Message::Close(None))),
                [a, b, ..] => Ok(Some(Message::Close(Some(u16::from_be_bytes([*a, *b]))))),
                [_] => Err(WsError::Protocol("a one-byte close payload")),
            },
            0x9 => Ok(Some(Message::Ping(frame.payload))),
            0xA => Ok(Some(Message::Pong)),
            _ => Err(WsError::Protocol("an unknown opcode")),
        }
    }
}

fn text(bytes: Vec<u8>) -> Result<Message, WsError> {
    String::from_utf8(bytes)
        .map(Message::Text)
        .map_err(|_| WsError::NotUtf8)
}

/// Writes one unmasked, unfragmented frame.
fn write_frame(stream: &mut impl Write, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | opcode);
    match payload.len() {
        len @ 0..=125 => header.push(len as u8),
        len @ 126..=0xffff => {
            header.push(126);
            header.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            header.push(127);
            header.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    stream.flush()
}

/// Writes a text message.
pub(super) fn write_text(stream: &mut impl Write, text: &str) -> io::Result<()> {
    write_frame(stream, 0x1, text.as_bytes())
}

/// Answers a ping.
pub(super) fn write_pong(stream: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    write_frame(stream, 0xA, payload)
}

/// Writes a close frame with `code` and a short reason.
pub(super) fn write_close(stream: &mut impl Write, code: u16, reason: &str) -> io::Result<()> {
    let mut payload = code.to_be_bytes().to_vec();
    let mut end = reason.len().min(MAX_CONTROL - 2);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    payload.extend_from_slice(reason.get(..end).unwrap_or_default().as_bytes());
    write_frame(stream, 0x8, &payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// A masked client frame, as a browser sends it.
    fn client_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [0x37_u8, 0xfa, 0x21, 0x3d];
        let mut out = vec![(if fin { 0x80 } else { 0 }) | opcode];
        match payload.len() {
            len @ 0..=125 => out.push(0x80 | len as u8),
            len @ 126..=0xffff => {
                out.push(0x80 | 126);
                out.extend_from_slice(&(len as u16).to_be_bytes());
            }
            len => {
                out.push(0x80 | 127);
                out.extend_from_slice(&(len as u64).to_be_bytes());
            }
        }
        out.extend_from_slice(&mask);
        out.extend(payload.iter().zip(mask.iter().cycle()).map(|(b, k)| b ^ k));
        out
    }

    fn read_all(bytes: &[u8], max: usize) -> Result<Vec<Message>, WsError> {
        let mut reader = Reader::new(max, Vec::new());
        let mut stream = bytes;
        let mut out = Vec::new();
        loop {
            match reader.read(&mut stream) {
                Ok(Some(message)) => out.push(message),
                Ok(None) => {}
                Err(WsError::Closed) => return Ok(out),
                Err(error) => return Err(error),
            }
        }
    }

    #[test]
    fn the_accept_key_is_rfc_6455s_example() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=",
            "RFC 6455 section 1.3"
        );
        assert!(valid_key("dGhlIHNhbXBsZSBub25jZQ=="), "a valid key");
        assert!(!valid_key("short=="), "too short");
        assert!(!valid_key("dGhlIHNhbXBsZSBub25jZQ!!"), "not base64");
    }

    #[test]
    fn base64_matches_rfc_4648() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), encoded, "{plain}");
        }
    }

    #[test]
    fn masked_text_pings_and_fragments_are_read() -> Result<()> {
        let mut bytes = client_frame(true, 0x1, b"{\"type\":\"ping\"}");
        bytes.extend(client_frame(true, 0x9, b"hi"));
        bytes.extend(client_frame(false, 0x1, b"hel"));
        bytes.extend(client_frame(true, 0xA, b""));
        bytes.extend(client_frame(true, 0x0, "lo é".as_bytes()));
        bytes.extend(client_frame(true, 0x1, &vec![b'x'; 300]));
        bytes.extend(client_frame(true, 0x8, &1000_u16.to_be_bytes()));
        let messages = read_all(&bytes, 4096).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(
            messages,
            vec![
                Message::Text("{\"type\":\"ping\"}".into()),
                Message::Ping(b"hi".to_vec()),
                Message::Pong,
                Message::Text("hello é".into()),
                Message::Text("x".repeat(300)),
                Message::Close(Some(1000)),
            ],
            "in order, a control frame inside a fragmented message"
        );
        Ok(())
    }

    #[test]
    fn a_frame_split_across_reads_is_kept_until_whole() -> Result<()> {
        let bytes = client_frame(true, 0x1, b"split");
        let mut reader = Reader::new(4096, Vec::new());
        for (i, byte) in bytes.iter().enumerate() {
            let got = reader
                .read(&mut std::slice::from_ref(byte))
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            if i + 1 < bytes.len() {
                assert!(got.is_none(), "byte {i}: not whole yet");
            } else {
                assert_eq!(got, Some(Message::Text("split".into())), "whole");
            }
        }
        Ok(())
    }

    #[test]
    fn an_oversized_frame_is_refused_from_its_header() {
        // The header claims a gigabyte; only the header is sent.
        let mut header = vec![0x81, 0x80 | 127];
        header.extend_from_slice(&(1_u64 << 30).to_be_bytes());
        header.extend_from_slice(&[0, 0, 0, 0]);
        assert!(
            matches!(read_all(&header, 4096), Err(WsError::TooBig)),
            "refused before any payload"
        );
        let big = client_frame(true, 0x1, &vec![b'a'; 4097]);
        assert!(
            matches!(read_all(&big, 4096), Err(WsError::TooBig)),
            "one over"
        );
        let mut fragments = client_frame(false, 0x1, &vec![b'a'; 3000]);
        fragments.extend(client_frame(true, 0x0, &vec![b'a'; 3000]));
        assert!(
            matches!(read_all(&fragments, 4096), Err(WsError::TooBig)),
            "fragments add up"
        );
    }

    #[test]
    fn protocol_violations_close_the_connection() -> Result<()> {
        let mut unmasked = client_frame(true, 0x1, b"x");
        unmasked[1] &= 0x7f;
        let mut reserved = client_frame(true, 0x1, b"x");
        reserved[0] |= 0x40;
        let mut top_bit = vec![0x81, 0x80 | 127];
        top_bit.extend_from_slice(&u64::MAX.to_be_bytes());
        for (name, bytes, code) in [
            ("unmasked", unmasked, close::PROTOCOL),
            ("reserved bits", reserved, close::PROTOCOL),
            ("top bit length", top_bit, close::PROTOCOL),
            ("binary", client_frame(true, 0x2, b"x"), close::UNSUPPORTED),
            (
                "bad utf-8",
                client_frame(true, 0x1, &[0xff, 0xfe]),
                close::NOT_UTF8,
            ),
            (
                "stray continuation",
                client_frame(true, 0x0, b"x"),
                close::PROTOCOL,
            ),
            (
                "fragmented ping",
                client_frame(false, 0x9, b"x"),
                close::PROTOCOL,
            ),
            (
                "long ping",
                client_frame(true, 0x9, &[0; 126]),
                close::PROTOCOL,
            ),
            (
                "unknown opcode",
                client_frame(true, 0x3, b"x"),
                close::PROTOCOL,
            ),
            (
                "one-byte close",
                client_frame(true, 0x8, b"x"),
                close::PROTOCOL,
            ),
        ] {
            let Err(error) = read_all(&bytes, 4096) else {
                anyhow::bail!("{name} was accepted");
            };
            assert_eq!(error.close_code(), Some(code), "{name}: {error:?}");
        }
        Ok(())
    }

    #[test]
    fn server_frames_are_unmasked_with_the_right_length_form() -> Result<()> {
        for len in [0_usize, 125, 126, 65535, 65536] {
            let mut out = Vec::new();
            write_text(&mut out, &"a".repeat(len))?;
            let header = match len {
                0..=125 => 2,
                126..=65535 => 4,
                _ => 10,
            };
            assert_eq!(out.len(), header + len, "length {len}");
            assert_eq!(out.first(), Some(&0x81), "FIN text");
            assert_eq!(out.get(1).map(|b| b & 0x80), Some(0), "unmasked");
        }
        let mut out = Vec::new();
        write_close(&mut out, close::POLICY, &"é".repeat(100))?;
        assert!(out.len() <= 2 + MAX_CONTROL, "a close fits a control frame");
        Ok(())
    }
}
