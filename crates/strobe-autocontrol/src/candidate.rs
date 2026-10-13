// ============================================================================
// Module:       strobe_autocontrol::candidate
// Description:  The vocabulary of the engine: a candidate literal, where it
//               sits, what kind of control it would become, and its range.
//
// Dependencies: serde
// ============================================================================

//! Candidates.
//!
//! A [`Candidate`] is one number (or one colour) in a scene's GLSL that
//! could become a control. It may have several [`Site`]s: the same
//! `#define` used from three pass files is one candidate with the
//! definition as its single site, and `iTime * 0.3` written in two places
//! is one candidate with two. Everything here is plain data; the analysis
//! that finds candidates is in [`crate::analyze`] and the rewrite that
//! applies them is in [`crate::rewrite`].

use serde::Serialize;

/// What the literal was doing, which decides how it is rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The value of a top-level `#define NAME <number>`.
    Define,
    /// The value of a top-level `const float|int NAME = <number>;`.
    Const,
    /// The factor in `TIME * k`, `k * TIME`, `iTime * k`, or `k * iTime`.
    TimeScale,
    /// The bound of a `for` loop: `i < 64`.
    LoopBound,
    /// A `vec3(r, g, b)` (or `vec4(r, g, b, a)`) of literals in `0..1`.
    Colour,
    /// A factor applied to a coordinate: `uv * 3.0`, `p *= 2.0`.
    Scale,
    /// A factor inside `sin`/`cos` applied to a variable: `sin(x * 10.0)`.
    Frequency,
}

impl Kind {
    /// The word reports and descriptions use.
    pub fn label(self) -> &'static str {
        match self {
            Self::Define => "define",
            Self::Const => "const",
            Self::TimeScale => "time scale",
            Self::LoopBound => "loop bound",
            Self::Colour => "colour",
            Self::Scale => "scale",
            Self::Frequency => "frequency",
        }
    }
}

/// The control group a candidate lands in. The declaration order is the
/// order the generated controls are listed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// How fast things move.
    Motion,
    /// Size, scale, frequency and other geometry.
    Shape,
    /// Colour and brightness.
    Colour,
    /// Loop counts: cost against fidelity.
    Quality,
}

impl Group {
    /// The `UI_GROUP` string written to `scene.json`.
    pub fn ui_name(self) -> &'static str {
        match self {
            Self::Motion => "Motion",
            Self::Shape => "Shape",
            Self::Colour => "Colour",
            Self::Quality => "Quality",
        }
    }
}

/// What the value is, which decides the uniform's type and the range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    /// A float, ranged `[0, 2x]` (symmetric when negative).
    Float,
    /// An integer the GLSL expects as `int` (a loop count); the uniform is
    /// a float slider and the site reads `int(name + 0.5)`.
    Int,
    /// An integer-valued float (`i < 30.0`); the site reads
    /// `floor(name + 0.5)`.
    Count,
    /// Three components in `0..1`: a `color` control.
    Colour,
}

impl ValueType {
    /// Whether the control only makes sense at whole numbers.
    pub fn is_integral(self) -> bool {
        match self {
            Self::Int | Self::Count => true,
            Self::Float | Self::Colour => false,
        }
    }
}

/// How one site is rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rewrite {
    /// The span is a literal (or a colour constructor's components) and is
    /// replaced by the uniform expression.
    Value,
    /// The span is `TIME * k` (either order) and is replaced by a `speed`
    /// control's output, which integrates `k` per second, so changing the
    /// speed live does not jump the phase.
    Integrator,
    /// The span is a `#define` body: it becomes `(expression)`.
    DefineBody,
    /// The span is a whole `const T NAME = literal;` statement: it becomes
    /// `#define NAME (expression)`, because a `const` cannot read a uniform.
    ConstStatement,
}

/// One place in the source a candidate is applied at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Site {
    /// The pass file the text is in, as `PASS_SOURCES` names it
    /// (`main.glsl` for a scene without `PASS_SOURCES`).
    pub file: String,
    /// 1-based line in that file.
    pub line: u32,
    /// 1-based column (in characters) in that line.
    pub column: u32,
    /// Byte span `[start, end)` in the file.
    pub start: usize,
    /// Exclusive end of the span.
    pub end: usize,
    /// The original text of the span.
    pub literal: String,
    /// How the span is replaced.
    pub rewrite: Rewrite,
}

/// A literal in the scene that could be a control.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    /// Stable position in the analysis order, from 0.
    pub id: usize,
    /// What the literal was doing.
    pub kind: Kind,
    /// Where its control would be listed.
    pub group: Group,
    /// What the value is.
    pub value_type: ValueType,
    /// The original value, per component (one, or three for a colour).
    pub default: Vec<f32>,
    /// The control's lower bound, per component.
    pub min: Vec<f32>,
    /// The control's upper bound, per component.
    pub max: Vec<f32>,
    /// A readable base name from the identifier or context. It is not yet
    /// unique; [`crate::naming`] makes the final names.
    pub name: String,
    /// What tells this candidate apart from another with the same `name`
    /// in the same expression (`low`/`high` for the ends of a `mix`, `x`/`y`
    /// for a vector's components). [`crate::naming`] uses it before a number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The text of the line the first site is on, trimmed, for reports.
    pub context: String,
    /// Where the literal is, first site first.
    pub sites: Vec<Site>,
}

impl Candidate {
    /// The original literal text of the first site.
    pub fn literal(&self) -> &str {
        self.sites.first().map_or("", |site| site.literal.as_str())
    }

    /// Whether this is a speed control (`TIME * k` becomes an integrator).
    pub fn is_integrator(&self) -> bool {
        self.sites
            .iter()
            .any(|site| site.rewrite == Rewrite::Integrator)
    }

    /// The first site, which is where the report says the candidate is.
    pub fn primary(&self) -> Option<&Site> {
        self.sites.first()
    }
}

/// The hard cap on a loop-count control's maximum (decision D97): a slider
/// must not be able to hang a 2012 integrated GPU.
pub const LOOP_CAP: f32 = 256.0;

/// The range `[min, max]` for a float default `x`: `[0, 2x]`, or
/// `[-2|x|, 2|x|]` when `x` is negative.
pub fn float_range(x: f32) -> (f32, f32) {
    if x < 0.0 {
        (2.0 * x, -2.0 * x)
    } else {
        (0.0, 2.0 * x)
    }
}

/// The range for a loop bound `n`: from a quarter of it (at least 1) to
/// twice it, never above [`LOOP_CAP`] unless the original already is.
/// The maximum is therefore at most `2n`, as the hang guard requires.
pub fn loop_range(n: f32) -> (f32, f32) {
    let low = (n / 4.0).round().max(1.0).min(n);
    let high = (2.0 * n).min(LOOP_CAP.max(n));
    (low, high)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_ranges_follow_the_sign() {
        assert_eq!(float_range(3.0), (0.0, 6.0), "positive: zero to twice");
        assert_eq!(float_range(-0.5), (-1.0, 1.0), "negative: symmetric");
        assert_eq!(float_range(0.0), (0.0, 0.0), "zero stays zero");
    }

    #[test]
    fn loop_ranges_never_exceed_twice_the_default_or_the_cap() {
        for n in [4.0_f32, 5.0, 16.0, 64.0, 110.0, 200.0, 256.0, 400.0] {
            let (low, high) = loop_range(n);
            assert!(high <= 2.0 * n, "{n}: max {high} is over twice the default");
            assert!(
                high <= LOOP_CAP.max(n),
                "{n}: max {high} is over the cap or the default"
            );
            assert!(
                (1.0..=n).contains(&low),
                "{n}: min {low} must be 1..=default"
            );
            assert!(high >= n, "{n}: the default must be inside the range");
        }
        assert_eq!(loop_range(64.0), (16.0, 128.0), "a typical raymarch loop");
    }
}
