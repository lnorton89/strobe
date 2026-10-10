// ============================================================================
// Module:       strobe_shadertoy::listener::wire
// Description:  The little HTTP/1.1 the listener speaks: read a request head,
//               write a JSON response, drain what is left.
//
// Dependencies: std::io, std::net
// ============================================================================

//! The wire format.
//!
//! The listener has one route and one client, so it speaks exactly as much
//! HTTP as that needs and refuses the rest, rather than depending on a
//! general server (D114):
//!
//! - one request per connection, `Connection: close` on every response;
//! - the head is read up to [`MAX_HEAD`] bytes and must be UTF-8, with a
//!   request line of three parts and `name: value` headers;
//! - bodies must carry a `Content-Length`; `Transfer-Encoding` is refused;
//! - nothing is read of a body until the request has been authorised, and
//!   an oversized one is never read at all.
//!
//! After a rejection made without reading the body, [`drain`] lets the
//! client finish sending (up to a bound) before the socket closes. Closing
//! with unread data makes the operating system answer with a reset, and a
//! reset can destroy the response the client had not read yet.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

/// The most bytes a request head (line and headers) may take.
pub(super) const MAX_HEAD: usize = 16 * 1024;

/// The most headers a request may carry.
const MAX_HEADERS: usize = 64;

/// How much of an unwanted body is read and discarded before closing.
const DRAIN_BYTES: usize = 8 * 1024 * 1024;

/// How long draining may take.
const DRAIN_TIME: Duration = Duration::from_secs(2);

/// A parsed request head.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Head {
    /// `POST`, `GET`, ...
    pub method: String,
    /// The request target, as sent.
    pub target: String,
    /// Header names (lower-cased) and values (trimmed).
    pub headers: Vec<(String, String)>,
    /// Body bytes that arrived with the head.
    pub leftover: Vec<u8>,
}

impl Head {
    /// The value of header `name` (lower-case), if sent exactly once.
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        let mut found = self.headers.iter().filter(|(n, _)| n == name);
        let first = found.next()?;
        found.next().is_none().then_some(first.1.as_str())
    }

    /// Whether header `name` appears more than once.
    pub(super) fn repeated(&self, name: &str) -> bool {
        self.headers.iter().filter(|(n, _)| n == name).count() > 1
    }
}

/// Why a head could not be read.
#[derive(Debug)]
pub(super) enum HeadError {
    /// The peer connected and sent nothing (a port probe).
    Closed,
    /// The head is longer than [`MAX_HEAD`].
    TooLarge,
    /// The head is not HTTP the listener understands.
    Malformed(&'static str),
    /// The read failed or timed out.
    Io(io::Error),
}

/// Reads and parses a request head from `stream`.
pub(super) fn read_head(stream: &mut impl Read) -> Result<Head, HeadError> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 2048];
    let end = loop {
        let read = stream.read(&mut chunk).map_err(HeadError::Io)?;
        if read == 0 {
            return Err(if buffer.is_empty() {
                HeadError::Closed
            } else {
                HeadError::Malformed("the request ended before its headers did")
            });
        }
        // Resume the search three bytes back: the terminator may straddle
        // two reads.
        let from = buffer.len().saturating_sub(3);
        buffer.extend_from_slice(chunk.get(..read).unwrap_or_default());
        if let Some(at) = find(&buffer, from, b"\r\n\r\n") {
            break at;
        }
        if buffer.len() > MAX_HEAD {
            return Err(HeadError::TooLarge);
        }
    };
    if end > MAX_HEAD {
        return Err(HeadError::TooLarge);
    }
    let head_bytes = buffer.get(..end).unwrap_or_default();
    let leftover = buffer.get(end + 4..).unwrap_or_default().to_vec();
    let text = std::str::from_utf8(head_bytes)
        .map_err(|_| HeadError::Malformed("the headers are not text"))?;
    parse(text, leftover)
}

/// Index of `needle` in `haystack` at or after `from`.
fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// Parses a head (without the blank line).
fn parse(text: &str, leftover: Vec<u8>) -> Result<Head, HeadError> {
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(HeadError::Malformed(
            "the request line is not METHOD TARGET VERSION",
        ));
    };
    if method.is_empty()
        || !method.bytes().all(|b| b.is_ascii_uppercase())
        || target.is_empty()
        || !version.starts_with("HTTP/1.")
    {
        return Err(HeadError::Malformed("the request line is not HTTP/1.x"));
    }
    let mut headers = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(HeadError::Malformed("a header has no colon"));
        };
        if name.is_empty()
            || name
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        {
            return Err(HeadError::Malformed("a header name is not valid"));
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        if headers.len() > MAX_HEADERS {
            return Err(HeadError::Malformed("too many headers"));
        }
    }
    Ok(Head {
        method: method.to_string(),
        target: target.to_string(),
        headers,
        leftover,
    })
}

/// The reason phrase of a status the listener uses.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// Writes a JSON response and flushes it. `allow` adds an `Allow` header
/// (for a 405). There is deliberately no CORS header of any kind: the
/// browser must not let a web page read these answers (D114).
pub(super) fn respond(
    stream: &mut impl Write,
    status: u16,
    body: &serde_json::Value,
    allow: Option<&str>,
) -> io::Result<()> {
    let body = body.to_string();
    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n",
        reason(status),
        body.len()
    );
    if let Some(allow) = allow {
        head.push_str(&format!("Allow: {allow}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

/// Closes our side, then reads and discards what the client is still
/// sending, within [`DRAIN_BYTES`] and [`DRAIN_TIME`].
pub(super) fn drain(stream: &mut TcpStream) {
    let _ = stream.shutdown(Shutdown::Write);
    let started = Instant::now();
    let mut sink = [0_u8; 16 * 1024];
    let mut total = 0;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    while total < DRAIN_BYTES && started.elapsed() < DRAIN_TIME {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(read) => total += read,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn head(text: &str) -> Result<Head, HeadError> {
        read_head(&mut text.as_bytes())
    }

    #[test]
    fn a_normal_head_is_parsed_with_its_leftover_body() -> Result<()> {
        let parsed = head(
            "POST /import HTTP/1.1\r\nHost: 127.0.0.1:1\r\nX-Strobe-Token: abc \r\n\
             Content-Length: 5\r\n\r\nhello",
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(parsed.method, "POST", "method");
        assert_eq!(parsed.target, "/import", "target");
        assert_eq!(
            parsed.header("x-strobe-token"),
            Some("abc"),
            "trimmed value"
        );
        assert_eq!(parsed.header("content-length"), Some("5"), "length");
        assert_eq!(parsed.leftover, b"hello", "body bytes that came with it");
        Ok(())
    }

    #[test]
    fn a_header_sent_twice_is_not_a_value() -> Result<()> {
        let parsed = head("POST / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n")
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(parsed.header("content-length"), None, "ambiguous");
        assert!(parsed.repeated("content-length"), "flagged as repeated");
        Ok(())
    }

    #[test]
    fn garbage_is_malformed_not_a_panic() {
        for text in [
            "\r\n\r\n",
            "GET\r\n\r\n",
            "GET / HTTP/1.1 extra\r\n\r\n",
            "get / HTTP/1.1\r\n\r\n",
            "GET / SPDY/3\r\n\r\n",
            "GET / HTTP/1.1\r\nno colon here\r\n\r\n",
            "GET / HTTP/1.1\r\nbad name: x\r\n\r\n",
            "GET / HTTP/1.1\r\n: empty\r\n\r\n",
        ] {
            assert!(
                matches!(head(text), Err(HeadError::Malformed(_))),
                "`{text:?}` must be malformed, got {:?}",
                head(text)
            );
        }
    }

    #[test]
    fn nothing_sent_is_closed_and_a_truncated_head_is_malformed() {
        assert!(matches!(head(""), Err(HeadError::Closed)), "empty");
        assert!(
            matches!(
                head("POST / HTTP/1.1\r\nHost: x"),
                Err(HeadError::Malformed(_))
            ),
            "truncated"
        );
    }

    #[test]
    fn a_head_over_the_cap_is_too_large() {
        let long = format!(
            "GET / HTTP/1.1\r\nX: {}\r\n\r\n",
            "a".repeat(MAX_HEAD + 100)
        );
        assert!(matches!(head(&long), Err(HeadError::TooLarge)), "too large");
        let many: String = (0..80).map(|i| format!("H{i}: v\r\n")).collect();
        let many = format!("GET / HTTP/1.1\r\n{many}\r\n");
        assert!(
            matches!(head(&many), Err(HeadError::Malformed(_))),
            "too many headers"
        );
    }

    #[test]
    fn a_terminator_split_across_reads_is_found() -> Result<()> {
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                match self.0.split_first() {
                    Some((first, rest)) if !buf.is_empty() => {
                        buf[0] = *first;
                        self.0 = rest;
                        Ok(1)
                    }
                    Some(_) | None => Ok(0),
                }
            }
        }
        let parsed = read_head(&mut OneByte(b"GET /x HTTP/1.1\r\nA: b\r\n\r\n"))
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(parsed.target, "/x", "target");
        assert_eq!(parsed.header("a"), Some("b"), "header");
        Ok(())
    }

    #[test]
    fn a_response_has_length_close_and_no_cors() -> Result<()> {
        let mut out = Vec::new();
        respond(
            &mut out,
            405,
            &serde_json::json!({"error": "nope"}),
            Some("POST"),
        )?;
        let text = String::from_utf8(out)?;
        assert!(
            text.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
            "{text}"
        );
        assert!(text.contains("Allow: POST\r\n"), "allow: {text}");
        assert!(text.contains("Connection: close\r\n"), "close: {text}");
        assert!(text.contains("Content-Length: 16\r\n"), "length: {text}");
        assert!(
            !text.to_ascii_lowercase().contains("access-control"),
            "must not grant CORS: {text}"
        );
        assert!(text.ends_with("{\"error\":\"nope\"}"), "body: {text}");
        Ok(())
    }
}
