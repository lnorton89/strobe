// ============================================================================
// Module:       strobe_shadertoy::glsl
// Description:  A lossless GLSL tokenizer: just enough to rewrite identifiers
//               and delete statements without touching anything else.
//
// Dependencies: std only
// ============================================================================

//! A small, lossless GLSL lexer.
//!
//! The converter has to change a few things in code it did not write
//! (`texture2D` becomes `texture`, a `precision` line disappears) and must
//! not damage the rest. A regular expression would happily rewrite
//! `texture2D` inside a comment, or inside `mytexture2D`. This lexer splits
//! source into tokens such that **concatenating every token's text gives
//! back the input byte for byte**; a rewrite replaces some tokens and
//! leaves the rest alone.
//!
//! It knows only what that needs: identifiers, numbers (so `1e3` is not an
//! identifier `e3`), punctuation, whitespace, the two comment forms, and
//! preprocessor lines (a `#` as the first thing on a line, through the
//! line's end, following `\` continuations and block comments). It does not
//! parse. Strings do not exist in GLSL.

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A name: `[A-Za-z_][A-Za-z0-9_]*`.
    Ident,
    /// A numeric literal, including its suffix and exponent.
    Number,
    /// One punctuation character.
    Punct,
    /// A run of spaces, tabs and newlines.
    Space,
    /// `// ...` up to (not including) the newline.
    LineComment,
    /// `/* ... */`, possibly spanning lines.
    BlockComment,
    /// A whole preprocessor line, continuations included.
    Directive,
}

/// One token: its kind and the exact source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    /// What it is.
    pub kind: Kind,
    /// The source text, verbatim.
    pub text: &'a str,
}

/// Splits `source` into tokens. Never fails: unterminated comments run to
/// the end, and stray bytes become [`Kind::Punct`].
pub fn lex(source: &str) -> Vec<Token<'_>> {
    lex_with(source, true)
}

/// [`lex`], optionally treating `#` at a line start as ordinary
/// punctuation (for the body of a directive that was lexed already).
fn lex_with(source: &str, directives: bool) -> Vec<Token<'_>> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut line_start = true;
    while at < bytes.len() {
        let start = at;
        let byte = bytes[at];
        let kind = if byte.is_ascii_whitespace() {
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                if bytes[at] == b'\n' {
                    line_start = true;
                }
                at += 1;
            }
            // A newline inside the run, then more blanks, still leaves us
            // at a line start for a following `#`.
            Kind::Space
        } else if directives && byte == b'#' && line_start {
            at = directive_end(bytes, at);
            Kind::Directive
        } else if byte == b'/' && bytes.get(at + 1) == Some(&b'/') {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            Kind::LineComment
        } else if byte == b'/' && bytes.get(at + 1) == Some(&b'*') {
            at = block_comment_end(bytes, at);
            Kind::BlockComment
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            Kind::Ident
        } else if byte.is_ascii_digit() || (byte == b'.' && next_is_digit(bytes, at)) {
            at = number_end(bytes, at);
            Kind::Number
        } else {
            // One whole character, so multi-byte text stays valid UTF-8.
            at += source[at..].chars().next().map_or(1, char::len_utf8);
            Kind::Punct
        };
        if kind != Kind::Space {
            line_start = false;
        }
        if let Some(text) = source.get(start..at) {
            tokens.push(Token { kind, text });
        }
    }
    tokens
}

fn next_is_digit(bytes: &[u8], at: usize) -> bool {
    bytes.get(at + 1).is_some_and(u8::is_ascii_digit)
}

/// The end of a block comment starting at `at`.
fn block_comment_end(bytes: &[u8], at: usize) -> usize {
    let mut i = at + 2;
    while i < bytes.len() {
        if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

/// The end of the preprocessor line starting at `at`: the next newline
/// that is not escaped by `\` and not inside a block comment.
fn directive_end(bytes: &[u8], at: usize) -> usize {
    let mut i = at;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => return i,
            b'\\' if bytes.get(i + 1) == Some(&b'\n') => i += 2,
            b'\\' if bytes.get(i + 1) == Some(&b'\r') && bytes.get(i + 2) == Some(&b'\n') => {
                i += 3;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = block_comment_end(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}

/// The end of a numeric literal starting at `at`: digits, `.`, letters
/// (suffixes, hex digits, `e`), and a sign directly after an exponent.
fn number_end(bytes: &[u8], at: usize) -> usize {
    let hex = bytes[at] == b'0' && matches!(bytes.get(at + 1), Some(b'x' | b'X'));
    let mut i = at;
    while i < bytes.len() {
        let byte = bytes[i];
        let exponent_sign =
            (byte == b'+' || byte == b'-') && i > at && !hex && matches!(bytes[i - 1], b'e' | b'E');
        if byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || exponent_sign {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// The directive's name and the text after it: `#  define X 1` gives
/// `("define", " X 1")`. `None` for a bare `#`.
pub fn split_directive(text: &str) -> Option<(&str, &str)> {
    let body = text.strip_prefix('#')?.trim_start();
    let end = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(body.len());
    if end == 0 {
        return None;
    }
    Some((body.get(..end)?, body.get(end..)?))
}

/// Tokens of a directive's argument text (everything after its name),
/// with `#` no longer special.
pub fn lex_directive_body(body: &str) -> Vec<Token<'_>> {
    lex_with(body, false)
}

/// Every identifier in `source` outside comments, including the bodies of
/// preprocessor lines, in no particular order and without repeats.
pub fn identifiers(source: &str) -> std::collections::BTreeSet<String> {
    let mut found = std::collections::BTreeSet::new();
    for token in lex(source) {
        match token.kind {
            Kind::Ident => {
                found.insert(token.text.to_string());
            }
            Kind::Directive => {
                if let Some((_, body)) = split_directive(token.text) {
                    for inner in lex_directive_body(body) {
                        if inner.kind == Kind::Ident {
                            found.insert(inner.text.to_string());
                        }
                    }
                }
            }
            Kind::Number | Kind::Punct | Kind::Space | Kind::LineComment | Kind::BlockComment => {}
        }
    }
    found
}

/// `source` with every identifier `from` (outside comments, directive
/// bodies included) replaced by `to`. Everything else is untouched.
pub fn rename_ident(source: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for token in lex(source) {
        match token.kind {
            Kind::Ident if token.text == from => out.push_str(to),
            Kind::Directive => match split_directive(token.text) {
                Some((_, body)) => {
                    let prefix_len = token.text.len() - body.len();
                    out.push_str(token.text.get(..prefix_len).unwrap_or(""));
                    for inner in lex_directive_body(body) {
                        if inner.kind == Kind::Ident && inner.text == from {
                            out.push_str(to);
                        } else {
                            out.push_str(inner.text);
                        }
                    }
                }
                None => out.push_str(token.text),
            },
            Kind::Ident
            | Kind::Number
            | Kind::Punct
            | Kind::Space
            | Kind::LineComment
            | Kind::BlockComment => out.push_str(token.text),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renaming_touches_only_whole_identifiers_outside_comments() {
        let got = rename_ident(
            "gl_FragColor = a; // gl_FragColor\n#define X gl_FragColor\nmy_gl_FragColor;",
            "gl_FragColor",
            "out_c",
        );
        assert_eq!(
            got, "out_c = a; // gl_FragColor\n#define X out_c\nmy_gl_FragColor;",
            "code and macro bodies change, comments and longer names do not"
        );
    }

    fn kinds(source: &str) -> Vec<(Kind, &str)> {
        lex(source).into_iter().map(|t| (t.kind, t.text)).collect()
    }

    #[test]
    fn tokens_concatenate_back_to_the_source() {
        let source = "#version 300 es\r\nvoid main() { // hi\n  float x = 1.5e-3 + .5; /* a\nb */ }\n\
                      #define A(x) \\\n  (x)\n";
        let joined: String = lex(source).iter().map(|t| t.text).collect();
        assert_eq!(joined, source, "a lossless lexer must reproduce its input");
    }

    #[test]
    fn identifiers_numbers_and_comments_are_told_apart() {
        let got = kinds("texture2D(a, 1e3) // texture2D\n/* texture2D */ x1");
        let idents: Vec<&str> = got
            .iter()
            .filter(|(k, _)| *k == Kind::Ident)
            .map(|(_, t)| *t)
            .collect();
        assert_eq!(idents, vec!["texture2D", "a", "x1"], "comments hide names");
        assert!(
            got.contains(&(Kind::Number, "1e3")),
            "an exponent is part of its number, not an identifier `e3`: {got:?}"
        );
    }

    #[test]
    fn number_forms_stay_whole() {
        for literal in [
            "1.0", ".5", "5.", "1e-3", "2.5E+4", "0x1F", "10u", "1.0f", "3.",
        ] {
            let got = kinds(literal);
            assert_eq!(got, vec![(Kind::Number, literal)], "`{literal}`");
        }
        // A minus that is not after an exponent starts a new token.
        assert_eq!(kinds("1-2").len(), 3, "1 - 2");
    }

    #[test]
    fn directives_are_whole_lines_with_continuations() {
        let source = "#define M(a) \\\n  a * 2\nfloat y;\n";
        let got = kinds(source);
        assert_eq!(
            got[0],
            (Kind::Directive, "#define M(a) \\\n  a * 2"),
            "continued line"
        );
        assert!(
            got.contains(&(Kind::Ident, "float")),
            "code after it is code"
        );
    }

    #[test]
    fn a_hash_in_the_middle_of_a_line_is_not_a_directive() {
        let got = kinds("a # b");
        assert!(
            got.iter().all(|(k, _)| *k != Kind::Directive),
            "only a line-start # begins a directive: {got:?}"
        );
    }

    #[test]
    fn a_block_comment_inside_a_directive_may_span_lines() {
        let source = "#define A 1 /* x\ny */ + 2\nint z;";
        let got = kinds(source);
        assert_eq!(got[0].0, Kind::Directive, "one directive");
        assert!(
            got[0].1.ends_with("+ 2"),
            "through the comment: {:?}",
            got[0]
        );
    }

    #[test]
    fn directive_names_and_bodies_split() {
        assert_eq!(
            split_directive("#define X 1"),
            Some(("define", " X 1")),
            "plain"
        );
        assert_eq!(
            split_directive("#  extension GL_X : enable"),
            Some(("extension", " GL_X : enable")),
            "spaced"
        );
        assert_eq!(split_directive("#"), None, "bare hash");
    }

    #[test]
    fn identifiers_include_directive_bodies_but_not_comments() {
        let found = identifiers("#define T texture2D\nfloat a; // gone\n/* gone */");
        let list: Vec<&str> = found.iter().map(String::as_str).collect();
        assert_eq!(
            list,
            vec!["T", "a", "float", "texture2D"],
            "found: {list:?}"
        );
    }

    #[test]
    fn multibyte_text_survives() {
        let source = "// caf\u{e9} \u{2014}\nfloat \u{b0};";
        let joined: String = lex(source).iter().map(|t| t.text).collect();
        assert_eq!(joined, source, "non-ASCII must not split a character");
    }
}
