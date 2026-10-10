// ============================================================================
// Module:       strobe_autocontrol::analyze
// Description:  Static analysis: finds every literal in a scene's GLSL that
//               could become a control, merges repeats, and derives each
//               candidate's group, name and range.
//
// Dependencies: crate::source, crate::candidate, crate::naming
// ============================================================================

//! Candidate analysis. Pure: no GL, no clock, no randomness.
//!
//! [`analyze`] walks every GLSL file of a scene in pass order and runs the
//! detectors:
//!
//! | Detector | Finds | Becomes |
//! |---|---|---|
//! | `consts` | top-level `#define N 0.3`, `const float N = 0.3;` | a float (or, as a loop bound, an integer) |
//! | `expr::time_scales` | `TIME * k`, `iTime * k`, either order | `speed`, an integrating control |
//! | `expr::loops` | `for (..; i < 64; ..)` | a capped integer, `Quality` |
//! | `expr::colours` | `vec3(r, g, b)` in `0..1` | a `color` |
//! | `expr::scales` | `uv * 3.0`, `p *= 2.0`, `sin(x * 10.0)` | a float, `Shape` |
//!
//! A pattern is only a candidate here; whether it matters is decided later
//! by measuring (the scoring stage). Candidates found at the same place in
//! a Shadertoy `Common` tab that every pass file carries a copy of, the same
//! `#define`, or the same time factor or colour written twice, are one
//! candidate with several sites, because they are one idea and one knob.
//!
//! **Order and ids are deterministic**: files in pass order, tokens in
//! source order, ids from 0.

mod consts;
mod expr;

use crate::candidate::{Candidate, Group, Kind, Rewrite, Site, ValueType, float_range, loop_range};
use crate::naming::readable;
use crate::source::{Scan, Sources};
use std::collections::HashMap;

/// One detection, before merging and before it is placed in a file.
#[derive(Debug, Clone)]
struct Hit {
    kind: Kind,
    rewrite: Rewrite,
    /// Byte span in the file.
    span: (usize, usize),
    /// The text to show for it.
    literal: String,
    /// One value, or three for a colour.
    value: Vec<f32>,
    value_type: ValueType,
    /// A readable base name.
    name: String,
    group: Group,
    /// Hits with the same key are one candidate.
    key: String,
}

/// The result of the analysis.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Analysis {
    /// Every candidate, in source order.
    pub candidates: Vec<Candidate>,
}

/// Finds the candidates in every file of `sources`.
pub fn analyze(sources: &Sources) -> Analysis {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for (path, text) in sources.iter() {
        let scan = Scan::new(path, text);
        for hit in file_hits(&scan) {
            let (line, column) = scan.line_col(hit.span.0);
            let site = Site {
                file: path.to_string(),
                line,
                column,
                start: hit.span.0,
                end: hit.span.1,
                literal: hit.literal.clone(),
                rewrite: hit.rewrite,
            };
            if let Some(&existing) = by_key.get(&hit.key) {
                if let Some(candidate) = candidates.get_mut(existing) {
                    let duplicate = candidate
                        .sites
                        .iter()
                        .any(|s| s.file == site.file && s.start == site.start);
                    if !duplicate {
                        candidate.sites.push(site);
                    }
                }
                continue;
            }
            by_key.insert(hit.key.clone(), candidates.len());
            candidates.push(candidate_from(candidates.len(), &hit, site, &scan));
        }
    }
    Analysis { candidates }
}

/// A candidate from its first hit.
fn candidate_from(id: usize, hit: &Hit, site: Site, scan: &Scan<'_>) -> Candidate {
    let (min, max) = match hit.value_type {
        ValueType::Float => {
            let (lo, hi) = float_range(hit.value[0]);
            (vec![lo], vec![hi])
        }
        ValueType::Int | ValueType::Count => {
            let (lo, hi) = loop_range(hit.value[0]);
            (vec![lo], vec![hi])
        }
        ValueType::Colour => (vec![0.0; 3], vec![1.0; 3]),
    };
    Candidate {
        id,
        kind: hit.kind,
        group: hit.group,
        value_type: hit.value_type,
        default: hit.value.clone(),
        min,
        max,
        name: hit.name.clone(),
        context: scan.line_text(hit.span.0),
        sites: vec![site],
    }
}

/// Every hit in one file, in source order, with overlaps resolved.
fn file_hits(scan: &Scan<'_>) -> Vec<Hit> {
    let mut named = consts::find(scan);
    let loops = expr::loops(scan);
    let mut hits: Vec<Hit> = Vec::new();

    // Loop bounds, literal or named.
    for found in &loops {
        if let Some(hit) = expr::loop_hit(scan, found) {
            hits.push(hit);
        } else if scan.is_any_ident(found.bound) && !found.indexes_with_var {
            let name = scan.t(found.bound);
            for constant in named.iter_mut().filter(|n| n.name == name) {
                constant.loop_bound = true;
            }
        }
    }

    // Time scales; a constant used this way belongs to the speed control.
    for (hit, constant) in expr::time_scales(scan, &named) {
        if let Some(name) = constant {
            for owner in named.iter_mut().filter(|n| n.name == name) {
                owner.time_used = true;
            }
        }
        hits.push(hit);
    }

    hits.extend(expr::colours(scan));
    hits.extend(expr::scales(scan));
    hits.extend(named.iter().filter_map(named_hit));

    resolve_overlaps(hits)
}

/// The hit for a named constant, if it is a good candidate.
fn named_hit(named: &consts::Named) -> Option<Hit> {
    if !named.safe
        || named.time_used
        || named.uses == 0
        || consts::is_math_constant(&named.name, f64::from(named.value))
    {
        return None;
    }
    // Integers are only candidates as loop bounds: an `int` constant is
    // otherwise a mode switch, an array size or a count of things.
    if !named.is_float && !named.loop_bound {
        return None;
    }
    let loop_count = named.loop_bound;
    if loop_count && !(4.0..=4096.0).contains(&named.value) {
        return None;
    }
    let name = readable(&named.name);
    let (value_type, group) = if loop_count {
        (
            if named.is_float {
                ValueType::Count
            } else {
                ValueType::Int
            },
            Group::Quality,
        )
    } else {
        (ValueType::Float, group_for_name(&name))
    };
    let (kind, rewrite) = match named.form {
        consts::Form::Define => (Kind::Define, Rewrite::DefineBody),
        consts::Form::Const => (Kind::Const, Rewrite::ConstStatement),
    };
    let key = format!(
        "{}:{}={}",
        kind.label().replace(' ', "_"),
        named.name,
        named.literal
    );
    Some(Hit {
        kind,
        rewrite,
        span: named.span,
        literal: named.literal.clone(),
        value: vec![named.value],
        value_type,
        name,
        group,
        key,
    })
}

/// The group a plain float constant belongs to, judged by its name.
fn group_for_name(snake: &str) -> Group {
    const MOTION: [&str; 7] = [
        "speed", "rate", "tempo", "velocity", "spin", "flow", "drift",
    ];
    const COLOUR: [&str; 12] = [
        "col", "color", "colour", "tint", "hue", "sat", "bright", "glow", "fade", "alpha",
        "contrast", "exposure",
    ];
    if MOTION.iter().any(|word| snake.contains(word)) {
        Group::Motion
    } else if COLOUR.iter().any(|word| snake.contains(word)) {
        Group::Colour
    } else {
        Group::Shape
    }
}

/// Drops hits that overlap a more specific one, and sorts by position.
///
/// Priority is colour, time scale, loop bound, frequency, scale, then named
/// constants; two hits only overlap when one detector's pattern contains
/// another's number.
fn resolve_overlaps(mut hits: Vec<Hit>) -> Vec<Hit> {
    let rank = |hit: &Hit| match hit.kind {
        Kind::Colour => 0,
        Kind::TimeScale => 1,
        Kind::LoopBound => 2,
        Kind::Frequency => 3,
        Kind::Scale => 4,
        Kind::Define | Kind::Const => 5,
    };
    hits.sort_by_key(|hit| (rank(hit), hit.span.0));
    let mut accepted: Vec<Hit> = Vec::new();
    for hit in hits {
        let clash = accepted
            .iter()
            .any(|kept| hit.span.0 < kept.span.1 && kept.span.0 < hit.span.1);
        if !clash {
            accepted.push(hit);
        }
    }
    accepted.sort_by_key(|hit| hit.span.0);
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::collections::BTreeMap;
    use strobe_scene::SceneFolder;
    use strobe_scene::scene::SceneDef;

    fn sources_of(files: &[(&str, &str)]) -> Result<Sources> {
        let names: Vec<String> = files.iter().map(|(n, _)| format!("\"{n}\"")).collect();
        let passes: Vec<String> = (1..files.len())
            .map(|i| format!("{{\"TARGET\": \"t{i}\"}}"))
            .collect();
        let json = format!(
            "{{\"PASSES\": [{}], \"PASS_SOURCES\": [{}]}}",
            passes.join(","),
            names.join(",")
        );
        let by_name: BTreeMap<&str, &str> = files.iter().copied().collect();
        let folder = SceneFolder {
            dir: std::path::PathBuf::from("."),
            def: SceneDef::parse(&json, "t")?,
            shader: by_name.get("main.glsl").copied().unwrap_or("").to_string(),
            pass_shaders: files.iter().map(|(_, t)| (*t).to_string()).collect(),
            script: None,
        };
        Ok(Sources::from_folder(&folder))
    }

    fn analyse_one(code: &str) -> Result<Analysis> {
        let folder = SceneFolder {
            dir: std::path::PathBuf::from("."),
            def: SceneDef::parse("{}", "t")?,
            shader: code.to_string(),
            pass_shaders: Vec::new(),
            script: None,
        };
        Ok(analyze(&Sources::from_folder(&folder)))
    }

    fn summary(analysis: &Analysis) -> Vec<(Kind, String, String)> {
        analysis
            .candidates
            .iter()
            .map(|c| (c.kind, c.name.clone(), c.literal().to_string()))
            .collect()
    }

    #[test]
    fn every_kind_is_found_in_one_scene() -> Result<()> {
        let code = "\
#define GLOW 0.7
const float SOFT = 1.5;
vec4 renderMain() {
    float t = TIME * 0.3;
    vec3 sky = vec3(0.2, 0.4, 0.9);
    vec2 grid = _uv * 6.0;
    float acc = 0.0;
    for (int i = 0; i < 48; i++) { acc += GLOW * SOFT; }
    return vec4(sky * acc + t + grid.x, 1.0);
}
";
        let got = summary(&analyse_one(code)?);
        let kinds: Vec<Kind> = got.iter().map(|(k, ..)| *k).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::Define,
                Kind::Const,
                Kind::TimeScale,
                Kind::Colour,
                Kind::Scale,
                Kind::LoopBound
            ],
            "one of each, in source order: {got:?}"
        );
        Ok(())
    }

    #[test]
    fn candidates_record_where_they_are() -> Result<()> {
        let code =
            "float a;\nvec4 renderMain() {\n    float t = TIME * 0.3;\n    return vec4(t);\n}\n";
        let analysis = analyse_one(code)?;
        let site = analysis.candidates[0].sites[0].clone();
        assert_eq!(site.file, "main.glsl", "the pass file");
        assert_eq!((site.line, site.column), (3, 15), "line and column");
        assert_eq!(&code[site.start..site.end], "TIME * 0.3", "the byte span");
        assert_eq!(site.literal, "TIME * 0.3", "the literal text");
        Ok(())
    }

    #[test]
    fn ranges_follow_the_value() -> Result<()> {
        let code = "\
#define NEG -0.5
#define POS 3.0
vec4 renderMain() {
    float a = NEG + POS;
    for (int i = 0; i < 64; i++) { a += 1.0; }
    return vec4(a);
}
";
        let analysis = analyse_one(code)?;
        let range = |name: &str| {
            analysis
                .candidates
                .iter()
                .find(|c| c.name == name)
                .map(|c| (c.min[0], c.max[0]))
        };
        assert_eq!(range("neg"), Some((-1.0, 1.0)), "negative: symmetric");
        assert_eq!(range("pos"), Some((0.0, 6.0)), "positive: zero to twice");
        assert_eq!(
            range("steps"),
            Some((16.0, 128.0)),
            "a loop: capped at twice"
        );
        Ok(())
    }

    #[test]
    fn a_define_that_scales_time_belongs_to_the_speed_control() -> Result<()> {
        let code = "#define RATE 0.4\nvec4 renderMain() { return vec4(sin(TIME * RATE)); }\n";
        let got = summary(&analyse_one(code)?);
        assert_eq!(
            got,
            vec![(
                Kind::TimeScale,
                "speed".to_string(),
                "TIME * RATE".to_string()
            )],
            "one speed control, and the define is not offered twice"
        );
        Ok(())
    }

    #[test]
    fn an_int_define_is_only_a_candidate_as_a_loop_bound() -> Result<()> {
        let code = "\
#define STEPS 32
#define MODE 2
vec4 renderMain() {
    float a = float(MODE);
    for (int i = 0; i < STEPS; i++) { a += 1.0; }
    return vec4(a);
}
";
        let got = summary(&analyse_one(code)?);
        assert_eq!(
            got,
            vec![(Kind::Define, "steps".to_string(), "32".to_string())],
            "STEPS bounds a loop; MODE is a switch"
        );
        let analysis = analyse_one(code)?;
        assert_eq!(
            analysis.candidates[0].group,
            Group::Quality,
            "quality group"
        );
        assert_eq!(analysis.candidates[0].value_type, ValueType::Int, "an int");
        Ok(())
    }

    #[test]
    fn repeated_time_scales_and_colours_are_one_candidate_with_two_sites() -> Result<()> {
        let code = "\
vec4 renderMain() {
    float a = sin(TIME * 0.5);
    float b = cos(TIME * 0.5);
    vec3 x = vec3(0.9, 0.4, 0.1);
    vec3 y = mix(x, vec3(0.9, 0.4, 0.1), a);
    return vec4(y * b, 1.0);
}
";
        let analysis = analyse_one(code)?;
        let sites: Vec<usize> = analysis.candidates.iter().map(|c| c.sites.len()).collect();
        assert_eq!(
            sites,
            vec![2, 2],
            "the speed and the colour, two sites each"
        );
        Ok(())
    }

    #[test]
    fn a_common_tab_copied_into_two_passes_is_one_candidate() -> Result<()> {
        let common = "// ---- Shadertoy tab: Common ----\n#define BLOOM 0.8\n";
        let tail = "// ---- Strobe entry (generated) ----\n";
        let a = [
            "float shim_a;\n",
            common,
            "// ---- Shadertoy tab: Buffer A ----\n",
            "void mainImage(out vec4 c, in vec2 f) { c = vec4(BLOOM); }\n",
            tail,
        ]
        .concat();
        let b = [
            "float shim_b;\nmore shim;\n",
            common,
            "// ---- Shadertoy tab: Image ----\n",
            "void mainImage(out vec4 c, in vec2 f) { c = vec4(BLOOM * 2.0); }\n",
            tail,
        ]
        .concat();
        let sources = sources_of(&[("passes/a.glsl", &a), ("main.glsl", &b)])?;
        let analysis = analyze(&sources);
        assert_eq!(analysis.candidates.len(), 1, "{:?}", summary(&analysis));
        let files: Vec<&str> = analysis.candidates[0]
            .sites
            .iter()
            .map(|s| s.file.as_str())
            .collect();
        assert_eq!(
            files,
            vec!["passes/a.glsl", "main.glsl"],
            "both copies are sites"
        );
        Ok(())
    }

    #[test]
    fn two_buffers_that_reuse_a_define_name_with_different_values_stay_apart() -> Result<()> {
        let a = "#define SCALE 2.0\nvec4 renderMain() { return vec4(SCALE); }\n";
        let b = "#define SCALE 8.0\nvec4 renderMain() { return vec4(SCALE); }\n";
        let sources = sources_of(&[("passes/a.glsl", a), ("main.glsl", b)])?;
        let analysis = analyze(&sources);
        let values: Vec<f32> = analysis.candidates.iter().map(|c| c.default[0]).collect();
        assert_eq!(
            values,
            vec![2.0, 8.0],
            "different values are different knobs"
        );
        Ok(())
    }

    #[test]
    fn the_shim_of_a_converted_pass_is_not_analysed() -> Result<()> {
        let code = "\
vec3 shim = vec3(0.2, 0.4, 0.9);
// ---- Shadertoy tab: Image ----
void mainImage(out vec4 c, in vec2 f) { c = vec4(1.0); }
// ---- Strobe entry (generated) ----
vec4 renderMain() { float t = TIME * 0.3; return vec4(t); }
";
        let analysis = analyse_one(code)?;
        assert!(
            analysis.candidates.is_empty(),
            "only the author's tab is scanned: {:?}",
            summary(&analysis)
        );
        Ok(())
    }

    #[test]
    fn lines_are_untouched_by_analysis_and_ids_are_sequential() -> Result<()> {
        let code = "#define A 0.7\n#define B 0.9\nvec4 renderMain() { return vec4(A * B); }\n";
        let analysis = analyse_one(code)?;
        let ids: Vec<usize> = analysis.candidates.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![0, 1], "ids count from zero in source order");
        let again = analyse_one(code)?;
        assert_eq!(analysis, again, "the analysis is deterministic");
        Ok(())
    }
}
