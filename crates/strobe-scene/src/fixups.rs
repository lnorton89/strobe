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
    bytes.get(index..end).is_some_and(is_float_token)
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
    bytes.get(index..token_end).is_some_and(is_float_token)
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

/// One global initializer moved into `main` by
/// [`hoist_global_initializers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hoisted {
    /// The line of the user's code the declaration is on, 1-based, so the
    /// assembler can put a `#line` before the assignment and a driver error
    /// in it still points at `main.glsl`.
    pub line: u32,
    /// `name = expression;`, the expression verbatim (newlines and all).
    pub assignment: String,
}

/// The value types a hoisted global may have. Arrays, structs and samplers
/// are left alone.
const VALUE_TYPES: [&str; 19] = [
    "float", "int", "uint", "bool", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4", "uvec2",
    "uvec3", "uvec4", "bvec2", "bvec3", "bvec4", "mat2", "mat3", "mat4",
];

/// Turns every global `T name = expression;` into `T name;` and returns the
/// assignments, in source order, for the assembler to run at the top of
/// `main`.
///
/// GLSL ES 3.00 requires a global's initializer to be a constant
/// expression; desktop GLSL drivers (Mesa, NVIDIA) accept any expression.
/// Real Synesthesia scenes rely on that: 31 of 65 on the owner's machine
/// open with `vec4 iMouse = vec4(MouseXY*RENDERSIZE, MouseClick,
/// MouseClick);`, which made ANGLE reject them (TORNADO T2, D192).
/// Assigning at the top of `main` is equivalent: a global can only be read
/// from a function, every function runs from `main`, and the assignments
/// keep their order, so one global may still use an earlier one.
///
/// Only a plain declaration of one of [`VALUE_TYPES`] (optionally with a
/// precision qualifier) is touched; `const`, `uniform`, arrays, structs and
/// declarations of several names are left as they are. The `= expression`
/// is blanked to spaces with its newlines kept, so no line moves.
pub fn hoist_global_initializers(source: &str) -> (Cow<'_, str>, Vec<Hoisted>) {
    let bytes = source.as_bytes();
    let mut found: Vec<(usize, usize, Hoisted)> = Vec::new();
    let mut depth = 0_usize;
    let mut statement = 0;
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let next = bytes.get(i + 1).copied();
        if byte == b'/' && next == Some(b'/') {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if byte == b'/' && next == Some(b'*') {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if byte == b'#' && depth == 0 && starts_line(bytes, i) {
            i = skip_directive(bytes, i);
            statement = i;
            continue;
        }
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    statement = i + 1;
                }
            }
            b';' if depth == 0 => {
                if let Some(hoist) = global_initializer(source, statement, i) {
                    found.push(hoist);
                }
                statement = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if found.is_empty() {
        return (Cow::Borrowed(source), Vec::new());
    }
    let mut rewritten = String::with_capacity(source.len());
    let mut copied = 0;
    let mut hoisted = Vec::with_capacity(found.len());
    for (equals, end, hoist) in found {
        rewritten.push_str(source.get(copied..equals).unwrap_or_default());
        for blanked in source.get(equals..end).unwrap_or_default().chars() {
            rewritten.push(if blanked == '\n' { '\n' } else { ' ' });
        }
        copied = end;
        hoisted.push(hoist);
    }
    rewritten.push_str(source.get(copied..).unwrap_or_default());
    (Cow::Owned(rewritten), hoisted)
}

/// Whether only spaces or tabs precede `index` on its line.
fn starts_line(bytes: &[u8], index: usize) -> bool {
    let mut back = index;
    while back > 0 {
        match bytes.get(back - 1) {
            Some(b' ' | b'\t') => back -= 1,
            Some(b'\n') | None => return true,
            Some(_) => return false,
        }
    }
    true
}

/// The index of the newline ending the preprocessor directive at `start`,
/// following `\` line continuations.
fn skip_directive(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while let Some(&byte) = bytes.get(i) {
        if byte == b'\n' && bytes.get(i.wrapping_sub(1)) != Some(&b'\\') {
            return i;
        }
        i += 1;
    }
    bytes.len()
}

/// The next token in `bytes[from..end]` after spaces and comments: an
/// identifier or number as one token, anything else as one byte.
fn next_token(bytes: &[u8], mut from: usize, end: usize) -> Option<(usize, usize)> {
    while from < end {
        let byte = *bytes.get(from)?;
        let next = bytes.get(from + 1).copied();
        if byte.is_ascii_whitespace() {
            from += 1;
        } else if byte == b'/' && next == Some(b'/') {
            from = skip_line_comment(bytes, from);
        } else if byte == b'/' && next == Some(b'*') {
            from = skip_block_comment(bytes, from);
        } else if is_word(byte) {
            let mut to = from;
            while to < end && bytes.get(to).is_some_and(|b| is_word(*b)) {
                to += 1;
            }
            return Some((from, to));
        } else {
            return Some((from, from + 1));
        }
    }
    None
}

/// Reads the statement `source[start..end]` (its `;` at `end`) as
/// `[precision] TYPE name = expression`, and returns where the `=` is, the
/// end, and the assignment to hoist.
fn global_initializer(source: &str, start: usize, end: usize) -> Option<(usize, usize, Hoisted)> {
    const PRECISIONS: [&str; 3] = ["lowp", "mediump", "highp"];
    let bytes = source.as_bytes();
    let word = |(from, to): (usize, usize)| source.get(from..to).unwrap_or_default();

    let mut token = next_token(bytes, start, end)?;
    if PRECISIONS.contains(&word(token)) {
        token = next_token(bytes, token.1, end)?;
    }
    if !VALUE_TYPES.contains(&word(token)) {
        return None;
    }
    let name = next_token(bytes, token.1, end)?;
    if !word(name)
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
    {
        return None;
    }
    let equals = next_token(bytes, name.1, end)?;
    if word(equals) != "=" || bytes.get(equals.1) == Some(&b'=') {
        return None;
    }
    let expression = source.get(equals.1..end)?;
    if expression.trim().is_empty() || has_top_level_comma(expression.as_bytes()) {
        return None;
    }
    let line = 1 + bytes.get(..name.0)?.iter().filter(|b| **b == b'\n').count();
    Some((
        equals.0,
        end,
        Hoisted {
            line: u32::try_from(line).ok()?,
            assignment: format!("{} ={expression};", word(name)),
        },
    ))
}

/// Whether `expression` has a `,` outside any parentheses or brackets,
/// which means a declaration of several names.
fn has_top_level_comma(expression: &[u8]) -> bool {
    let mut depth = 0_i32;
    let mut i = 0;
    while let Some(&byte) = expression.get(i) {
        let next = expression.get(i + 1).copied();
        if byte == b'/' && next == Some(b'/') {
            i = skip_line_comment(expression, i);
            continue;
        }
        if byte == b'/' && next == Some(b'*') {
            i = skip_block_comment(expression, i);
            continue;
        }
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b',' if depth == 0 => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod hoist_tests {
    use super::*;

    #[test]
    fn a_non_constant_global_initializer_moves_into_main() {
        let source = "vec4 iMouse = vec4(MouseXY*RENDERSIZE, MouseClick, MouseClick);\n\
                      vec4 renderMain() { return iMouse; }\n";
        let (code, hoisted) = hoist_global_initializers(source);
        assert_eq!(
            code.lines().next().map(|l| l.replace(' ', "")),
            Some("vec4iMouse;".to_string()),
            "the initializer is blanked, the declaration kept"
        );
        assert_eq!(code.len(), source.len(), "nothing moves");
        assert_eq!(
            hoisted,
            vec![Hoisted {
                line: 1,
                assignment: "iMouse = vec4(MouseXY*RENDERSIZE, MouseClick, MouseClick);".into()
            }],
            "the assignment, with its line"
        );
    }

    #[test]
    fn only_plain_global_value_declarations_are_touched() {
        let source = "const float k = 2.0;\nuniform float u;\nfloat a[2] = float[](1.0, 2.0);\n\
                      float x = 1.0, y = 2.0;\nstruct S { float f; };\n\
                      void f() { float local = TIME; }\n#define Z vec2 z = vec2(1.0);\n";
        let (code, hoisted) = hoist_global_initializers(source);
        assert!(hoisted.is_empty(), "nothing to hoist: {hoisted:?}");
        assert!(matches!(code, Cow::Borrowed(_)), "and no allocation");
    }

    #[test]
    fn several_globals_keep_their_order_lines_and_qualifiers() {
        let source = "// header\nhighp vec2 m = RENDERSIZE * 0.5;\n\nfloat r = length(\n  m);\n\
                      vec4 renderMain() { return vec4(r); }\n";
        let (code, hoisted) = hoist_global_initializers(source);
        assert_eq!(
            code.lines().count(),
            source.lines().count(),
            "line count kept"
        );
        assert!(code.contains("highp vec2 m "), "precision kept: {code}");
        let lines: Vec<u32> = hoisted.iter().map(|h| h.line).collect();
        assert_eq!(lines, vec![2, 4], "declaration lines");
        assert_eq!(
            hoisted.get(1).map(|h| h.assignment.as_str()),
            Some("r = length(\n  m);"),
            "a multi-line expression is kept verbatim"
        );
    }

    #[test]
    fn a_comparison_is_not_an_initializer() {
        let (_, hoisted) = hoist_global_initializers("bool b == c;\n");
        assert!(hoisted.is_empty(), "== is not =");
    }
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
