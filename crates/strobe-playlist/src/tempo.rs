// ============================================================================
// Module:       strobe_playlist::tempo
// Description:  What the playlist needs to know about the music: a beat
//               snapshot, when to trust its BPM, and which bar a beat
//               position falls in.
//
// Dependencies: None.
// ============================================================================

//! Tempo for beat-locked cues.
//!
//! The playlist crate does not depend on `strobe-audio`. The app builds a
//! [`BeatInfo`] each frame from the audio frame (`syn_BPM`,
//! `syn_BPMConfidence` and the beat-locked grid position) and passes it to
//! the clock. Keeping the type here means the scheduling logic is testable
//! with hand-made beats, and an audio-side rename cannot reach the file
//! format or the clock.

use crate::duration::BEATS_PER_BAR;

/// The BPM below which a reading is not music (matches `strobe-audio`).
pub const BPM_MIN: f64 = 50.0;

/// The BPM above which a reading is not music (matches `strobe-audio`).
pub const BPM_MAX: f64 = 220.0;

/// A snapshot of the audio analysis's tempo state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatInfo {
    /// The detected tempo, beats per minute (`syn_BPM`).
    pub bpm: f32,
    /// How much to believe it, 0..1 (`syn_BPMConfidence`).
    pub confidence: f32,
    /// The beat-locked grid position in beats since the grid started. The
    /// integer part steps once per beat and bar `n` begins when the value
    /// reaches `4n`. It only has to be continuous and increasing in normal
    /// running; a reset to a smaller value is tolerated.
    pub beats: f64,
}

impl BeatInfo {
    /// No music: 120 BPM, no confidence, the grid at zero. Cues then run
    /// by the fallback tempo and end exactly on time.
    pub const SILENT: Self = Self {
        bpm: 120.0,
        confidence: 0.0,
        beats: 0.0,
    };

    /// The tempo to use for beat-counted lengths: the live BPM when its
    /// confidence is above `threshold` and it is a plausible tempo,
    /// otherwise `fallback`.
    pub fn usable_bpm(&self, threshold: f32, fallback: f64) -> f64 {
        match self.confident_bpm(threshold) {
            Some(bpm) => bpm,
            None => fallback,
        }
    }

    /// The live BPM, if it is trustworthy.
    pub fn confident_bpm(&self, threshold: f32) -> Option<f64> {
        let bpm = f64::from(self.bpm);
        let sane = self.confidence.is_finite()
            && self.confidence > threshold
            && bpm.is_finite()
            && (BPM_MIN..=BPM_MAX).contains(&bpm)
            && self.beats.is_finite();
        sane.then_some(bpm)
    }

    /// The bar the grid position is in.
    pub fn bar(&self) -> i64 {
        bar_of(self.beats)
    }
}

impl Default for BeatInfo {
    fn default() -> Self {
        Self::SILENT
    }
}

/// The bar a beat position falls in (bar 0 is beats `0..4`).
pub fn bar_of(beats: f64) -> i64 {
    (beats / f64::from(BEATS_PER_BAR)).floor() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(bpm: f32, confidence: f32) -> BeatInfo {
        BeatInfo {
            bpm,
            confidence,
            beats: 10.0,
        }
    }

    #[test]
    fn live_bpm_is_used_only_above_the_threshold() {
        assert_eq!(beat(128.0, 0.9).usable_bpm(0.5, 120.0), 128.0, "confident");
        assert_eq!(
            beat(128.0, 0.5).usable_bpm(0.5, 120.0),
            120.0,
            "exactly at the threshold"
        );
        assert_eq!(
            beat(128.0, 0.1).usable_bpm(0.5, 120.0),
            120.0,
            "not confident"
        );
    }

    #[test]
    fn implausible_readings_fall_back() {
        for bpm in [0.0, 10.0, 400.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                beat(bpm, 1.0).usable_bpm(0.5, 100.0),
                100.0,
                "bpm {bpm} should fall back"
            );
        }
        assert_eq!(
            beat(120.0, f32::NAN).usable_bpm(0.5, 100.0),
            100.0,
            "NaN confidence"
        );
        let lost = BeatInfo {
            beats: f64::NAN,
            ..beat(120.0, 1.0)
        };
        assert_eq!(
            lost.usable_bpm(0.5, 100.0),
            100.0,
            "a lost grid is not trusted"
        );
    }

    #[test]
    fn bars_are_four_beats_wide() {
        assert_eq!(bar_of(0.0), 0, "start of bar 0");
        assert_eq!(bar_of(3.99), 0, "end of bar 0");
        assert_eq!(bar_of(4.0), 1, "downbeat of bar 1");
        assert_eq!(bar_of(-0.5), -1, "before the grid");
        assert_eq!(BeatInfo::SILENT.bar(), 0, "silent grid");
    }
}
