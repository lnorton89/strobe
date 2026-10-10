// ============================================================================
// Module:       strobe_autocontrol::suggest
// Description:  Proposes audio-reactive mappings for the strongest generated
//               controls. Suggestions are data; nothing is applied by default.
//
// Dependencies: crate::candidate
// ============================================================================

//! Audio binding suggestions.
//!
//! A control that visibly changes the picture is a good thing to move with
//! the music, and the strongest few are worth proposing. The rules are
//! deliberately plain and follow what VJ scenes already do (the library's
//! `plasma-pulse` breathes its scale with the mids; the "Strobe" group's
//! `st_bass_flash` kicks brightness on bass hits):
//!
//! | Control | Band | Why |
//! |---|---|---|
//! | shape: scale, zoom, frequency | `syn_BassLevel` | size breathes with the kick |
//! | colour, brightness, glow | `syn_HighLevel` | shimmer follows the highs |
//! | speed | `syn_Level` | things move faster when it is loud |
//! | quality | none | a loop count must never jump around |
//!
//! ## Nothing is bound without being asked
//!
//! [`AudioSuggestion`] only describes a mapping. When the caller opts in
//! (`Options::bind_audio`), the rewrite *can* express a float or colour
//! mapping in SSF itself, with no engine support: the site reads
//! `(value * (1.0 + value_audio * syn_BassLevel))`, and `value_audio` is a
//! new slider whose default is 0, so the scene renders exactly as before
//! until the operator raises it. A speed control is an integrator (the
//! uniform is already `TIME * k` integrated by the host), so GLSL cannot
//! modulate it; its suggestion is data only, for the app to apply.

use crate::candidate::{Candidate, Group, ValueType};

/// How many controls get a suggestion.
pub const MAX_SUGGESTIONS: usize = 3;

/// A proposed reactive mapping.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AudioSuggestion {
    /// The control to move.
    pub control: String,
    /// The `syn_*` uniform to move it with.
    pub band: String,
    /// A starting depth, `0..1`: how much of its value the band adds.
    pub amount: f32,
    /// Why this pairing.
    pub reason: String,
    /// Whether the rewrite can express it in GLSL (a float or colour site)
    /// when binding is switched on. A speed or integer control cannot.
    pub expressible: bool,
}

/// The mapping for one candidate, if it has a sensible one.
fn mapping(candidate: &Candidate) -> Option<(&'static str, f32, &'static str)> {
    match (candidate.group, candidate.value_type) {
        (Group::Shape, ValueType::Float) => Some((
            "syn_BassLevel",
            0.3,
            "Size breathes with the kick: more bass, a larger value.",
        )),
        (Group::Colour, ValueType::Colour | ValueType::Float) => Some((
            "syn_HighLevel",
            0.4,
            "Brightness follows the highs, so the colour shimmers on hats.",
        )),
        (Group::Motion, ValueType::Float) => Some((
            "syn_Level",
            0.5,
            "Motion speeds up as the room gets louder.",
        )),
        (Group::Shape | Group::Colour | Group::Motion, ValueType::Int | ValueType::Count)
        | (
            Group::Quality,
            ValueType::Float | ValueType::Int | ValueType::Count | ValueType::Colour,
        )
        | (Group::Shape, ValueType::Colour)
        | (Group::Motion, ValueType::Colour) => None,
    }
}

/// Whether the rewrite can fold the mapping into the GLSL.
pub fn expressible(candidate: &Candidate) -> bool {
    !candidate.is_integrator()
        && matches!(candidate.value_type, ValueType::Float | ValueType::Colour)
}

/// Suggestions for the strongest controls.
///
/// `kept` is `(candidate, final name)` in rank order, strongest first. The
/// first [`MAX_SUGGESTIONS`] that have a mapping get one, and no two use the
/// same band while an unused band would do, so the three do not all pump
/// together.
pub fn suggest(kept: &[(&Candidate, &str)]) -> Vec<AudioSuggestion> {
    let mut out: Vec<AudioSuggestion> = Vec::new();
    for (candidate, name) in kept {
        if out.len() >= MAX_SUGGESTIONS {
            break;
        }
        let Some((band, amount, reason)) = mapping(candidate) else {
            continue;
        };
        if out.iter().any(|s| s.band == band) {
            continue;
        }
        out.push(AudioSuggestion {
            control: (*name).to_string(),
            band: band.to_string(),
            amount,
            reason: reason.to_string(),
            expressible: expressible(candidate),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::Kind;

    fn candidate(group: Group, value_type: ValueType, kind: Kind) -> Candidate {
        Candidate {
            id: 0,
            kind,
            group,
            value_type,
            default: vec![1.0],
            min: vec![0.0],
            max: vec![2.0],
            name: "x".to_string(),
            context: String::new(),
            sites: Vec::new(),
        }
    }

    #[test]
    fn shape_goes_to_bass_colour_to_highs_and_quality_to_nothing() {
        let shape = candidate(Group::Shape, ValueType::Float, Kind::Scale);
        let colour = candidate(Group::Colour, ValueType::Colour, Kind::Colour);
        let quality = candidate(Group::Quality, ValueType::Int, Kind::LoopBound);
        let got = suggest(&[(&quality, "steps"), (&shape, "zoom"), (&colour, "tint")]);
        let pairs: Vec<(&str, &str)> = got
            .iter()
            .map(|s| (s.control.as_str(), s.band.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![("zoom", "syn_BassLevel"), ("tint", "syn_HighLevel")],
            "quality is skipped, shape and colour are paired"
        );
        assert!(got.iter().all(|s| s.expressible), "both fold into GLSL");
    }

    #[test]
    fn a_speed_control_is_suggested_but_cannot_be_expressed() {
        let mut speed = candidate(Group::Motion, ValueType::Float, Kind::TimeScale);
        speed.sites.push(crate::candidate::Site {
            file: "main.glsl".to_string(),
            line: 1,
            column: 1,
            start: 0,
            end: 1,
            literal: "TIME * 1.0".to_string(),
            rewrite: crate::candidate::Rewrite::Integrator,
        });
        let got = suggest(&[(&speed, "speed")]);
        assert_eq!(got.len(), 1, "one suggestion");
        assert!(
            !got[0].expressible,
            "an integrator cannot be modulated in GLSL"
        );
    }

    #[test]
    fn no_more_than_three_and_no_repeated_band() {
        let shapes: Vec<Candidate> = (0..5)
            .map(|_| candidate(Group::Shape, ValueType::Float, Kind::Scale))
            .collect();
        let names = ["a1", "a2", "a3", "a4", "a5"];
        let kept: Vec<(&Candidate, &str)> = shapes.iter().zip(names).collect();
        let got = suggest(&kept);
        assert_eq!(
            got.len(),
            1,
            "five shapes share one band: only the strongest"
        );
    }
}
