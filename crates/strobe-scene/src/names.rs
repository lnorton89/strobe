// ============================================================================
// Module:       strobe_scene::names
// Description:  Name normalisation for the JavaScript API, GLSL identifier
//               checks, and the uniform names Strobe reserves.
//
// Dependencies: std only
// ============================================================================

//! Names.
//!
//! The Synesthesia JavaScript API matches control and uniform names
//! case-insensitively and ignores spacing characters, so `setControl("Rate
//! In", ..)`, `setControl("rate_in", ..)` and `setControl("RATEIN", ..)`
//! all reach the same control. [`normalize`] is that rule, and it is
//! applied at load time to every declared name so that two names a script
//! could not tell apart are a load error rather than a coin toss at run
//! time.

/// The key a name is matched by: lowercase, with whitespace, `_` and `-`
/// removed.
pub fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether `name` can be declared as a GLSL identifier.
///
/// ASCII letters, digits and `_`, not starting with a digit, and not using
/// the `gl_` prefix or a double underscore, which GLSL reserves.
pub fn is_glsl_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with("gl_")
        && !name.contains("__")
}

/// Uniform and global names the assembler declares for every scene.
///
/// A control or pass target with one of these names would redeclare it
/// and fail to compile with a message about line 40 of a file the author
/// never wrote, so the loader rejects it with this list instead.
pub const RESERVED: &[&str] = &[
    "RENDERSIZE",
    "TIME",
    "FRAMECOUNT",
    "PASSINDEX",
    "PI",
    "_xy",
    "_uv",
    "_uvc",
    "_mouse",
    "_click",
    "_muv",
    "_muvc",
    "syn_FinalPass",
    "syn_Media",
    "syn_MediaType",
    "syn_Spectrum",
    "syn_LevelTrail",
    "renderMain",
    "main",
];

/// Whether `name` is reserved, either outright or as an audio uniform.
pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name) || crate::audio::SCALARS.contains(&name) || name.starts_with("strobe_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_variants_normalise_to_one_key() {
        for variant in ["rate_in", "Rate In", "RATEIN", "rate-in", "  rate__In "] {
            assert_eq!(normalize(variant), "ratein", "`{variant}` should match");
        }
    }

    #[test]
    fn glsl_identifiers_are_checked() {
        for good in ["rate", "_private", "a1", "Rate_In"] {
            assert!(is_glsl_identifier(good), "`{good}` is a valid identifier");
        }
        for bad in ["", "1abc", "rate in", "gl_Position", "a__b", "rate-in", "é"] {
            assert!(
                !is_glsl_identifier(bad),
                "`{bad}` is not a valid identifier"
            );
        }
    }

    #[test]
    fn audio_and_standard_uniforms_are_reserved() {
        assert!(is_reserved("TIME"), "standard uniform");
        assert!(is_reserved("syn_BassLevel"), "audio uniform");
        assert!(is_reserved("strobe_anything"), "strobe_ prefix");
        assert!(!is_reserved("rate_in"), "an ordinary control name");
    }
}
