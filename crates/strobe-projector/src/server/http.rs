// ============================================================================
// Module:       strobe_projector::server::http
// Description:  The little HTTP/1.1 the projector page speaks: read one
//               request head under a deadline, split its query and cookies,
//               and write a response with the page's security headers.
//
// Dependencies: std::io, std::time
// ============================================================================

//! The wire format (D502), modelled on `strobe-remote`'s listener (D250)
//! and kept separate from it, as that one is from the Shadertoy listener:
//! neither crate depends on the other.
//!
//! - one request per connection, `Connection: close` on every response;
//! - no request body is read: the API's POSTs carry none, and bytes after
//!   the head are ignored;
//! - the head is at most [`MAX_HEAD`] bytes of UTF-8 with at most
//!   [`MAX_HEADERS`] headers, and must arrive whole before a deadline;
//! - every response is `no-store`, `nosniff`, `X-Frame-Options: DENY` and
//!   `no-referrer`, with no CORS header of any kind.

use std::io::{self, Read, Write};
use std::time::Instant;

/// The most bytes a request head may take.
pub const MAX_HEAD: usize = 8 * 1024;

/// The most headers a request may carry.
const MAX_HEADERS: usize = 48;

/// A parsed request head.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Head {
    /// `GET`, `POST`, ...
    pub method: String,
    /// The request target, as sent.
    pub target: String,
    /// Header names (lower-cased) and values (trimmed).
    pub headers: Vec<(String, String)>,
}

impl Head {
    /// The value of header `name` (lower-case), if sent exactly once.
    pub fn header(&self, name: &str) -> Option<&str> {
        let mut found = self.headers.iter().filter(|(n, _)| n == name);
        let first = found.next()?;
        found.next().is_none().then_some(first.1.as_str())
    }

    /// The path without query or fragment.
    pub fn path(&self) -> &str {
        self.target.split(['?', '#']).next().unwrap_or_default()
    }

    /// The first value of query parameter `name`, percent-decoded.
    pub fn query(&self, name: &str) -> Option<String> {
        let query = self.target.split_once('?')?.1;
        let query = query.split('#').next().unwrap_or_default();
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key) == name).then(|| decode(value))
        })
    }

    /// The value of cookie `name`.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .filter(|(n, _)| n == "cookie")
            .flat_map(|(_, value)| value.split(';'))
            .find_map(|pair| {
                let (key, value) = pair.trim().split_once('=')?;
                (key.trim() == name).then(|| value.trim().trim_matches('"').to_string())
            })
    }
}

/// `text` with `%XX` escapes and `+` decoded, as a form query is written.
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'+' => out.push(b' '),
            b'%' => {
                let escaped = bytes
                    .get(at + 1..at + 3)
                    .and_then(|hex| std::str::from_utf8(hex).ok())
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                if let Some(value) = escaped {
                    out.push(value);
                    at += 2;
                } else {
                    out.push(b'%');
                }
            }
            other => out.push(other),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Why a head could not be read.
#[derive(Debug)]
pub enum HeadError {
    /// The peer connected and sent nothing (a port probe).
    Closed,
    /// The head is longer than [`MAX_HEAD`].
    TooLarge,
    /// The head did not arrive before the deadline.
    TooSlow,
    /// The head is not HTTP this server understands.
    Malformed(&'static str),
    /// The read failed.
    Io(io::Error),
}

/// Reads and parses a request head, which must be complete by `deadline`.
/// The stream should have a read timeout well under the deadline's span.
pub fn read_head(stream: &mut impl Read, deadline: Instant) -> Result<Head, HeadError> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    let end = loop {
        if Instant::now() >= deadline {
            return Err(HeadError::TooSlow);
        }
        let read = match stream.read(&mut chunk) {
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(HeadError::Io(error)),
        };
        if read == 0 {
            return Err(if buffer.is_empty() {
                HeadError::Closed
            } else {
                HeadError::Malformed("the request ended before its headers did")
            });
        }
        // The terminator may straddle two reads.
        let from = buffer.len().saturating_sub(3);
        buffer.extend_from_slice(chunk.get(..read).unwrap_or_default());
        if let Some(at) = buffer
            .get(from..)
            .and_then(|tail| tail.windows(4).position(|window| window == b"\r\n\r\n"))
        {
            break from + at;
        }
        if buffer.len() > MAX_HEAD {
            return Err(HeadError::TooLarge);
        }
    };
    if end > MAX_HEAD {
        return Err(HeadError::TooLarge);
    }
    let text = std::str::from_utf8(buffer.get(..end).unwrap_or_default())
        .map_err(|_| HeadError::Malformed("the headers are not text"))?;
    parse(text)
}

/// Parses a head (without the blank line).
fn parse(text: &str) -> Result<Head, HeadError> {
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
        || !(version == "HTTP/1.1" || version == "HTTP/1.0")
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
    })
}

/// The reason phrase of a status the server uses.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// A response to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The status code.
    pub status: u16,
    /// `Content-Type`.
    pub content_type: &'static str,
    /// Extra header lines, each without its CRLF.
    pub headers: Vec<String>,
    /// The body.
    pub body: Vec<u8>,
}

impl Reply {
    /// A JSON reply.
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            headers: Vec::new(),
            body: value.to_string().into_bytes(),
        }
    }

    /// An HTML reply.
    pub fn html(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    /// A reply of `body` as `content_type`.
    pub fn bytes(status: u16, content_type: &'static str, body: &[u8]) -> Self {
        Self {
            status,
            content_type,
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    /// An empty reply.
    pub const fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// The same reply with one more header line.
    #[must_use]
    pub fn with_header(mut self, line: String) -> Self {
        self.headers.push(line);
        self
    }
}

/// Writes `reply` and flushes it.
pub fn respond(stream: &mut impl Write, reply: &Reply) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n",
        reply.status,
        reason(reply.status),
        reply.content_type,
        reply.body.len()
    );
    for line in &reply.headers {
        head.push_str(line);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&reply.body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use std::time::Duration;

    fn head(text: &str) -> Result<Head> {
        read_head(
            &mut text.as_bytes(),
            Instant::now() + Duration::from_secs(5),
        )
        .map_err(|error| anyhow!("{error:?}"))
    }

    #[test]
    fn query_cookie_and_bearer_are_read() -> Result<()> {
        let parsed = head(
            "GET /?token=a%2Bb+c&x=1 HTTP/1.1\r\nCookie: theme=dark; pj_token=\"t0k\"\r\n\
             Authorization: Bearer xyz\r\n\r\n",
        )?;
        assert_eq!(parsed.path(), "/", "path");
        assert_eq!(parsed.query("token").as_deref(), Some("a+b c"), "decoded");
        assert_eq!(parsed.query("missing"), None, "absent");
        assert_eq!(parsed.cookie("pj_token").as_deref(), Some("t0k"), "cookie");
        assert_eq!(parsed.header("authorization"), Some("Bearer xyz"), "header");
        Ok(())
    }

    #[test]
    fn garbage_is_malformed_not_a_panic() {
        for text in [
            "\r\n\r\n",
            "GET\r\n\r\n",
            "get / HTTP/1.1\r\n\r\n",
            "GET http://x/ HTTP/1.1\r\n\r\n",
            "GET / HTTP/1.1\r\nno colon\r\n\r\n",
        ] {
            assert!(head(text).is_err(), "{text:?} must be refused");
        }
        let long = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD));
        assert!(head(&long).is_err(), "too large");
        assert_eq!(decode("%zz%4"), "%zz%4", "bad escapes kept");
    }

    #[test]
    fn a_reply_carries_the_security_headers() -> Result<()> {
        let mut out = Vec::new();
        respond(
            &mut out,
            &Reply::html(401, "no").with_header("WWW-Authenticate: Bearer".into()),
        )?;
        let text = String::from_utf8(out)?;
        assert!(text.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{text}");
        for header in [
            "Content-Length: 2\r\n",
            "Cache-Control: no-store\r\n",
            "X-Content-Type-Options: nosniff\r\n",
            "X-Frame-Options: DENY\r\n",
            "Referrer-Policy: no-referrer\r\n",
            "WWW-Authenticate: Bearer\r\n",
        ] {
            assert!(text.contains(header), "{header}: {text}");
        }
        assert!(
            !text.to_ascii_lowercase().contains("access-control"),
            "no CORS"
        );
        Ok(())
    }
}
