// ============================================================================
// Module:       strobe_shadertoy::listener::token
// Description:  The per-install secret the userscript sends in X-Strobe-Token:
//               generated from the OS random source, stored in the data
//               directory, compared in constant time.
//
// Dependencies: getrandom
// ============================================================================

//! The import token.
//!
//! A web page in the same browser can make requests to `127.0.0.1`, so the
//! listener cannot trust "it came from localhost". It trusts a secret
//! instead: 32 random bytes from the operating system, written as 64 hex
//! characters to `import-token` in the data directory the first time the
//! listener is enabled and reused after. The person pastes it into the
//! userscript once (the app shows it and copies it).
//!
//! The file is created readable by its owner only on Unix. On Windows it
//! inherits the per-user ACL of `%LOCALAPPDATA%`, which other accounts
//! cannot read; there is no portable safe-Rust call to tighten it further.
//!
//! [`Token::matches`] compares without stopping at the first differing byte.
//! Over loopback that is almost academic, and it costs four lines.

use std::fmt;
use std::io::Write as _;
use std::path::Path;

/// The token file's name inside the data directory.
pub const TOKEN_FILE: &str = "import-token";

/// Random bytes in a token (hex doubles the length).
const BYTES: usize = 32;

/// A listener secret.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A fresh random token.
    pub fn generate() -> std::io::Result<Self> {
        let mut bytes = [0_u8; BYTES];
        getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(Self(hex(&bytes)))
    }

    /// Accepts `text` as a token if it is 64 hex characters (what
    /// [`Self::generate`] makes).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        (text.len() == BYTES * 2 && text.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self(text.to_ascii_lowercase()))
    }

    /// The token in `data_dir/import-token`, made and stored if it is
    /// missing or damaged.
    pub fn load_or_create(data_dir: &Path) -> std::io::Result<Self> {
        let path = data_dir.join(TOKEN_FILE);
        if let Some(token) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| Self::parse(&text))
        {
            return Ok(token);
        }
        std::fs::create_dir_all(data_dir)?;
        let token = Self::generate()?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(token.0.as_bytes())?;
        file.write_all(b"\n")?;
        Ok(token)
    }

    /// The token as text, for showing to the person and for tests.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether `candidate` is this token. Compares every byte.
    pub fn matches(&self, candidate: &str) -> bool {
        let (a, b) = (self.0.as_bytes(), candidate.trim().as_bytes());
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0_u8, |diff, (x, y)| diff | (x ^ y)) == 0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(***)")
    }
}

/// Lowercase hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_generated_token_is_64_hex_characters_and_never_repeats() -> Result<()> {
        let a = Token::generate()?;
        let b = Token::generate()?;
        assert_eq!(a.expose().len(), 64, "length");
        assert!(a.expose().bytes().all(|c| c.is_ascii_hexdigit()), "hex");
        assert_ne!(a, b, "two tokens must differ");
        Ok(())
    }

    #[test]
    fn matching_is_exact_and_ignores_only_surrounding_whitespace() -> Result<()> {
        let token = Token::generate()?;
        assert!(token.matches(token.expose()), "same");
        assert!(
            token.matches(&format!(" {}\r\n", token.expose())),
            "trimmed"
        );
        assert!(!token.matches(""), "empty");
        assert!(!token.matches(&token.expose()[1..]), "short");
        let mut flipped = token.expose().to_string();
        flipped.replace_range(0..1, if flipped.starts_with('a') { "b" } else { "a" });
        assert!(!token.matches(&flipped), "one character off");
        Ok(())
    }

    #[test]
    fn the_token_is_stored_and_reused_and_a_damaged_file_is_replaced() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Token::load_or_create(&dir)?;
        let second = Token::load_or_create(&dir)?;
        assert_eq!(first, second, "reused");
        std::fs::write(dir.join(TOKEN_FILE), "garbage")?;
        let third = Token::load_or_create(&dir)?;
        assert_ne!(first, third, "a damaged file is replaced");
        assert_eq!(
            std::fs::read_to_string(dir.join(TOKEN_FILE))?.trim(),
            third.expose(),
            "and rewritten"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.join(TOKEN_FILE))?
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "group/other have no access: {mode:o}");
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn debug_never_prints_the_secret() -> Result<()> {
        let token = Token::generate()?;
        assert!(!format!("{token:?}").contains(token.expose()), "debug");
        Ok(())
    }
}
