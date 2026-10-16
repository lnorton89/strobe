// ============================================================================
// Module:       strobe_scene::names
// Description:  Name normalisation for the JavaScript API, GLSL identifier
//               checks, and the uniform names Strobe reserves.
//
// Dependencies: std; strobe-ssf-lib (the names its library uses)
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
//!
//! [`is_reserved`] refuses the names a scene cannot declare: what the
//! assembler and the SSF library already use, and GLSL's keywords.

use std::collections::HashSet;
use std::sync::LazyLock;

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
    // The preamble's object-like macros: a uniform by one of these names is
    // declared, then every mention of it is rewritten to something else.
    "texture2D",
    "texture2DLod",
];

/// The GLSL 3.30 core keywords that are not type names ([`is_type_name`]).
/// A keyword is never an identifier, on any driver and in either of
/// Strobe's dialects (GLSL ES 3.00 reserves the few it lacks), so a control
/// with one of these names cannot compile anywhere (render test
/// `every_reserved_glsl_word_fails_to_compile_as_a_control`).
///
/// Words GLSL only reserves for future use (`half`, `filter`, `input`,
/// `output`, `double`...) are not here. Mesa and ANGLE refuse them, but a
/// lenient driver may not, and a scene that plays there must still load.
pub const KEYWORDS: &[&str] = &[
    "attribute",
    "const",
    "uniform",
    "varying",
    "layout",
    "centroid",
    "flat",
    "smooth",
    "noperspective",
    "break",
    "continue",
    "do",
    "for",
    "while",
    "switch",
    "case",
    "default",
    "if",
    "else",
    "in",
    "out",
    "inout",
    "float",
    "int",
    "void",
    "bool",
    "true",
    "false",
    "invariant",
    "discard",
    "return",
    "uint",
    "lowp",
    "mediump",
    "highp",
    "precision",
    "struct",
];

/// Whether `name` is one of GLSL 3.30's vector, matrix or sampler type
/// keywords: `vec3`, `mat2x4`, `usampler2DArray`, `sampler2DShadow`.
pub fn is_type_name(name: &str) -> bool {
    const SIZES: &[&str] = &["2", "3", "4"];
    const MATRICES: &[&str] = &[
        "2", "3", "4", "2x2", "2x3", "2x4", "3x2", "3x3", "3x4", "4x2", "4x3", "4x4",
    ];
    const SAMPLERS: &[&str] = &[
        "1D",
        "2D",
        "3D",
        "Cube",
        "1DArray",
        "2DArray",
        "2DRect",
        "Buffer",
        "2DMS",
        "2DMSArray",
    ];
    const SHADOWS: &[&str] = &[
        "1DShadow",
        "2DShadow",
        "CubeShadow",
        "1DArrayShadow",
        "2DArrayShadow",
        "2DRectShadow",
    ];
    let after = |prefixes: &[&str], suffixes: &[&str]| {
        prefixes.iter().any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|rest| suffixes.contains(&rest))
        })
    };
    after(&["vec", "ivec", "bvec", "uvec"], SIZES)
        || after(&["mat"], MATRICES)
        || after(&["sampler", "isampler", "usampler"], SAMPLERS)
        || after(&["sampler"], SHADOWS)
}

/// Whether `name` is reserved: a uniform or global the assembler declares,
/// an audio uniform, a `strobe_` name, a GLSL keyword or type, or a name
/// the SSF library defines or calls. Any of these as a control, pass,
/// image or script uniform fails to compile with an error in code the
/// author never wrote, so the loader refuses it instead, by name.
pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
        || crate::audio::SCALARS.contains(&name)
        || name.starts_with("strobe_")
        || KEYWORDS.contains(&name)
        || is_type_name(name)
        || library_names().contains(name)
}

/// Every name the SSF library defines (a function, a `#define`, a `const`)
/// or calls (a built-in such as `texture` or `mix`): declared as a uniform
/// before the library, any of them breaks it. Read once from its source.
pub fn library_names() -> &'static HashSet<&'static str> {
    static NAMES: LazyLock<HashSet<&'static str>> =
        LazyLock::new(|| declared_or_called(strobe_ssf_lib::LIBRARY));
    &NAMES
}

/// The identifiers `source` calls or defines as functions, the names after
/// `#define` and the names `const` declares. Comments are skipped, so a word
/// before a parenthesis in prose is not taken for a call.
fn declared_or_called(source: &str) -> HashSet<&str> {
    let mut names = HashSet::new();
    let bytes = source.as_bytes();
    // The two words before this one, reset by any punctuation between.
    let mut before: [&str; 2] = ["", ""];
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let rest = source.get(i..).unwrap_or_default();
        if rest.starts_with("//") {
            i = rest.find('\n').map_or(bytes.len(), |n| i + n);
            continue;
        }
        if rest.starts_with("/*") {
            i = rest.find("*/").map_or(bytes.len(), |n| i + n + 2);
            continue;
        }
        if !(byte.is_ascii_alphabetic() || byte == b'_') {
            if !byte.is_ascii_whitespace() && byte != b'#' {
                before = ["", ""];
            }
            i += 1;
            continue;
        }
        let start = i;
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            i += 1;
        }
        let Some(word) = source.get(start..i) else {
            break;
        };
        let called = source
            .get(i..)
            .is_some_and(|after| after.trim_start().starts_with('('));
        if called || before[1] == "define" || before[0] == "const" {
            names.insert(word);
        }
        before = [before[1], word];
    }
    names
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

    #[test]
    fn names_that_can_never_compile_are_reserved() {
        // Keywords, and what the SSF library calls, defines or declares: as
        // a uniform they fail to compile in both dialects with an error in
        // generated code (render test
        // `every_reserved_glsl_word_fails_to_compile_as_a_control`).
        for name in [
            "smooth",
            "flat",
            "in",
            "default",
            "vec3",
            "mat3x4",
            "isampler2DArray",
            "texture",
            "mix",
            "fract",
            "_rotate",
            "_Media_Contrast",
            "syn_UserImage",
            "texture2D",
        ] {
            assert!(is_reserved(name), "`{name}` should be reserved");
        }
        // Words GLSL only reserves for the future may compile on a lenient
        // driver; a built-in the library never calls is the user's to shadow.
        for name in [
            "half", "filter", "output", "sample", "double", "distance", "sign", "noise", "color",
        ] {
            assert!(!is_reserved(name), "`{name}` should stay usable");
        }
    }
}
