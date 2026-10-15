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
//!
//! ## `pow` of a negative base
//!
//! GLSL leaves `pow(x, y)` undefined for `x < 0`, and the drivers disagree.
//! The native Windows drivers compile it as `exp2(y * log2(x))`, which is
//! NaN there. Direct3D's compiler takes `log2(|x|)` instead, so under
//! ANGLE the same call is `|x|^y`, and it folds `pow(x, 1.0)` to `x`. Real
//! scenes depend on the answer: a reaction-diffusion scene stops growing
//! and a pattern scene goes black on ANGLE. On ES, `pow` is written out as
//! the native sequence, so every ES driver gets the native answer (D301).
//! Desktop is left to its driver.

use crate::directives::Branches;
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

/// Rewrites `pow(x, y)` as `exp2(log2(x) * (y))`, the sequence the native
/// drivers compile `pow` to, so a negative base gives NaN on ES as it does
/// there, instead of Direct3D's `|x|^y` (D301). Returns `source` untouched
/// (no allocation) when nothing is rewritten.
///
/// Left alone, because no driver disagrees on them: a base that is a plain
/// unsigned literal, and an exponent that is an integer literal other than
/// 1 or -1 (`pow(x, 2.0)`), which compilers expand into multiplications.
/// Nested calls are each rewritten, comments are copied, the arguments keep
/// their order and no newline is added, so lines still map.
pub fn pow_as_exp2_log2(source: &str) -> Cow<'_, str> {
    if !source.contains("pow") {
        return Cow::Borrowed(source);
    }
    let rewritten = rewrite_pows(source);
    if rewritten == source {
        Cow::Borrowed(source)
    } else {
        Cow::Owned(rewritten)
    }
}

/// [`pow_as_exp2_log2`] on any span of code, recursing into arguments.
fn rewrite_pows(source: &str) -> String {
    const POW: &str = "pow";
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len() + 16);
    let mut copied = 0;
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let next = bytes.get(i + 1).copied();
        if byte == b'/' && next == Some(b'/') {
            i = skip_line_comment(bytes, i);
        } else if byte == b'/' && next == Some(b'*') {
            i = skip_block_comment(bytes, i);
        } else if byte.is_ascii_digit() {
            while bytes.get(i).is_some_and(|b| is_word(*b) || *b == b'.') {
                i += 1;
            }
        } else if is_word(byte) {
            let start = i;
            while bytes.get(i).is_some_and(|b| is_word(*b)) {
                i += 1;
            }
            let member = start > 0 && bytes.get(start - 1) == Some(&b'.');
            let open = skip_spaces(bytes, i);
            if source.get(start..i) != Some(POW) || member || bytes.get(open) != Some(&b'(') {
                continue;
            }
            let Some((comma, close)) = two_arguments(bytes, open + 1) else {
                continue;
            };
            let base = source.get(open + 1..comma).unwrap_or_default();
            let exponent = source.get(comma + 1..close).unwrap_or_default();
            out.push_str(source.get(copied..start).unwrap_or_default());
            if driver_agrees(base, exponent) {
                out.push_str(source.get(start..=open).unwrap_or_default());
                out.push_str(&rewrite_pows(base));
                out.push(',');
                out.push_str(&rewrite_pows(exponent));
                out.push(')');
            } else {
                out.push_str("exp2(log2(");
                out.push_str(&rewrite_pows(base));
                out.push_str(") * (");
                out.push_str(&rewrite_pows(exponent));
                out.push_str("))");
            }
            i = close + 1;
            copied = i;
        } else {
            i += 1;
        }
    }
    out.push_str(source.get(copied..).unwrap_or_default());
    out
}

/// The offsets of the `,` between a call's two arguments and of the `)`
/// closing it, the arguments starting at `from`; `None` unless the call
/// has exactly two.
fn two_arguments(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut depth = 0_i32;
    let mut comma = None;
    let mut i = from;
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
        match byte {
            b'(' | b'[' => depth += 1,
            b')' if depth == 0 => return comma.map(|comma| (comma, i)),
            b']' if depth == 0 => return None,
            b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                if comma.is_some() {
                    return None;
                }
                comma = Some(i);
            }
            // A statement end inside a call means the text is not one.
            b';' | b'{' | b'}' => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether every driver gives `pow(base, exponent)` the same value, so it
/// is left as `pow`: the base is a plain unsigned literal, or the exponent
/// is an integer literal other than 1 and -1.
fn driver_agrees(base: &str, exponent: &str) -> bool {
    let number = |text: &str| -> Option<f64> {
        let text = text.trim();
        let digits = text.strip_suffix(['f', 'F']).unwrap_or(text);
        let unsigned = digits.strip_prefix('-').unwrap_or(digits);
        if !unsigned
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_digit() || b == b'.')
        {
            return None;
        }
        digits.parse::<f64>().ok()
    };
    let base = base.trim();
    if !base.starts_with('-') && number(base).is_some() {
        return true;
    }
    number(exponent).is_some_and(|n| n.fract() == 0.0 && n.abs() != 1.0)
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
/// Only a plain declaration of one of [`VALUE_TYPES`] or of a struct the
/// source declares (optionally with a precision qualifier) is touched,
/// including a declaration of several names; `const`, `uniform` and arrays
/// are left as they are. The `= expression` is blanked to spaces with its
/// newlines kept, so no line moves.
pub fn hoist_global_initializers(
    source: &str,
    mut branches: Branches,
) -> (Cow<'_, str>, Vec<Hoisted>) {
    let bytes = source.as_bytes();
    let structs = struct_names(source);
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
        if byte == b'#' && starts_line(bytes, i) {
            let end = skip_directive(bytes, i);
            branches.follow(source.get(i + 1..end).unwrap_or_default());
            i = end;
            statement = i;
            continue;
        }
        if !branches.active() {
            // Indentation first, so an indented `#endif` is still seen.
            i = if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
                i + 1
            } else {
                skip_line_comment(bytes, i)
            };
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
                found.extend(global_initializers(source, &structs, statement, i));
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
/// `[precision] TYPE name [= expression] [, name [= expression]]...` and
/// returns, for each initializer, where its `=` starts, where its
/// expression ends, and the assignment to hoist. Names and commas stay, so
/// `float a = x, b = y;` becomes `float a , b ;` (TORNADO T2: one real
/// scene declares five audio globals in one statement).
fn global_initializers(
    source: &str,
    structs: &[&str],
    start: usize,
    end: usize,
) -> Vec<(usize, usize, Hoisted)> {
    const PRECISIONS: [&str; 3] = ["lowp", "mediump", "highp"];
    let bytes = source.as_bytes();
    let word = |(from, to): (usize, usize)| source.get(from..to).unwrap_or_default();
    let mut out = Vec::new();

    let Some(mut token) = next_token(bytes, start, end) else {
        return out;
    };
    if PRECISIONS.contains(&word(token)) {
        let Some(next) = next_token(bytes, token.1, end) else {
            return out;
        };
        token = next;
    }
    if !VALUE_TYPES.contains(&word(token)) && !structs.contains(&word(token)) {
        return out;
    }
    let mut cursor = token.1;
    loop {
        let Some(name) = next_token(bytes, cursor, end) else {
            return out;
        };
        if !word(name)
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        {
            return Vec::new();
        }
        let Some(after) = next_token(bytes, name.1, end) else {
            return out;
        };
        match word(after) {
            "," => cursor = after.1,
            "=" if bytes.get(after.1) != Some(&b'=') => {
                let stop = top_level_comma(bytes, after.1, end).unwrap_or(end);
                let Some(expression) = source.get(after.1..stop) else {
                    return Vec::new();
                };
                if expression.trim().is_empty() {
                    return Vec::new();
                }
                let line = 1 + bytes
                    .get(..name.0)
                    .map_or(0, |b| b.iter().filter(|c| **c == b'\n').count());
                let Ok(line) = u32::try_from(line) else {
                    return Vec::new();
                };
                out.push((
                    after.0,
                    stop,
                    Hoisted {
                        line,
                        assignment: format!("{} ={expression};", word(name)),
                    },
                ));
                if stop == end {
                    return out;
                }
                cursor = stop + 1;
            }
            // An array, a function prototype, or anything else: leave the
            // whole statement alone.
            _ => return Vec::new(),
        }
    }
}

/// The names the source declares with `struct Name`, so a global of a user
/// struct type is hoisted too (TORNADO T3: `ring white = ring(..., syn_BassLevel);`).
fn struct_names(source: &str) -> Vec<&str> {
    let bytes = source.as_bytes();
    let mut names = Vec::new();
    let mut cursor = 0;
    while let Some(token) = next_token(bytes, cursor, bytes.len()) {
        cursor = token.1;
        if source.get(token.0..token.1) == Some("struct")
            && let Some(name) = next_token(bytes, cursor, bytes.len())
            && let Some(text) = source.get(name.0..name.1)
            && text
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        {
            names.push(text);
            cursor = name.1;
        }
    }
    names
}

/// The first `,` in `bytes[from..end]` outside parentheses and brackets.
fn top_level_comma(bytes: &[u8], from: usize, end: usize) -> Option<usize> {
    let mut depth = 0_i32;
    let mut i = from;
    while i < end {
        let byte = *bytes.get(i)?;
        let next = bytes.get(i + 1).copied();
        if byte == b'/' && next == Some(b'/') {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if byte == b'/' && next == Some(b'*') {
            i = skip_block_comment(bytes, i);
            continue;
        }
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b',' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Turns `uniform T name = value;` into the global `T name = value;`.
///
/// Desktop GLSL lets a uniform carry an initial value; GLSL ES rejects it
/// (TORNADO T4: a real scene declares `uniform vec2 mousePos = ...;`).
/// Nothing sets such a uniform (it is neither a control nor one of the
/// generated uniforms, which never have initializers), so it always holds
/// its initial value, which is exactly what a global with that initializer
/// holds. The keyword is blanked to spaces, so nothing moves, and
/// [`hoist_global_initializers`] then handles a value that is not a
/// constant expression.
pub fn uniform_initializers(source: &str, branches: Branches) -> Cow<'_, str> {
    let bytes = source.as_bytes();
    let mut keywords = Vec::new();
    for (start, end) in top_level_statements(source, branches) {
        let Some((first, after)) = next_token(bytes, start, end) else {
            continue;
        };
        if bytes.get(first..after) != Some(b"uniform") {
            continue;
        }
        if has_initializer(bytes, after, end) {
            keywords.push(first);
        }
    }
    if keywords.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut out = String::with_capacity(source.len());
    let mut copied = 0;
    for keyword in keywords {
        out.push_str(source.get(copied..keyword).unwrap_or_default());
        out.push_str("       ");
        copied = keyword + "uniform".len();
    }
    out.push_str(source.get(copied..).unwrap_or_default());
    Cow::Owned(out)
}

/// The `start..end` ranges of every statement at brace depth 0, each
/// ending at its `;`. Comments and directives are skipped.
fn top_level_statements(source: &str, mut branches: Branches) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut statements = Vec::new();
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
        if byte == b'#' && starts_line(bytes, i) {
            let end = skip_directive(bytes, i);
            branches.follow(source.get(i + 1..end).unwrap_or_default());
            i = end;
            statement = i;
            continue;
        }
        if !branches.active() {
            // Indentation first, so an indented `#endif` is still seen.
            i = if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
                i + 1
            } else {
                skip_line_comment(bytes, i)
            };
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
                statements.push((statement, i));
                statement = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    statements
}

/// Whether `bytes[from..end]` holds an `=` that is not part of `==`,
/// `<=`, `>=` or `!=`.
fn has_initializer(bytes: &[u8], from: usize, end: usize) -> bool {
    let mut at = from;
    while let Some((start, stop)) = next_token(bytes, at, end) {
        if bytes.get(start) == Some(&b'=') {
            let before = start.checked_sub(1).and_then(|i| bytes.get(i));
            let after = bytes.get(start + 1);
            let compound =
                matches!(before, Some(b'=' | b'<' | b'>' | b'!')) || after == Some(&b'=');
            if !compound {
                return true;
            }
        }
        at = stop;
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
        let (code, hoisted) = hoist_global_initializers(source, Branches::default());
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
                      struct S { float f; };\n\
                      void f() { float local = TIME; }\n#define Z vec2 z = vec2(1.0);\n";
        let (code, hoisted) = hoist_global_initializers(source, Branches::default());
        assert!(hoisted.is_empty(), "nothing to hoist: {hoisted:?}");
        assert!(matches!(code, Cow::Borrowed(_)), "and no allocation");
    }

    #[test]
    fn several_globals_keep_their_order_lines_and_qualifiers() {
        let source = "// header\nhighp vec2 m = RENDERSIZE * 0.5;\n\nfloat r = length(\n  m);\n\
                      vec4 renderMain() { return vec4(r); }\n";
        let (code, hoisted) = hoist_global_initializers(source, Branches::default());
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
    fn a_declaration_of_several_names_hoists_each_initializer() {
        let source = "float a = mix(1.0, 2.0, syn_BassLevel), b, c = TIME;\n";
        let (code, hoisted) = hoist_global_initializers(source, Branches::default());
        assert_eq!(
            code.replace(' ', ""),
            "floata,b,c;\n",
            "names and commas kept: {code}"
        );
        let assignments: Vec<&str> = hoisted.iter().map(|h| h.assignment.as_str()).collect();
        assert_eq!(
            assignments,
            vec!["a = mix(1.0, 2.0, syn_BassLevel);", "c = TIME;"],
            "each initializer, the call's commas untouched"
        );
    }

    #[test]
    fn a_global_of_a_user_struct_type_is_hoisted() {
        let source = "struct ring { float r; float w; };\n\
                      ring white = ring(1.0, syn_BassLevel);\n";
        let (_, hoisted) = hoist_global_initializers(source, Branches::default());
        assert_eq!(
            hoisted.first().map(|h| h.assignment.as_str()),
            Some("white = ring(1.0, syn_BassLevel);"),
            "the struct global's initializer moves into main"
        );
    }

    #[test]
    fn a_comparison_is_not_an_initializer() {
        let (_, hoisted) = hoist_global_initializers("bool b == c;\n", Branches::default());
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

#[cfg(test)]
mod pow_tests {
    use super::*;

    fn fix(source: &str) -> String {
        pow_as_exp2_log2(source).into_owned()
    }

    #[test]
    fn pow_becomes_exp2_of_log2_in_argument_order() {
        assert_eq!(
            fix("float g = pow(n - 0.5, 1.0);"),
            "float g = exp2(log2(n - 0.5) * ( 1.0));",
            "exponent 1, which some compilers fold to the base"
        );
        assert_eq!(
            fix("v = pow (tex(a, b).rgb, vec3(0.25));"),
            "v = exp2(log2(tex(a, b).rgb) * ( vec3(0.25)));",
            "a call with its own commas, and a space before the parenthesis"
        );
        assert_eq!(
            fix("x = pow(pow(a, k) - 1.0, f(i, j));"),
            "x = exp2(log2(exp2(log2(a) * ( k)) - 1.0) * ( f(i, j)));",
            "nested calls are each rewritten"
        );
        assert_eq!(
            fix("x = pow(-2.0, y) + pow(a, -1.0);"),
            "x = exp2(log2(-2.0) * ( y)) + exp2(log2(a) * ( -1.0));",
            "a negative literal base, and exponent -1"
        );
    }

    #[test]
    fn calls_every_driver_agrees_on_are_left_alone() {
        for source in [
            "x = pow(2.0, y);",
            "x = pow(.5, y);",
            "x = pow(a, 2.0) + pow(a, 3) + pow(a, -2.0) + pow(a, 0.0);",
            "x = power(a, b) + mypow(a, b) + pow_2(a);",
            "// pow(a, b)\n/* pow(c, d) */",
            "float pow;",
            "x = pow(a);",
        ] {
            assert!(
                matches!(pow_as_exp2_log2(source), Cow::Borrowed(_)),
                "`{source}` should be untouched"
            );
        }
        assert_eq!(
            fix("x = pow(pow(a, 0.5), 2.0);"),
            "x = pow(exp2(log2(a) * ( 0.5)), 2.0);",
            "an integer power keeps pow, and its base is still rewritten"
        );
    }

    #[test]
    fn lines_are_kept_and_a_split_call_is_still_found() {
        let source = "float a = pow(\n  n - 0.5, // base\n  0.5);\n#define P(x) pow(x, 0.25)\n";
        let fixed = fix(source);
        assert_eq!(
            fixed,
            "float a = exp2(log2(\n  n - 0.5) * ( // base\n  0.5));\n#define P(x) exp2(log2(x) * ( 0.25))\n",
            "multi-line call with a comment, and a macro body"
        );
        assert_eq!(
            fixed.lines().count(),
            source.lines().count(),
            "error lines still map"
        );
    }
}

#[cfg(test)]
mod uniform_tests {
    use super::*;

    #[test]
    fn a_uniform_with_a_value_becomes_a_global_in_place() {
        let source = "uniform vec2 mousePos = vec2(0.5, 0.5);\nuniform float plain;\n";
        assert_eq!(
            uniform_initializers(source, Branches::default()),
            "        vec2 mousePos = vec2(0.5, 0.5);\nuniform float plain;\n",
            "only the initialised uniform loses its keyword, column for column"
        );
    }

    #[test]
    fn uniforms_without_values_and_locals_are_left_alone() {
        let source =
            "uniform float a;\nvoid f() { uniform_thing = 1; }\n// uniform float b = 1.0;\n";
        assert!(
            matches!(
                uniform_initializers(source, Branches::default()),
                Cow::Borrowed(_)
            ),
            "nothing to rewrite"
        );
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;

    #[test]
    fn a_dead_branch_is_neither_hoisted_nor_rewritten() {
        let source = "#ifdef KODELIFE\nfloat beam = 3.5 * x;\nuniform float u = 1.0;\n  #endif\nfloat live = 2.0 * x;\n";
        let (_, hoisted) = hoist_global_initializers(source, Branches::default());
        assert_eq!(
            hoisted
                .iter()
                .map(|h| h.assignment.as_str())
                .collect::<Vec<_>>(),
            vec!["live = 2.0 * x;"],
            "the KODELIFE block is never compiled; assigning its names would hit the controls' uniforms"
        );
        assert!(
            matches!(
                uniform_initializers(source, Branches::default()),
                Cow::Borrowed(_)
            ),
            "nor is its uniform rewritten"
        );
    }
}
