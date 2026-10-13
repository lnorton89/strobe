// ============================================================================
// Module:       strobe_remote::secret
// Description:  The remote's secrets: random bytes from the OS, a client
//               token that never prints, its SHA-256 for storage, and a
//               comparison that does not stop at the first difference.
//
// Dependencies: getrandom, sha2
// ============================================================================

//! Secrets (L-03, D251).
//!
//! A [`Token`] is 32 bytes from the operating system's random source,
//! written as 64 lowercase hex characters. That is what a phone keeps and
//! sends in an `authenticate` message. The desktop never stores it: it keeps
//! [`Token::digest`], the token's SHA-256, and compares digests with
//! [`same`], so the file in the data directory cannot be replayed as a
//! credential and a timing difference says nothing about a token.
//!
//! Neither a token nor a pairing code ever prints: `Debug` writes `***`,
//! there is no `Display`, and the only way to the text is
//! [`Token::expose`], which the listener calls once, to put the token in
//! the `paired` message body.

use sha2::{Digest as _, Sha256};
use std::fmt;

/// Random bytes in a token (hex doubles the length).
const TOKEN_BYTES: usize = 32;

/// `N` bytes from the operating system's random source.
pub(crate) fn random<const N: usize>() -> std::io::Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes)
}

/// Lowercase hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Whether `a` and `b` are equal, comparing every byte of the shorter
/// length's worth either way, so the time taken does not say where they
/// first differ.
pub(crate) fn same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0_u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// The SHA-256 of `text`, as hex: how a token is kept on disk.
pub(crate) fn digest(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// A paired client's bearer token. Prints as `Token(***)`.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A fresh random token.
    pub fn generate() -> std::io::Result<Self> {
        Ok(Self(hex(&random::<TOKEN_BYTES>()?)))
    }

    /// The token's text, for the `paired` message body and nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The SHA-256 the desktop stores in its place.
    pub fn digest(&self) -> String {
        digest(&self.0)
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_token_is_64_hex_characters_and_never_repeats() -> Result<()> {
        let a = Token::generate()?;
        let b = Token::generate()?;
        assert_eq!(a.expose().len(), 64, "32 bytes as hex");
        assert!(
            a.expose()
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "lowercase hex"
        );
        assert!(
            a.expose().len() <= crate::MAX_SECRET_LEN,
            "fits the protocol's secret bound"
        );
        assert_ne!(a, b, "two tokens differ");
        Ok(())
    }

    #[test]
    fn a_token_never_prints() -> Result<()> {
        let token = Token::generate()?;
        let printed = format!("{token:?} {:?}", vec![token.clone()]);
        assert!(!printed.contains(token.expose()), "debug: {printed}");
        assert!(printed.contains("***"), "redacted: {printed}");
        Ok(())
    }

    #[test]
    fn the_stored_digest_is_sha256_and_not_the_token() -> Result<()> {
        assert_eq!(
            digest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "the FIPS 180-2 test vector"
        );
        let token = Token::generate()?;
        assert_ne!(token.digest(), token.expose(), "a digest is not the token");
        assert_eq!(token.digest().len(), 64, "256 bits of hex");
        Ok(())
    }

    #[test]
    fn comparison_is_exact() {
        assert!(same(b"abc", b"abc"), "equal");
        assert!(!same(b"abc", b"abd"), "last byte differs");
        assert!(!same(b"abc", b"ab"), "length differs");
        assert!(same(b"", b""), "both empty");
        assert_eq!(hex(&[0x00, 0x7f, 0xff]), "007fff", "hex");
    }
}
