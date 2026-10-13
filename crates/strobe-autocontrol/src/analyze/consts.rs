// ============================================================================
// Module:       strobe_autocontrol::analyze::consts
// Description:  Finds top-level named constants (`#define NAME 0.3` and
//               `const float NAME = 0.3;`) and checks they are safe to turn
//               into uniforms.
//
// Dependencies: crate::source, strobe-shadertoy (directive tokenizer)
// ============================================================================

//! Named constants.
//!
//! A `#define` or top-level `const` is the cheapest candidate to find and the
//! one an author most clearly meant as a knob. It is also where a rewrite
//! can break a shader that looked fine, because a uniform is not a constant
//! expression. A constant is **unsafe** to rewrite, and is dropped here, if
//! it is used:
//!
//! - in a preprocessor condition (`#if SAMPLES > 4`);
//! - as an array size or a `case` label (`float w[N]`, `case MODE:`);
//! - in another `const` initialiser, or at global scope outside a function;
//! - when it is defined twice, or undefined.
//!
//! What remains might still be used in a way this scan cannot see (through
//! another macro, say). The scoring stage compiles the rewrite, and a
//! constant whose rewrite does not compile is dropped there.

use crate::source::Scan;
use std::collections::BTreeSet;
use strobe_shadertoy::glsl::{self, Kind};

/// Which spelling the constant was declared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Form {
    /// `#define NAME <number>`.
    Define,
    /// `const float|int NAME = <number>;`.
    Const,
}

/// One top-level named constant.
#[derive(Debug, Clone)]
pub(super) struct Named {
    /// The identifier.
    pub name: String,
    /// Its value.
    pub value: f32,
    /// Whether the literal was written as a float (`2.0`) rather than an
    /// integer (`2`).
    pub is_float: bool,
    /// How it was declared.
    pub form: Form,
    /// The byte span the rewrite replaces: a define's body, or a whole
    /// `const` statement.
    pub span: (usize, usize),
    /// The literal's text, without sign handling (`0.25`).
    pub literal: String,
    /// Whether every use of the name is one a uniform can stand in for.
    pub safe: bool,
    /// Set by the caller when the name bounds a `for` loop.
    pub loop_bound: bool,
    /// Set by the caller when the name scales `TIME` (the time scale
    /// candidate then owns it).
    pub time_used: bool,
    /// How many times the name is used outside its definition.
    pub uses: usize,
    /// The components when the constant is a `vec3` of literals (then
    /// `value` is the first of them).
    pub rgb: Option<[f32; 3]>,
}

/// A parsed numeric literal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Num {
    /// The value.
    pub value: f64,
    /// Whether it was spelled as a float.
    pub is_float: bool,
}

/// Parses a GLSL number token. Hex, octal and unsigned literals are not
/// candidates and give `None`.
pub(super) fn parse_number(text: &str) -> Option<Num> {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("0x") || lower.ends_with('u') {
        return None;
    }
    let (body, suffixed) = match lower.strip_suffix("lf") {
        Some(rest) => (rest, true),
        None => match lower.strip_suffix('f') {
            Some(rest) => (rest, true),
            None => (lower.as_str(), false),
        },
    };
    let is_float = suffixed || body.contains(['.', 'e']);
    if !is_float && body.len() > 1 && body.starts_with('0') {
        return None; // octal
    }
    let value: f64 = body.parse().ok()?;
    value.is_finite().then_some(Num { value, is_float })
}

/// Names that are mathematical constants, not tuning values.
const MATH_NAMES: [&str; 14] = [
    "PI",
    "TAU",
    "PI2",
    "TWO_PI",
    "HALF_PI",
    "E",
    "PHI",
    "SQRT2",
    "SQRT3",
    "INV_PI",
    "M_PI",
    "EPS",
    "EPSILON",
    "TOLERANCE",
];

/// Values that are mathematical constants whatever they are called.
const MATH_VALUES: [f64; 9] = [
    std::f64::consts::PI,
    std::f64::consts::TAU,
    std::f64::consts::FRAC_PI_2,
    std::f64::consts::FRAC_PI_4,
    std::f64::consts::FRAC_1_PI,
    std::f64::consts::E,
    std::f64::consts::SQRT_2,
    1.732_050_8,
    1.618_034,
];

/// Constants of well-known hash functions: the sine hash
/// (`fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453)`), the
/// `vec2(127.1, 311.7)` family, and Dave Hoskins' hash-without-sine
/// (`.1031, .1030, .0973, .1099`, `33.33`, `19.19`).
const HASH_VALUES: [f64; 19] = [
    43_758.545_3,
    12.989_8,
    78.233,
    127.1,
    311.7,
    269.5,
    183.3,
    419.2,
    371.9,
    113.5,
    271.9,
    124.6,
    0.103_1,
    0.103_0,
    0.097_3,
    0.109_9,
    33.33,
    19.19,
    23.140_692_632_779_26,
];

/// Whether `value` is one of the [`HASH_VALUES`].
pub(super) fn is_hash_value(value: f64) -> bool {
    HASH_VALUES
        .iter()
        .any(|h| (value.abs() - h).abs() <= 1e-6 * h.max(1.0))
}

/// Whether a constant looks like mathematics rather than a knob.
pub(super) fn is_math_constant(name: &str, value: f64) -> bool {
    let upper = name.to_ascii_uppercase();
    MATH_NAMES.contains(&upper.as_str())
        || value.abs() < 1e-3
        || value.abs() >= 1e6
        || MATH_VALUES.iter().any(|m| (value.abs() - m).abs() < 2e-3)
        || is_hash_value(value)
}

/// Every top-level named constant in the scan, with its safety decided.
pub(super) fn find(scan: &Scan<'_>) -> Vec<Named> {
    let mut found = Vec::new();
    let mut undefined = BTreeSet::new();
    let mut in_conditions = BTreeSet::new();
    for i in 0..scan.len() {
        match scan.kind(i) {
            Some(Kind::Directive) => {
                let Some((word, body)) = glsl::split_directive(scan.t(i)) else {
                    continue;
                };
                match word {
                    "define" => {
                        if let Some(named) = parse_define(scan, i, body) {
                            found.push(named);
                        }
                    }
                    "undef" => {
                        for token in glsl::lex_directive_body(body) {
                            if token.kind == Kind::Ident {
                                undefined.insert(token.text.to_string());
                            }
                        }
                    }
                    "if" | "elif" => {
                        for token in glsl::lex_directive_body(body) {
                            if token.kind == Kind::Ident {
                                in_conditions.insert(token.text.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some(Kind::Ident) if scan.is_ident(i, "const") => {
                if let Some(named) = parse_const(scan, i) {
                    found.push(named);
                }
            }
            Some(Kind::Ident | Kind::Number | Kind::Punct | Kind::Space)
            | Some(Kind::LineComment | Kind::BlockComment)
            | None => {}
        }
    }
    let counts: Vec<usize> = found
        .iter()
        .map(|a| found.iter().filter(|b| b.name == a.name).count())
        .collect();
    for (named, count) in found.iter_mut().zip(counts) {
        named.safe = named.safe
            && count == 1
            && !undefined.contains(&named.name)
            && !in_conditions.contains(&named.name);
        let (uses, unsafe_use) = examine_uses(scan, named);
        named.uses = uses;
        if unsafe_use {
            named.safe = false;
        }
        // A vector constant multiplied by the clock is a set of
        // frequencies, not a colour.
        if named.rgb.is_some() && used_with_clock(scan, &named.name) {
            named.time_used = true;
        }
    }
    found
}

/// Whether `name` is used next to `TIME` or `iTime` (`TIME * FREQS`).
fn used_with_clock(scan: &Scan<'_>, name: &str) -> bool {
    let clock = |i: usize| scan.is_ident(i, "TIME") || scan.is_ident(i, "iTime");
    (0..scan.len()).any(|i| scan.is_ident(i, name) && ((i >= 2 && clock(i - 2)) || clock(i + 2)))
}

/// `#define NAME <number>` at token `i`, if that is what it is.
fn parse_define(scan: &Scan<'_>, i: usize, body: &str) -> Option<Named> {
    let directive = scan.tok(i)?;
    let body_start = directive.start + (directive.text.len() - body.len());
    let mut at = body_start;
    let inner: Vec<(Kind, &str, usize, usize)> = glsl::lex_directive_body(body)
        .into_iter()
        .map(|token| {
            let start = at;
            at += token.text.len();
            (token.kind, token.text, start, at)
        })
        .collect();
    let sig: Vec<&(Kind, &str, usize, usize)> = inner
        .iter()
        .filter(|(kind, ..)| !matches!(kind, Kind::Space | Kind::LineComment | Kind::BlockComment))
        .collect();
    let (name_kind, name, _, name_end) = **sig.first()?;
    if name_kind != Kind::Ident {
        return None;
    }
    // `#define F(x) ...` is a macro with parameters: the `(` touches the name.
    if inner
        .iter()
        .find(|(_, _, start, _)| *start == name_end)
        .is_some_and(|(kind, text, ..)| *kind == Kind::Punct && *text == "(")
    {
        return None;
    }
    let rest = &sig[1..];
    // `#define COL vec3(0.9, 0.4, 0.1)`: a colour constant.
    let texts: Vec<(Kind, &str)> = rest.iter().map(|(kind, text, ..)| (*kind, *text)).collect();
    if let (Some(rgb), Some(first), Some(last)) = (literal_vec3(&texts), rest.first(), rest.last())
    {
        return Some(Named {
            name: name.to_string(),
            value: rgb[0],
            is_float: true,
            form: Form::Define,
            span: (first.2, last.3),
            literal: body
                .get(first.2 - body_start..last.3 - body_start)
                .unwrap_or("")
                .to_string(),
            safe: true,
            loop_bound: false,
            time_used: false,
            uses: 0,
            rgb: Some(rgb),
        });
    }
    let (open, rest) = match rest.first() {
        Some((Kind::Punct, "(", ..)) => (true, &rest[1..]),
        _ => (false, rest),
    };
    let (negative, rest) = match rest.first() {
        Some((Kind::Punct, "-", ..)) => (true, &rest[1..]),
        _ => (false, rest),
    };
    let (number_kind, number_text, ..) = **rest.first()?;
    if number_kind != Kind::Number {
        return None;
    }
    let rest = &rest[1..];
    let rest = if open {
        match rest.first() {
            Some((Kind::Punct, ")", ..)) => &rest[1..],
            _ => return None,
        }
    } else {
        rest
    };
    if !rest.is_empty() {
        return None;
    }
    let num = parse_number(number_text)?;
    let first = sig.get(1)?;
    let last = sig.last()?;
    Some(Named {
        name: name.to_string(),
        value: (if negative { -num.value } else { num.value }) as f32,
        is_float: num.is_float,
        form: Form::Define,
        span: (first.2, last.3),
        literal: format!("{}{number_text}", if negative { "-" } else { "" }),
        safe: true,
        loop_bound: false,
        time_used: false,
        uses: 0,
        rgb: None,
    })
}

/// The three components of `vec3(r, g, b)` written with literal numbers
/// and nothing else, given as `(kind, text)` significant tokens.
fn literal_vec3(tokens: &[(Kind, &str)]) -> Option<[f32; 3]> {
    let [
        (Kind::Ident, "vec3"),
        (Kind::Punct, "("),
        (Kind::Number, r),
        (Kind::Punct, ","),
        (Kind::Number, g),
        (Kind::Punct, ","),
        (Kind::Number, b),
        (Kind::Punct, ")"),
    ] = tokens
    else {
        return None;
    };
    let value = |text: &str| parse_number(text).map(|num| num.value as f32);
    Some([value(r)?, value(g)?, value(b)?])
}

/// `const vec3 NAME = vec3(r, g, b);` starting at `i`, at the top level
/// and alone on its line: a colour constant.
fn parse_const_colour(scan: &Scan<'_>, i: usize) -> Option<Named> {
    if !scan.is_ident(i + 1, "vec3") || !scan.is_any_ident(i + 2) || !scan.is(i + 3, "=") {
        return None;
    }
    let texts: Vec<(Kind, &str)> = (i + 4..i + 12)
        .map(|j| (scan.kind(j).unwrap_or(Kind::Space), scan.t(j)))
        .collect();
    let rgb = literal_vec3(&texts)?;
    if !scan.is(i + 12, ";") {
        return None;
    }
    let start = scan.start(i);
    let end = scan.end(i + 12);
    let tail = scan.rest_of_line(end).trim();
    let clean =
        scan.line_head(start).trim().is_empty() && (tail.is_empty() || tail.starts_with("//"));
    Some(Named {
        name: scan.t(i + 2).to_string(),
        value: rgb[0],
        is_float: true,
        form: Form::Const,
        span: (start, end),
        literal: scan
            .text
            .get(scan.start(i + 4)..scan.end(i + 11))
            .unwrap_or("")
            .to_string(),
        safe: clean,
        loop_bound: false,
        time_used: false,
        uses: 0,
        rgb: Some(rgb),
    })
}

/// `const float|int NAME = <number>;` (or a `const vec3` colour) starting
/// at `i`, at the top level and alone on its line.
fn parse_const(scan: &Scan<'_>, i: usize) -> Option<Named> {
    if scan.brace_depth(i) != 0 || scan.paren_depth(i) != 0 || scan.statement_start(i) != i {
        return None;
    }
    let ty = scan.t(i + 1);
    if ty == "vec3" {
        return parse_const_colour(scan, i);
    }
    if ty != "float" && ty != "int" {
        return None;
    }
    if !scan.is_any_ident(i + 2) || !scan.is(i + 3, "=") {
        return None;
    }
    let negative = scan.is(i + 4, "-");
    let number_at = i + 4 + usize::from(negative);
    if !scan.is_number(number_at) || !scan.is(number_at + 1, ";") {
        return None;
    }
    let num = parse_number(scan.t(number_at))?;
    if ty == "int" && num.is_float {
        return None;
    }
    let start = scan.start(i);
    let end = scan.end(number_at + 1);
    // The statement becomes a `#define`, which owns its whole line.
    let tail = scan.rest_of_line(end).trim();
    let clean =
        scan.line_head(start).trim().is_empty() && (tail.is_empty() || tail.starts_with("//"));
    Some(Named {
        name: scan.t(i + 2).to_string(),
        value: (if negative { -num.value } else { num.value }) as f32,
        is_float: ty == "float",
        form: Form::Const,
        span: (start, end),
        literal: format!("{}{}", if negative { "-" } else { "" }, scan.t(number_at)),
        safe: clean,
        loop_bound: false,
        time_used: false,
        uses: 0,
        rgb: None,
    })
}

/// Counts the uses of `named` outside its definition and reports whether
/// any of them cannot take a uniform.
fn examine_uses(scan: &Scan<'_>, named: &Named) -> (usize, bool) {
    let mut uses = 0;
    let mut bad = false;
    for i in 0..scan.len() {
        if !scan.is_ident(i, &named.name) {
            continue;
        }
        let start = scan.start(i);
        if start >= named.span.0 && start < named.span.1 {
            continue;
        }
        // The name token of a `const` statement is inside its span; a
        // define's name token is inside the directive token and is never
        // visited, so every hit here is a real use.
        uses += 1;
        let global = scan.brace_depth(i) == 0 && scan.paren_depth(i) == 0;
        let array_size = (i > 0 && scan.is(i - 1, "[")) || scan.is(i + 1, "]");
        let case_label = i > 0 && scan.is_ident(i - 1, "case");
        let in_const = scan.is_ident(scan.statement_start(i), "const");
        if global || array_size || case_label || in_const {
            bad = true;
        }
    }
    // Uses inside other macros' bodies are directive text.
    for i in 0..scan.len() {
        if scan.kind(i) != Some(Kind::Directive) {
            continue;
        }
        let Some(("define", body)) = glsl::split_directive(scan.t(i)) else {
            continue;
        };
        // The constant's own `#define` is not a use of it.
        if named.span.0 >= scan.start(i) && named.span.0 < scan.end(i) {
            continue;
        }
        let mentions = glsl::lex_directive_body(body)
            .iter()
            .any(|token| token.kind == Kind::Ident && token.text == named.name);
        if mentions {
            uses += 1;
        }
    }
    (uses, bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(text: &str) -> Vec<Named> {
        find(&Scan::new("main.glsl", text))
    }

    #[test]
    fn numbers_parse_in_every_glsl_spelling() {
        for (text, value, is_float) in [
            ("1", 1.0, false),
            ("1.0", 1.0, true),
            (".5", 0.5, true),
            ("5.", 5.0, true),
            ("1e-3", 0.001, true),
            ("2.5f", 2.5, true),
            ("12", 12.0, false),
        ] {
            let got = parse_number(text);
            assert_eq!(got, Some(Num { value, is_float }), "`{text}`");
        }
        for text in ["0x1F", "10u", "010"] {
            assert_eq!(parse_number(text), None, "`{text}` is not a tuning value");
        }
    }

    #[test]
    fn defines_and_consts_are_found_with_their_values() {
        let found = named("#define GLOW 0.7\nconst float K = -0.25;\nconst int N = 8;\n");
        let summary: Vec<(String, f32, bool)> = found
            .iter()
            .map(|n| (n.name.clone(), n.value, n.is_float))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("GLOW".to_string(), 0.7, true),
                ("K".to_string(), -0.25, true),
                ("N".to_string(), 8.0, false),
            ],
            "name, value and float-ness"
        );
    }

    #[test]
    fn macros_with_parameters_and_expressions_are_not_constants() {
        let found = named("#define F(x) (x * 2.0)\n#define E (1.0 + 2.0)\n#define T texture\n");
        assert!(
            found.is_empty(),
            "none of these is a plain number: {found:?}"
        );
    }

    #[test]
    fn a_parenthesised_define_is_a_constant() {
        let found = named("#define R (0.5)\n");
        assert_eq!(found.len(), 1, "parentheses around a number are fine");
        assert_eq!(found[0].span, (10, 15), "the span covers the parentheses");
    }

    #[test]
    fn uses_that_need_a_constant_make_it_unsafe() {
        for (text, why) in [
            ("#define N 4\nfloat w[N];\n", "array size"),
            (
                "const int M = 2;\nvoid f(int i) { switch (i) { case M: break; } }\n",
                "case label",
            ),
            ("#define N 4\n#if N > 2\n#endif\n", "preprocessor condition"),
            ("#define S 3.0\nconst float T = S * 2.0;\n", "another const"),
            ("#define S 3.0\nfloat g = S;\n", "global initialiser"),
            ("#define S 3.0\n#define S 4.0\n", "defined twice"),
            ("#define S 3.0\n#undef S\n", "undefined"),
        ] {
            let found = named(text);
            let first = found.first();
            assert!(
                first.is_some_and(|n| !n.safe),
                "{why}: should be unsafe, got {found:?}"
            );
        }
    }

    #[test]
    fn a_use_inside_a_function_is_safe() {
        let found = named("#define S 3.0\nfloat f(float x) { return x * S; }\n");
        assert!(found[0].safe, "an ordinary use takes a uniform");
        assert_eq!(found[0].uses, 1, "one use");
    }

    #[test]
    fn a_const_must_own_its_line_to_become_a_define() {
        let found = named("const float A = 1.5; float b;\nconst float B = 2.5; // note\n");
        assert!(
            !found[0].safe,
            "code after the semicolon: cannot become a #define"
        );
        assert!(found[1].safe, "a trailing comment is fine");
    }

    #[test]
    fn local_consts_are_not_top_level() {
        let found = named("void f() { const float h = 0.002; }\n");
        assert!(found.is_empty(), "only top-level constants: {found:?}");
    }

    #[test]
    fn math_constants_are_recognised() {
        for (name, value) in [
            ("PI", 3.0),
            ("K", std::f64::consts::PI),
            ("TWO", std::f64::consts::TAU),
            ("EPS", 0.5),
            ("TINY", 1e-5),
        ] {
            assert!(is_math_constant(name, value), "{name} = {value}");
        }
        assert!(!is_math_constant("GLOW", 0.7), "a tuning value");
    }
}
