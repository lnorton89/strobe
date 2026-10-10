// ============================================================================
// Module:       strobe_autocontrol::analyze::expr
// Description:  Finds the literals that sit in expressions: time scales,
//               loop bounds, colour constructors, and coordinate scales or
//               frequencies.
//
// Dependencies: crate::source, crate::candidate, super::consts, crate::naming
// ============================================================================

//! Expression-level detectors.
//!
//! Each detector is a pattern over the significant tokens of one file and
//! returns [`Hit`]s. They share a policy: **a false positive costs more
//! than a miss.** A knob that does nothing, or that breaks the picture, is
//! worse than a knob that was never offered. So each pattern is narrow,
//! says what it will not touch, and leaves anything at global scope alone
//! (a uniform is not a constant expression). The traps each one avoids are
//! named in its comments and pinned by tests at the bottom.

use super::Hit;
use super::consts::{Named, Num, parse_number};
use crate::candidate::{Group, Kind, Rewrite, ValueType};
use crate::naming::snake_case;
use crate::source::Scan;

/// Identifiers that mean "the scene clock".
const TIME_IDENTS: [&str; 2] = ["TIME", "iTime"];

/// Identifiers that hold a screen or object coordinate.
const COORDS: [&str; 11] = [
    "_uv",
    "_uvc",
    "_xy",
    "uv",
    "p",
    "fragCoord",
    "st",
    "pos",
    "coord",
    "q",
    "uvs",
];

/// Calls whose vector arguments are directions or weights, not colours.
const NOT_COLOUR_CALLS: [&str; 8] = [
    "dot",
    "cross",
    "normalize",
    "reflect",
    "refract",
    "length",
    "distance",
    "faceforward",
];

/// Variable names that hold geometry; a `vec3` assigned to one is not a
/// colour.
const GEOMETRY_NAMES: [&str; 22] = [
    "ro",
    "rd",
    "dir",
    "direction",
    "pos",
    "position",
    "p",
    "n",
    "normal",
    "l",
    "lightdir",
    "light_dir",
    "offset",
    "size",
    "k",
    "h",
    "e",
    "off",
    "axis",
    "up",
    "origin",
    "target",
];

/// Names that say nothing about what a value is, so they do not make a good
/// control name.
const GENERIC_NAMES: [&str; 20] = [
    "uv", "p", "q", "st", "pos", "coord", "col", "color", "colour", "c", "d", "t", "x", "y", "r",
    "v", "res", "result", "ret", "o",
];

/// Substrings of function names whose bodies are number-theory, not art:
/// scaling a coordinate in a hash changes the noise, not the picture.
const OPAQUE_FUNCTIONS: [&str; 9] = [
    "hash", "rand", "noise", "fbm", "permute", "mod289", "dither", "random", "grad",
];

/// A `for` loop of the form `for (T i = a; i < BOUND; ...)`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Loop {
    /// Significant-token index of `BOUND`.
    pub bound: usize,
    /// Whether the loop body indexes something with the loop variable
    /// (`w[i]`): the bound may then be an array size.
    pub indexes_with_var: bool,
}

/// Finds the simple counted loops in the scan.
pub(super) fn loops(scan: &Scan<'_>) -> Vec<Loop> {
    let mut found = Vec::new();
    for i in 0..scan.len() {
        if !scan.is_ident(i, "for") || !scan.is(i + 1, "(") || scan.brace_depth(i) == 0 {
            continue;
        }
        let Some(close) = scan.matching(i + 1) else {
            continue;
        };
        let inner = scan.paren_depth(i + 2);
        let semis: Vec<usize> = (i + 2..close)
            .filter(|j| scan.is(*j, ";") && scan.paren_depth(*j) == inner)
            .collect();
        let [first, second] = semis[..] else {
            continue;
        };
        // The loop variable: the identifier left of the first `=`.
        let Some(eq) = (i + 2..first).find(|j| scan.is(*j, "=")) else {
            continue;
        };
        if eq == 0 || !scan.is_any_ident(eq - 1) {
            continue;
        }
        let var = scan.t(eq - 1);
        let cond = first + 1;
        if scan.t(cond) != var {
            continue;
        }
        let bound = match scan.op(cond + 1) {
            "<" => cond + 2,
            "<=" => cond + 3,
            _ => continue,
        };
        if bound + 1 != second {
            continue;
        }
        let body = close + 1;
        let end = if scan.is(body, "{") {
            scan.matching(body).unwrap_or(body)
        } else {
            (body..scan.len())
                .find(|j| scan.is(*j, ";"))
                .unwrap_or(body)
        };
        let indexes_with_var = (body..end).any(|j| {
            scan.is(j, "[")
                && scan
                    .matching(j)
                    .is_some_and(|k| (j + 1..k).any(|m| scan.is_ident(m, var)))
        });
        found.push(Loop {
            bound,
            indexes_with_var,
        });
    }
    found
}

/// The hit for a literal loop bound.
pub(super) fn loop_hit(scan: &Scan<'_>, found: &Loop) -> Option<Hit> {
    let bound = found.bound;
    if !scan.is_number(bound) || found.indexes_with_var || scan.brace_depth(bound) == 0 {
        return None;
    }
    let num = parse_number(scan.t(bound))?;
    if !(4.0..=4096.0).contains(&num.value) {
        return None;
    }
    let (start, end) = (scan.start(bound), scan.end(bound));
    Some(Hit {
        kind: Kind::LoopBound,
        rewrite: Rewrite::Value,
        span: (start, end),
        literal: scan.t(bound).to_string(),
        value: vec![num.value as f32],
        value_type: if num.is_float {
            ValueType::Count
        } else {
            ValueType::Int
        },
        name: "steps".to_string(),
        group: Group::Quality,
        key: format!("loop:{}", scan.origin(start)),
    })
}

/// What a time multiplier's other factor is.
struct Factor {
    value: f32,
    text: String,
    /// The named constant it came from, if it was an identifier.
    named: Option<String>,
}

/// The factor at `k`: a nonzero number, or a named constant.
fn factor(scan: &Scan<'_>, k: usize, named: &[Named]) -> Option<Factor> {
    if scan.is_number(k) {
        let Num { value, .. } = parse_number(scan.t(k))?;
        return (value != 0.0).then(|| Factor {
            value: value as f32,
            text: scan.t(k).to_string(),
            named: None,
        });
    }
    if scan.is_any_ident(k) {
        let constant = named.iter().find(|n| n.name == scan.t(k))?;
        return (constant.value != 0.0).then(|| Factor {
            value: constant.value,
            text: constant.name.clone(),
            named: Some(constant.name.clone()),
        });
    }
    None
}

/// Whether the token before `i` makes `x * TIME` unsafe to collapse: in
/// `a / 0.3 * TIME` the `0.3 * TIME` is not a unit.
fn divides_into(scan: &Scan<'_>, i: usize) -> bool {
    i > 0 && (scan.is(i - 1, "/") || scan.is(i - 1, "%") || scan.is(i - 1, "."))
}

/// `TIME * k` and `k * TIME`, with the name of the constant when `k` is one.
///
/// Traps avoided: `a / TIME * k` and `a / k * TIME` (the factor is not a
/// unit with `TIME`), `TIME *= ...`, a member named `TIME`, a zero
/// factor, and anything at global scope.
pub(super) fn time_scales(scan: &Scan<'_>, named: &[Named]) -> Vec<(Hit, Option<String>)> {
    let mut found = Vec::new();
    for i in 0..scan.len() {
        if scan.brace_depth(i) == 0 {
            continue;
        }
        let is_time = |at: usize| {
            scan.is_any_ident(at)
                && TIME_IDENTS.contains(&scan.t(at))
                && !(at > 0 && scan.is(at - 1, "."))
        };
        // `TIME * k`
        let (clock, k, first, last) = if is_time(i) && scan.op(i + 1) == "*" {
            (scan.t(i), i + 2, i, i + 2)
        // `k * TIME`
        } else if scan.op(i + 1) == "*" && is_time(i + 2) {
            (scan.t(i + 2), i, i, i + 2)
        } else {
            continue;
        };
        if divides_into(scan, first) {
            continue;
        }
        let Some(f) = factor(scan, k, named) else {
            continue;
        };
        let (start, end) = (scan.start(first), scan.end(last));
        found.push((
            Hit {
                kind: Kind::TimeScale,
                rewrite: Rewrite::Integrator,
                span: (start, end),
                literal: scan.text.get(start..end).unwrap_or("").to_string(),
                value: vec![f.value],
                value_type: ValueType::Float,
                name: "speed".to_string(),
                group: Group::Motion,
                key: format!("speed:{clock}:{}", f.text),
            },
            f.named,
        ));
    }
    found
}

/// The identifier of the enclosing top-level function, if the token is in one.
fn enclosing_function<'a>(scan: &Scan<'a>, i: usize) -> Option<&'a str> {
    let open = (0..i)
        .rev()
        .find(|j| scan.is(*j, "{") && scan.brace_depth(*j) == 0)?;
    if open == 0 || !scan.is(open - 1, ")") {
        return None;
    }
    let mut depth = 0_u32;
    for j in (0..open).rev() {
        if scan.is(j, ")") {
            depth += 1;
        } else if scan.is(j, "(") {
            depth -= 1;
            if depth == 0 {
                return (j > 0 && scan.is_any_ident(j - 1)).then(|| scan.t(j - 1));
            }
        }
    }
    None
}

/// Whether token `i` is inside a function whose name says it is a hash or
/// a noise: scaling its coordinate is number theory, not a control.
fn in_opaque_function(scan: &Scan<'_>, i: usize) -> bool {
    enclosing_function(scan, i).is_some_and(|name| {
        let lower = name.to_ascii_lowercase();
        OPAQUE_FUNCTIONS.iter().any(|word| lower.contains(word))
    })
}

/// The identifier a statement assigns to, if it has the form
/// `[type] name = ...` or `name op= ...`.
pub(super) fn assigned_name(scan: &Scan<'_>, i: usize) -> Option<String> {
    let mut at = scan.statement_start(i);
    while matches!(scan.t(at), "const" | "highp" | "mediump" | "lowp") {
        at += 1;
    }
    if !scan.is_any_ident(at) || scan.t(at) == "return" {
        return None;
    }
    if scan.is_any_ident(at + 1) && scan.op(at + 2) == "=" {
        return Some(scan.t(at + 1).to_string());
    }
    if matches!(scan.op(at + 1), "=" | "+=" | "-=" | "*=" | "/=") {
        return Some(scan.t(at).to_string());
    }
    if scan.is(at + 1, ".") && scan.is_any_ident(at + 2) && scan.op(at + 3) == "=" {
        return Some(scan.t(at).to_string());
    }
    None
}

/// The name of the function call that directly encloses token `i`.
fn enclosing_call<'a>(scan: &Scan<'a>, i: usize) -> Option<&'a str> {
    let mut depth = 0_u32;
    for j in (0..i).rev() {
        if scan.is(j, ";") || scan.is(j, "{") || scan.is(j, "}") {
            return None;
        }
        if scan.is(j, ")") {
            depth += 1;
        } else if scan.is(j, "(") {
            if depth == 0 {
                return (j > 0 && scan.is_any_ident(j - 1)).then(|| scan.t(j - 1));
            }
            depth -= 1;
        }
    }
    None
}

/// `vec3(r, g, b)` and `vec4(r, g, b, a)` of literals in `0..1`.
///
/// Traps avoided: single-argument splats (`vec3(0.5)`), vectors whose
/// components are all `0` or `1` (axes and masks), all-equal greys, anything
/// negative or over one, luminance weights and directions (arguments of
/// `dot`, `normalize`, `cross`...), geometry variables (`ro`, `rd`, `n`),
/// a `const` statement (a uniform cannot initialise one), and global scope.
pub(super) fn colours(scan: &Scan<'_>) -> Vec<Hit> {
    let mut found = Vec::new();
    for i in 0..scan.len() {
        let width = match scan.t(i) {
            "vec3" if scan.is_any_ident(i) => 3,
            "vec4" if scan.is_any_ident(i) => 4,
            _ => continue,
        };
        if !scan.is(i + 1, "(") || scan.brace_depth(i) == 0 || (i > 0 && scan.is(i - 1, ".")) {
            continue;
        }
        // width numbers separated by commas, then `)`.
        let mut values = Vec::with_capacity(width);
        let mut ok = true;
        for n in 0..width {
            let at = i + 2 + 2 * n;
            match scan
                .is_number(at)
                .then(|| parse_number(scan.t(at)))
                .flatten()
            {
                Some(num) if (0.0..=1.0).contains(&num.value) => values.push(num.value as f32),
                _ => {
                    ok = false;
                    break;
                }
            }
            let sep = at + 1;
            let expected = if n + 1 == width { ")" } else { "," };
            if !scan.is(sep, expected) {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        let rgb = [values[0], values[1], values[2]];
        let binary = rgb.iter().all(|c| *c == 0.0 || *c == 1.0);
        let grey = rgb[0] == rgb[1] && rgb[1] == rgb[2];
        if binary || grey {
            continue;
        }
        if scan.is_ident(scan.statement_start(i), "const") {
            continue;
        }
        if enclosing_call(scan, i).is_some_and(|call| NOT_COLOUR_CALLS.contains(&call)) {
            continue;
        }
        let assigned = assigned_name(scan, i);
        let lower = assigned.as_deref().map(snake_case);
        if lower
            .as_deref()
            .is_some_and(|name| GEOMETRY_NAMES.contains(&name))
        {
            continue;
        }
        let (start, end) = if width == 3 {
            (scan.start(i), scan.end(i + 7))
        } else {
            (scan.start(i + 2), scan.end(i + 6))
        };
        let name = match lower.as_deref() {
            Some(name) if !GENERIC_NAMES.contains(&name) => {
                if name.ends_with("col") || name.ends_with("color") || name.ends_with("colour") {
                    name.to_string()
                } else {
                    format!("{name}_color")
                }
            }
            Some(_) | None => "color".to_string(),
        };
        found.push(Hit {
            kind: Kind::Colour,
            rewrite: Rewrite::Value,
            span: (start, end),
            literal: scan.text.get(start..end).unwrap_or("").to_string(),
            value: rgb.to_vec(),
            value_type: ValueType::Colour,
            name,
            group: Group::Colour,
            key: format!("colour:{:.4},{:.4},{:.4}", rgb[0], rgb[1], rgb[2]),
        });
    }
    found
}

/// Whether a scale value is worth offering.
///
/// Zero, one, a half and tiny values are structure (`* 0.5` centring, an
/// epsilon), and `2.0` followed by a minus is the `2.0 * uv - 1.0` remap.
fn scale_value_ok(value: f64, then_minus: bool) -> bool {
    let magnitude = value.abs();
    if magnitude < 0.05 || magnitude >= 1e5 {
        return false;
    }
    if (magnitude - 1.0).abs() < 1e-9 || (magnitude - 0.5).abs() < 1e-9 {
        return false;
    }
    !((magnitude - 2.0).abs() < 1e-9 && then_minus)
}

/// The index just past a coordinate expression starting at `i`
/// (`uv`, `uv.x`, `p.xy`), if one starts there.
fn coordinate_end(scan: &Scan<'_>, i: usize) -> Option<usize> {
    if !scan.is_any_ident(i) || !COORDS.contains(&scan.t(i)) || (i > 0 && scan.is(i - 1, ".")) {
        return None;
    }
    let mut end = i + 1;
    if scan.is(end, ".") && scan.is_any_ident(end + 1) {
        let swizzle = scan.t(end + 1);
        let valid =
            (1..=4).contains(&swizzle.len()) && swizzle.chars().all(|c| "xyzwrgbastpq".contains(c));
        if !valid {
            return None;
        }
        end += 2;
    }
    (!scan.is(end, "(") && !scan.is(end, "[")).then_some(end)
}

/// Whether `x` is an identifier a frequency factor can apply to without
/// being time or an audio value (those have their own candidates).
fn plain_variable(scan: &Scan<'_>, i: usize) -> bool {
    let name = scan.t(i);
    scan.is_any_ident(i)
        && !TIME_IDENTS.contains(&name)
        && !name.starts_with("syn_")
        && !name.starts_with("strobe_")
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && !(name.starts_with('i') && name.chars().nth(1).is_some_and(|c| c.is_ascii_uppercase()))
}

/// Whether the statement containing token `i` mentions a coordinate.
fn statement_has_coordinate(scan: &Scan<'_>, i: usize) -> bool {
    let start = scan.statement_start(i);
    let end = (i..scan.len())
        .find(|j| scan.is(*j, ";"))
        .unwrap_or(scan.len());
    (start..end).any(|j| scan.is_any_ident(j) && COORDS.contains(&scan.t(j)))
}

/// The pieces of one `variable * number` factor.
struct Product {
    number: usize,
    after: usize,
    compound: bool,
}

/// `X * N`, `N * X` and `X *= N` anchored at token `i`, where X is a
/// variable (with an optional swizzle) accepted by `accept`.
fn products(
    scan: &Scan<'_>,
    i: usize,
    accept: &dyn Fn(&Scan<'_>, usize) -> Option<usize>,
) -> Option<Product> {
    // `X * N` and `X *= N`
    if let Some(end) = accept(scan, i) {
        match scan.op(end) {
            "*" if scan.is_number(end + 1) => {
                return Some(Product {
                    number: end + 1,
                    after: end + 2,
                    compound: false,
                });
            }
            "*=" if scan.is_number(end + 2) => {
                return Some(Product {
                    number: end + 2,
                    after: end + 3,
                    compound: true,
                });
            }
            _ => {}
        }
    }
    // `N * X`
    let leading = scan.is_number(i) && scan.op(i + 1) == "*" && !(i > 0 && scan.is(i - 1, "."));
    let end = if leading { accept(scan, i + 2) } else { None };
    end.map(|after| Product {
        number: i,
        after,
        compound: false,
    })
}

/// Zoom, scale and frequency factors applied to coordinates.
///
/// - `uv * 3.0`, `3.0 * uv`, `p.xy * 4.0`: a **scale**.
/// - `uv *= 2.0`: a **zoom**.
/// - `sin(x * 10.0)` and `cos(10.0 * uv.x)`: a **frequency**; the
///   variable may be any plain lowercase name when the statement also
///   mentions a coordinate.
///
/// Traps avoided: the `* 2.0 - 1.0` / `2.0 * fragCoord - iResolution.xy`
/// remaps, `* 0.5` centring, `* 1.0`, epsilons, factors inside hash and
/// noise functions, `TIME` and audio uniforms, and global scope.
pub(super) fn scales(scan: &Scan<'_>) -> Vec<Hit> {
    let mut found: Vec<Hit> = Vec::new();
    let mut claimed: Vec<usize> = Vec::new();

    // Frequencies first, so a coordinate inside `sin(...)` is a frequency
    // rather than a scale.
    for i in 0..scan.len() {
        if !(scan.is_ident(i, "sin") || scan.is_ident(i, "cos")) || !scan.is(i + 1, "(") {
            continue;
        }
        let Some(close) = scan.matching(i + 1) else {
            continue;
        };
        let depth = scan.paren_depth(i + 2);
        let coordinate_context = statement_has_coordinate(scan, i);
        let accept = |s: &Scan<'_>, at: usize| -> Option<usize> {
            if let Some(end) = coordinate_end(s, at) {
                return Some(end);
            }
            if coordinate_context && plain_variable(s, at) {
                let mut end = at + 1;
                if s.is(end, ".") && s.is_any_ident(end + 1) {
                    end += 2;
                }
                return (!s.is(end, "(") && !s.is(end, "[")).then_some(end);
            }
            None
        };
        for j in i + 2..close {
            if scan.paren_depth(j) != depth {
                continue;
            }
            let Some(product) = products(scan, j, &accept) else {
                continue;
            };
            let number = product.number;
            if let Some(hit) = scale_hit(scan, &product, Kind::Frequency)
                && !claimed.contains(&number)
            {
                claimed.push(number);
                found.push(hit);
            }
        }
    }

    for i in 0..scan.len() {
        let Some(product) = products(scan, i, &|s, at| coordinate_end(s, at)) else {
            continue;
        };
        if claimed.contains(&product.number) {
            continue;
        }
        if let Some(hit) = scale_hit(scan, &product, Kind::Scale) {
            claimed.push(product.number);
            found.push(hit);
        }
    }
    found.sort_by_key(|hit| hit.span.0);
    found
}

/// Turns one product into a hit, applying the value and context filters.
fn scale_hit(scan: &Scan<'_>, product: &Product, kind: Kind) -> Option<Hit> {
    let number = product.number;
    if scan.brace_depth(number) == 0 || in_opaque_function(scan, number) {
        return None;
    }
    let num = parse_number(scan.t(number))?;
    if !scale_value_ok(num.value, scan.is(product.after, "-")) {
        return None;
    }
    let (start, end) = (scan.start(number), scan.end(number));
    let assigned = assigned_name(scan, number).map(|n| snake_case(&n));
    let specific = assigned
        .as_deref()
        .filter(|name| !GENERIC_NAMES.contains(name));
    let name = match (kind, product.compound, specific) {
        (Kind::Frequency, _, Some(name)) => format!("{name}_freq"),
        (Kind::Frequency, _, None) => "freq".to_string(),
        (_, true, _) => "zoom".to_string(),
        (_, false, Some(name)) => format!("{name}_scale"),
        (_, false, None) => "scale".to_string(),
    };
    Some(Hit {
        kind,
        rewrite: Rewrite::Value,
        span: (start, end),
        literal: scan.t(number).to_string(),
        value: vec![num.value as f32],
        value_type: ValueType::Float,
        name,
        group: Group::Shape,
        key: format!("scale:{}", scan.origin(start)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::consts;

    fn texts(hits: &[Hit]) -> Vec<String> {
        hits.iter().map(|h| h.literal.clone()).collect()
    }

    fn colour_literals(code: &str) -> Vec<String> {
        texts(&colours(&Scan::new("f.glsl", code)))
    }

    fn scale_literals(code: &str) -> Vec<String> {
        texts(&scales(&Scan::new("f.glsl", code)))
    }

    fn time_literals(code: &str) -> Vec<String> {
        let scan = Scan::new("f.glsl", code);
        let named = consts::find(&scan);
        time_scales(&scan, &named)
            .into_iter()
            .map(|(hit, _)| hit.literal)
            .collect()
    }

    fn in_main(body: &str) -> String {
        format!("vec4 renderMain() {{\n{body}\n}}\n")
    }

    #[test]
    fn time_multipliers_in_both_orders_are_found() {
        let code = in_main("float a = TIME * 0.3; float b = 2.5 * iTime; float c = sin(TIME*4.0);");
        assert_eq!(
            time_literals(&code),
            vec!["TIME * 0.3", "2.5 * iTime", "TIME*4.0"],
            "TIME and iTime, either order"
        );
    }

    #[test]
    fn time_traps_are_left_alone() {
        for body in [
            "float a = x / TIME * 0.3;",
            "float a = x / 0.3 * TIME;",
            "float a = TIME * 0.0;",
            "float a = TIME + 0.3;",
            "float a = foo.TIME * 0.3;",
            "float a = TIME * x;",
        ] {
            assert!(
                time_literals(&in_main(body)).is_empty(),
                "`{body}` must not be a time scale"
            );
        }
        assert!(
            time_literals("float g = TIME * 0.3;").is_empty(),
            "global scope cannot read a uniform"
        );
    }

    #[test]
    fn a_named_constant_can_scale_time() {
        let code = format!("#define RATE 0.4\n{}", in_main("float a = TIME * RATE;"));
        let scan = Scan::new("f.glsl", &code);
        let named = consts::find(&scan);
        let hits = time_scales(&scan, &named);
        assert_eq!(hits.len(), 1, "one expression");
        assert_eq!(
            hits[0].1.as_deref(),
            Some("RATE"),
            "and it names the constant"
        );
        assert!((hits[0].0.value[0] - 0.4).abs() < 1e-6, "with its value");
    }

    #[test]
    fn loop_bounds_become_quality_candidates() {
        let code = in_main(
            "for (int i = 0; i < 64; i++) { x += 1.0; }\nfor (float j = 0.0; j <= 8.0; j++) x += j;",
        );
        let scan = Scan::new("f.glsl", &code);
        let hits: Vec<Hit> = loops(&scan)
            .iter()
            .filter_map(|l| loop_hit(&scan, l))
            .collect();
        assert_eq!(texts(&hits), vec!["64", "8.0"], "both bounds");
        assert_eq!(hits[0].value_type, ValueType::Int, "an int loop");
        assert_eq!(hits[1].value_type, ValueType::Count, "a float loop");
    }

    #[test]
    fn loop_traps_are_left_alone() {
        for body in [
            "for (int i = 0; i < 3; i++) { x += 1.0; }",
            "for (int i = 0; i < 64; i++) { x += w[i]; }",
            "for (int i = 0; i < n; i++) { x += 1.0; }",
            "for (int i = 64; i > 0; i--) { x += 1.0; }",
            "for (int i = 0; j < 64; i++) { x += 1.0; }",
        ] {
            let code = in_main(body);
            let scan = Scan::new("f.glsl", &code);
            let hits: Vec<Hit> = loops(&scan)
                .iter()
                .filter_map(|l| loop_hit(&scan, l))
                .collect();
            assert!(hits.is_empty(), "`{body}` must not be a quality candidate");
        }
    }

    #[test]
    fn colour_constructors_are_found() {
        let code = in_main("vec3 sky = vec3(0.2, 0.4, 0.9);\nvec4 c = vec4(0.9, 0.5, 0.1, 1.0);");
        assert_eq!(
            colour_literals(&code),
            vec!["vec3(0.2, 0.4, 0.9)", "0.9, 0.5, 0.1"],
            "a vec3 whole, and a vec4's colour part"
        );
    }

    #[test]
    fn colour_traps_are_left_alone() {
        for body in [
            "vec3 c = vec3(0.5);",
            "vec3 c = vec3(1.0, 0.0, 0.0);",
            "vec3 c = vec3(0.5, 0.5, 0.5);",
            "vec3 c = vec3(0.2, 0.4, 1.5);",
            "vec3 c = vec3(-0.2, 0.4, 0.5);",
            "float l = dot(c, vec3(0.299, 0.587, 0.114));",
            "vec3 l = normalize(vec3(0.35, 0.9, 0.25));",
            "vec3 ro = vec3(0.1, 0.4, 0.2);",
            "const vec3 c = vec3(0.2, 0.4, 0.9);",
            "vec3 c = vec3(a, 0.4, 0.9);",
        ] {
            assert!(
                colour_literals(&in_main(body)).is_empty(),
                "`{body}` must not be a colour"
            );
        }
    }

    #[test]
    fn scale_zoom_and_frequency_factors_are_found() {
        let code = in_main(
            "vec2 grid = uv * 3.0;\nuv *= 1.5;\nfloat wave = sin(uv.x * 10.0);\nfloat v = cos(7.0 * p.y);",
        );
        let scan = Scan::new("f.glsl", &code);
        let hits = scales(&scan);
        let summary: Vec<(String, Kind, String)> = hits
            .iter()
            .map(|h| (h.literal.clone(), h.kind, h.name.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("3.0".to_string(), Kind::Scale, "grid_scale".to_string()),
                ("1.5".to_string(), Kind::Scale, "zoom".to_string()),
                ("10.0".to_string(), Kind::Frequency, "wave_freq".to_string()),
                ("7.0".to_string(), Kind::Frequency, "freq".to_string()),
            ],
            "scale, zoom, and two frequencies"
        );
    }

    #[test]
    fn a_frequency_on_a_plain_variable_needs_a_coordinate_in_the_statement() {
        assert_eq!(
            scale_literals(&in_main("float s = sin(x * 10.0) + uv.y;")),
            vec!["10.0"],
            "the statement mentions uv"
        );
        assert!(
            scale_literals(&in_main("float s = sin(x * 10.0);")).is_empty(),
            "no coordinate in sight: not clearly a spatial frequency"
        );
    }

    #[test]
    fn scale_traps_are_left_alone() {
        for body in [
            "vec2 a = uv * 2.0 - 1.0;",
            "vec2 a = 2.0 * fragCoord - iResolution.xy;",
            "vec2 a = uv * 0.5;",
            "vec2 a = uv * 1.0;",
            "vec2 a = uv * 0.001;",
            "float a = sin(TIME * 3.0);",
            "float a = sin(syn_BassLevel * 3.0 + uv.x);",
            "vec2 a = uv * b;",
            "vec2 a = uv / 4.0;",
        ] {
            assert!(
                scale_literals(&in_main(body)).is_empty(),
                "`{body}` must not be a scale"
            );
        }
        assert!(
            scale_literals("vec2 g = uv * 3.0;").is_empty(),
            "global scope"
        );
    }

    #[test]
    fn hash_and_noise_functions_are_opaque() {
        let code =
            "float hash(vec2 p) {\n  p = fract(p * 0.3183099);\n  return fract(p.x * 17.0);\n}\n";
        assert!(
            scale_literals(code).is_empty(),
            "scaling a coordinate inside a hash is not a knob"
        );
    }

    #[test]
    fn two_to_the_remap_is_only_skipped_when_a_minus_follows() {
        assert_eq!(
            scale_literals(&in_main("vec2 a = uv * 2.0;")),
            vec!["2.0"],
            "a plain doubling is a real zoom"
        );
    }
}
