// ============================================================================
// Module:       strobe_shadertoy::acquire::rest
// Description:  Path B: GET /api/v1/shaders/{id}?key=KEY, returning the JSON
//               text for the same parser a dropped file goes through.
//
// Dependencies: super::{fetch, key}; crate::id
// ============================================================================

//! The REST client (path B).
//!
//! ```text
//! GET https://www.shadertoy.com/api/v1/shaders/<ID>?key=<KEY>
//! ```
//!
//! returns `{"Shader": {...}}` for a shader published "Public + API" and
//! `{"Error": "..."}` for anything else (private, unpublished, bad key).
//! [`fetch_shader_json`] returns the **text** unparsed. The caller hands it
//! to `Shader::from_json`, which understands both replies and turns the
//! error one into a message, so a REST import and a dropped file share one
//! parser, and the original text is kept verbatim as `shadertoy.json`.
//!
//! Shadertoy sits behind Cloudflare. A server-side client can be refused
//! with a 403 or a challenge page even with a valid key; that is reported
//! with advice to use the userscript instead, never retried, and never
//! worked around.

use super::fetch::{API_TIMEOUT, Endpoints, Fetch, FetchError, Limits, MAX_SHADER_BYTES};
use super::key::ApiKey;
use crate::id::ShaderId;

/// Why a shader could not be fetched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RestError {
    /// The request failed. The message names the shader, never the key.
    #[error("could not fetch shader {id} from {url}: {source}")]
    Fetch {
        /// The shader.
        id: String,
        /// The URL with the key hidden.
        url: String,
        /// Why.
        source: FetchError,
    },
    /// The server answered 200 with something that is not text.
    #[error("Shadertoy's answer for {0} is not text (a challenge page or a proxy error?)")]
    NotText(String),
    /// The answer is a web page, not the API's JSON: Cloudflare's check.
    #[error(
        "Shadertoy answered {0} with a web page instead of JSON, which is usually its bot \
         check. Send the shader from your browser with the userscript, or export its JSON"
    )]
    WebPage(String),
}

/// Fetches the JSON text of shader `id`.
pub fn fetch_shader_json(
    fetch: &dyn Fetch,
    endpoints: &Endpoints,
    id: &ShaderId,
    key: &ApiKey,
) -> Result<String, RestError> {
    let url = endpoints.shader_url(id.as_str(), key.expose());
    let body = fetch
        .get(
            &url,
            Limits {
                max_bytes: MAX_SHADER_BYTES,
                timeout: API_TIMEOUT,
            },
        )
        .map_err(|source| RestError::Fetch {
            id: id.to_string(),
            url: endpoints.shader_url_redacted(id.as_str()),
            source,
        })?;
    let text = String::from_utf8(body).map_err(|_| RestError::NotText(id.to_string()))?;
    let start = text.trim_start();
    if start.starts_with('<') {
        return Err(RestError::WebPage(id.to_string()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::cell::RefCell;

    /// A `Fetch` that records the URL and answers with a fixed result.
    struct Canned {
        answer: Result<Vec<u8>, FetchError>,
        seen: RefCell<Vec<(String, Limits)>>,
    }

    impl Canned {
        fn new(answer: Result<Vec<u8>, FetchError>) -> Self {
            Self {
                answer,
                seen: RefCell::new(Vec::new()),
            }
        }
    }

    impl Fetch for Canned {
        fn get(&self, url: &str, limits: Limits) -> Result<Vec<u8>, FetchError> {
            self.seen.borrow_mut().push((url.to_string(), limits));
            self.answer.clone()
        }
    }

    fn parts() -> Result<(Endpoints, ShaderId, ApiKey)> {
        Ok((
            Endpoints::new("https://example.test"),
            ShaderId::parse("XsXXDn")?,
            ApiKey::parse("Bd8tWD").ok_or_else(|| anyhow::anyhow!("key"))?,
        ))
    }

    #[test]
    fn the_request_goes_to_the_api_path_with_the_key_and_limits() -> Result<()> {
        let (endpoints, id, key) = parts()?;
        let fetch = Canned::new(Ok(br#"{"Shader":{}}"#.to_vec()));
        let text = fetch_shader_json(&fetch, &endpoints, &id, &key)?;
        assert_eq!(text, r#"{"Shader":{}}"#, "body returned verbatim");
        let seen = fetch.seen.borrow();
        let (url, limits) = seen.first().ok_or_else(|| anyhow::anyhow!("no request"))?;
        assert_eq!(
            url, "https://example.test/api/v1/shaders/XsXXDn?key=Bd8tWD",
            "url"
        );
        assert_eq!(limits.max_bytes, MAX_SHADER_BYTES, "body cap");
        assert_eq!(limits.timeout, API_TIMEOUT, "timeout");
        Ok(())
    }

    #[test]
    fn a_failure_names_the_shader_and_hides_the_key() -> Result<()> {
        let (endpoints, id, key) = parts()?;
        let fetch = Canned::new(Err(FetchError::Status { status: 403 }));
        let error = fetch_shader_json(&fetch, &endpoints, &id, &key)
            .err()
            .ok_or_else(|| anyhow::anyhow!("should fail"))?
            .to_string();
        assert!(error.contains("XsXXDn"), "names the shader: {error}");
        assert!(error.contains("403"), "status: {error}");
        assert!(error.contains("userscript"), "advice: {error}");
        assert!(!error.contains("Bd8tWD"), "leaked the key: {error}");
        Ok(())
    }

    #[test]
    fn a_challenge_page_is_recognised_and_binary_is_refused() -> Result<()> {
        let (endpoints, id, key) = parts()?;
        let page = Canned::new(Ok(
            b"  <!DOCTYPE html><title>Just a moment...</title>".to_vec()
        ));
        let error = fetch_shader_json(&page, &endpoints, &id, &key).err();
        assert!(
            matches!(error, Some(RestError::WebPage(_))),
            "web page: {error:?}"
        );
        let binary = Canned::new(Ok(vec![0xff, 0xfe, 0x00]));
        let error = fetch_shader_json(&binary, &endpoints, &id, &key).err();
        assert!(
            matches!(error, Some(RestError::NotText(_))),
            "binary: {error:?}"
        );
        Ok(())
    }
}
