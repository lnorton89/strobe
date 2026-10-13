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
use super::consts::{Named, Num, is_hash_value, parse_number};
use super::context::{
    call_chain, enclosing_function, function_span, helper, in_fade_polynomial, in_hash,
    is_geometry_function, is_geometry_name, role, specific, user_call,
};
use crate::candidate::{Group, Kind, Rewrite, ValueType};
use crate::naming::{is_glsl_word, readable};
use crate::source::Scan;
use strobe_shadertoy::glsl::{self, Kind as TokKind};

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

/// Calls whose vector arguments are directions, weights or hash input,
/// not colours.
const NOT_COLOUR_CALLS: [&str; 9] = [
    "dot",
    "cross",
    "normalize",
    "reflect",
    "refract",
    "length",
    "distance",
    "faceforward",
    "fract",
];

/// Substrings of function names whose bodies are number-theory, not art:
/// scaling a coordinate in a hash changes the noise, not the picture.
const OPAQUE_FUNCTIONS: [&str; 7] = [
    "hash", "noise", "fbm", "permute", "mod289", "dither", "random",
];

/// Words in the name of a function a march loop calls to measure the
/// scene (`map`, `getDist`, `sceneSDF`, `distanceField`).
const MARCH_CALLS: [&str; 8] = [
    "map", "sdf", "de", "df", "scene", "field", "dist", "distance",
];

/// A `for` loop of the form `for (T i = a; i < BOUND; ...)`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Loop {
    /// Significant-token index of `BOUND`.
    pub bound: usize,
    /// Whether the loop body indexes something with the loop variable
    /// (`w[i]`): the bound may then be an array size.
    pub indexes_with_var: bool,
    /// The loop body's significant tokens, `[start, end)`.
    pub body: (usize, usize),
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
            body: (body, end),
        });
    }
    found
}

/// A loop's name, from what its function is called and what its body calls
/// (D259):
///
/// - in a function named for shadows or occlusion, `shadow_steps` or
///   `ao_steps`; named for marching or tracing, `march_steps`;
/// - a body that calls `map`, `sdf`, `de`, `field` or an `sd*` function is a
///   ray march: `march_steps` (also in `render_main_image` and the like);
///   one that calls a noise is a sum of `octaves`;
/// - otherwise `steps` in an entry point and `<function>_steps` in a
///   helper.
fn loop_name(scan: &Scan<'_>, found: &Loop) -> String {
    let function = enclosing_function(scan, found.bound).map(readable);
    let words: Vec<&str> = function
        .as_deref()
        .map(|f| f.split('_').collect())
        .unwrap_or_default();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    if has(&["shadow", "shadows", "softshadow", "shade"]) {
        return "shadow_steps".to_string();
    }
    if has(&["ao", "occlusion", "ambient"]) {
        return "ao_steps".to_string();
    }
    if has(&[
        "march",
        "raymarch",
        "trace",
        "raytrace",
        "intersect",
        "cast",
    ]) {
        return "march_steps".to_string();
    }
    let (start, end) = found.body;
    let calls = |test: &dyn Fn(&str) -> bool| {
        (start..end).any(|j| scan.is_any_ident(j) && scan.is(j + 1, "(") && test(scan.t(j)))
    };
    // A built-in `distance` is not a distance field.
    if calls(&|name| {
        let snake = readable(name);
        !is_glsl_word(name)
            && (snake.split('_').any(|word| MARCH_CALLS.contains(&word))
                || snake.split('_').next() == Some("sd"))
    }) {
        return "march_steps".to_string();
    }
    if calls(&|name| {
        let snake = readable(name);
        snake.split('_').any(|word| word.ends_with("noise"))
    }) {
        return "octaves".to_string();
    }
    match helper(scan, found.bound) {
        Some(function) => format!("{function}_steps"),
        None => "steps".to_string(),
    }
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
        name: loop_name(scan, found),
        role: None,
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

/// The factor at `k`: a nonzero number, or a named float constant.
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
        let constant = named
            .iter()
            .find(|n| n.name == scan.t(k) && n.rgb.is_none())?;
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

/// Words that declare the identifier after them.
const DECLARING: [&str; 9] = [
    "float", "int", "in", "out", "inout", "highp", "mediump", "lowp", "const",
];

/// Whether the function around token `at` declares its own `clock` (a
/// parameter or local called `TIME`): then `TIME * k` there is not the
/// scene clock, and an integrator would change the picture (D259).
fn clock_shadowed(scan: &Scan<'_>, at: usize, clock: &str) -> bool {
    let Some((open, close)) = function_span(scan, at) else {
        return false;
    };
    (open..close).any(|j| scan.is_ident(j, clock) && j > 0 && DECLARING.contains(&scan.t(j - 1)))
}

/// Words that already say a name is a rate.
const MOTION_WORDS: [&str; 8] = [
    "speed", "rate", "tempo", "velocity", "spin", "flow", "drift", "freq",
];

/// A speed control's name: the constant it came from, or the variable the
/// product is assigned to, the function it is passed to, the helper it is
/// in, in that order (`float spokes = .. iTime * 0.3` is `spokes_speed`,
/// `p *= rot(TIME * 0.3)` is `rot_speed`); `speed` when none says anything.
fn speed_name(scan: &Scan<'_>, at: usize, constant: Option<&str>) -> String {
    let rate = |name: String| {
        if MOTION_WORDS.iter().any(|word| name.contains(word)) {
            name
        } else {
            format!("{name}_speed")
        }
    };
    if let Some(name) = constant.and_then(specific) {
        return rate(name);
    }
    assigned_name(scan, at)
        .and_then(|n| specific(&n))
        .or_else(|| user_call(scan, at))
        .or_else(|| helper(scan, at))
        .map_or_else(|| "speed".to_string(), rate)
}

/// `TIME * k`, `k * TIME` and `TIME / k`, with the name of the constant
/// when `k` is one.
///
/// Traps avoided: `a / TIME * k` and `a / k * TIME` (the factor is not a
/// unit with `TIME`), `TIME *= ...`, a member named `TIME`, a zero
/// factor, a function with its own `TIME` parameter or local, and
/// anything at global scope.
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
        // `TIME * k`, `TIME / k`, `k * TIME`
        let (clock, k, first, last, divided) = if is_time(i) && scan.op(i + 1) == "*" {
            (scan.t(i), i + 2, i, i + 2, false)
        } else if is_time(i) && scan.op(i + 1) == "/" && scan.is_number(i + 2) {
            (scan.t(i), i + 2, i, i + 2, true)
        } else if scan.op(i + 1) == "*" && is_time(i + 2) {
            (scan.t(i + 2), i, i, i + 2, false)
        } else {
            continue;
        };
        if divides_into(scan, first) || clock_shadowed(scan, i, clock) {
            continue;
        }
        let Some(mut f) = factor(scan, k, named) else {
            continue;
        };
        if divided {
            f.value = 1.0 / f.value;
            f.text = format!("/{}", f.text);
        }
        let (start, end) = (scan.start(first), scan.end(last));
        found.push((
            Hit {
                kind: Kind::TimeScale,
                rewrite: Rewrite::Integrator,
                span: (start, end),
                literal: scan.text.get(start..end).unwrap_or("").to_string(),
                value: vec![f.value],
                value_type: ValueType::Float,
                name: speed_name(scan, first, f.named.as_deref()),
                role: role(scan, first),
                group: Group::Motion,
                key: format!("speed:{clock}:{}", f.text),
            },
            f.named,
        ));
    }
    found
}

/// Whether token `i` is inside a function whose name says it is a hash or
/// a noise: scaling its coordinate is number theory, not a control. `rand`
/// and `grad` count only as the start of a word, so `gradient` and `brand`
/// are not opaque.
fn in_opaque_function(scan: &Scan<'_>, i: usize) -> bool {
    enclosing_function(scan, i).is_some_and(|name| {
        let lower = name.to_ascii_lowercase();
        let snake = readable(name);
        OPAQUE_FUNCTIONS.iter().any(|word| lower.contains(word))
            || snake.split('_').any(|word| {
                word.starts_with("rand")
                    || (word.starts_with("grad") && !word.starts_with("gradient"))
            })
    })
}

/// The identifier a statement assigns to, if it has the form
/// `[type] name = ...` or `name op= ...`. For `name.field = ...` it is the
/// field, unless the field is a swizzle (`col.rgb = ...` assigns `col`).
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
    if scan.is(at + 1, ".")
        && scan.is_any_ident(at + 2)
        && matches!(scan.op(at + 3), "=" | "+=" | "-=" | "*=" | "/=")
    {
        let field = scan.t(at + 2);
        let swizzle = field.len() <= 4 && field.chars().all(|c| "xyzwrgbastpq".contains(c));
        return Some(if swizzle { scan.t(at) } else { field }.to_string());
    }
    None
}

/// The name of the function call that directly encloses token `i`.
fn enclosing_call<'a>(scan: &Scan<'a>, i: usize) -> Option<&'a str> {
    call_chain(scan, i).first().map(|(name, _)| *name)
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
        // A `vec4` keeps its alpha, so it is an RGBA colour even when two of
        // its colour components are zero; a mostly-zero `vec3` is an axis.
        let axis = width == 3 && mostly_zero(rgb);
        if !colour_values_ok(rgb) || axis || values.iter().any(|v| is_hash_value(f64::from(*v))) {
            continue;
        }
        if scan.is_ident(scan.statement_start(i), "const") {
            continue;
        }
        if enclosing_call(scan, i).is_some_and(|call| NOT_COLOUR_CALLS.contains(&call)) {
            continue;
        }
        let assigned = assigned_name(scan, i).map(|name| readable(&name));
        if assigned.as_deref().is_some_and(is_geometry_name) || in_geometry(scan, i) {
            continue;
        }
        let (start, end) = if width == 3 {
            (scan.start(i), scan.end(i + 7))
        } else {
            (scan.start(i + 2), scan.end(i + 6))
        };
        let close = scan.matching(i + 1).unwrap_or(i + 1);
        let base = assigned
            .as_deref()
            .and_then(specific)
            .or_else(|| multiplicand(scan, i, close))
            .or_else(|| user_call(scan, i))
            .or_else(|| helper(scan, i));
        found.push(Hit {
            kind: Kind::Colour,
            rewrite: Rewrite::Value,
            span: (start, end),
            literal: scan.text.get(start..end).unwrap_or("").to_string(),
            value: rgb.to_vec(),
            value_type: ValueType::Colour,
            name: colour_name(base),
            // `vec3 a = ..; vec3 b = ..;` mixed later: the short names are
            // at least the two ends' roles.
            role: role(scan, i).or_else(|| {
                assigned
                    .filter(|name| name.chars().count() <= 2)
                    .filter(|name| name.chars().all(|c| c.is_ascii_alphabetic()))
            }),
            group: Group::Colour,
            key: format!("colour:{:.4},{:.4},{:.4}", rgb[0], rgb[1], rgb[2]),
        });
    }
    found
}

/// Whether three components can be a colour: all in `0..1`, not an axis
/// or mask (every component 0 or 1), and not a grey.
pub(super) fn colour_values_ok(rgb: [f32; 3]) -> bool {
    let in_range = rgb.iter().all(|c| (0.0..=1.0).contains(c));
    let binary = rgb.iter().all(|c| *c == 0.0 || *c == 1.0);
    let grey = rgb[0] == rgb[1] && rgb[1] == rgb[2];
    in_range && !binary && !grey
}

/// Whether two of three components are zero: in a `vec3` that is a
/// direction or an offset along one axis more often than a colour (D259).
pub(super) fn mostly_zero(rgb: [f32; 3]) -> bool {
    rgb.iter().filter(|c| **c == 0.0).count() >= 2
}

/// A colour control's name from a base: `fog_col` and `sky_color` stay as
/// they are, `glow` becomes `glow_color`, and no base gives `color`.
pub(super) fn colour_name(base: Option<String>) -> String {
    match base {
        Some(name)
            if name.ends_with("col") || name.ends_with("color") || name.ends_with("colour") =>
        {
            name
        }
        Some(name) => format!("{name}_color"),
        None => "color".to_string(),
    }
}

/// The variable a constructor at `i` (closing at `close`) is multiplied by,
/// when it says something: `glow * vec3(..)` or `vec3(..) * glow` is the
/// glow's colour.
fn multiplicand(scan: &Scan<'_>, i: usize, close: usize) -> Option<String> {
    let before = (i >= 2 && scan.is(i - 1, "*") && scan.is_any_ident(i - 2))
        .then(|| i - 2)
        .filter(|at| !(*at > 0 && scan.is(at - 1, ".")));
    let after = (scan.is(close + 1, "*")
        && scan.is_any_ident(close + 2)
        && !scan.is(close + 3, "(")
        && !scan.is(close + 3, "."))
    .then_some(close + 2);
    before
        .or(after)
        .and_then(|at| specific(scan.t(at)))
        .filter(|name| !is_geometry_name(name))
}

/// Whether the constructor at `i` sits in geometry: added to or
/// subtracted from a position (`p - vec3(..)`, `ro + vec3(..)`), passed
/// to a distance or transform function (`sdBox(p, vec3(..))`), or anywhere
/// in a distance function (`map`, `sdf`, `sd*`).
fn in_geometry(scan: &Scan<'_>, i: usize) -> bool {
    let close = scan.matching(i + 1).unwrap_or(i + 1);
    let is_position = |at: usize| {
        scan.is_any_ident(at)
            && !scan.is(at + 1, "(")
            && (COORDS.contains(&scan.t(at)) || is_geometry_name(&readable(scan.t(at))))
    };
    let signed = |at: usize| scan.is(at, "+") || scan.is(at, "-");
    let before = i >= 2 && signed(i - 1) && is_position(i - 2);
    let after = signed(close + 1) && is_position(close + 2);
    let call = call_chain(scan, i)
        .iter()
        .any(|(name, _)| is_geometry_function(name));
    let function = enclosing_function(scan, i).is_some_and(is_geometry_function);
    before || after || call || function
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
    /// The first token of the coordinate expression.
    coordinate: usize,
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
                    coordinate: i,
                });
            }
            "*=" if scan.is_number(end + 2) => {
                return Some(Product {
                    number: end + 2,
                    after: end + 3,
                    compound: true,
                    coordinate: i,
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
        coordinate: i + 2,
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
    // Mathematics, not art: fade polynomials and hash arithmetic (D259).
    if in_fade_polynomial(scan, number) || in_hash(scan, number) {
        return None;
    }
    let num = parse_number(scan.t(number))?;
    if !scale_value_ok(num.value, scan.is(product.after, "-")) {
        return None;
    }
    let (start, end) = (scan.start(number), scan.end(number));
    // The function the product is passed to (`fbm(p * 3.0)`), the
    // variable it is assigned to, or the helper it is in, says what it
    // scales.
    let assigned = || assigned_name(scan, number).and_then(|n| specific(&n));
    let context = match kind {
        Kind::Frequency => assigned()
            .or_else(|| user_call(scan, number))
            .or_else(|| helper(scan, number)),
        Kind::Define
        | Kind::Const
        | Kind::TimeScale
        | Kind::LoopBound
        | Kind::Colour
        | Kind::Scale => user_call(scan, number)
            .or_else(assigned)
            .or_else(|| helper(scan, number)),
    };
    let name = match (kind, product.compound, context) {
        (Kind::Frequency, _, Some(name)) => format!("{name}_freq"),
        (Kind::Frequency, _, None) => "freq".to_string(),
        (_, true, _) => "zoom".to_string(),
        (_, false, Some(name)) => format!("{name}_scale"),
        (_, false, None) => "scale".to_string(),
    };
    // A one-component swizzle (`uv.x * 3.0`) tells a scale apart from its
    // sibling on the other axis; otherwise the enclosing constructor or mix.
    let swizzle = (scan.is(product.coordinate + 1, ".")
        && matches!(scan.t(product.coordinate + 2), "x" | "y" | "z" | "w"))
    .then(|| scan.t(product.coordinate + 2).to_string());
    Some(Hit {
        kind,
        rewrite: Rewrite::Value,
        span: (start, end),
        literal: scan.t(number).to_string(),
        value: vec![num.value as f32],
        value_type: ValueType::Float,
        name,
        role: swizzle.or_else(|| role(scan, number)),
        group: Group::Shape,
        key: format!("scale:{}", scan.origin(start)),
    })
}

/// `TIME * k`, `k * TIME` and `TIME / k` inside a `#define` body
/// (`#define T (TIME * 0.3)`), which the expression scan never sees because
/// a directive is one token (D259).
///
/// The site is a span inside the directive, rewritten to the speed
/// control's output like any other. A macro pastes text, so when the
/// product is not inside parentheses in the body, a use after `/` or `%`
/// would read differently once the product is one identifier; such a
/// macro is skipped. So is a function-like macro with its own `TIME`
/// parameter.
pub(super) fn define_time_scales(scan: &Scan<'_>) -> Vec<Hit> {
    let mut found = Vec::new();
    for d in 0..scan.len() {
        if scan.kind(d) != Some(TokKind::Directive) {
            continue;
        }
        let Some(tok) = scan.tok(d) else {
            continue;
        };
        let Some(("define", body)) = glsl::split_directive(tok.text) else {
            continue;
        };
        let body_start = tok.start + (tok.text.len() - body.len());
        let mut at = body_start;
        let tokens: Vec<(TokKind, &str, usize, usize)> = glsl::lex_directive_body(body)
            .into_iter()
            .map(|token| {
                let start = at;
                at += token.text.len();
                (token.kind, token.text, start, at)
            })
            .filter(|(kind, ..)| {
                !matches!(
                    kind,
                    TokKind::Space | TokKind::LineComment | TokKind::BlockComment
                )
            })
            .collect();
        let Some((TokKind::Ident, macro_name, _, name_end)) = tokens.first().copied() else {
            continue;
        };
        // A function-like macro's parameters come first; one called like
        // the clock shadows it.
        let mut first_body = 1;
        let mut parameters: Vec<&str> = Vec::new();
        if tokens.get(1).is_some_and(|t| t.1 == "(" && t.2 == name_end) {
            let Some(close) = tokens.iter().position(|t| t.1 == ")") else {
                continue;
            };
            parameters = tokens
                .get(2..close)
                .unwrap_or(&[])
                .iter()
                .filter(|t| t.0 == TokKind::Ident)
                .map(|t| t.1)
                .collect();
            first_body = close + 1;
        }
        let text = |k: usize| tokens.get(k).map_or("", |t| t.1);
        let kind = |k: usize| tokens.get(k).map(|t| t.0);
        let is_clock = |k: usize| {
            kind(k) == Some(TokKind::Ident)
                && TIME_IDENTS.contains(&text(k))
                && !parameters.contains(&text(k))
                && !(k > first_body && text(k - 1) == ".")
        };
        let number = |k: usize| {
            (kind(k) == Some(TokKind::Number))
                .then(|| parse_number(text(k)))
                .flatten()
                .filter(|num| num.value != 0.0)
        };
        let mut depth = 0_i32;
        for k in first_body..tokens.len() {
            match text(k) {
                "(" => depth += 1,
                ")" => depth -= 1,
                _ => {}
            }
            let (clock, first, last, value, divided) =
                if is_clock(k) && text(k + 1) == "*" && number(k + 2).is_some() {
                    (text(k), k, k + 2, number(k + 2), false)
                } else if is_clock(k) && text(k + 1) == "/" && number(k + 2).is_some() {
                    (text(k), k, k + 2, number(k + 2), true)
                } else if number(k).is_some() && text(k + 1) == "*" && is_clock(k + 2) {
                    (text(k + 2), k, k + 2, number(k), false)
                } else {
                    continue;
                };
            let Some(value) = value else {
                continue;
            };
            if k > first_body && matches!(text(k - 1), "/" | "%" | ".") {
                continue;
            }
            // `*`/`/` bind tighter than anything after them, but a use of
            // an unparenthesised product after `/` or `%` would not.
            let bare = depth == 0;
            if bare && macro_used_after_division(scan, macro_name) {
                continue;
            }
            let (Some(start), Some(end)) = (
                tokens.get(first).map(|t| t.2),
                tokens.get(last).map(|t| t.3),
            ) else {
                continue;
            };
            let number_at = if is_clock(k) { last } else { k };
            let factor_text = if divided {
                format!("/{}", text(number_at))
            } else {
                text(number_at).to_string()
            };
            let value = if divided {
                (1.0 / value.value) as f32
            } else {
                value.value as f32
            };
            let name = specific(macro_name).map_or_else(
                || "speed".to_string(),
                |name| {
                    if MOTION_WORDS.iter().any(|word| name.contains(word)) {
                        name
                    } else {
                        format!("{name}_speed")
                    }
                },
            );
            found.push(Hit {
                kind: Kind::TimeScale,
                rewrite: Rewrite::Integrator,
                span: (start, end),
                literal: scan.text.get(start..end).unwrap_or("").to_string(),
                value: vec![value],
                value_type: ValueType::Float,
                name,
                role: None,
                group: Group::Motion,
                key: format!("speed:{clock}:{factor_text}"),
            });
        }
    }
    found
}

/// Whether `name` is used anywhere right after a `/` or `%`.
fn macro_used_after_division(scan: &Scan<'_>, name: &str) -> bool {
    (1..scan.len()).any(|i| scan.is_ident(i, name) && (scan.is(i - 1, "/") || scan.is(i - 1, "%")))
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

    fn names(hits: &[Hit]) -> Vec<(String, Option<String>)> {
        hits.iter()
            .map(|h| (h.name.clone(), h.role.clone()))
            .collect()
    }

    fn named(name: &str, role: Option<&str>) -> (String, Option<String>) {
        (name.to_string(), role.map(str::to_string))
    }

    // T26: fade polynomials and hash constants were offered as controls.
    #[test]
    fn fade_polynomials_and_hash_arithmetic_are_not_controls() {
        for body in [
            "vec2 q = fract(uv); q = q * q * (3.0 - 2.0 * q);",
            "vec2 q = fract(uv); q = q * q * q * (q * (q * 6.0 - 15.0) + 10.0);",
            "float h = fract(sin(uv.x * 12.9898 + uv.y * 78.233) * 43758.5453);",
            "float h = fract(sin(dot(uv, vec2(12.9898, 78.233))) * 43758.5453);",
            "float h = fract(sin(p.x * 3.7 + p.y * 1.3) * 951.135);",
            "vec3 p3 = fract(vec3(uv.xyx) * vec3(0.1031, 0.1030, 0.0973));",
            "vec2 w = uv * 43758.5453;",
        ] {
            let code = in_main(body);
            assert!(
                scale_literals(&code).is_empty() && colour_literals(&code).is_empty(),
                "`{body}` is mathematics, not a knob"
            );
        }
        assert_eq!(
            scale_literals(&in_main("vec2 a = uv * 3.0 - 2.0;")),
            vec!["3.0"],
            "a scale followed by an offset is still a scale"
        );
    }

    // T26: light and camera positions were classified as colours.
    #[test]
    fn positions_directions_and_sizes_are_not_colours() {
        for body in [
            "vec3 lk = vec3(0.2, 0.5, 0.3);",
            "vec3 lp = vec3(0.3, 0.6, 0.2);",
            "vec3 lightDir = vec3(0.3, 0.8, 0.5);",
            "vec3 light = vec3(0.4, 0.7, 0.2);",
            "vec3 camPos = vec3(0.4, 0.7, 0.2);",
            "vec3 d = p - vec3(0.2, 0.3, 0.4);",
            "float b = sdBox(p, vec3(0.3, 0.5, 0.2));",
            "vec3 c = vec3(0.0, 0.0, 0.5);",
        ] {
            assert!(
                colour_literals(&in_main(body)).is_empty(),
                "`{body}` is geometry, not a colour"
            );
        }
        assert!(
            colour_literals("float map(vec3 p) { vec3 c = vec3(0.2, 0.4, 0.3); return c.x; }")
                .is_empty(),
            "a vec3 in a distance function is geometry"
        );
        assert_eq!(
            colour_literals(&in_main("vec3 lightCol = vec3(1.0, 0.9, 0.7);")),
            vec!["vec3(1.0, 0.9, 0.7)"],
            "a light's colour is a colour"
        );
    }

    // T26: 116 of 266 names were a stem and a number.
    #[test]
    fn names_come_from_the_code_around_the_literal() {
        let code = "\
float field(vec3 p) { vec3 q = fract(p * 0.7); return length(q); }
vec4 renderMain() {
    float n = fbm(_uv * 3.0);
    vec2 orbit = vec2(cos(TIME * 1.3), sin(TIME * 1.7));
    _uv *= rot(TIME * 0.2);
    float spokes = cos(n + TIME * 0.3);
    vec3 col = mix(vec3(0.9, 0.4, 0.1), vec3(0.1, 0.2, 0.6), n);
    col += glow * vec3(0.9, 0.5, 0.2);
    mat.albedo = vec3(0.8, 0.3, 0.2);
    float wave = sin(_uv.x * 9.0) * sin(_uv.y * 7.0);
    return vec4(col, 1.0);
}
";
        let scan = Scan::new("f.glsl", code);
        let named_time = time_scales(&scan, &[]);
        let speeds: Vec<(String, Option<String>)> = named_time
            .iter()
            .map(|(h, _)| (h.name.clone(), h.role.clone()))
            .collect();
        assert_eq!(
            speeds,
            vec![
                named("orbit_speed", Some("x")),
                named("orbit_speed", Some("y")),
                named("rot_speed", None),
                named("spokes_speed", None),
            ],
            "the assigned variable or the function it is passed to"
        );
        assert_eq!(
            names(&colours(&scan)),
            vec![
                named("color", Some("low")),
                named("color", Some("high")),
                named("glow_color", None),
                named("albedo_color", None),
            ],
            "a mix's ends, a multiplier, a field"
        );
        assert_eq!(
            names(&scales(&scan)),
            vec![
                named("field_scale", None),
                named("fbm_scale", None),
                named("wave_freq", Some("x")),
                named("wave_freq", Some("y")),
            ],
            "the helper, the function the coordinate goes to, and the axis"
        );
    }

    #[test]
    fn time_divided_by_a_number_is_a_speed_and_a_local_time_is_not() {
        let code = in_main("float a = TIME / 4.0; float b = x / TIME / 2.0;");
        let scan = Scan::new("f.glsl", &code);
        let hits = time_scales(&scan, &[]);
        assert_eq!(hits.len(), 1, "only the plain quotient");
        assert!(
            hits.first()
                .is_some_and(|(hit, _)| (hit.value[0] - 0.25).abs() < 1e-6),
            "TIME / 4.0 runs at a quarter speed"
        );
        let shadowed = "float wave(vec2 p, float TIME) { return sin(p.x + TIME * 3.0); }\n\
                        vec4 renderMain() { return vec4(wave(_uv, TIME + 2.0)); }\n";
        assert!(
            time_literals(shadowed).is_empty(),
            "a parameter called TIME is not the clock"
        );
    }

    #[test]
    fn loops_are_named_for_what_they_do() {
        let code = "\
float softShadow(vec3 ro, vec3 rd) { float r = 1.0; for (int i = 0; i < 24; i++) { r *= 0.9; } return r; }
vec4 render_main_image() { float t = 0.0; for (int i = 0; i < 80; i++) { t += map(vec3(t)); } return vec4(t); }
float clouds(vec2 p) { float v = 0.0; for (int i = 0; i < 6; i++) { v += noise(p); p *= 2.0; } return v; }
float glow(vec2 p) { float v = 0.0; for (int i = 0; i < 9; i++) { v += 0.1; } return v; }
";
        let scan = Scan::new("f.glsl", code);
        let found: Vec<String> = loops(&scan)
            .iter()
            .filter_map(|l| loop_hit(&scan, l))
            .map(|h| h.name)
            .collect();
        assert_eq!(
            found,
            vec!["shadow_steps", "march_steps", "octaves", "glow_steps"],
            "shadow, march, octaves, and a helper's own name"
        );
    }

    // T26 misses: a speed or a colour inside a `#define`.
    #[test]
    fn a_speed_inside_a_define_body_is_found() {
        let found = |code: &str| -> Vec<String> {
            define_time_scales(&Scan::new("f.glsl", code))
                .into_iter()
                .map(|h| format!("{} {}", h.literal, h.name))
                .collect()
        };
        assert_eq!(
            found("#define T (TIME * 0.3)\nvec4 renderMain() { return vec4(1.0 / T); }\n"),
            vec!["TIME * 0.3 speed"],
            "parenthesised: safe after a division"
        );
        assert_eq!(
            found("#define SPIN iTime*0.5\nvec4 renderMain() { return vec4(sin(SPIN)); }\n"),
            vec!["iTime*0.5 spin"],
            "bare, and never divided by"
        );
        assert!(
            found("#define T TIME*0.3\nvec4 renderMain() { return vec4(1.0 / T); }\n").is_empty(),
            "bare and divided by: the paste would change"
        );
        assert!(
            found("#define W(TIME) (TIME * 2.0)\nvec4 renderMain() { return vec4(W(1.0)); }\n")
                .is_empty(),
            "a parameter called TIME"
        );
    }

    #[test]
    fn gradient_is_not_an_opaque_function_name() {
        let code = "vec3 gradient(vec2 uv) { return vec3(fract(uv.x * 3.0)); }\n\
                    float rand2(vec2 p) { return fract(p.x * 3.0); }\n\
                    float hash12(vec2 p) { return fract(p.x * 5.0); }\n";
        assert_eq!(
            scale_literals(code),
            vec!["3.0"],
            "`gradient` draws; `rand2` and `hash12` are number theory"
        );
    }
}
