// ============================================================================
// Module:       strobe_app::show::beat
// Description:  Builds the playlist clock's BeatInfo from an audio frame's
//               scalars: tempo, confidence and the beat counter.
//
// Dependencies: strobe-scene (SCALARS); strobe-playlist (BeatInfo)
// ============================================================================

//! The beat the playlist clock hears.
//!
//! [`strobe_playlist::BeatInfo`] is three numbers. They come from three of
//! the audio scalars the shaders already read:
//!
//! | `BeatInfo` | Scalar | Notes |
//! |---|---|---|
//! | `bpm` | `syn_BPM` | 50 to 220 once the analyser has locked on |
//! | `confidence` | `syn_BPMConfidence` | 0 to 1; the clock trusts it above 0.5 |
//! | `beats` | `syn_BeatTime` | a counter whose integer part steps once per beat |
//! | `downbeat` | the frame's `downbeat` | used only at [`DOWNBEAT_CONFIDENCE`] or above |
//!
//! The live analyser's `syn_BeatTime` counts detected beats; the synthetic
//! track's is the beat position, `time * BPM / 60` (D330). Either way the
//! clock only needs a position whose integer part steps once per beat and
//! never runs backwards. A bar is
//! `floor((beats - downbeat) / 4)`: the analyser's downbeat estimate
//! (D233) says which count starts a bar, and while it is unsure (a kick on
//! every beat gives it nothing to go on) bars are counted from the
//! counter's zero, as before (D75).
//!
//! The scalar positions are looked up once ([`BeatReader::new`]); reading a
//! frame is three indexed loads and no string comparison, so it is fit for
//! the frame loop.

use strobe_playlist::BeatInfo;
use strobe_scene::audio::SCALARS;

/// The downbeat estimate's confidence from which bars start on it.
pub const DOWNBEAT_CONFIDENCE: f32 = 0.5;

/// Where the three scalars sit in an audio frame.
#[derive(Debug, Clone, Copy)]
pub struct BeatReader {
    bpm: Option<usize>,
    confidence: Option<usize>,
    beat_time: Option<usize>,
}

impl BeatReader {
    /// Finds the three scalars in [`SCALARS`].
    pub fn new() -> Self {
        let find = |name: &str| SCALARS.iter().position(|candidate| *candidate == name);
        Self {
            bpm: find("syn_BPM"),
            confidence: find("syn_BPMConfidence"),
            beat_time: find("syn_BeatTime"),
        }
    }

    /// The beat in `scalars` (an audio frame's, in [`SCALARS`] order), with
    /// the frame's `downbeat` (`[offset, confidence]`).
    ///
    /// A missing or non-finite scalar reads as silence for that part: no
    /// confidence means the clock falls back to its configured tempo and
    /// ends cues exactly on time, which is the safe direction to fail. An
    /// unsure or out-of-range downbeat counts bars from zero.
    pub fn read(&self, scalars: &[f32], downbeat: [f32; 2]) -> BeatInfo {
        let at = |index: Option<usize>| {
            index
                .and_then(|i| scalars.get(i))
                .copied()
                .filter(|value| value.is_finite())
        };
        let silent = BeatInfo::SILENT;
        BeatInfo {
            bpm: at(self.bpm).unwrap_or(silent.bpm),
            confidence: at(self.confidence).unwrap_or(silent.confidence),
            beats: at(self.beat_time).map_or(silent.beats, f64::from),
            downbeat: match downbeat {
                [offset, confidence]
                    if confidence >= DOWNBEAT_CONFIDENCE && (0.0..4.0).contains(&offset) =>
                {
                    f64::from(offset.round())
                }
                [_, _] => 0.0,
            },
        }
    }
}

impl Default for BeatReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    const NO_DOWNBEAT: [f32; 2] = [0.0, 0.0];

    fn synthetic(time: f32) -> Vec<f32> {
        let mut scalars = vec![0.0; SCALARS.len()];
        strobe_scene::synthetic::fill(time, &mut scalars);
        scalars
    }

    #[test]
    fn the_synthetic_track_reads_as_a_confident_120_bpm_grid() {
        let reader = BeatReader::new();
        let beat = reader.read(&synthetic(10.3), NO_DOWNBEAT);
        assert!((beat.bpm - 120.0).abs() < 1e-3, "bpm {}", beat.bpm);
        assert!(beat.confidence > 0.5, "confidence {}", beat.confidence);
        assert!(
            beat.confident_bpm(0.5).is_some(),
            "the clock should trust the synthetic track"
        );
    }

    #[test]
    fn the_beat_counter_steps_once_per_beat_and_a_bar_is_four_of_them() -> Result<()> {
        let reader = BeatReader::new();
        // 120 BPM: a beat every half second, a bar every two seconds.
        let at = |t: f32| reader.read(&synthetic(t), NO_DOWNBEAT);
        let first = at(4.05);
        let later = at(4.55);
        assert!(
            (later.beats.floor() - first.beats.floor() - 1.0).abs() < f64::EPSILON,
            "one beat later the counter's integer part is one higher: {} then {}",
            first.beats,
            later.beats
        );
        let mut bars = Vec::new();
        for step in 0..40 {
            bars.push(at(4.0 + step as f32 * 0.25).bar());
        }
        let first_bar = *bars.first().ok_or_else(|| anyhow!("no samples"))?;
        let last_bar = *bars.last().ok_or_else(|| anyhow!("no samples"))?;
        assert_eq!(
            last_bar - first_bar,
            4,
            "9.75 seconds at 120 BPM crosses four bar lines: {bars:?}"
        );
        assert!(
            bars.windows(2).all(|pair| pair[1] >= pair[0]),
            "the bar never runs backwards: {bars:?}"
        );
        Ok(())
    }

    #[test]
    fn a_short_or_poisoned_frame_reads_as_silence_not_as_garbage() {
        let reader = BeatReader::new();
        let empty = reader.read(&[], NO_DOWNBEAT);
        assert_eq!(empty, BeatInfo::SILENT, "no scalars at all");
        let mut scalars = synthetic(3.0);
        for index in [reader.bpm, reader.confidence, reader.beat_time]
            .into_iter()
            .flatten()
        {
            if let Some(slot) = scalars.get_mut(index) {
                *slot = f32::NAN;
            }
        }
        assert_eq!(
            reader.read(&scalars, NO_DOWNBEAT),
            BeatInfo::SILENT,
            "NaN scalars"
        );
    }

    #[test]
    fn a_sure_downbeat_moves_the_bar_lines_and_an_unsure_one_does_not() {
        let reader = BeatReader::new();
        let scalars = synthetic(10.3);
        let sure = reader.read(&scalars, [2.0, 0.8]);
        assert!((sure.downbeat - 2.0).abs() < f64::EPSILON, "sure: {sure:?}");
        for downbeat in [[2.0, 0.3], [7.0, 1.0], [f32::NAN, 1.0], [1.0, f32::NAN]] {
            assert!(
                reader.read(&scalars, downbeat).downbeat.abs() < f64::EPSILON,
                "{downbeat:?} counts bars from zero"
            );
        }
    }
}
