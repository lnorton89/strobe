// ============================================================================
// Module:       strobe_shadertoy::acquire::http
// Description:  The real HTTP client: ureq with rustls, behind the `Fetch`
//               trait. Compiled only with the `net` feature.
//
// Dependencies: ureq (rustls, ring, webpki-roots)
// ============================================================================

//! The HTTP client.
//!
//! `ureq` is a blocking client with no async runtime, `rustls` is TLS in
//! Rust, and the root certificates are Mozilla's bundle compiled in, so a
//! machine with a broken or missing system certificate store still works
//! (D110). Proxies come from the environment (`HTTPS_PROXY`), which is what
//! a school or office network and this project's cloud containers need.
//!
//! One [`UreqFetch`] holds one connection pool. Requests are made from a
//! worker thread, never the event loop, and each carries its own time limit
//! ([`Limits`]) that covers connecting, sending and reading the body.

use super::fetch::{Fetch, FetchError, Limits};
use std::time::Duration;

/// Redirects followed. Shadertoy's media is served directly; a couple are
/// allowed for a CDN hop, not a chain.
const MAX_REDIRECTS: u32 = 2;

/// An HTTP client for Shadertoy.
#[derive(Debug, Clone)]
pub struct UreqFetch {
    agent: ureq::Agent,
}

impl UreqFetch {
    /// A client identifying itself as `strobe/<version>`, using the
    /// proxy from the environment if there is one.
    pub fn new() -> Self {
        Self::build(true)
    }

    /// [`Self::new`] ignoring any proxy in the environment, for talking to
    /// a server on this machine (the tests' mock Shadertoy).
    pub fn direct() -> Self {
        Self::build(false)
    }

    fn build(env_proxy: bool) -> Self {
        let builder = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(MAX_REDIRECTS)
            .user_agent(concat!(
                "strobe/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/lnorton89/strobe)"
            ));
        let config = if env_proxy {
            builder.build()
        } else {
            builder.proxy(None).build()
        };
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Default for UreqFetch {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetch for UreqFetch {
    fn get(&self, url: &str, limits: Limits) -> Result<Vec<u8>, FetchError> {
        let request = self
            .agent
            .get(url)
            .config()
            .timeout_global(Some(limits.timeout))
            .build();
        let mut response = request.call().map_err(|e| map_error(&e, limits.timeout))?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(FetchError::Status { status });
        }
        response
            .body_mut()
            .with_config()
            .limit(limits.max_bytes as u64)
            .read_to_vec()
            .map_err(|e| map_error(&e, limits.timeout))
    }
}

/// Classifies a `ureq` error. The text never contains the URL: it comes
/// from the error's own display, passed through [`super::redact_key`]
/// anyway in case a future version prints one.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "ureq::Error is #[non_exhaustive]; everything not named is a plain failure"
)]
fn map_error(error: &ureq::Error, timeout: Duration) -> FetchError {
    match error {
        ureq::Error::Timeout(_) => FetchError::TimedOut(timeout.as_secs()),
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
            FetchError::Unreachable(super::redact_key(&error.to_string()))
        }
        ureq::Error::BodyExceedsLimit(limit) => FetchError::TooLarge(*limit as usize),
        ureq::Error::Io(io) => match io.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                FetchError::TimedOut(timeout.as_secs())
            }
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected => {
                FetchError::Unreachable(super::redact_key(&error.to_string()))
            }
            _ => FetchError::Other(super::redact_key(&error.to_string())),
        },
        _ => FetchError::Other(super::redact_key(&error.to_string())),
    }
}
