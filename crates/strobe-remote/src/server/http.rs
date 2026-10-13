// ============================================================================
// Module:       strobe_remote::server::http
// Description:  The little HTTP/1.1 the remote listener speaks: read one
//               request head under a deadline, and write a response.
//
// Dependencies: std::io, std::time
// ============================================================================

//! The wire format before the WebSocket (D250).
//!
//! The listener serves one page and one WebSocket, so like the Shadertoy
//! listener (D114) it speaks only as much HTTP as that needs and refuses the
//! rest:
//!
//! - one request per connection, `Connection: close` on every response that
//!   is not a WebSocket upgrade;
//! - only `GET` (and `HEAD` for the page); no request has a body, so a
//!   `Content-Length` other than 0 or any `Transfer-Encoding` is refused;
//! - the head is at most [`MAX_HEAD`] bytes of UTF-8 with at most
//!   [`MAX_HEADERS`] headers, and must arrive whole before a deadline, so a
//!   client that trickles one byte at a time is cut off;
//! - responses carry no CORS headers of any kind.
//!
//! The two parsers are separate on purpose: this one reads with a deadline
//! and hands back the bytes after the head to the WebSocket reader, and the
//! import listener reads bodies. Neither crate depends on the other.

use std::io::{self, Read, Write};
use std::time::Instant;

/// The most bytes a request head may take. Browsers send well under 2 KiB.
pub(super) const MAX_HEAD: usize = 8 * 1024;

/// The most headers a request may carry.
const MAX_HEADERS: usize = 48;

/// A parsed request head.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Head {
    /// `GET`, `HEAD`, ...
    pub method: String,
    /// The request target, as sent.
    pub target: String,
    /// Header names (lower-cased) and values (trimmed).
    pub headers: Vec<(String, String)>,
    /// Bytes that arrived after the head.
    pub leftover: Vec<u8>,
}

impl Head {
    /// The value of header `name` (lower-case), if sent exactly once.
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        let mut found = self.headers.iter().filter(|(n, _)| n == name);
        let first = found.next()?;
        found.next().is_none().then_some(first.1.as_str())
    }

    /// Whether header `name` appears at all.
    pub(super) fn has(&self, name: &str) -> bool {
        self.headers.iter().any(|(n, _)| n == name)
    }

    /// Whether a comma-separated header (`Connection`, `Upgrade`) has
    /// `token` among its values, in any case.
    pub(super) fn lists(&self, name: &str, token: &str) -> bool {
        self.headers
            .iter()
            .filter(|(n, _)| n == name)
            .flat_map(|(_, value)| value.split(','))
            .any(|item| item.trim().eq_ignore_ascii_case(token))
    }

    /// The path without query or fragment.
    pub(super) fn path(&self) -> &str {
        self.target.split(['?', '#']).next().unwrap_or_default()
    }
}

/// Why a head could not be read.
#[derive(Debug)]
pub(super) enum HeadError {
    /// The peer connected and sent nothing (a port probe).
    Closed,
    /// The head is longer than [`MAX_HEAD`].
    TooLarge,
    /// The head did not arrive before the deadline.
    TooSlow,
    /// The head is not HTTP the listener understands.
    Malformed(&'static str),
    /// The read failed.
    Io(io::Error),
}

/// Whether a read error is a read timeout rather than a failure.
pub(super) fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

/// Reads and parses a request head, which must be complete by `deadline`.
/// The stream should have a read timeout well under the deadline's span.
pub(super) fn read_head(stream: &mut impl Read, deadline: Instant) -> Result<Head, HeadError> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    let end = loop {
        if Instant::now() >= deadline {
            return Err(HeadError::TooSlow);
        }
        let read = match stream.read(&mut chunk) {
            Ok(read) => read,
            Err(error) if timed_out(&error) => continue,
            Err(error) => return Err(HeadError::Io(error)),
        };
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
        || !target.starts_with('/')
        || version != "HTTP/1.1"
    {
        return Err(HeadError::Malformed("the request line is not HTTP/1.1"));
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
            || value.bytes().any(|b| b.is_ascii_control() && b != b'\t')
        {
            return Err(HeadError::Malformed("a header is not valid"));
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
        101 => "Switching Protocols",
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        421 => "Misdirected Request",
        426 => "Upgrade Required",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// A response to write.
pub(super) struct Response<'a> {
    /// The status code.
    pub status: u16,
    /// `Content-Type`.
    pub content_type: &'static str,
    /// Extra header lines, each without its CRLF.
    pub headers: &'a [String],
    /// The body.
    pub body: &'a [u8],
    /// Leave the body out (a `HEAD` request) but keep its length.
    pub head_only: bool,
}

impl<'a> Response<'a> {
    /// A short plain-text error.
    pub(super) fn text(status: u16, body: &'a str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            headers: &[],
            body: body.as_bytes(),
            head_only: false,
        }
    }
}

/// Writes `response` and flushes it. Every response is `no-store`,
/// `nosniff` and `Connection: close`; there is deliberately no CORS header.
pub(super) fn respond(stream: &mut impl Write, response: &Response<'_>) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len()
    );
    for line in response.headers {
        head.push_str(line);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if !response.head_only {
        stream.write_all(response.body)?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::time::Duration;

    fn head(text: &str) -> Result<Head, HeadError> {
        read_head(
            &mut text.as_bytes(),
            Instant::now() + Duration::from_secs(5),
        )
    }

    #[test]
    fn an_upgrade_request_is_parsed() -> Result<()> {
        let parsed = head(
            "GET /ws?x=1 HTTP/1.1\r\nHost: 10.0.0.2:47812\r\nConnection: keep-alive, Upgrade\r\n\
             Upgrade: websocket\r\n\r\nZ",
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(parsed.method, "GET", "method");
        assert_eq!(parsed.path(), "/ws", "path without the query");
        assert_eq!(parsed.header("host"), Some("10.0.0.2:47812"), "host");
        assert!(parsed.lists("connection", "upgrade"), "a listed token");
        assert!(!parsed.lists("connection", "close"), "an absent one");
        assert_eq!(parsed.leftover, b"Z", "bytes after the head");
        Ok(())
    }

    #[test]
    fn garbage_is_malformed_not_a_panic() {
        for text in [
            "\r\n\r\n",
            "GET\r\n\r\n",
            "GET / HTTP/1.1 extra\r\n\r\n",
            "get / HTTP/1.1\r\n\r\n",
            "GET / HTTP/1.0\r\n\r\n",
            "GET http://x/ HTTP/1.1\r\n\r\n",
            "GET / HTTP/1.1\r\nno colon\r\n\r\n",
            "GET / HTTP/1.1\r\nbad name: x\r\n\r\n",
            "GET / HTTP/1.1\r\nX: a\x00b\r\n\r\n",
        ] {
            assert!(
                matches!(head(text), Err(HeadError::Malformed(_))),
                "{text:?} must be malformed, got {:?}",
                head(text)
            );
        }
        assert!(matches!(head(""), Err(HeadError::Closed)), "empty");
        let long = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD));
        assert!(matches!(head(&long), Err(HeadError::TooLarge)), "too large");
    }

    #[test]
    fn a_head_that_never_ends_misses_its_deadline() {
        /// Times out forever, like a client that stopped sending.
        struct Silent;
        impl Read for Silent {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            }
        }
        let started = Instant::now();
        let result = read_head(&mut Silent, started + Duration::from_millis(30));
        assert!(matches!(result, Err(HeadError::TooSlow)), "{result:?}");
    }

    #[test]
    fn a_response_has_length_close_nosniff_and_no_cors() -> Result<()> {
        let mut out = Vec::new();
        respond(&mut out, &Response::text(403, "no"))?;
        let text = String::from_utf8(out)?;
        assert!(text.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{text}");
        assert!(text.contains("Content-Length: 2\r\n"), "{text}");
        assert!(text.contains("Connection: close\r\n"), "{text}");
        assert!(
            text.contains("X-Content-Type-Options: nosniff\r\n"),
            "{text}"
        );
        assert!(
            !text.to_ascii_lowercase().contains("access-control"),
            "no CORS: {text}"
        );
        assert!(text.ends_with("\r\n\r\nno"), "{text}");
        Ok(())
    }
}
