// ============================================================================
// Module:       strobe_shadertoy::acquire::key
// Description:  The Shadertoy API key: found in an environment variable or a
//               file in the data directory, validated, never printed.
//
// Dependencies: std only
// ============================================================================

//! The API key.
//!
//! Path B (the REST API) needs the key from `shadertoy.com/myapps`. It is
//! looked up in this order:
//!
//! 1. the `STROBE_SHADERTOY_KEY` environment variable;
//! 2. the file `shadertoy.key` in the data directory (the folder that also
//!    holds the logs), first line, whitespace trimmed.
//!
//! It is **not** put in the OS keychain: the `keyring` crate drags in a
//! platform service stack for one short string that Shadertoy itself treats
//! as low-value (it is a read-only, per-app key the owner can regenerate),
//! and the data directory is already per-user (D117).
//!
//! The key goes into a URL query, so it is validated to be nothing but
//! letters, digits, `_` and `-`: a key file cannot inject `&`, `#` or a
//! space into the request. [`ApiKey`] prints as `ApiKey(***)`.

use std::fmt;
use std::path::{Path, PathBuf};

/// The environment variable that holds the key.
pub const KEY_ENV: &str = "STROBE_SHADERTOY_KEY";

/// The key file's name inside the data directory.
pub const KEY_FILE: &str = "shadertoy.key";

/// Where keys are made.
pub const KEY_PAGE: &str = "https://www.shadertoy.com/myapps";

/// The longest key accepted. Real keys are six characters.
const MAX_KEY_LEN: usize = 64;

/// Why there is no usable key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// Neither the variable nor the file exists.
    #[error(
        "no Shadertoy API key: set {KEY_ENV}, or put the key on one line in {}. \
         Make a key at {KEY_PAGE}",
        .file.display()
    )]
    Missing {
        /// The file that was looked for.
        file: PathBuf,
    },
    /// A key was found but it is not a plausible key.
    #[error(
        "the Shadertoy API key from {source_name} is not valid (expected letters and digits, \
         up to {MAX_KEY_LEN} characters)"
    )]
    Invalid {
        /// `STROBE_SHADERTOY_KEY` or the file path.
        source_name: String,
    },
}

/// A validated Shadertoy API key.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Validates `text` as a key.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let valid = !text.is_empty()
            && text.len() <= MAX_KEY_LEN
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        valid.then(|| Self(text.to_string()))
    }

    /// Finds the key: the environment first, then `data_dir/shadertoy.key`.
    pub fn find(data_dir: &Path) -> Result<Self, KeyError> {
        Self::find_with(std::env::var(KEY_ENV).ok().as_deref(), data_dir)
    }

    /// [`Self::find`] with the environment value passed in, so tests need
    /// not touch the process environment.
    pub fn find_with(env_value: Option<&str>, data_dir: &Path) -> Result<Self, KeyError> {
        if let Some(value) = env_value.filter(|v| !v.trim().is_empty()) {
            return Self::parse(value).ok_or_else(|| KeyError::Invalid {
                source_name: KEY_ENV.to_string(),
            });
        }
        let file = data_dir.join(KEY_FILE);
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                // Notepad's "UTF-8 with BOM" starts the file with one.
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                let first = text.lines().next().unwrap_or_default();
                Self::parse(first).ok_or_else(|| KeyError::Invalid {
                    source_name: file.display().to_string(),
                })
            }
            Err(_) => Err(KeyError::Missing { file }),
        }
    }

    /// The key, for building the request URL only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn plausible_keys_pass_and_url_injection_does_not() {
        assert!(ApiKey::parse("Bd8tWD").is_some(), "a real-looking key");
        assert!(ApiKey::parse("  Bd8tWD\n").is_some(), "whitespace trimmed");
        for bad in [
            "",
            "   ",
            "a b",
            "k&x=1",
            "k#frag",
            "k/../x",
            &"a".repeat(65),
        ] {
            assert!(ApiKey::parse(bad).is_none(), "`{bad}` must be refused");
        }
    }

    #[test]
    fn the_environment_beats_the_file_and_the_file_is_used_without_it() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-key-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(KEY_FILE), "FromFile\nsecond line ignored\n")?;
        assert_eq!(
            ApiKey::find_with(Some("FromEnv"), &dir)?.expose(),
            "FromEnv",
            "env first"
        );
        assert_eq!(
            ApiKey::find_with(None, &dir)?.expose(),
            "FromFile",
            "file next"
        );
        assert_eq!(
            ApiKey::find_with(Some("  "), &dir)?.expose(),
            "FromFile",
            "a blank variable is no variable"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_key_file_saved_by_notepad_is_read() -> Result<()> {
        // Notepad's "UTF-8 with BOM", and CRLF line ends.
        let dir = std::env::temp_dir().join(format!("strobe-key-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(KEY_FILE), "\u{feff}Bd8tWD\r\n")?;
        let found = ApiKey::find_with(None, &dir).map(|key| key.expose().to_string());
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(found?, "Bd8tWD", "the BOM is not part of the key");
        Ok(())
    }

    #[test]
    fn a_missing_key_says_where_to_put_it_and_where_to_get_one() {
        let dir = Path::new("definitely/not/here");
        let error = ApiKey::find_with(None, dir).map(|_| ()).unwrap_err_text();
        for needle in [KEY_ENV, KEY_FILE, KEY_PAGE] {
            assert!(error.contains(needle), "missing `{needle}` in: {error}");
        }
    }

    #[test]
    fn an_invalid_key_is_reported_without_echoing_it() {
        let error = ApiKey::find_with(Some("bad key!"), Path::new("x"))
            .map(|_| ())
            .unwrap_err_text();
        assert!(!error.contains("bad key"), "echoed the key: {error}");
        assert!(error.contains(KEY_ENV), "names its source: {error}");
    }

    #[test]
    fn debug_hides_the_key() {
        let key = ApiKey::parse("Bd8tWD");
        assert_eq!(format!("{key:?}"), "Some(ApiKey(***))", "debug");
    }

    /// Test helper: the error text of a result that must be an error.
    trait ErrText {
        fn unwrap_err_text(self) -> String;
    }

    impl ErrText for Result<(), KeyError> {
        fn unwrap_err_text(self) -> String {
            match self {
                Ok(()) => "unexpected success".to_string(),
                Err(error) => error.to_string(),
            }
        }
    }
}
