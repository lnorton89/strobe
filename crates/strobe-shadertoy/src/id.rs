// ============================================================================
// Module:       strobe_shadertoy::id
// Description:  Layer (a): turns a pasted Shadertoy URL, embed link or bare
//               id into a validated ShaderId.
//
// Dependencies: thiserror
// ============================================================================

//! Shader identifiers.
//!
//! People paste whatever is in their address bar, so the accepted forms are
//! generous: `https://www.shadertoy.com/view/XsXXDn`, the same with a query
//! (`?gui=true&t=10`) or fragment, `shadertoy.com/embed/XsXXDn` with no
//! scheme, an API path (`/api/v1/shaders/XsXXDn`), or the six characters
//! alone. Anything else is an error that says what was wrong, because the
//! alternative to a good message here is a 404 from a network call that
//! was never going to work.
//!
//! An id is six ASCII letters and digits. Every id Shadertoy has issued
//! has that shape; [`ID_LEN`] is the one place to change if that ever
//! stops being true.

use std::fmt;

/// Characters in a Shadertoy shader id.
pub const ID_LEN: usize = 6;

/// The hosts a Shadertoy link may have.
const HOSTS: [&str; 2] = ["shadertoy.com", "www.shadertoy.com"];

/// A validated Shadertoy shader id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShaderId(String);

/// Why some text is not a shader id or link.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// Nothing but whitespace.
    #[error("no shader id or URL given")]
    Empty,
    /// A link to some other site.
    #[error("`{0}` is not a shadertoy.com address")]
    WrongHost(String),
    /// A shadertoy.com link that does not point at one shader.
    #[error(
        "`{0}` is a shadertoy.com address but not a shader page \
         (expected /view/ID, /embed/ID or /api/v1/shaders/ID)"
    )]
    NotAShaderPage(String),
    /// The id part has the wrong shape.
    #[error("`{0}` is not a shader id (expected {ID_LEN} letters and digits, like XsXXDn)")]
    BadId(String),
}

impl ShaderId {
    /// Parses a bare id or a Shadertoy link.
    pub fn parse(text: &str) -> Result<Self, IdError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(IdError::Empty);
        }
        if !text.contains(['/', '.', ':']) {
            return Self::from_bare(text);
        }

        // Drop the scheme, then the query and fragment.
        let lower = text.to_ascii_lowercase();
        let without_scheme = ["https://", "http://", "//"]
            .iter()
            .find(|scheme| lower.starts_with(**scheme))
            .map_or(text, |scheme| &text[scheme.len()..]);
        let path = without_scheme
            .split(['?', '#'])
            .next()
            .unwrap_or(without_scheme);

        let mut parts = path.split('/');
        let host = parts.next().unwrap_or_default();
        if !HOSTS.contains(&host.to_ascii_lowercase().as_str()) {
            return Err(IdError::WrongHost(host.to_string()));
        }
        let segments: Vec<&str> = parts.filter(|part| !part.is_empty()).collect();
        match segments.as_slice() {
            ["view" | "embed", id] | ["api", "v1", "shaders", id] => Self::from_bare(id),
            _ => Err(IdError::NotAShaderPage(text.to_string())),
        }
    }

    fn from_bare(text: &str) -> Result<Self, IdError> {
        if text.len() == ID_LEN && text.bytes().all(|b| b.is_ascii_alphanumeric()) {
            Ok(Self(text.to_string()))
        } else {
            Err(IdError::BadId(text.to_string()))
        }
    }

    /// The id as written (case matters).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The shader's page: `https://www.shadertoy.com/view/ID`.
    pub fn page_url(&self) -> String {
        format!("https://www.shadertoy.com/view/{}", self.0)
    }

    /// The path of the shader on the REST API, without the host or key.
    pub fn api_path(&self) -> String {
        format!("/api/v1/shaders/{}", self.0)
    }
}

impl fmt::Display for ShaderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn every_accepted_form_gives_the_same_id() -> Result<()> {
        for text in [
            "XsXXDn",
            "  XsXXDn\n",
            "https://www.shadertoy.com/view/XsXXDn",
            "http://shadertoy.com/view/XsXXDn",
            "HTTPS://WWW.SHADERTOY.COM/view/XsXXDn",
            "shadertoy.com/view/XsXXDn",
            "www.shadertoy.com/embed/XsXXDn",
            "https://www.shadertoy.com/embed/XsXXDn?gui=true&t=10&paused=false",
            "https://www.shadertoy.com/view/XsXXDn#comments",
            "https://www.shadertoy.com/view/XsXXDn/",
            "//www.shadertoy.com/view/XsXXDn",
            "https://www.shadertoy.com/api/v1/shaders/XsXXDn?key=abc",
        ] {
            let id = ShaderId::parse(text)?;
            assert_eq!(id.as_str(), "XsXXDn", "`{text}`");
        }
        Ok(())
    }

    #[test]
    fn urls_are_built_from_the_id() -> Result<()> {
        let id = ShaderId::parse("4dXGRr")?;
        assert_eq!(
            id.page_url(),
            "https://www.shadertoy.com/view/4dXGRr",
            "page"
        );
        assert_eq!(id.api_path(), "/api/v1/shaders/4dXGRr", "api");
        assert_eq!(id.to_string(), "4dXGRr", "display");
        Ok(())
    }

    #[test]
    fn bad_input_says_what_is_wrong() {
        let cases = [
            ("", IdError::Empty),
            ("   ", IdError::Empty),
            (
                "https://example.com/view/XsXXDn",
                IdError::WrongHost("example.com".to_string()),
            ),
            (
                "https://www.shadertoy.com/browse",
                IdError::NotAShaderPage("https://www.shadertoy.com/browse".to_string()),
            ),
            (
                "https://www.shadertoy.com/user/iq",
                IdError::NotAShaderPage("https://www.shadertoy.com/user/iq".to_string()),
            ),
            ("short", IdError::BadId("short".to_string())),
            ("XsXXDn7", IdError::BadId("XsXXDn7".to_string())),
            ("Xs XDn", IdError::BadId("Xs XDn".to_string())),
            (
                "https://www.shadertoy.com/view/bad!id",
                IdError::BadId("bad!id".to_string()),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(ShaderId::parse(text), Err(expected), "`{text}`");
        }
    }

    #[test]
    fn case_is_preserved() -> Result<()> {
        assert_eq!(
            ShaderId::parse("xsxxdn")?.as_str(),
            "xsxxdn",
            "ids are case-sensitive"
        );
        Ok(())
    }
}
