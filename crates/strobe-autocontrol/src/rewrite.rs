// ============================================================================
// Module:       strobe_autocontrol::rewrite
// Description:  Applies chosen candidates to a copy of the scene's GLSL:
//               literals become uniform references, line numbers unchanged,
//               and the matching CONTROLS entries are written.
//
// Dependencies: crate::candidate, crate::source, strobe-scene (ControlDef), serde_json
// ============================================================================

//! The rewrite.
//!
//! Controls become uniforms by themselves: the assembler declares one
//! `uniform` per `CONTROLS` entry in every pass program. So rewriting a
//! scene is two edits and no declarations:
//!
//! 1. in the GLSL, each chosen site's span is replaced by an expression of
//!    the control's uniform ([`site_text`]);
//! 2. in `scene.json`, a `CONTROLS` entry is added ([`control_line`]).
//!
//! ## Line numbers do not move
//!
//! A driver's error message and the app's "pass file: line" report quote
//! lines of the file. Every replacement is on the line it started on, and a
//! span that crossed lines gets the same number of newlines appended after
//! its replacement, so no later line shifts.
//!
//! ## What each site becomes
//!
//! | Site | Replacement |
//! |---|---|
//! | float literal | `name` |
//! | integer loop bound | `int(name + 0.5)` |
//! | float loop bound | `floor(name + 0.5)` |
//! | colour constructor | `name` (a `vec3` uniform) |
//! | `TIME * k` | `name` (a `slider speed`, which integrates `k` per second) |
//! | `#define N 0.3` | `#define N (name)` |
//! | `const float N = 0.3;` | `#define N (name)` (a `const` cannot read a uniform) |
//!
//! With an audio binding (opt-in, off by default) a float or colour site
//! reads `(name * (1.0 + name_audio * syn_BassLevel))`, which is `name`
//! exactly while `name_audio` stays at its default of 0.

use crate::candidate::{Candidate, Rewrite, Site, ValueType};
use crate::source::Sources;
use std::collections::BTreeMap;
use strobe_shadertoy::glsl::{self, Kind};

/// A reactive mapping generated into the GLSL and `CONTROLS`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AudioBind {
    /// The name of the `<name>_audio` amount control.
    pub amount: String,
    /// The `syn_*` uniform that drives it.
    pub band: String,
}

/// A candidate chosen to become a control, with its final name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PlannedControl {
    /// Index into the candidate list (the candidate's `id`).
    pub candidate: usize,
    /// The uniform's name.
    pub name: String,
    /// An opt-in audio mapping.
    pub audio: Option<AudioBind>,
}

/// The expression that stands for the control's value at a site.
fn value_expression(candidate: &Candidate, name: &str, audio: Option<&AudioBind>) -> String {
    let base = match candidate.value_type {
        ValueType::Float | ValueType::Colour => name.to_string(),
        ValueType::Int => format!("int({name} + 0.5)"),
        ValueType::Count => format!("floor({name} + 0.5)"),
    };
    match audio {
        Some(bind) => format!("({base} * (1.0 + {} * {}))", bind.amount, bind.band),
        None => base,
    }
}

/// The name a `const float|int NAME = ...;` statement declares.
fn const_name(statement: &str) -> Option<String> {
    let mut idents = glsl::lex(statement)
        .into_iter()
        .filter(|token| token.kind == Kind::Ident);
    idents.next()?; // const
    idents.next()?; // the type
    idents.next().map(|token| token.text.to_string())
}

/// What replaces `site`'s span.
pub fn site_text(
    candidate: &Candidate,
    site: &Site,
    original: &str,
    name: &str,
    audio: Option<&AudioBind>,
) -> String {
    let expression = value_expression(candidate, name, audio);
    match site.rewrite {
        Rewrite::Value | Rewrite::Integrator => expression,
        Rewrite::DefineBody => format!("({expression})"),
        Rewrite::ConstStatement => match const_name(original) {
            Some(constant) => format!("#define {constant} ({expression})"),
            None => original.to_string(),
        },
    }
}

/// `sources` with every planned site replaced.
///
/// Edits are applied from the end of each file backwards so earlier byte
/// offsets stay valid. Overlapping edits cannot happen (the analysis
/// resolved overlaps) but would be skipped rather than corrupt the file.
pub fn apply(sources: &Sources, candidates: &[Candidate], planned: &[PlannedControl]) -> Sources {
    let mut edits: BTreeMap<&str, Vec<(usize, usize, String)>> = BTreeMap::new();
    for plan in planned {
        let Some(candidate) = candidates.get(plan.candidate) else {
            continue;
        };
        for site in &candidate.sites {
            let Some(text) = sources.text(&site.file) else {
                continue;
            };
            let original = text.get(site.start..site.end).unwrap_or("");
            let mut replacement =
                site_text(candidate, site, original, &plan.name, plan.audio.as_ref());
            // Keep every later line where it was.
            replacement.extend(original.chars().filter(|c| *c == '\n'));
            edits
                .entry(site.file.as_str())
                .or_default()
                .push((site.start, site.end, replacement));
        }
    }
    let mut out = sources.clone();
    for (file, mut list) in edits {
        let Some(text) = sources.text(file) else {
            continue;
        };
        list.sort_by_key(|(start, ..)| std::cmp::Reverse(*start));
        let mut edited = text.to_string();
        let mut floor = usize::MAX;
        for (start, end, replacement) in list {
            if end > floor || edited.get(start..end).is_none() {
                continue;
            }
            edited.replace_range(start..end, &replacement);
            floor = start;
        }
        out.set(file, edited);
    }
    out
}

/// A JSON number the way scenes write them: always with a decimal point.
fn number(value: f32) -> String {
    format!("{value:?}")
}

fn numbers(values: &[f32]) -> String {
    let parts: Vec<String> = values.iter().map(|v| number(*v)).collect();
    format!("[{}]", parts.join(", "))
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// A readable account of where a control came from, for `DESCRIPTION`.
pub fn describe(candidate: &Candidate) -> String {
    let more = candidate.sites.len().saturating_sub(1);
    let place = candidate
        .primary()
        .map_or_else(String::new, |site| format!("{}:{}", site.file, site.line));
    let was = match candidate.value_type {
        ValueType::Colour => format!("colour {}", candidate.literal()),
        ValueType::Float | ValueType::Int | ValueType::Count => {
            format!("{} `{}`", candidate.kind.label(), candidate.literal())
        }
    };
    let extra = match more {
        0 => String::new(),
        1 => ", and 1 more place".to_string(),
        n => format!(", and {n} more places"),
    };
    format!("Generated from {was} at {place}{extra}.")
}

/// One `CONTROLS` entry as a single line of JSON.
///
/// `scoring` writes the plain variant the scorer uses (no smoothing, so a
/// nudged value takes effect on the next frame); otherwise the performance
/// variant, with smoothing where it helps.
pub fn control_line(candidate: &Candidate, name: &str, scoring: bool) -> String {
    let single = |values: &[f32]| number(values.first().copied().unwrap_or(0.0));
    let ty = match candidate.value_type {
        ValueType::Colour if scoring => "color",
        ValueType::Colour => "color smooth",
        ValueType::Int | ValueType::Count => "slider",
        ValueType::Float if candidate.is_integrator() => "slider speed",
        ValueType::Float if scoring => "slider",
        ValueType::Float => "slider smooth",
    };
    let params = if ty.ends_with("smooth") {
        ", \"PARAMS\": 0.1"
    } else {
        ""
    };
    let body = if candidate.value_type == ValueType::Colour {
        format!("\"DEF_COLOR\": {}", numbers(&candidate.default))
    } else {
        format!(
            "\"MIN\": {}, \"MAX\": {}, \"DEFAULT\": {}",
            single(&candidate.min),
            single(&candidate.max),
            single(&candidate.default)
        )
    };
    let description = if scoring {
        String::new()
    } else {
        format!(", \"DESCRIPTION\": {}", quoted(&describe(candidate)))
    };
    format!(
        "{{ \"NAME\": {}, \"TYPE\": \"{ty}\", {body}{params}, \"UI_GROUP\": {}{description} }}",
        quoted(name),
        quoted(candidate.group.ui_name()),
    )
}

/// The `<name>_audio` amount control for an audio binding.
pub fn audio_control_line(candidate: &Candidate, bind: &AudioBind) -> String {
    let what = match candidate.value_type {
        ValueType::Colour => "brightness",
        ValueType::Float | ValueType::Int | ValueType::Count => "the value",
    };
    let text = format!(
        "How much {} pushes {what}. 0 is off.",
        bind.band.trim_start_matches("syn_")
    );
    format!(
        "{{ \"NAME\": {}, \"TYPE\": \"slider smooth\", \"MIN\": 0.0, \"MAX\": 1.0, \"DEFAULT\": 0.0, \"PARAMS\": 0.1, \"UI_GROUP\": \"Audio\", \"DESCRIPTION\": {} }}",
        quoted(&bind.amount),
        quoted(&text)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::analyze;
    use anyhow::Result;
    use strobe_scene::SceneFolder;
    use strobe_scene::control::ControlDef;
    use strobe_scene::json::RawControl;
    use strobe_scene::scene::SceneDef;

    fn folder(code: &str) -> Result<SceneFolder> {
        Ok(SceneFolder {
            dir: std::path::PathBuf::from("."),
            def: SceneDef::parse("{}", "t")?,
            shader: code.to_string(),
            pass_shaders: Vec::new(),
            script: None,
        })
    }

    /// Rewrites every candidate in `code` under its base name; returns the
    /// new text.
    fn rewrite_all(code: &str) -> Result<(String, Vec<Candidate>)> {
        let base = folder(code)?;
        let sources = Sources::from_folder(&base);
        let analysis = analyze(&sources);
        let planned: Vec<PlannedControl> = analysis
            .candidates
            .iter()
            .map(|c| PlannedControl {
                candidate: c.id,
                name: format!("{}_{}", c.name, c.id),
                audio: None,
            })
            .collect();
        let out = apply(&sources, &analysis.candidates, &planned);
        let text = out.text("main.glsl").unwrap_or("").to_string();
        Ok((text, analysis.candidates))
    }

    #[test]
    fn each_kind_becomes_its_expression() -> Result<()> {
        let code = "\
#define GLOW 0.7
const float SOFT = 1.5;
vec4 renderMain() {
    float t = TIME * 0.3;
    vec3 sky = vec3(0.2, 0.4, 0.9);
    vec2 grid = _uv * 6.0;
    for (int i = 0; i < 48; i++) { t += GLOW * SOFT; }
    for (float j = 0.0; j < 12.0; j++) { t += j; }
    return vec4(sky * t + grid.x, 1.0);
}
";
        let (out, candidates) = rewrite_all(code)?;
        let expected = "\
#define GLOW (glow_0)
#define SOFT (soft_1)
vec4 renderMain() {
    float t = speed_2;
    vec3 sky = sky_color_3;
    vec2 grid = _uv * grid_scale_4;
    for (int i = 0; i < int(steps_5 + 0.5); i++) { t += GLOW * SOFT; }
    for (float j = 0.0; j < floor(steps_6 + 0.5); j++) { t += j; }
    return vec4(sky * t + grid.x, 1.0);
}
";
        assert_eq!(out, expected, "{} candidates", candidates.len());
        Ok(())
    }

    #[test]
    fn line_numbers_do_not_move_even_when_a_span_crossed_lines() -> Result<()> {
        let code = "vec4 renderMain() {\n    vec3 c = vec3(0.9,\n                  0.4,\n                  0.1);\n    float after = 1.0;\n    return vec4(c, after);\n}\n";
        let (out, _) = rewrite_all(code)?;
        assert_eq!(
            out.lines().count(),
            code.lines().count(),
            "the line count is unchanged"
        );
        let line_of = |text: &str, needle: &str| {
            text.lines()
                .position(|l| l.contains(needle))
                .unwrap_or(usize::MAX)
        };
        assert_eq!(
            line_of(&out, "after = 1.0"),
            line_of(code, "after = 1.0"),
            "the code after the span is on the same line"
        );
        Ok(())
    }

    #[test]
    fn a_negative_define_is_replaced_with_its_sign() -> Result<()> {
        let (out, _) =
            rewrite_all("#define OFFSET -0.5\nvec4 renderMain() { return vec4(OFFSET); }\n")?;
        assert!(out.starts_with("#define OFFSET (offset_0)\n"), "{out}");
        Ok(())
    }

    #[test]
    fn the_audio_binding_wraps_the_expression_and_is_inert_at_zero() -> Result<()> {
        let base = folder("vec4 renderMain() { vec2 g = _uv * 6.0; return vec4(g, 0.0, 1.0); }\n")?;
        let sources = Sources::from_folder(&base);
        let analysis = analyze(&sources);
        let planned = [PlannedControl {
            candidate: 0,
            name: "scale".to_string(),
            audio: Some(AudioBind {
                amount: "scale_audio".to_string(),
                band: "syn_BassLevel".to_string(),
            }),
        }];
        let out = apply(&sources, &analysis.candidates, &planned);
        let text = out.text("main.glsl").unwrap_or("");
        assert!(
            text.contains("_uv * (scale * (1.0 + scale_audio * syn_BassLevel))"),
            "{text}"
        );
        Ok(())
    }

    #[test]
    fn generated_control_lines_are_valid_scene_controls() -> Result<()> {
        let code = "\
#define GLOW 0.7
vec4 renderMain() {
    vec3 sky = vec3(0.2, 0.4, 0.9);
    float t = TIME * 0.3;
    float a = 0.0;
    for (int i = 0; i < 48; i++) { a += GLOW; }
    return vec4(sky * a + t, 1.0);
}
";
        let base = folder(code)?;
        let analysis = analyze(&Sources::from_folder(&base));
        for candidate in &analysis.candidates {
            for scoring in [true, false] {
                let line = control_line(candidate, "my_control", scoring);
                let raw: RawControl = serde_json::from_str(&line)?;
                let def = ControlDef::from_raw(&raw)?;
                assert_eq!(def.name, "my_control", "{line}");
                assert_eq!(
                    def.default[0], candidate.default[0],
                    "the default is the original value: {line}"
                );
                if candidate.value_type != ValueType::Colour {
                    assert_eq!(
                        (def.min[0], def.max[0]),
                        (candidate.min[0], candidate.max[0]),
                        "range: {line}"
                    );
                }
            }
        }
        let kinds: Vec<&str> = analysis
            .candidates
            .iter()
            .map(|c| if c.is_integrator() { "speed" } else { "other" })
            .collect();
        assert!(kinds.contains(&"speed"), "an integrator was exercised");
        Ok(())
    }

    #[test]
    fn a_time_scale_becomes_a_speed_control() -> Result<()> {
        let base = folder("vec4 renderMain() { return vec4(TIME * 0.3); }\n")?;
        let analysis = analyze(&Sources::from_folder(&base));
        let line = control_line(&analysis.candidates[0], "speed", false);
        let raw: RawControl = serde_json::from_str(&line)?;
        assert_eq!(raw.ty, "slider speed", "{line}");
        Ok(())
    }
}
