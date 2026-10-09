// ============================================================================
// Module:       strobe_playlist::error
// Description:  Errors with a place in the file: ParseError (line, column,
//               the offending text) and PlaylistError for file I/O.
//
// Dependencies: thiserror
// ============================================================================

//! Playlist errors.
//!
//! A playlist is hand-edited and hot-reloaded, so an error has to say
//! *where* and *what*, in the author's terms: `line 14, column 12: "3mn" is
//! not a duration (try 90s, 3m, ...)`. The app shows that string as is and
//! keeps the previous playlist loaded.

use std::path::PathBuf;

/// A problem in playlist text, with its position.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.render())]
pub struct ParseError {
    /// 1-based line.
    pub line: usize,
    /// 1-based column, counted in characters.
    pub column: usize,
    /// What is wrong. Messages from this crate quote the offending text.
    pub message: String,
    /// The source line at the position, for errors whose message
    /// does not quote it (the TOML syntax errors); empty otherwise.
    pub near: String,
}

impl ParseError {
    /// The one-line form shown to people.
    fn render(&self) -> String {
        let mut text = format!(
            "line {}, column {}: {}",
            self.line, self.column, self.message
        );
        if !self.near.is_empty() {
            text.push_str(&format!(" (near \"{}\")", self.near));
        }
        text
    }
}

/// Line and column (both 1-based) of byte `offset` in `text`.
pub(crate) fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = text.get(..offset).unwrap_or("");
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let column = before.get(line_start..).map_or(0, |s| s.chars().count()) + 1;
    (line, column)
}

/// Up to `limit` characters of the source line containing `offset`, trimmed,
/// as a hint to where an error is.
pub(crate) fn snippet(text: &str, offset: usize, limit: usize) -> String {
    let offset = offset.min(text.len());
    let before = text.get(..offset).unwrap_or("");
    let start = before.rfind('\n').map_or(0, |at| at + 1);
    let line = text.get(start..).unwrap_or("").lines().next().unwrap_or("");
    line.trim().chars().take(limit).collect()
}

/// Reading or writing a playlist file.
#[derive(Debug, thiserror::Error)]
pub enum PlaylistError {
    /// The text is not a valid playlist.
    #[error("{0}")]
    Parse(#[from] ParseError),
    /// The file could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_are_one_based_and_count_characters() {
        let text = "ab\ncdé\nxyz";
        assert_eq!(line_column(text, 0), (1, 1), "start");
        assert_eq!(line_column(text, 3), (2, 1), "first char of line 2");
        assert_eq!(line_column(text, 8), (3, 1), "after a multi-byte char");
        assert_eq!(
            line_column(text, 7),
            (2, 4),
            "the newline ends line 2 after 3 characters"
        );
        assert_eq!(line_column(text, 99), (3, 4), "past the end clamps");
    }

    #[test]
    fn display_carries_position_message_and_context() {
        let plain = ParseError {
            line: 14,
            column: 12,
            message: "\"3mn\" is not a duration".into(),
            near: String::new(),
        };
        assert_eq!(
            plain.to_string(),
            "line 14, column 12: \"3mn\" is not a duration",
            "plain"
        );
        let near = ParseError {
            near: "oops =".into(),
            ..plain
        };
        assert!(
            near.to_string().ends_with("(near \"oops =\")"),
            "with context: {near}"
        );
    }

    #[test]
    fn snippets_stop_at_the_line_and_the_limit() {
        assert_eq!(
            snippet("abc def\nnext", 4, 40),
            "abc def",
            "the whole line, not the rest of it"
        );
        assert_eq!(snippet("a\n  next  \nz", 5, 40), "next", "trimmed");
        assert_eq!(snippet("abcdefgh", 0, 3), "abc", "limited");
        assert_eq!(
            snippet("abc", 50, 3),
            "abc",
            "past the end is the last line"
        );
    }
}
