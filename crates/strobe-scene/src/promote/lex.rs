// ============================================================================
// Module:       strobe_scene::promote::lex
// Description:  A GLSL tokenizer that keeps byte offsets, skips comments and
//               preprocessor lines, and records object-like and function-like
//               `#define`s and `#undef`s.
//
// Dependencies: none
// ============================================================================

//! Tokens with byte spans, for the int→float pass.
//!
//! The pass edits the text at token boundaries, so every token keeps the
//! byte range it came from. Comments and whitespace are dropped.
//! Preprocessor lines are dropped too: their tokens are not GLSL. `#define`
//! is kept aside, because a macro's name appears in the code and the pass
//! needs to know what type it stands for. Of each `#if` group one branch
//! is kept (see [`lex_with`]).

use crate::directives::Branches;

/// What a token is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A name or keyword.
    Ident,
    /// An integer literal: decimal, octal or hex, with an optional `u`.
    Int,
    /// A floating-point literal.
    Float,
    /// An operator or punctuation, one to three bytes.
    Punct,
}

/// One token and the bytes it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Token {
    /// Its class.
    pub kind: Kind,
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

/// A `#define`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Define<'a> {
    /// The macro's name.
    pub name: &'a str,
    /// `true` for `#define NAME(args) ...`.
    pub function_like: bool,
    /// A function-like macro's parameter names, trimmed, as written: an
    /// empty list for `NAME()`. Empty for an object-like macro.
    pub params: Vec<&'a str>,
    /// The replacement text, trimmed, without a trailing `//` comment.
    pub body: &'a str,
}

/// A tokenized source.
#[derive(Debug, Default)]
pub(super) struct Lexed<'a> {
    /// The tokens, in order.
    pub tokens: Vec<Token>,
    /// The `#define`s, in order.
    pub defines: Vec<Define<'a>>,
    /// The names of the `#undef`s, in order.
    pub undefs: Vec<&'a str>,
}

/// Operators of three and two bytes, longest first.
const LONG_PUNCT: [&str; 21] = [
    "<<=", ">>=", "++", "--", "+=", "-=", "*=", "/=", "%=", "==", "!=", "<=", ">=", "&&", "||",
    "^^", "<<", ">>", "&=", "|=", "^=",
];

/// Splits `text` into tokens, with no macros defined beforehand and every
/// undecided branch kept.
pub(super) fn lex(text: &str) -> Lexed<'_> {
    lex_with(text, &mut Branches::default())
}

/// Splits `text` into tokens, following `branches` through its
/// conditionals: of each `#if` group only the branch [`Branches`] keeps is
/// tokenized. Two branches of one group often declare the same thing
/// twice, which would not parse.
pub(super) fn lex_with<'a>(text: &'a str, branches: &mut Branches) -> Lexed<'a> {
    let bytes = text.as_bytes();
    let mut out = Lexed::default();
    let mut at = 0;
    let mut line_start = true;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'\n' {
            line_start = true;
            at += 1;
            continue;
        }
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes.get(at..at + 2) == Some(b"//") {
            at = line_end(bytes, at);
            continue;
        }
        if bytes.get(at..at + 2) == Some(b"/*") {
            at = block_comment_end(bytes, at);
            continue;
        }
        if byte == b'#' && line_start {
            let end = directive_end(bytes, at);
            let directive = text.get(at + 1..end).unwrap_or_default();
            let active = branches.active();
            if !branches.follow(directive) && active {
                if let Some(define) = parse_define(directive) {
                    out.defines.push(define);
                } else if let Some(name) = parse_undef(directive) {
                    out.undefs.push(name);
                }
            }
            at = end;
            continue;
        }
        line_start = false;
        if !branches.active() {
            at = line_end(bytes, at);
            continue;
        }
        let (kind, end) = if byte.is_ascii_digit()
            || (byte == b'.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        {
            number(bytes, at)
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            (Kind::Ident, word_end(bytes, at))
        } else {
            (Kind::Punct, punct_end(text, at))
        };
        out.tokens.push(Token {
            kind,
            start: at,
            end,
        });
        at = end;
    }
    out
}

/// The end of a `//` comment: the newline, which is left for the caller.
fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes
        .get(from..)
        .and_then(|rest| rest.iter().position(|&b| b == b'\n'))
        .map_or(bytes.len(), |offset| from + offset)
}

/// One past the `*/` that closes the comment at `from`.
fn block_comment_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from + 2;
    while at + 1 < bytes.len() {
        if bytes.get(at..at + 2) == Some(b"*/") {
            return at + 2;
        }
        at += 1;
    }
    bytes.len()
}

/// The end of a preprocessor line, following `\` continuations. The newline
/// is left for the caller.
fn directive_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'\n' {
            let continued = at > 0 && bytes.get(at - 1) == Some(&b'\\')
                || at > 1 && bytes.get(at - 2..at) == Some(b"\\\r");
            if !continued {
                return at;
            }
        }
        at += 1;
    }
    bytes.len()
}

/// Reads `define NAME ...` from a directive's text after the `#`.
fn parse_define(directive: &str) -> Option<Define<'_>> {
    let rest = directive.trim_start().strip_prefix("define")?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start();
    let name_len = rest
        .bytes()
        .position(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
        .unwrap_or(rest.len());
    let name = rest.get(..name_len).filter(|name| !name.is_empty())?;
    let after = rest.get(name_len..)?;
    let function_like = after.starts_with('(');
    let (params, body) = if function_like {
        let close = after.find(')')?;
        let list = after.get(1..close)?.trim();
        let params = if list.is_empty() {
            Vec::new()
        } else {
            list.split(',').map(str::trim).collect()
        };
        (params, after.get(close + 1..)?)
    } else {
        (Vec::new(), after)
    };
    let body = body
        .find("//")
        .map_or(body, |cut| body.get(..cut).unwrap_or(body));
    Some(Define {
        name,
        function_like,
        params,
        body: body.trim(),
    })
}

/// Reads `undef NAME` from a directive's text after the `#`.
fn parse_undef(directive: &str) -> Option<&str> {
    let rest = directive.trim_start().strip_prefix("undef")?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start();
    let name_len = rest
        .bytes()
        .position(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
        .unwrap_or(rest.len());
    rest.get(..name_len).filter(|name| !name.is_empty())
}

/// The end of an identifier.
fn word_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while bytes
        .get(at)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        at += 1;
    }
    at
}

/// Reads a numeric literal and classifies it.
fn number(bytes: &[u8], from: usize) -> (Kind, usize) {
    let hex = bytes.get(from) == Some(&b'0') && matches!(bytes.get(from + 1), Some(b'x' | b'X'));
    let mut at = from;
    while let Some(&byte) = bytes.get(at) {
        let exponent_sign = !hex
            && matches!(byte, b'+' | b'-')
            && at > from
            && matches!(bytes.get(at - 1), Some(b'e' | b'E'));
        if byte.is_ascii_alphanumeric() || byte == b'.' || exponent_sign {
            at += 1;
        } else {
            break;
        }
    }
    let token = bytes.get(from..at).unwrap_or_default();
    let float = !hex
        && token
            .iter()
            .any(|b| matches!(b, b'.' | b'e' | b'E' | b'f' | b'F'));
    (if float { Kind::Float } else { Kind::Int }, at)
}

/// The end of the operator at `from`: the longest one that matches, or one
/// character.
fn punct_end(text: &str, from: usize) -> usize {
    let rest = text.get(from..).unwrap_or_default();
    if let Some(long) = LONG_PUNCT.iter().find(|op| rest.starts_with(**op)) {
        return from + long.len();
    }
    from + rest.chars().next().map_or(1, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(Kind, &str)> {
        lex(text)
            .tokens
            .iter()
            .map(|t| (t.kind, text.get(t.start..t.end).unwrap_or("?")))
            .collect()
    }

    #[test]
    fn numbers_are_classified_as_glsl_does() {
        assert_eq!(
            kinds("1 1.0 .5 1e3 2.5e-3 0x1F 7u 1.0f"),
            vec![
                (Kind::Int, "1"),
                (Kind::Float, "1.0"),
                (Kind::Float, ".5"),
                (Kind::Float, "1e3"),
                (Kind::Float, "2.5e-3"),
                (Kind::Int, "0x1F"),
                (Kind::Int, "7u"),
                (Kind::Float, "1.0f"),
            ],
            "every literal form"
        );
    }

    #[test]
    fn comments_and_directives_are_skipped_and_defines_kept() {
        let text = "#define N 4 // count\n#define R(a, b ) mat2(a)\n#undef Q\n  # if N\nx /* 1 */ += 2; // 3\n";
        let lexed = lex(text);
        assert_eq!(
            kinds(text),
            vec![
                (Kind::Ident, "x"),
                (Kind::Punct, "+="),
                (Kind::Int, "2"),
                (Kind::Punct, ";")
            ],
            "only code tokens"
        );
        assert_eq!(
            lexed.defines,
            vec![
                Define {
                    name: "N",
                    function_like: false,
                    params: Vec::new(),
                    body: "4"
                },
                Define {
                    name: "R",
                    function_like: true,
                    params: vec!["a", "b"],
                    body: "mat2(a)"
                },
            ],
            "both defines, the parameters trimmed and the comment cut off"
        );
        assert_eq!(lexed.undefs, vec!["Q"], "the #undef is recorded");
    }

    #[test]
    fn one_branch_of_each_conditional_is_kept() {
        let text = "#ifdef KODE\na\n#else\nb\n#endif\n#if 0\nc\n#elif defined(X)\nd\n#else\ne\n#endif\n#if FOO > 1\nf\n#else\ng\n#endif\n";
        let mut branches = Branches::new(["X".to_owned()]);
        let lexed = lex_with(text, &mut branches);
        let words: Vec<&str> = lexed
            .tokens
            .iter()
            .filter_map(|t| text.get(t.start..t.end))
            .collect();
        assert_eq!(
            words,
            vec!["b", "d", "f"],
            "decided branches follow the preprocessor, an undecided one keeps the first"
        );
    }

    #[test]
    fn a_continued_directive_is_one_line() {
        assert_eq!(
            kinds("#define A \\\n  1\nb"),
            vec![(Kind::Ident, "b")],
            "the continuation belongs to the directive"
        );
    }
}
