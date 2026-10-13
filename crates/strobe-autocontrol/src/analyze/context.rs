// ============================================================================
// Module:       strobe_autocontrol::analyze::context
// Description:  What surrounds a literal: the calls it is passed through, the
//               function it is in, the name it is assigned to, and whether it
//               is a fade polynomial's or a hash's arithmetic.
//
// Dependencies: crate::source, crate::naming, super::consts
// ============================================================================

//! Context around a literal.
//!
//! The detectors in [`super::expr`] decide *whether* a literal is a knob;
//! this module answers the questions they share about its surroundings:
//!
//! - **What to call it** (D259). A name from the code beats a stem with a
//!   number: the variable the value is assigned to (`float spokes = ...`
//!   gives `spokes_speed`), the user function it is passed to
//!   (`fbm(p * 3.0)` gives `fbm_scale`), the helper it sits in
//!   (`vec3 ramp(..)` gives `ramp_color`). Names of one or two letters and
//!   names that say nothing (`col`, `uv`, `res`) are not used.
//! - **Its role**, which tells two candidates with one name apart: the
//!   ends of a `mix` are `low` and `high`, a vector constructor's
//!   arguments `x`, `y`, `z`, `w`.
//! - **Traps** that are mathematics, not art: the smoothstep and quintic
//!   fade polynomials (`t*t*(3.0-2.0*t)`, `t*t*t*(t*(t*6.0-15.0)+10.0)`)
//!   and hash arithmetic (`fract(sin(..) * 43758.5453)`, `12.9898`,
//!   `78.233`, Dave Hoskins' `.1031`).
//! - **Geometry**: a `vec3` that is a light position or a box size is not a
//!   colour, even with every component in `0..1`.

use super::consts::{is_hash_value, parse_number};
use crate::naming::{is_glsl_word, readable};
use crate::source::Scan;

/// The entry points; a literal in one of them takes no name from it.
const ENTRY_FUNCTIONS: [&str; 3] = ["renderMain", "mainImage", "main"];

/// Names that say nothing about what a value is, so they do not make a good
/// control name. Any name of one or two letters is generic as well.
const GENERIC_NAMES: [&str; 22] = [
    "pos",
    "coord",
    "coords",
    "col",
    "color",
    "colour",
    "res",
    "result",
    "ret",
    "frag_color",
    "out_color",
    "frag_coord",
    "time",
    "tmp",
    "temp",
    "val",
    "value",
    "out",
    "output",
    "final",
    "uvs",
    "rgb",
];

/// Words in a name that make a `vec3` a colour whatever else the name says
/// (`light_col` is a colour, `light_dir` is not).
const COLOUR_WORDS: [&str; 22] = [
    "col",
    "color",
    "colour",
    "tint",
    "rgb",
    "albedo",
    "diffuse",
    "emission",
    "emissive",
    "glow",
    "sky",
    "fog",
    "haze",
    "ambient",
    "amb",
    "palette",
    "pal",
    "hue",
    "ink",
    "paint",
    "bg",
    "background",
];

/// Words in a name that make a `vec3` a position, a direction or a size.
const GEOMETRY_WORDS: [&str; 45] = [
    "ro",
    "rd",
    "dir",
    "direction",
    "pos",
    "position",
    "p",
    "n",
    "normal",
    "nor",
    "l",
    "light",
    "lig",
    "sun",
    "lightdir",
    "offset",
    "off",
    "size",
    "k",
    "h",
    "e",
    "axis",
    "up",
    "origin",
    "target",
    "center",
    "centre",
    "cam",
    "camera",
    "eye",
    "look",
    "lookat",
    "ta",
    "lp",
    "lk",
    "ld",
    "lo",
    "la",
    "vel",
    "velocity",
    "forward",
    "right",
    "box",
    "radius",
    "dims",
];

/// Functions that take geometry: a `vec3` passed to one is a size or a
/// position. Names whose first word is `sd` or `op` (`sdBox`, `opRep`)
/// count too.
const GEOMETRY_CALLS: [&str; 8] = [
    "map",
    "sdf",
    "de",
    "df",
    "translate",
    "rotate",
    "repeat",
    "intersect",
];

/// Whether a readable name says nothing.
fn is_generic(name: &str) -> bool {
    name.chars().count() <= 2 || GENERIC_NAMES.contains(&name)
}

/// `name` made readable, if it says something.
pub(super) fn specific(name: &str) -> Option<String> {
    let name = readable(name);
    (!is_generic(&name)).then_some(name)
}

/// The calls that enclose token `i` within its statement, innermost
/// first, each with the index of the argument `i` is in.
pub(super) fn call_chain<'a>(scan: &Scan<'a>, i: usize) -> Vec<(&'a str, usize)> {
    let mut chain = Vec::new();
    let mut depth = 0_u32;
    let mut commas = 0;
    let mut j = i;
    while j > 0 {
        j -= 1;
        if depth == 0 && (scan.is(j, ";") || scan.is(j, "{") || scan.is(j, "}")) {
            break;
        }
        if scan.is(j, ")") || scan.is(j, "]") {
            depth += 1;
        } else if scan.is(j, "(") || scan.is(j, "[") {
            if depth == 0 {
                if scan.is(j, "(") && j > 0 && scan.is_any_ident(j - 1) {
                    chain.push((scan.t(j - 1), commas));
                }
                commas = 0;
            } else {
                depth -= 1;
            }
        } else if depth == 0 && scan.is(j, ",") {
            commas += 1;
        }
    }
    chain
}

/// The first function of the author's (not a GLSL built-in or
/// constructor) that token `i` is passed to, made readable.
pub(super) fn user_call(scan: &Scan<'_>, i: usize) -> Option<String> {
    call_chain(scan, i)
        .into_iter()
        .map(|(name, _)| name)
        .find(|name| !is_glsl_word(name) && !name.starts_with("syn_"))
        .and_then(specific)
}

/// The identifier of the enclosing top-level function, if the token is in one.
pub(super) fn enclosing_function<'a>(scan: &Scan<'a>, i: usize) -> Option<&'a str> {
    let (open, _) = function_span(scan, i)?;
    (open > 0 && scan.is_any_ident(open - 1)).then(|| scan.t(open - 1))
}

/// The enclosing top-level function of token `i`: the index of its
/// parameter list's `(` and of its body's closing `}`.
pub(super) fn function_span(scan: &Scan<'_>, i: usize) -> Option<(usize, usize)> {
    let body = (0..i)
        .rev()
        .find(|j| scan.is(*j, "{") && scan.brace_depth(*j) == 0)?;
    if body == 0 || !scan.is(body - 1, ")") {
        return None;
    }
    let mut depth = 0_u32;
    for j in (0..body).rev() {
        if scan.is(j, ")") {
            depth += 1;
        } else if scan.is(j, "(") {
            depth -= 1;
            if depth == 0 {
                let close = scan.matching(body).unwrap_or(scan.len());
                return Some((j, close));
            }
        }
    }
    None
}

/// The readable name of the helper function token `i` is in; `None` in an
/// entry point (including names like `render_main_image`).
pub(super) fn helper(scan: &Scan<'_>, i: usize) -> Option<String> {
    let function = enclosing_function(scan, i)?;
    if ENTRY_FUNCTIONS.contains(&function) {
        return None;
    }
    let name = readable(function);
    if name.split('_').any(|word| word == "main") {
        return None;
    }
    specific(&name)
}

/// What tells token `i` apart from a sibling with the same name: `low` or
/// `high` as an end of a `mix`, `x`..`w` as a vector constructor's
/// argument.
pub(super) fn role(scan: &Scan<'_>, i: usize) -> Option<String> {
    const COMPONENTS: [&str; 4] = ["x", "y", "z", "w"];
    for (name, argument) in call_chain(scan, i) {
        match name {
            "mix" => {
                return match argument {
                    0 => Some("low".to_string()),
                    1 => Some("high".to_string()),
                    _ => None,
                };
            }
            "vec2" | "vec3" | "vec4" => {
                return COMPONENTS.get(argument).map(|c| (*c).to_string());
            }
            _ => {}
        }
    }
    None
}

/// Whether a readable name is a position, direction or size rather than a
/// colour (`light_dir`, `lk`, `ro`, `box_size`; not `light_col`).
pub(super) fn is_geometry_name(name: &str) -> bool {
    let words: Vec<&str> = name.split('_').collect();
    if words.iter().any(|word| COLOUR_WORDS.contains(word)) || name.ends_with("col") {
        return false;
    }
    words.iter().any(|word| GEOMETRY_WORDS.contains(word))
}

/// Whether a function by this name takes or computes geometry (`map`,
/// `sdBox`, `opRep`, `rotate`).
pub(super) fn is_geometry_function(name: &str) -> bool {
    let snake = readable(name);
    let first = snake.split('_').next().unwrap_or("");
    GEOMETRY_CALLS.contains(&snake.as_str()) || first == "sd" || first == "op"
}

/// The value of number token `i`, if it is one.
fn value_at(scan: &Scan<'_>, i: usize) -> Option<f64> {
    scan.is_number(i)
        .then(|| parse_number(scan.t(i)))
        .flatten()
        .map(|num| num.value)
}

/// The tokens of the statement token `i` is in: `[start, end)`.
fn statement(scan: &Scan<'_>, i: usize) -> (usize, usize) {
    let start = scan.statement_start(i);
    let end = (i..scan.len())
        .find(|j| scan.is(*j, ";") || scan.is(*j, "{") || scan.is(*j, "}"))
        .unwrap_or(scan.len());
    (start, end)
}

/// Whether number token `i` is a coefficient of a fade polynomial: the
/// `3.0` and `2.0` of smoothstep's `t*t*(3.0-2.0*t)`, or the `6.0`,
/// `15.0` and `10.0` of the quintic `t*t*t*(t*(t*6.0-15.0)+10.0)`.
/// Changing one does not reshape the picture, it breaks the curve.
pub(super) fn in_fade_polynomial(scan: &Scan<'_>, i: usize) -> bool {
    let near = |value: Option<f64>, target: f64| value.is_some_and(|v| (v - target).abs() < 1e-9);
    let Some(value) = value_at(scan, i) else {
        return false;
    };
    if ![2.0, 3.0, 6.0, 10.0, 15.0]
        .iter()
        .any(|t| near(Some(value), *t))
    {
        return false;
    }
    let (start, end) = statement(scan, i);
    // Cubic: `3.0 - 2.0 * t` or `3.0 - t * 2.0`.
    for k in start..end {
        if !near(value_at(scan, k), 3.0) || !scan.is(k + 1, "-") {
            continue;
        }
        if near(value_at(scan, k + 2), 2.0) && scan.is(k + 3, "*") && (i == k || i == k + 2) {
            return true;
        }
        if scan.is_any_ident(k + 2)
            && scan.is(k + 3, "*")
            && near(value_at(scan, k + 4), 2.0)
            && (i == k || i == k + 4)
        {
            return true;
        }
    }
    // Quintic: 6, 15 (subtracted) and 10 in one statement.
    let has = |target: f64| (start..end).any(|k| near(value_at(scan, k), target));
    let minus_fifteen =
        (start..end).any(|k| near(value_at(scan, k), 15.0) && k > 0 && scan.is(k - 1, "-"));
    minus_fifteen
        && has(6.0)
        && has(10.0)
        && [6.0, 15.0, 10.0].iter().any(|t| near(Some(value), *t))
}

/// Whether number token `i` is hash arithmetic: a well-known hash
/// constant, a large number with a fraction (`43758.5453`), or anything
/// inside a `fract(..)` that also holds a `sin` or `cos`.
pub(super) fn in_hash(scan: &Scan<'_>, i: usize) -> bool {
    let Some(value) = value_at(scan, i) else {
        return false;
    };
    if is_hash_value(value) || (value.abs() >= 1000.0 && value.fract() != 0.0) {
        return true;
    }
    let (start, _) = statement(scan, i);
    (start..i).any(|j| {
        scan.is_ident(j, "fract")
            && scan.is(j + 1, "(")
            && scan.matching(j + 1).is_some_and(|close| {
                i < close
                    && (j + 2..close).any(|k| scan.is_ident(k, "sin") || scan.is_ident(k, "cos"))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at<'a>(scan: &Scan<'a>, text: &str, nth: usize) -> Option<usize> {
        (0..scan.len()).filter(|i| scan.t(*i) == text).nth(nth)
    }

    #[test]
    fn the_call_chain_lists_enclosing_calls_with_argument_positions() -> anyhow::Result<()> {
        let code = "void f() { x = mix(a, vec2(cos(t * 1.3), sin(t * 1.7)), k); }";
        let scan = Scan::new("f.glsl", code);
        let i = at(&scan, "1.7", 0).ok_or_else(|| anyhow::anyhow!("1.7 is a token"))?;
        assert_eq!(
            call_chain(&scan, i),
            vec![("sin", 0), ("vec2", 1), ("mix", 1)],
            "innermost first"
        );
        assert_eq!(
            role(&scan, i).as_deref(),
            Some("y"),
            "the vec2's second argument"
        );
        let first = at(&scan, "a", 0).ok_or_else(|| anyhow::anyhow!("a is a token"))?;
        assert_eq!(
            role(&scan, first).as_deref(),
            Some("low"),
            "mix's first end"
        );
        Ok(())
    }

    #[test]
    fn fade_polynomial_coefficients_are_recognised() {
        for (code, literal) in [
            ("void f() { u = t*t*(3.0-2.0*t); }", "3.0"),
            ("void f() { u = t*t*(3.0-2.0*t); }", "2.0"),
            ("void f() { u = f*f*(3.-f*2.); }", "2."),
            ("void f() { u = t*t*t*(t*(t*6.0-15.0)+10.0); }", "6.0"),
            ("void f() { u = t*t*t*(t*(t*6.0-15.0)+10.0); }", "15.0"),
            ("void f() { u = t*t*t*(t*(t*6.0-15.0)+10.0); }", "10.0"),
        ] {
            let scan = Scan::new("f.glsl", code);
            let found = at(&scan, literal, 0).is_some_and(|i| in_fade_polynomial(&scan, i));
            assert!(found, "`{literal}` in `{code}` is a fade coefficient");
        }
        for (code, literal) in [
            ("void f() { u = uv * 3.0 - 1.0; }", "3.0"),
            ("void f() { u = p * 6.0 + 10.0; }", "6.0"),
            ("void f() { u = p * 2.0; }", "2.0"),
        ] {
            let scan = Scan::new("f.glsl", code);
            let found = at(&scan, literal, 0).is_some_and(|i| in_fade_polynomial(&scan, i));
            assert!(!found, "`{literal}` in `{code}` is an ordinary factor");
        }
    }

    #[test]
    fn hash_arithmetic_is_recognised() {
        for (code, literal) in [
            ("void f() { h = fract(sin(x) * 43758.5453); }", "43758.5453"),
            (
                "void f() { h = fract(sin(p.x * 12.9898 + p.y * 78.233) * 4.5); }",
                "12.9898",
            ),
            ("void f() { h = fract(sin(p.x * 3.7 + p.y) * 9.1); }", "3.7"),
            ("void f() { q = fract(p * 0.1031); }", "0.1031"),
            ("void f() { q = p * 1234.567; }", "1234.567"),
        ] {
            let scan = Scan::new("f.glsl", code);
            let found = at(&scan, literal, 0).is_some_and(|i| in_hash(&scan, i));
            assert!(found, "`{literal}` in `{code}` is hash arithmetic");
        }
        let code = "void f() { q = fract(p * 3.0); r = sin(p.x * 3.0); }";
        let scan = Scan::new("f.glsl", code);
        for nth in 0..2 {
            let found = at(&scan, "3.0", nth).is_some_and(|i| in_hash(&scan, i));
            assert!(!found, "a fract without a sine, or a sine without a fract");
        }
    }

    #[test]
    fn geometry_names_are_told_from_colour_names() {
        for name in [
            "lk",
            "lp",
            "ro",
            "light_dir",
            "light",
            "sun",
            "box_size",
            "cam_pos",
        ] {
            assert!(is_geometry_name(name), "`{name}` is geometry");
        }
        for name in [
            "light_col",
            "sun_color",
            "sky",
            "tint",
            "base",
            "glow_col",
            "fogcol",
        ] {
            assert!(!is_geometry_name(name), "`{name}` may be a colour");
        }
        for name in ["sdBox", "opRep", "map", "rotate"] {
            assert!(is_geometry_function(name), "`{name}` takes geometry");
        }
        assert!(!is_geometry_function("palette"), "a palette makes colour");
    }

    #[test]
    fn helpers_name_and_entry_points_do_not() {
        let code = "float field(vec3 p) { return 1.0; }\nvec4 render_main_image() { return vec4(1.0); }\nvec4 renderMain() { return vec4(1.0); }";
        let scan = Scan::new("f.glsl", code);
        let ones: Vec<usize> = (0..scan.len()).filter(|i| scan.t(*i) == "1.0").collect();
        let names: Vec<Option<String>> = ones.iter().map(|i| helper(&scan, *i)).collect();
        assert_eq!(
            names,
            vec![Some("field".to_string()), None, None],
            "a helper names its literals, an entry point does not"
        );
    }

    #[test]
    fn specific_names_skip_short_and_generic_ones() {
        assert_eq!(specific("spokes").as_deref(), Some("spokes"), "a real name");
        assert_eq!(
            specific("p_o1234_fog").as_deref(),
            Some("fog"),
            "node prefix gone"
        );
        for name in ["t", "pt", "col", "fragColor", "time"] {
            assert_eq!(specific(name), None, "`{name}` says nothing");
        }
    }
}
