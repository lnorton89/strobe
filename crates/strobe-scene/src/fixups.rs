// ============================================================================
// Module:       strobe_scene::fixups
// Description:  Source rewrites that let desktop-only SSF idioms compile on
//               GLSL ES, one idiom at a time, without changing line numbers.
//
// Dependencies: std only
// ============================================================================

//! GLSL ES fix-ups for desktop-only SSF idioms (Q8).
//!
//! SSF scenes are written against desktop GLSL, which converts `int` to
//! `float` implicitly. GLSL ES 3.00 does not, so a scene that compiles
//! everywhere Synesthesia runs can still be rejected on the ANGLE path.
//! Each fix-up here is a small, token-aware rewrite of the *user's* code,
//! applied only for [`crate::assemble::Dialect::Es300`], that turns one
//! such idiom into the explicit form both dialects accept. They are
//! deliberately narrow: a rewrite is only worth having when it is
//! unambiguous without knowing any types, because a full fix needs a GLSL
//! parser (Q8 records what is left).
//!
//! Rewrites never add or remove a line, so a driver error still maps to the
//! right line of `main.glsl` ([`crate::assemble::map_error_line`]).
//!
//! ## `PASSINDEX` against a float literal
//!
//! `PASSINDEX` is an `int` (R4: real scenes compare it with `== 0` and with
//! `== 0.0`). `PASSINDEX == 0.0` is `int == float`, an error in ES. The
//! rewrite turns the `PASSINDEX` in a comparison whose other side is a
//! float literal into `float(PASSINDEX)`. A comparison with an int literal,
//! or any other use, is left alone, so the int idiom keeps compiling and
//! the program's one `PASSINDEX` constant still folds the branch away.

use std::borrow::Cow;

/// The identifier being rewritten.
const PASSINDEX: &str = "PASSINDEX";

/// Rewrites `PASSINDEX` compared with a float literal as
/// `float(PASSINDEX)`; returns `source` untouched (no allocation) when
/// there is nothing to rewrite.
pub fn passindex_float_compares(source: &str) -> Cow<'_, str> {
    if !source.contains(PASSINDEX) {
        return Cow::Borrowed(source);
    }
    let bytes = source.as_bytes();
    let mut rewritten = String::new();
    let mut copied = 0;
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let next = bytes.get(i + 1).copied();
        if byte == b'/' && next == Some(b'/') {
            i = skip_line_comment(bytes, i);
        } else if byte == b'/' && next == Some(b'*') {
            i = skip_block_comment(bytes, i);
        } else if byte.is_ascii_digit() {
            // A number, including `1e5` and `0x1F`, so its letters are not
            // read as the start of an identifier.
            while bytes.get(i).is_some_and(|b| is_word(*b) || *b == b'.') {
                i += 1;
            }
        } else if is_word(byte) {
            let start = i;
            while bytes.get(i).is_some_and(|b| is_word(*b)) {
                i += 1;
            }
            if source.get(start..i) == Some(PASSINDEX) && compared_with_float(bytes, start, i) {
                rewritten.push_str(source.get(copied..start).unwrap_or_default());
                rewritten.push_str("float(PASSINDEX)");
                copied = i;
            }
        } else {
            i += 1;
        }
    }
    if copied == 0 {
        return Cow::Borrowed(source);
    }
    rewritten.push_str(source.get(copied..).unwrap_or_default());
    Cow::Owned(rewritten)
}

/// Whether `byte` can be part of an identifier or a number.
fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The index of the newline ending the `//` comment at `start`, or the end.
fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    bytes
        .get(start..)
        .and_then(|rest| rest.iter().position(|b| *b == b'\n'))
        .map_or(bytes.len(), |offset| start + offset)
}

/// The index after the `*/` ending the comment at `start`, or the end.
fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    bytes
        .get(start + 2..)
        .and_then(|rest| rest.windows(2).position(|pair| pair == b"*/"))
        .map_or(bytes.len(), |offset| start + 2 + offset + 2)
}

/// Whether the identifier at `start..end` is an operand of a comparison
/// whose other operand is a float literal.
fn compared_with_float(bytes: &[u8], start: usize, end: usize) -> bool {
    let after = skip_spaces(bytes, end);
    if let Some(length) = comparison_at(bytes, after)
        && float_literal_at(bytes, skip_spaces(bytes, after + length))
    {
        return true;
    }
    let mut before = start;
    while before > 0 && bytes.get(before - 1).is_some_and(u8::is_ascii_whitespace) {
        before -= 1;
    }
    comparison_ending_at(bytes, before) && float_literal_ending_before(bytes, before)
}

fn skip_spaces(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    index
}

/// The length of the comparison operator starting at `index`, if any.
/// `<<` and `>>` are shifts, not comparisons.
fn comparison_at(bytes: &[u8], index: usize) -> Option<usize> {
    let first = bytes.get(index).copied()?;
    let second = bytes.get(index + 1).copied();
    match (first, second) {
        (b'=' | b'!' | b'<' | b'>', Some(b'=')) => Some(2),
        (b'<', Some(b'<')) | (b'>', Some(b'>')) => None,
        (b'<' | b'>', _) => Some(1),
        _ => None,
    }
}

/// Whether a comparison operator ends just before `end` (spaces already
/// skipped).
fn comparison_ending_at(bytes: &[u8], end: usize) -> bool {
    let Some(last) = end.checked_sub(1).and_then(|i| bytes.get(i)).copied() else {
        return false;
    };
    let before_last = end.checked_sub(2).and_then(|i| bytes.get(i)).copied();
    match last {
        b'=' => matches!(before_last, Some(b'=' | b'!' | b'<' | b'>')),
        b'<' | b'>' => before_last != Some(last),
        _ => false,
    }
}

/// Whether a float literal (`0.0`, `.5`, `1.`, `1e3`) starts at `index`.
fn float_literal_at(bytes: &[u8], index: usize) -> bool {
    let mut end = index;
    while let Some(&byte) = bytes.get(end) {
        let exponent_sign = matches!(byte, b'+' | b'-')
            && end > index
            && matches!(bytes.get(end - 1), Some(b'e' | b'E'))
            && !is_hex(bytes, index);
        if is_word(byte) || byte == b'.' || exponent_sign {
            end += 1;
        } else {
            break;
        }
    }
    bytes
        .get(index..end)
        .is_some_and(|token| is_float_token(token))
}

/// Whether a float literal ends just before `end` (spaces already skipped).
fn float_literal_ending_before(bytes: &[u8], end: usize) -> bool {
    // Step back over the comparison operator and the spaces before it.
    let mut index = end;
    while index > 0
        && bytes
            .get(index - 1)
            .is_some_and(|b| matches!(b, b'=' | b'!' | b'<' | b'>'))
    {
        index -= 1;
    }
    while index > 0 && bytes.get(index - 1).is_some_and(u8::is_ascii_whitespace) {
        index -= 1;
    }
    let token_end = index;
    while index > 0
        && bytes
            .get(index - 1)
            .is_some_and(|b| is_word(*b) || *b == b'.')
    {
        index -= 1;
    }
    bytes
        .get(index..token_end)
        .is_some_and(|token| is_float_token(token))
}

/// Whether the literal at `index` is hexadecimal.
fn is_hex(bytes: &[u8], index: usize) -> bool {
    bytes.get(index) == Some(&b'0') && matches!(bytes.get(index + 1), Some(b'x' | b'X'))
}

/// Whether `token` is a floating-point literal: starts with a digit or `.`,
/// is not hexadecimal, and has a `.` or an exponent.
fn is_float_token(token: &[u8]) -> bool {
    let Some(&first) = token.first() else {
        return false;
    };
    if !(first.is_ascii_digit() || first == b'.') || is_hex(token, 0) {
        return false;
    }
    token.contains(&b'.') || token.iter().any(|b| matches!(b, b'e' | b'E'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(source: &str) -> String {
        passindex_float_compares(source).into_owned()
    }

    #[test]
    fn a_float_literal_on_the_right_is_cast() {
        assert_eq!(
            fix("if (PASSINDEX == 0.0) { }"),
            "if (float(PASSINDEX) == 0.0) { }",
            "== 0.0"
        );
        for op in ["!=", "<", ">", "<=", ">="] {
            assert_eq!(
                fix(&format!("x = PASSINDEX {op} 1.5;")),
                format!("x = float(PASSINDEX) {op} 1.5;"),
                "{op}"
            );
        }
        assert_eq!(
            fix("PASSINDEX==2."),
            "float(PASSINDEX)==2.",
            "tight, bare dot"
        );
        assert_eq!(fix("PASSINDEX<.5"), "float(PASSINDEX)<.5", "leading dot");
        assert_eq!(
            fix("PASSINDEX >= 1e2"),
            "float(PASSINDEX) >= 1e2",
            "exponent"
        );
    }

    #[test]
    fn a_float_literal_on_the_left_is_cast() {
        assert_eq!(
            fix("if (1.0 == PASSINDEX) { }"),
            "if (1.0 == float(PASSINDEX)) { }",
            "literal first"
        );
        assert_eq!(fix("0.5<PASSINDEX"), "0.5<float(PASSINDEX)", "tight");
        assert_eq!(fix("2.0 >= PASSINDEX"), "2.0 >= float(PASSINDEX)", ">=");
    }

    #[test]
    fn int_comparisons_and_other_uses_are_left_alone() {
        for source in [
            "if (PASSINDEX == 0) { }",
            "if (PASSINDEX == 1u) { }",
            "if (PASSINDEX == 0x10) { }",
            "int k = PASSINDEX + 1;",
            "x = PASSINDEX << 1.0;",
            "x = PASSINDEX >> 2;",
            "y = float(PASSINDEX) == 0.0;",
            "if (MYPASSINDEX == 0.0) { }",
            "if (PASSINDEX_2 == 0.0) { }",
            "float a = 1.0; if (a == PASSINDEXED) { }",
        ] {
            assert!(
                matches!(passindex_float_compares(source), Cow::Borrowed(_)),
                "`{source}` should be untouched"
            );
        }
    }

    #[test]
    fn comments_are_not_rewritten_and_line_count_is_kept() {
        let source = "// PASSINDEX == 0.0\n/* PASSINDEX == 1.0\n */\nif (PASSINDEX == 2.0) {}\n";
        let fixed = fix(source);
        assert_eq!(
            fixed,
            "// PASSINDEX == 0.0\n/* PASSINDEX == 1.0\n */\nif (float(PASSINDEX) == 2.0) {}\n",
            "only the code was rewritten"
        );
        assert_eq!(
            fixed.lines().count(),
            source.lines().count(),
            "error lines still map"
        );
    }

    #[test]
    fn every_occurrence_in_a_chain_is_cast() {
        assert_eq!(
            fix("if (PASSINDEX == 0.0) a(); else if (PASSINDEX == 1.0) b();"),
            "if (float(PASSINDEX) == 0.0) a(); else if (float(PASSINDEX) == 1.0) b();",
            "both"
        );
    }
}
