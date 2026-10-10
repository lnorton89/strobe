// ============================================================================
// Module:       strobe_shadertoy::acquire::fetch
// Description:  The HTTP seam: a `Fetch` trait, the size and time limits of a
//               request, the errors, and the Shadertoy endpoints.
//
// Dependencies: std only (the ureq implementation is in `http`)
// ============================================================================

//! The HTTP seam.
//!
//! Everything that needs the network takes a `&dyn Fetch`, so the REST
//! client and the asset downloader are tested with a closure or a local
//! socket and the real client ([`super::UreqFetch`]) is one small file.
//!
//! ## Where requests may go
//!
//! [`Endpoints`] is the one place that knows Shadertoy's address. The
//! default is `https://www.shadertoy.com`; the `STROBE_SHADERTOY_BASE`
//! environment variable (or [`Endpoints::new`]) replaces it, which is how the
//! tests point the whole stack at a socket on `127.0.0.1`.
//!
//! A shader's JSON chooses the URLs of its textures, and that JSON can come
//! from a browser page or a stranger's file. So a texture is only fetched
//! when its URL is on Shadertoy itself, under `/media/` or `/presets/`
//! ([`Endpoints::asset_url`]); anything else is refused by name, never
//! requested. That keeps a pasted shader from making the app probe
//! `http://192.168.0.1/` or a local service.
//!
//! ## Secrets in errors
//!
//! The REST URL carries the API key in its query. Errors in this module
//! never print a URL, and [`redact_key`] is applied to any text that might
//! contain one.

use std::time::Duration;

/// Shadertoy's address.
pub const DEFAULT_BASE: &str = "https://www.shadertoy.com";

/// The environment variable that replaces [`DEFAULT_BASE`] (tests, and a
/// mirror if Shadertoy ever moves).
pub const BASE_ENV: &str = "STROBE_SHADERTOY_BASE";

/// The paths on Shadertoy a texture may be fetched from.
const ASSET_PREFIXES: [&str; 2] = ["/media/", "/presets/"];

/// The largest shader JSON accepted from the REST API.
pub const MAX_SHADER_BYTES: usize = 4 * 1024 * 1024;

/// The largest texture accepted.
pub const MAX_ASSET_BYTES: usize = 16 * 1024 * 1024;

/// The time one texture may take, connection and body together.
pub const ASSET_TIMEOUT: Duration = Duration::from_secs(10);

/// The time the REST call may take.
pub const API_TIMEOUT: Duration = Duration::from_secs(20);

/// What one request may cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The body is refused beyond this many bytes.
    pub max_bytes: usize,
    /// The whole request (connect, send, receive) is abandoned after this.
    pub timeout: Duration,
}

/// Why a request failed. None of the messages contain the URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The host could not be reached at all (DNS, refused, no route, TLS).
    /// Retrying the next file would fail the same way.
    #[error("could not connect: {0}")]
    Unreachable(String),
    /// The time limit passed.
    #[error("timed out after {0} s")]
    TimedOut(u64),
    /// The server answered with something other than 200.
    #[error("HTTP {status}{}", hint(*.status))]
    Status {
        /// The status code.
        status: u16,
    },
    /// The body was longer than the limit.
    #[error("the response is larger than {0} bytes")]
    TooLarge(usize),
    /// Anything else (a broken body, a malformed response).
    #[error("{0}")]
    Other(String),
}

impl FetchError {
    /// Whether the host itself is out of reach, so a caller with several
    /// requests to make should stop rather than wait out each one.
    pub fn host_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable(_) | Self::TimedOut(_))
    }
}

/// What a status code means to a person.
fn hint(status: u16) -> &'static str {
    match status {
        401 | 403 => {
            " (Shadertoy refused the request; it blocks some networks and automated clients. \
             Send the shader from your browser with the userscript, or export its JSON)"
        }
        404 => " (not found: check the id, and that the shader is public)",
        429 => " (Shadertoy is rate limiting this address; wait a few minutes)",
        500..=599 => " (Shadertoy had a server error; try again later)",
        _ => "",
    }
}

/// A blocking HTTP GET.
pub trait Fetch {
    /// Fetches `url` and returns the 200 response's body.
    fn get(&self, url: &str, limits: Limits) -> Result<Vec<u8>, FetchError>;
}

/// `text` with the value of any `key=` query parameter replaced by `***`.
pub fn redact_key(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("key=") {
        let (before, after) = rest.split_at(at + "key=".len());
        out.push_str(before);
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(after.len());
        out.push_str("***");
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Shadertoy's addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    base: String,
}

impl Endpoints {
    /// Endpoints under `base` (`https://host[:port]`, no trailing slash
    /// needed).
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim().trim_end_matches('/').to_string(),
        }
    }

    /// [`DEFAULT_BASE`], or [`BASE_ENV`] when that is set and not empty.
    pub fn from_env() -> Self {
        match std::env::var(BASE_ENV) {
            Ok(base) if !base.trim().is_empty() => Self::new(&base),
            Ok(_) | Err(_) => Self::new(DEFAULT_BASE),
        }
    }

    /// The base address.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The REST URL of a shader, **with the key in it**. Never log it; use
    /// [`Self::shader_url_redacted`].
    pub fn shader_url(&self, id: &str, key: &str) -> String {
        format!("{}/api/v1/shaders/{id}?key={key}", self.base)
    }

    /// [`Self::shader_url`] with the key hidden, for messages.
    pub fn shader_url_redacted(&self, id: &str) -> String {
        format!("{}/api/v1/shaders/{id}?key=***", self.base)
    }

    /// Where to fetch the texture the converter calls `url`, or why not.
    ///
    /// The converter spells every texture URL against [`DEFAULT_BASE`]. Only
    /// those on Shadertoy's `/media/` and `/presets/` are accepted, and they
    /// are fetched from this endpoint's base (which is the same host unless
    /// the base was replaced).
    pub fn asset_url(&self, url: &str) -> Result<String, String> {
        let path = [DEFAULT_BASE, "https://shadertoy.com"]
            .iter()
            .find_map(|origin| url.strip_prefix(origin))
            .ok_or_else(|| format!("`{url}` is not on shadertoy.com, so it is not downloaded"))?;
        if path.contains("..") || path.contains('\\') || path.contains('#') {
            return Err(format!("`{url}` has an unsafe path"));
        }
        if !ASSET_PREFIXES.iter().any(|prefix| path.starts_with(prefix)) {
            return Err(format!(
                "`{url}` is not under /media/ or /presets/, so it is not downloaded"
            ));
        }
        Ok(format!("{}{path}", self.base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_key_never_survives_redaction() {
        assert_eq!(
            redact_key("GET https://x/api/v1/shaders/XsXXDn?key=Bd8tWD&a=1 failed"),
            "GET https://x/api/v1/shaders/XsXXDn?key=***&a=1 failed",
            "mid-string"
        );
        assert_eq!(redact_key("key=abc"), "key=***", "at the end");
        assert_eq!(redact_key("no secrets"), "no secrets", "untouched");
    }

    #[test]
    fn errors_never_print_a_url_and_say_what_to_do() {
        let forbidden = FetchError::Status { status: 403 }.to_string();
        assert!(forbidden.contains("userscript"), "403 hint: {forbidden}");
        assert!(!forbidden.contains("http"), "no url: {forbidden}");
        assert!(
            FetchError::Status { status: 404 }
                .to_string()
                .contains("not found"),
            "404 hint"
        );
        assert_eq!(
            FetchError::Status { status: 418 }.to_string(),
            "HTTP 418",
            "no hint for the unknown"
        );
        assert!(
            FetchError::Unreachable("x".into()).host_unreachable(),
            "unreachable"
        );
        assert!(
            !FetchError::Status { status: 404 }.host_unreachable(),
            "a 404 is per file"
        );
    }

    #[test]
    fn shadertoy_media_is_allowed_and_rebased() -> Result<()> {
        let live = Endpoints::new(DEFAULT_BASE);
        assert_eq!(
            live.asset_url("https://www.shadertoy.com/media/a/abc.jpg")
                .map_err(anyhow::Error::msg)?,
            "https://www.shadertoy.com/media/a/abc.jpg",
            "live"
        );
        let mock = Endpoints::new("http://127.0.0.1:9999/");
        assert_eq!(
            mock.asset_url("https://www.shadertoy.com/presets/tex00.jpg")
                .map_err(anyhow::Error::msg)?,
            "http://127.0.0.1:9999/presets/tex00.jpg",
            "rebased onto the mock"
        );
        Ok(())
    }

    #[test]
    fn anything_else_is_refused_by_name() {
        let live = Endpoints::new(DEFAULT_BASE);
        for url in [
            "http://192.168.0.1/admin",
            "https://evil.example/media/a/x.jpg",
            "https://www.shadertoy.com.evil.example/media/a/x.jpg",
            "https://www.shadertoy.com/api/v1/shaders/XsXXDn?key=1",
            "https://www.shadertoy.com/media/../etc/passwd",
            "file:///etc/passwd",
            "/media/a/x.jpg",
        ] {
            let refused = live.asset_url(url);
            assert!(refused.is_err(), "`{url}` must be refused: {refused:?}");
        }
    }

    #[test]
    fn the_rest_url_carries_the_key_and_its_redacted_twin_does_not() {
        let endpoints = Endpoints::new(DEFAULT_BASE);
        assert_eq!(
            endpoints.shader_url("XsXXDn", "Bd8tWD"),
            "https://www.shadertoy.com/api/v1/shaders/XsXXDn?key=Bd8tWD",
            "real"
        );
        assert!(
            !endpoints.shader_url_redacted("XsXXDn").contains("Bd8tWD"),
            "redacted"
        );
    }
}
