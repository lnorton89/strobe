// ============================================================================
// Module:       strobe_scene::audio
// Description:  The names of the SSF audio uniforms: the contract between the
//               audio analyser, the shader preamble, and the JavaScript globals.
//
// Dependencies: std only
// ============================================================================

//! The audio uniform names.
//!
//! One list, read by three consumers that must agree: the assembler
//! declares each as a `uniform float`, the audio analyser produces a value
//! for each by index, and the script host exposes each as a JavaScript
//! global. A name added here reaches all three; a name spelled differently
//! in one of them would be a uniform that silently stays at zero.
//!
//! The four bands are Bass, Mid, MidHigh and High; each scalar family
//! (Level, Hits, Time, Presence) has an overall member and one per band.

/// Every scalar audio uniform, in the order the analyser fills them.
pub const SCALARS: &[&str] = &[
    // Levels: smoothed loudness, overall and per band, 0..1.
    "syn_Level",
    "syn_BassLevel",
    "syn_MidLevel",
    "syn_MidHighLevel",
    "syn_HighLevel",
    // Hits: transient energy, fast attack and decay.
    "syn_Hits",
    "syn_BassHits",
    "syn_MidHits",
    "syn_MidHighHits",
    "syn_HighHits",
    // Times: clocks that advance faster when their band is louder.
    "syn_Time",
    "syn_BassTime",
    "syn_MidTime",
    "syn_MidHighTime",
    "syn_HighTime",
    "syn_CurvedTime",
    // Presence: long-window level, for slow structural change.
    "syn_Presence",
    "syn_BassPresence",
    "syn_MidPresence",
    "syn_MidHighPresence",
    "syn_HighPresence",
    // Beat.
    "syn_OnBeat",
    "syn_ToggleOnBeat",
    "syn_RandomOnBeat",
    "syn_BeatTime",
    // Tempo.
    "syn_BPM",
    "syn_BPMConfidence",
    "syn_BPMTwitcher",
    "syn_BPMSin",
    "syn_BPMSin2",
    "syn_BPMSin4",
    "syn_BPMSin8",
    "syn_BPMTri",
    "syn_BPMTri2",
    "syn_BPMTri4",
    "syn_BPMTri8",
    // Long-form.
    "syn_FadeInOut",
    "syn_Intensity",
];

/// The value each scalar holds when no audio is running.
///
/// Zero for all of them except tempo, where zero would be nonsense a shader
/// might divide by: a silent room reads as 120 BPM, no confidence.
pub fn silent_value(name: &str) -> f32 {
    match name {
        "syn_BPM" => 120.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_scalar_is_named_once() {
        let unique: HashSet<&&str> = SCALARS.iter().collect();
        assert_eq!(unique.len(), SCALARS.len(), "a duplicated audio uniform");
    }

    #[test]
    fn every_scalar_is_a_syn_uniform() {
        for name in SCALARS {
            assert!(
                name.starts_with("syn_"),
                "`{name}` should carry the syn_ prefix"
            );
            assert!(
                crate::names::is_glsl_identifier(name),
                "`{name}` should be a valid identifier"
            );
        }
    }
}
