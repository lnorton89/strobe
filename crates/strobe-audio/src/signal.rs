// ============================================================================
// Module:       strobe_audio::signal
// Description:  Resolves each SSF scalar name, once, to the analyser quantity
//               that feeds it.
//
// Dependencies: strobe-scene (SCALARS, silent_value)
// ============================================================================

//! The mapping from uniform name to analyser quantity.
//!
//! `strobe_scene::audio::SCALARS` is the contract: its order is the order of
//! [`crate::AudioFrame::scalars`]. The analyser resolves the list to
//! [`Signal`]s once at construction, so filling a frame is a loop over an
//! enum rather than 38 string comparisons, and a name the analyser does not
//! know becomes a constant at its silent value instead of a hole.

use strobe_scene::audio::{SCALARS, silent_value};

/// An analyser quantity that fills one scalar slot.
///
/// Channel indices are 0 for the whole spectrum, then 1 to 4 for Bass, Mid,
/// MidHigh and High.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Signal {
    /// Smoothed loudness of one channel.
    Level(usize),
    /// Transient energy of one channel.
    Hits(usize),
    /// Level-driven clock of one channel.
    Time(usize),
    /// The convex-response clock.
    CurvedTime,
    /// Long-window level of one channel.
    Presence(usize),
    /// Impulse on each beat.
    OnBeat,
    /// Flips on each beat, logistic transition.
    ToggleOnBeat,
    /// New random value on each beat, logistic transition.
    RandomOnBeat,
    /// Counter of detected beats.
    BeatTime,
    /// Tempo estimate.
    Bpm,
    /// How trustworthy the tempo is.
    BpmConfidence,
    /// Beat counter that jumps exponentially toward each next beat.
    BpmTwitcher,
    /// Sine locked to the beat grid; the payload is the period in beats.
    BpmSin(f32),
    /// Triangle locked to the beat grid; the payload is the period in beats.
    BpmTri(f32),
    /// Slow rise and fall around the presence of music.
    FadeInOut,
    /// Slow accumulation with the intensity of the music.
    Intensity,
    /// A name this analyser does not produce, held at its silent value.
    Constant(f32),
}

impl Signal {
    /// The quantity behind the uniform called `name`.
    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "syn_Level" => Self::Level(0),
            "syn_BassLevel" => Self::Level(1),
            "syn_MidLevel" => Self::Level(2),
            "syn_MidHighLevel" => Self::Level(3),
            "syn_HighLevel" => Self::Level(4),
            "syn_Hits" => Self::Hits(0),
            "syn_BassHits" => Self::Hits(1),
            "syn_MidHits" => Self::Hits(2),
            "syn_MidHighHits" => Self::Hits(3),
            "syn_HighHits" => Self::Hits(4),
            "syn_Time" => Self::Time(0),
            "syn_BassTime" => Self::Time(1),
            "syn_MidTime" => Self::Time(2),
            "syn_MidHighTime" => Self::Time(3),
            "syn_HighTime" => Self::Time(4),
            "syn_CurvedTime" => Self::CurvedTime,
            "syn_Presence" => Self::Presence(0),
            "syn_BassPresence" => Self::Presence(1),
            "syn_MidPresence" => Self::Presence(2),
            "syn_MidHighPresence" => Self::Presence(3),
            "syn_HighPresence" => Self::Presence(4),
            "syn_OnBeat" => Self::OnBeat,
            "syn_ToggleOnBeat" => Self::ToggleOnBeat,
            "syn_RandomOnBeat" => Self::RandomOnBeat,
            "syn_BeatTime" => Self::BeatTime,
            "syn_BPM" => Self::Bpm,
            "syn_BPMConfidence" => Self::BpmConfidence,
            "syn_BPMTwitcher" => Self::BpmTwitcher,
            "syn_BPMSin" => Self::BpmSin(1.0),
            "syn_BPMSin2" => Self::BpmSin(2.0),
            "syn_BPMSin4" => Self::BpmSin(4.0),
            "syn_BPMSin8" => Self::BpmSin(8.0),
            "syn_BPMTri" => Self::BpmTri(1.0),
            "syn_BPMTri2" => Self::BpmTri(2.0),
            "syn_BPMTri4" => Self::BpmTri(4.0),
            "syn_BPMTri8" => Self::BpmTri(8.0),
            "syn_FadeInOut" => Self::FadeInOut,
            "syn_Intensity" => Self::Intensity,
            other => Self::Constant(silent_value(other)),
        }
    }

    /// The signals for every name in [`SCALARS`], in order.
    pub(crate) fn all() -> Vec<Self> {
        SCALARS.iter().map(|name| Self::from_name(name)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contract_name_is_produced() {
        for (name, signal) in SCALARS.iter().zip(Signal::all()) {
            assert!(
                !matches!(signal, Signal::Constant(_)),
                "`{name}` is in SCALARS but the analyser does not produce it"
            );
        }
    }

    #[test]
    fn unknown_names_hold_their_silent_value() {
        assert_eq!(
            Signal::from_name("syn_Mystery"),
            Signal::Constant(0.0),
            "an unknown uniform is a constant, not a hole"
        );
    }
}
