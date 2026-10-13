// ============================================================================
// Module:       strobe_autocontrol::naming
// Description:  Readable, unique, legal control names: snake_case from the
//               source identifier, numbered when shared, never colliding.
//
// Dependencies: strobe-scene (names, SceneFolder), strobe-shadertoy (glsl)
// ============================================================================

//! Control names.
//!
//! A generated control becomes a GLSL `uniform` in every pass of the scene,
//! so its name must not collide with anything: not an existing control (by
//! the script API's matching rule, which ignores case and `_`), not a
//! reserved Strobe or SSF name, not a GLSL keyword or built-in, and not any
//! identifier already in the scene's code or in the SSF function library,
//! because a local variable called `zoom` would silently shadow a uniform
//! called `zoom` at the very site that was rewritten to use it.
//!
//! The "Strobe" group's `st_` prefix (D92) is avoided as well, so a
//! generated control never looks like one of the five every import carries.

use crate::source::Sources;
use std::collections::BTreeSet;
use strobe_scene::SceneFolder;
use strobe_scene::names::{is_glsl_identifier, is_reserved, normalize};
use strobe_shadertoy::glsl;

/// GLSL keywords, reserved words and built-ins a uniform must not be
/// called.
const GLSL_WORDS: &str = "\
active asm atomic_uint attribute bool break buffer bvec2 bvec3 bvec4 case cast centroid class \
coherent common const continue default discard dmat2 dmat3 dmat4 do double dvec2 dvec3 dvec4 \
else enum extern external false filter fixed flat float for fvec2 fvec3 fvec4 goto half hvec2 \
hvec3 hvec4 highp if iimage2D image2D in inline inout input int interface invariant isampler2D \
ivec2 ivec3 ivec4 layout long lowp mat2 mat3 mat4 mediump namespace noinline noperspective out \
output packed partition patch precision public readonly resource restrict return sample sampler2D \
sampler3D samplerCube shared short sizeof smooth static struct subroutine superp switch template \
this true typedef uint uniform union unsigned using varying vec2 vec3 vec4 void volatile while \
writeonly \
abs acos acosh all any asin asinh atan atanh ceil clamp cos cosh cross degrees determinant \
distance dot equal exp exp2 faceforward floor fract fwidth greaterThan inverse inversesqrt \
isinf isnan length lessThan log log2 matrixCompMult max min mix mod modf noise normalize not \
notEqual outerProduct pow radians reflect refract round roundEven sign sin sinh smoothstep sqrt \
step tan tanh texture texelFetch transpose trunc \
main";

/// Converts an identifier of any style to `snake_case`:
/// `GLOW_STRENGTH`, `glowStrength` and `GlowStrength` all give
/// `glow_strength`, and `RGBColor` gives `rgb_color`.
pub fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (index, &c) in chars.iter().enumerate() {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if c.is_ascii_uppercase() {
            let previous = index.checked_sub(1).and_then(|p| chars.get(p)).copied();
            let next = chars.get(index + 1).copied();
            let after_lower_or_digit =
                previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit());
            let acronym_end = previous.is_some_and(|p| p.is_ascii_uppercase())
                && next.is_some_and(|n| n.is_ascii_lowercase());
            if (after_lower_or_digit || acronym_end) && !out.ends_with('_') {
                out.push('_');
            }
        }
        out.push(c.to_ascii_lowercase());
    }
    let mut collapsed = String::with_capacity(out.len());
    for c in out.chars() {
        if !(c == '_' && collapsed.ends_with('_')) {
            collapsed.push(c);
        }
    }
    collapsed.trim_matches('_').to_string()
}

/// Prefixes that belong to Strobe or GL, not to what a value means.
const NOISE_PREFIXES: [&str; 4] = ["strobe_", "gl_", "st_", "syn_"];

/// The rest of `text` after a Material Maker node prefix (`p_o12345_`, or
/// `o12345_`): the node's number says nothing about what a value does.
fn strip_node_prefix(text: &str) -> Option<&str> {
    let after_p = text.strip_prefix("p_").unwrap_or(text);
    let digits = after_p.strip_prefix('o')?;
    let count = digits.chars().take_while(char::is_ascii_digit).count();
    if count == 0 {
        return None;
    }
    digits
        .get(count..)?
        .strip_prefix('_')
        .filter(|rest| !rest.is_empty())
}

/// [`snake_case`] with the Strobe and GL prefixes and Material Maker's node
/// prefixes dropped, so the converter's `strobe_gl_FragColor` reads as
/// `frag_color` and `p_o4511_scale` as `scale`.
pub fn readable(name: &str) -> String {
    let mut text = snake_case(name);
    loop {
        let stripped = NOISE_PREFIXES
            .iter()
            .find_map(|prefix| text.strip_prefix(prefix).filter(|rest| !rest.is_empty()))
            .or_else(|| strip_node_prefix(&text));
        match stripped {
            Some(rest) => text = rest.to_string(),
            None => return text,
        }
    }
}

/// Whether `name` is a GLSL keyword, type or built-in function.
pub fn is_glsl_word(name: &str) -> bool {
    GLSL_WORDS.split_whitespace().any(|word| word == name)
}

/// Every name a new control must avoid.
#[derive(Debug, Clone, Default)]
pub struct Taken {
    /// Existing controls, in [`normalize`]d form.
    controls: BTreeSet<String>,
    /// Exact identifiers already in use anywhere in the scene's code, the
    /// SSF library, or GLSL itself.
    idents: BTreeSet<String>,
}

impl Taken {
    /// What `folder` and its `sources` already use.
    pub fn of(folder: &SceneFolder, sources: &Sources) -> Self {
        let mut taken = Self::default();
        for control in &folder.def.controls {
            taken.controls.insert(normalize(&control.name));
        }
        for transition in &folder.def.transition_uniforms {
            taken.controls.insert(normalize(&transition.name));
        }
        for pass in &folder.def.passes {
            if let Some(target) = &pass.target {
                taken.idents.insert(target.clone());
            }
        }
        for image in &folder.def.images {
            taken.idents.insert(image.name.clone());
        }
        taken
            .idents
            .extend(glsl::identifiers(strobe_ssf_lib::LIBRARY));
        for (_, text) in sources.iter() {
            taken.idents.extend(glsl::identifiers(text));
        }
        // A uniform the script creates is declared in every pass too.
        if let Some(script) = folder.script.as_deref() {
            for found in crate::script::scan(script) {
                match found.name {
                    crate::script::UniformName::Exact(name) => {
                        taken.idents.insert(name);
                    }
                    crate::script::UniformName::Prefix(_) => {}
                }
            }
        }
        taken
            .idents
            .extend(GLSL_WORDS.split_whitespace().map(str::to_string));
        taken
    }

    /// Whether `name` may be declared as a new control.
    pub fn is_free(&self, name: &str) -> bool {
        is_glsl_identifier(name)
            && !is_reserved(name)
            && !name.starts_with("st_")
            && !name.starts_with("syn_")
            && !self.controls.contains(&normalize(name))
            && !self.idents.contains(name)
    }

    /// Records `name` as used.
    pub fn claim(&mut self, name: &str) {
        self.controls.insert(normalize(name));
        self.idents.insert(name.to_string());
    }
}

/// Endings that already say what a control is, so a taken name is numbered
/// rather than given `_amount`.
const KIND_WORDS: [&str; 11] = [
    "color", "col", "colour", "speed", "scale", "freq", "steps", "zoom", "octaves", "amount",
    "tint",
];

/// A base name made legal: letters, digits and `_`, not starting with a
/// digit, at least three characters.
fn sanitize(base: &str) -> String {
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        name.insert_str(0, "ctl_");
    }
    if name.chars().count() < 3 {
        name.push_str("_amount");
    }
    name
}

/// Final, unique names for `bases`, in order, claimed in `taken`.
///
/// A base wanted once keeps its plain name (`zoom`); a base wanted by
/// several controls is numbered from 1 (`color_1`, `color_2`). A name that
/// is not free gets the next free number after `_1` instead.
pub fn assign(bases: &[String], taken: &mut Taken) -> Vec<String> {
    let plain: Vec<(String, Option<String>)> =
        bases.iter().map(|base| (base.clone(), None)).collect();
    assign_with_roles(&plain, taken)
}

/// [`assign`], where a base wanted by several controls is told apart by
/// each one's role before falling back to a number (D259): two colours
/// mixed by a ramp become `ramp_color_low` and `ramp_color_high`, the two
/// speeds of an orbit `orbit_speed_x` and `orbit_speed_y`. A role is used
/// only when every control sharing the base has one and no two of them
/// share it.
pub fn assign_with_roles(bases: &[(String, Option<String>)], taken: &mut Taken) -> Vec<String> {
    let cleaned: Vec<String> = bases.iter().map(|(base, _)| sanitize(base)).collect();
    let cleaned: Vec<String> = cleaned
        .iter()
        .enumerate()
        .map(|(index, base)| {
            let sharing: Vec<usize> = (0..cleaned.len())
                .filter(|other| cleaned.get(*other) == Some(base))
                .collect();
            if sharing.len() < 2 {
                return base.clone();
            }
            let roles: Vec<Option<&String>> = sharing
                .iter()
                .map(|other| bases.get(*other).and_then(|(_, role)| role.as_ref()))
                .collect();
            let distinct = roles.iter().all(Option::is_some)
                && roles
                    .iter()
                    .enumerate()
                    .all(|(a, role)| roles.iter().skip(a + 1).all(|other| other != role));
            match bases.get(index).and_then(|(_, role)| role.as_ref()) {
                Some(role) if distinct => format!("{base}_{role}"),
                Some(_) | None => base.clone(),
            }
        })
        .collect();
    let mut names = Vec::with_capacity(cleaned.len());
    for (index, base) in cleaned.iter().enumerate() {
        let shared = cleaned.iter().filter(|other| *other == base).count() > 1;
        let earlier = cleaned
            .iter()
            .take(index)
            .filter(|other| *other == base)
            .count();
        let mut first = if shared {
            format!("{base}_{}", earlier + 1)
        } else {
            base.clone()
        };
        // A plain name the scene already uses (`#define RINGS 9.0` beside a
        // variable `rings`) says what it is with `_amount` before it takes
        // a number (D259); a name that already ends in what it is (`zoom`,
        // `fog_color`) is numbered.
        if !shared && !taken.is_free(&first) && !KIND_WORDS.iter().any(|w| first.ends_with(w)) {
            let amount = format!("{first}_amount");
            if taken.is_free(&amount) {
                first = amount;
            }
        }
        let mut name = first.clone();
        let mut number = 2;
        while !taken.is_free(&name) {
            name = format!("{first}_{number}");
            number += 1;
            if number > 999 {
                name = format!("ctl_{index}");
                break;
            }
        }
        taken.claim(&name);
        names.push(name);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_of_any_style_become_snake_case() {
        for (input, expected) in [
            ("GLOW_STRENGTH", "glow_strength"),
            ("glowStrength", "glow_strength"),
            ("GlowStrength", "glow_strength"),
            ("RGBColor", "rgb_color"),
            ("K", "k"),
            ("_hidden_", "hidden"),
            ("MAX_STEPS2", "max_steps2"),
            ("a__b", "a_b"),
        ] {
            assert_eq!(snake_case(input), expected, "`{input}`");
        }
    }

    #[test]
    fn strobe_and_gl_prefixes_are_dropped_from_readable_names() {
        assert_eq!(
            readable("strobe_gl_FragColor"),
            "frag_color",
            "converter name"
        );
        assert_eq!(readable("gl_FragColor"), "frag_color", "legacy name");
        assert_eq!(readable("FogColor"), "fog_color", "no prefix");
        assert_eq!(readable("st_"), "st", "never empty");
    }

    #[test]
    fn short_and_numeric_names_are_made_legal() {
        assert_eq!(sanitize("k"), "k_amount", "too short to mean anything");
        assert_eq!(sanitize("2x"), "ctl_2x", "cannot start with a digit");
        assert_eq!(sanitize("a-b c"), "a_b_c", "illegal characters become _");
    }

    #[test]
    fn a_shared_base_is_numbered_and_a_single_one_is_not() {
        let mut taken = Taken::default();
        let names = assign(
            &["color".to_string(), "zoom".to_string(), "color".to_string()],
            &mut taken,
        );
        assert_eq!(names, vec!["color_1", "zoom", "color_2"], "numbering");
    }

    #[test]
    fn material_maker_node_prefixes_are_dropped() {
        for (input, expected) in [
            ("p_o12345_scale", "scale"),
            ("p_o7_color_a", "color_a"),
            ("o42_amount", "amount"),
            ("p_other", "p_other"),
            ("o_ring", "o_ring"),
            ("p_o12345_", "p_o12345"),
        ] {
            assert_eq!(readable(input), expected, "`{input}`");
        }
    }

    #[test]
    fn a_shared_base_is_told_apart_by_role_before_a_number() {
        let mut taken = Taken::default();
        let role = |base: &str, role: Option<&str>| (base.to_string(), role.map(str::to_string));
        let names = assign_with_roles(
            &[
                role("ramp_color", Some("low")),
                role("speed", Some("x")),
                role("ramp_color", Some("high")),
                role("speed", Some("y")),
                role("scale", Some("x")),
                role("scale", None),
                role("zoom", Some("x")),
            ],
            &mut taken,
        );
        assert_eq!(
            names,
            vec![
                "ramp_color_low",
                "speed_x",
                "ramp_color_high",
                "speed_y",
                "scale_1",
                "scale_2",
                "zoom"
            ],
            "roles when every sharer has a distinct one, numbers otherwise, nothing when unshared"
        );
    }

    #[test]
    fn a_taken_name_gets_a_number_instead() {
        let mut taken = Taken::default();
        taken.claim("zoom");
        let names = assign(&["zoom".to_string()], &mut taken);
        assert_eq!(names, vec!["zoom_2"], "zoom is in use");
        taken.claim("rings");
        let names = assign(&["rings".to_string()], &mut taken);
        assert_eq!(
            names,
            vec!["rings_amount"],
            "a taken plain name says what it is before taking a number"
        );
    }

    #[test]
    fn existing_controls_collide_by_the_script_api_rule() {
        let mut taken = Taken::default();
        taken.claim("Rate_In");
        assert!(!taken.is_free("ratein"), "case and underscores are ignored");
        assert!(taken.is_free("rate_out"), "a different name is free");
    }

    #[test]
    fn reserved_and_strobe_names_are_never_free() {
        let taken = Taken::default();
        for name in [
            "TIME",
            "syn_Level",
            "strobe_zoom",
            "st_zoom",
            "gl_x",
            "a__b",
            "1abc",
        ] {
            assert!(!taken.is_free(name), "`{name}` must not be offered");
        }
    }

    #[test]
    fn glsl_words_and_code_identifiers_are_avoided() -> anyhow::Result<()> {
        let mut taken = Taken::default();
        taken
            .idents
            .extend(GLSL_WORDS.split_whitespace().map(str::to_string));
        taken.idents.extend(glsl::identifiers("float zoom = 1.0;"));
        for name in ["step", "sample", "mix", "zoom"] {
            assert!(!taken.is_free(name), "`{name}` is taken");
        }
        Ok(())
    }
}
