// ============================================================================
// Module:       strobe_ssf_lib (library crate root)
// Description:  The SSF GLSL function library, embedded as source text for the
//               shader assembler to place ahead of every scene's code.
//
// Dependencies: None; the GLSL lives in ssf.glsl beside this file.
// ============================================================================

//! The Synesthesia Shader Format function library.
//!
//! Every documented SSF helper — `_scale`, `_noise`, `_fbm`, `_rotate`,
//! `_hsv2rgb`, `_palette`, `_loadMedia`, `_edgeDetect` and the rest —
//! re-implemented from its documented signature and behaviour. Exact
//! numerical output will differ from Synesthesia's own implementation;
//! the contract is behaviour (ranges, shapes, argument order), and the
//! renderer's golden-image tests lock Strobe's own output.
//!
//! ## One source, two dialects
//!
//! The same text must compile as desktop GLSL 3.30 core *and* as GLSL ES
//! 3.00, because the ANGLE fallback speaks ES. That rules out the
//! conveniences desktop GLSL allows and ES does not, and the file is
//! written to that discipline throughout:
//!
//! - every float literal has a decimal point (`1.0`, not `1`) — ES has no
//!   implicit int-to-float conversion;
//! - no built-in function is overloaded or redeclared;
//! - loop bounds are constant expressions, and nothing indexes an array
//!   with a non-constant where ES 3.00 would refuse it.
//!
//! The renderer's tests compile this library under both dialects on every
//! run, so a slip in either direction is a failing build.
//!
//! The library depends on uniforms the assembler declares before it
//! (`RENDERSIZE`, `syn_Media`, `syn_MediaType`, `_uv`, `_xy`), and every
//! private helper is prefixed `strobe_` so it cannot collide with a name a
//! scene defines.

/// The library's GLSL source.
pub const LIBRARY: &str = include_str!("ssf.glsl");

#[cfg(test)]
mod tests {
    use super::LIBRARY;

    /// Every documented SSF function name, which the library must define.
    const DOCUMENTED: &[&str] = &[
        "_scale",
        "_map",
        "_smin",
        "_rand",
        "_pulse",
        "_sqPulse",
        "_inRange",
        "_triWave",
        "_pixelate",
        "_mix3",
        "_mix4",
        "_mix5",
        "_nsin",
        "_ncos",
        "_nclamp",
        "_noise",
        "_fbm",
        "_statelessContinuousChaotic",
        "_hash11",
        "_hash12",
        "_hash13",
        "_hash21",
        "_hash22",
        "_hash23",
        "_hash31",
        "_hash32",
        "_hash33",
        "_hash41",
        "_hash42",
        "_hash43",
        "_hash44",
        "_uv2uvc",
        "_uvc2uv",
        "_rotate",
        "_mirror",
        "_toPolar",
        "_toPolarTrue",
        "_toRect",
        "_rgb2hsv",
        "_hsv2rgb",
        "_normalizeRGB",
        "_palette",
        "_brightness",
        "_gamma",
        "_contrast",
        "_saturation",
        "_hueRotate",
        "_invert",
        "_luminance",
        "_grayscale",
        "_dichrome",
        "_loadMedia",
        "_textureMedia",
        "_loadMediaAsMask",
        "_textureMediaAsMask",
        "_correctMediaCoords",
        "_isMediaActive",
        "_edgeDetectMedia",
        "_edgeDetectSobelMedia",
        "_edgeDetect",
        "_edgeDetectSobel",
        "_hBlurTiltShift",
        "_vBlurTiltShift",
    ];

    #[test]
    fn every_documented_function_is_defined() {
        let missing: Vec<&&str> = DOCUMENTED
            .iter()
            .filter(|name| !LIBRARY.contains(&format!(" {name}(")))
            .collect();
        assert!(
            missing.is_empty(),
            "the library does not define {missing:?}"
        );
    }

    #[test]
    fn no_float_literal_lacks_a_decimal_point_in_a_float_context() {
        // A cheap guard for the most common ES break: `* 2)` or `+ 1;` where
        // a float is meant. Integer contexts (loop counters, array sizes)
        // are written with `int` on the same line and are exempt.
        let offenders: Vec<(usize, &str)> = LIBRARY
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let code = line.split("//").next().unwrap_or_default();
                !code.contains("int ")
                    && !code.contains("for (")
                    && [
                        "* 2)", "+ 1)", "- 1)", "/ 2)", "* 2;", "+ 1;", "- 1;", "/ 2;",
                    ]
                    .iter()
                    .any(|pattern| code.contains(pattern))
            })
            .map(|(index, line)| (index + 1, line))
            .collect();
        assert!(
            offenders.is_empty(),
            "integer literal in a float context: {offenders:?}"
        );
    }
}
