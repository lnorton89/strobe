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

/// [`snake_case`] with the Strobe and GL prefixes dropped, so the converter's
/// `strobe_gl_FragColor` reads as `frag_color`.
pub fn readable(name: &str) -> String {
    let mut text = snake_case(name);
    loop {
        let stripped = NOISE_PREFIXES
            .iter()
            .find_map(|prefix| text.strip_prefix(prefix).filter(|rest| !rest.is_empty()));
        match stripped {
            Some(rest) => text = rest.to_string(),
            None => return text,
        }
    }
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
    let cleaned: Vec<String> = bases.iter().map(|base| sanitize(base)).collect();
    let mut names = Vec::with_capacity(cleaned.len());
    for (index, base) in cleaned.iter().enumerate() {
        let shared = cleaned.iter().filter(|other| *other == base).count() > 1;
        let earlier = cleaned
            .iter()
            .take(index)
            .filter(|other| *other == base)
            .count();
        let first = if shared {
            format!("{base}_{}", earlier + 1)
        } else {
            base.clone()
        };
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
    fn a_taken_name_gets_a_number_instead() {
        let mut taken = Taken::default();
        taken.claim("zoom");
        let names = assign(&["zoom".to_string()], &mut taken);
        assert_eq!(names, vec!["zoom_2"], "zoom is in use");
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
