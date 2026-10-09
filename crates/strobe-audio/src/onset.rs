// ============================================================================
// Module:       strobe_audio::onset
// Description:  Peak-picking on the bass onset strength: the detector behind
//               syn_OnBeat and the beat counters.
//
// Dependencies: std only
// ============================================================================

//! Beat detection by peak-picking.
//!
//! The analyser already turns bass spectral flux into a 0..1 "hit"
//! strength per hop. A beat is a local maximum of that strength that clears
//! a threshold, with a refractory gap after the previous beat. The peak is
//! confirmed one hop late (a maximum can only be recognised once the next
//! sample has fallen), which costs about 11 ms and is accounted for in the
//! beat grid's latency constant.

/// Hit strength a peak must exceed to count as a beat.
///
/// Hits are normalised so the loudest recent transient is 1.0. A third of
/// that rejects ghost notes and bleed but keeps a kick that is a good deal
/// softer than the loudest one in the last few seconds.
const THRESHOLD: f32 = 0.35;

/// Local-maximum detector with a refractory period.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BeatPicker {
    /// The previous sample, the candidate peak.
    previous: f32,
    /// The sample before that.
    before: f32,
    /// Seconds since the last beat.
    since_beat: f32,
}

impl BeatPicker {
    /// A picker that may fire immediately.
    pub(crate) fn new() -> Self {
        Self {
            previous: 0.0,
            before: 0.0,
            since_beat: f32::MAX,
        }
    }

    /// Feeds the next strength sample `dt` seconds after the last. Returns
    /// true when the *previous* sample was a beat. `min_gap` is the
    /// refractory period in seconds.
    pub(crate) fn step(&mut self, strength: f32, dt: f32, min_gap: f32) -> bool {
        self.since_beat = (self.since_beat + dt).min(1.0e6);
        let is_peak = self.previous > THRESHOLD
            && self.previous > self.before
            && self.previous >= strength
            && self.since_beat >= min_gap;
        if is_peak {
            self.since_beat = 0.0;
        }
        self.before = self.previous;
        self.previous = strength;
        is_peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(samples: &[f32], gap: f32) -> usize {
        let mut picker = BeatPicker::new();
        samples
            .iter()
            .filter(|s| picker.step(**s, 0.01, gap))
            .count()
    }

    #[test]
    fn one_peak_is_one_beat() {
        assert_eq!(
            run(&[0.0, 0.4, 0.9, 0.5, 0.0, 0.0], 0.2),
            1,
            "a single hump"
        );
    }

    #[test]
    fn a_flat_top_is_still_one_beat() {
        assert_eq!(
            run(&[0.0, 0.6, 1.0, 1.0, 1.0, 0.4, 0.0], 0.0),
            1,
            "a clipped plateau must not fire once per sample"
        );
    }

    #[test]
    fn weak_peaks_are_ignored() {
        assert_eq!(run(&[0.0, 0.2, 0.3, 0.2, 0.0], 0.0), 0, "below threshold");
    }

    #[test]
    fn the_refractory_gap_suppresses_close_peaks() {
        let samples = [0.0, 0.9, 0.0, 0.9, 0.0, 0.0];
        assert_eq!(
            run(&samples, 0.5),
            1,
            "second peak is 20 ms after the first"
        );
        assert_eq!(run(&samples, 0.0), 2, "without a gap both fire");
    }
}
